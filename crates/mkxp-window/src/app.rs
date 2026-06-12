use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};

use mkxp_types::MkxpError;
use tracing::{debug, error, info};

use crate::error::{ScriptExit, WindowError};
use crate::render_host::{RenderCommand, RenderError, spawn_render_thread};
use crate::runtime::{RuntimeConfig, RuntimeEvent, SharedRuntime};
use crate::script_host::{ScriptEngine, spawn_script_thread};
use crate::window_control::{WindowConfig, WindowController, WindowOutput};

// ── App ──

/// winit application host.
///
/// `App` owns the platform lifecycle and all host-thread handles. The generic
/// script engine parameter is the selected script implementation for this run;
/// restart creates a fresh `E::default()` without rebuilding the window or
/// render host.
pub(crate) struct App<E: ScriptEngine> {
    _engine: PhantomData<E>,
    event_loop_proxy: EventLoopProxy<RuntimeEvent>,
    config: RuntimeConfig,
    runtime: Option<Arc<SharedRuntime>>,
    script_thread: Option<JoinHandle<()>>,
    render_thread: Option<JoinHandle<()>>,
    render_command_sender: Option<mpsc::Sender<RenderCommand>>,
    window: Option<WindowController>,
    fatal_error: Option<WindowError>,
}

impl<E: ScriptEngine> App<E> {
    pub(crate) fn new(
        event_loop_proxy: EventLoopProxy<RuntimeEvent>,
        config: RuntimeConfig,
    ) -> Self {
        Self {
            _engine: PhantomData,
            event_loop_proxy,
            config,
            runtime: None,
            script_thread: None,
            render_thread: None,
            render_command_sender: None,
            window: None,
            fatal_error: None,
        }
    }
}

// ── ApplicationHandler ──

impl<E: ScriptEngine> ApplicationHandler<RuntimeEvent> for App<E> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.try_resumed(event_loop) {
            error!(%error, "window runtime initialisation failed");
            self.fatal_error = Some(error);
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shutdown();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(window) = &mut self.window {
            let outputs = window.on_window_event(event);
            self.apply_window_outputs(event_loop, outputs);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RuntimeEvent) {
        match event {
            RuntimeEvent::ScriptExited => {
                self.handle_script_exit(event_loop);
            }
            RuntimeEvent::RenderExited => {
                self.handle_render_exit(event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &mut self.window {
            let outputs = window.on_about_to_wait();
            self.apply_window_outputs(event_loop, outputs);
        }
    }
}

// ── Initialization ──

impl<E: ScriptEngine> App<E> {
    fn try_resumed(&mut self, event_loop: &ActiveEventLoop) -> Result<(), WindowError> {
        if self.runtime.is_some() {
            debug!("winit resumed after runtime was already initialized");
            return Ok(());
        }

        info!("initializing window runtime");
        let window = WindowController::new(
            event_loop,
            WindowConfig {
                title: self.config.window_title.clone(),
                inner_size: self.config.window_size,
                game_size: self.config.game_size,
                enable_reset: self.config.enable_reset,
                ..Default::default()
            },
        )?;
        debug!("window controller initialized");

        debug!("creating wgpu instance");
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface: wgpu::Surface<'static> = unsafe {
            // Safety: wgpu ties the surface lifetime to the borrowed winit
            // window. This host stores the surface inside `SharedRuntime` and
            // the window inside `WindowController`, so the compiler cannot see
            // the relationship directly. `App::shutdown` is the invariant that
            // makes the widened lifetime sound: it requests shutdown, wakes and
            // joins the script/render threads, drops `SharedRuntime` (and thus
            // `GraphicsState`/surface), and only then drops `WindowController`.
            // No cloned surface is exposed outside that ownership chain.
            std::mem::transmute(instance.create_surface(window.window())?)
        };
        debug!("wgpu surface created");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .ok_or_else(|| MkxpError::Init("no suitable GPU adapter".into()))?;
        debug!("wgpu adapter selected");

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))?;
        debug!("wgpu device acquired");

        let surface_capabilities = surface.get_capabilities(&adapter);
        let surface_format = surface_capabilities
            .formats
            .first()
            .copied()
            .ok_or_else(|| MkxpError::Init("surface reported no supported formats".into()))?;
        let present_mode = select_present_mode(self.config.vsync, &surface_capabilities);
        let size = window.window().inner_size();
        debug!(
            width = size.width,
            height = size.height,
            format = ?surface_format,
            present_mode = ?present_mode,
            "wgpu surface configuration resolved"
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let runtime = Arc::new(SharedRuntime::with_config(
            device,
            queue,
            surface,
            surface_config,
            self.config.clone(),
        ));
        info!("shared runtime initialized");

        // Create render command channel.
        let (render_tx, render_rx) = mpsc::channel();
        debug!("render command channel created");

        // Spawn the script thread.
        info!("spawning script thread");
        let script_thread =
            spawn_script_thread(E::default(), runtime.clone(), self.event_loop_proxy.clone());

        // Spawn the render thread.
        info!("spawning render thread");
        let render_thread =
            spawn_render_thread(runtime.clone(), render_rx, self.event_loop_proxy.clone());

        self.render_command_sender = Some(render_tx);
        self.render_thread = Some(render_thread);
        self.runtime = Some(runtime);
        self.script_thread = Some(script_thread);
        self.window = Some(window);

        info!("window runtime initialized");
        Ok(())
    }
}

fn select_present_mode(vsync: bool, capabilities: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    if !vsync
        && capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
    {
        wgpu::PresentMode::Immediate
    } else {
        wgpu::PresentMode::Fifo
    }
}

// ── Event helpers ──

impl<E: ScriptEngine> App<E> {
    fn apply_window_outputs(&mut self, event_loop: &ActiveEventLoop, outputs: Vec<WindowOutput>) {
        for output in outputs {
            match output {
                WindowOutput::SurfaceResized { width, height } => {
                    self.send_render_command(
                        event_loop,
                        RenderCommand::SurfaceResized { width, height },
                    );
                }
                WindowOutput::ViewportScaleModeChanged(mode) => {
                    self.send_render_command(
                        event_loop,
                        RenderCommand::ViewportScaleModeChanged(mode),
                    );
                }
                WindowOutput::QuitRequested => {
                    self.initiate_shutdown();
                    event_loop.exit();
                }
                WindowOutput::RestartRequested => {
                    self.request_script_restart();
                }
            }
        }
    }

    fn send_render_command(&mut self, event_loop: &ActiveEventLoop, command: RenderCommand) {
        let Some(sender) = &self.render_command_sender else {
            return;
        };

        if sender.send(command).is_err() {
            self.fatal_error = Some(WindowError::Render(RenderError::Panic(
                "render command channel closed".into(),
            )));
            self.initiate_shutdown();
            event_loop.exit();
        }
    }

    fn handle_script_exit(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(runtime) = &self.runtime else {
            return false;
        };

        let Some(result) = runtime.take_script_result() else {
            return false;
        };

        match result {
            Ok(ScriptExit::Finished) => info!("script engine finished"),
            Ok(ScriptExit::ShutdownRequested) => info!("script engine stopped after shutdown"),
            Ok(ScriptExit::RestartRequested) => {
                info!("script engine restarting");
                self.restart_script_thread();
                return true;
            }
            Err(error) => {
                let error = WindowError::from(error);
                error!(%error, "script engine exited with error");
                self.fatal_error = Some(error);
            }
        }

        self.initiate_shutdown();
        event_loop.exit();
        true
    }

    fn request_script_restart(&mut self) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        if runtime.control.is_shutdown_requested() {
            return;
        }

        runtime.control.request_restart();
        runtime.frame_sync.reset();
        runtime.frame_sync.wake_all();
        info!("script restart requested");
    }

    fn restart_script_thread(&mut self) {
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        if runtime.control.is_shutdown_requested() {
            return;
        }

        if let Some(handle) = self.script_thread.take() {
            let _ = handle.join();
            debug!("old script thread joined before restart");
        }

        runtime.prepare_script_restart();
        info!("spawning replacement script thread");
        self.script_thread = Some(spawn_script_thread(
            E::default(),
            runtime,
            self.event_loop_proxy.clone(),
        ));
    }

    fn handle_render_exit(&mut self, event_loop: &ActiveEventLoop) {
        let Some(runtime) = &self.runtime else {
            return;
        };

        if let Some(error) = runtime.take_render_error() {
            error!(%error, "render thread exited with error");
            self.fatal_error = Some(WindowError::from(error));
        } else {
            info!("render thread exited");
        }

        self.initiate_shutdown();
        event_loop.exit();
    }

    fn initiate_shutdown(&mut self) {
        if let Some(runtime) = &self.runtime {
            runtime.control.request_shutdown();
            runtime.frame_sync.wake_all();
        }
        // Also send RenderCommand::Shutdown so the render thread exits even if
        // it is blocked in drain_commands rather than wait_for_ready_or_shutdown.
        if let Some(sender) = &self.render_command_sender {
            let _ = sender.send(RenderCommand::Shutdown);
        }
        debug!("shutdown requested");
    }

    fn shutdown(&mut self) {
        self.initiate_shutdown();

        // Join script thread first so script stops mutating GraphicsState.
        if let Some(handle) = self.script_thread.take() {
            let _ = handle.join();
            debug!("script thread joined");
        }

        // Join render thread so it stops using GraphicsState/surface.
        if let Some(handle) = self.render_thread.take() {
            let _ = handle.join();
            debug!("render thread joined");
        }

        self.render_command_sender.take();
        self.runtime.take();
        self.window.take();
        info!("window runtime shut down");
    }

    pub(crate) fn take_fatal_error(&mut self) -> Option<WindowError> {
        self.fatal_error.take()
    }
}

impl<E: ScriptEngine> Drop for App<E> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(present_modes: Vec<wgpu::PresentMode>) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb],
            present_modes,
            alpha_modes: vec![wgpu::CompositeAlphaMode::Auto],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        }
    }

    #[test]
    fn present_mode_uses_fifo_when_vsync_is_enabled() {
        let capabilities =
            capabilities(vec![wgpu::PresentMode::Immediate, wgpu::PresentMode::Fifo]);

        assert_eq!(
            select_present_mode(true, &capabilities),
            wgpu::PresentMode::Fifo
        );
    }

    #[test]
    fn present_mode_uses_immediate_when_vsync_is_disabled_and_supported() {
        let capabilities =
            capabilities(vec![wgpu::PresentMode::Immediate, wgpu::PresentMode::Fifo]);

        assert_eq!(
            select_present_mode(false, &capabilities),
            wgpu::PresentMode::Immediate
        );
    }

    #[test]
    fn present_mode_falls_back_to_fifo_without_immediate_support() {
        let capabilities = capabilities(vec![wgpu::PresentMode::Fifo]);

        assert_eq!(
            select_present_mode(false, &capabilities),
            wgpu::PresentMode::Fifo
        );
    }
}
