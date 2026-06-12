mod script_host;
mod window_control;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

use mkxp_graphics::GraphicsState;
use mkxp_types::MkxpError;
use tracing::{error, info};

use crate::script_host::{DemoScriptEngine, spawn_script_thread};
use crate::window_control::{
    GAME_H, GAME_W, WindowConfig, WindowController, WindowControllerError, WindowOutput,
};

const DEFAULT_FPS: u32 = 60;

#[derive(Debug, Clone, Copy)]
enum RuntimeEvent {
    ScriptFrameReady,
    ScriptExited,
}

/// Synchronizes one script-produced frame with one winit-thread render.
///
/// The script side sets `ready = true` at the `Graphics.update` boundary and
/// then blocks. The winit side renders exactly one frame, flips `ready` back to
/// false, and wakes the script so the next game update can begin.
#[derive(Default)]
struct FrameSync {
    ready: Mutex<bool>,
    cv: std::sync::Condvar,
}

impl FrameSync {
    fn script_frame_ready_and_wait(
        &self,
        shutdown: &AtomicBool,
        wake_event_loop: impl FnOnce(),
    ) -> bool {
        let mut ready = self.ready.lock().unwrap();
        *ready = true;
        // A Condvar can wake the script thread, but it cannot wake winit's
        // parked event loop on every platform. The caller also sends a winit
        // user event so a ready frame is noticed even when there is no input.
        wake_event_loop();
        self.cv.notify_one();

        while *ready && !shutdown.load(Ordering::Acquire) {
            ready = self.cv.wait(ready).unwrap();
        }

        !shutdown.load(Ordering::Acquire)
    }

    fn is_ready(&self) -> bool {
        *self.ready.lock().unwrap()
    }

    fn render_finished(&self) {
        let mut ready = self.ready.lock().unwrap();
        *ready = false;
        self.cv.notify_one();
    }

    fn wake_all(&self) {
        self.cv.notify_all();
    }
}

struct SharedRuntime {
    graphics: Mutex<GraphicsState>,
    frame_sync: FrameSync,
    script_outcome: ScriptOutcomeSlot,
    shutdown: AtomicBool,
}

type ScriptRunResult = Result<ScriptExit, ScriptError>;

#[derive(Debug, thiserror::Error)]
enum WindowError {
    #[error(transparent)]
    WindowController(#[from] WindowControllerError),
    #[error("failed to create wgpu surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("failed to request GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("script error: {0}")]
    Script(String),
    #[error("script thread panicked: {0}")]
    ScriptPanic(String),
    #[error(transparent)]
    Mkxp(#[from] MkxpError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptExit {
    Finished,
    ShutdownRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptError {
    #[allow(
        dead_code,
        reason = "real Ruby exceptions will construct this once mkxp-binding is wired"
    )]
    Message(String),
    Panic(String),
}

impl std::fmt::Display for ScriptExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finished => f.write_str("script finished"),
            Self::ShutdownRequested => f.write_str("script shutdown requested"),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Panic(message) => write!(f, "script thread panicked: {message}"),
        }
    }
}

impl From<ScriptError> for WindowError {
    fn from(error: ScriptError) -> Self {
        match error {
            ScriptError::Message(message) => Self::Script(message),
            ScriptError::Panic(message) => Self::ScriptPanic(message),
        }
    }
}

#[derive(Default)]
struct ScriptOutcomeSlot {
    result: Mutex<Option<ScriptRunResult>>,
}

impl ScriptOutcomeSlot {
    fn record(&self, result: ScriptRunResult) {
        *self.result.lock().unwrap() = Some(result);
    }

    fn take(&self) -> Option<ScriptRunResult> {
        self.result.lock().unwrap().take()
    }
}

impl SharedRuntime {
    fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
    ) -> Self {
        Self {
            graphics: Mutex::new(GraphicsState::new(
                device,
                queue,
                surface,
                surface_config,
                GAME_W,
                GAME_H,
                DEFAULT_FPS,
            )),
            frame_sync: FrameSync::default(),
            script_outcome: ScriptOutcomeSlot::default(),
            shutdown: AtomicBool::new(false),
        }
    }

    fn record_script_result(&self, result: ScriptRunResult) {
        if result.is_err() {
            self.shutdown.store(true, Ordering::Release);
            self.frame_sync.wake_all();
        }
        self.script_outcome.record(result);
    }

    fn take_script_result(&self) -> Option<ScriptRunResult> {
        self.script_outcome.take()
    }
}

fn main() -> anyhow::Result<()> {
    run()
}

fn run() -> anyhow::Result<()> {
    mkxp_log::init(mkxp_log::LogConfig::default())?;

    let event_loop = EventLoop::<RuntimeEvent>::with_user_event()
        .build()
        .map_err(|error| MkxpError::Init(format!("failed to create event loop: {error}")))?;
    let proxy = event_loop.create_proxy();
    let mut app = App::new(proxy);
    event_loop
        .run_app(&mut app)
        .map_err(|error| MkxpError::Runtime(format!("event loop error: {error}")))?;

    if let Some(error) = app.take_fatal_error() {
        Err(error.into())
    } else {
        Ok(())
    }
}

struct App {
    event_loop_proxy: EventLoopProxy<RuntimeEvent>,
    runtime: Option<Arc<SharedRuntime>>,
    script_thread: Option<JoinHandle<()>>,
    window: Option<WindowController>,
    fatal_error: Option<WindowError>,
    next_frame_at: Instant,
}

impl App {
    fn new(event_loop_proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        let now = Instant::now();
        Self {
            event_loop_proxy,
            runtime: None,
            script_thread: None,
            window: None,
            fatal_error: None,
            next_frame_at: now,
        }
    }
}

impl ApplicationHandler<RuntimeEvent> for App {
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
            RuntimeEvent::ScriptFrameReady => {
                // The user event wakes the OS event loop; it does not override
                // `target_fps`. `render_if_script_ready` still gates rendering
                // with `next_frame_at`, keeping script timing stable.
                self.render_if_script_ready(event_loop);
                self.schedule_next_wake(event_loop);
            }
            RuntimeEvent::ScriptExited => {
                self.handle_script_exit(event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &mut self.window {
            let outputs = window.on_about_to_wait();
            self.apply_window_outputs(event_loop, outputs);
        }

        self.render_if_script_ready(event_loop);
        self.schedule_next_wake(event_loop);
    }
}

impl App {
    fn try_resumed(&mut self, event_loop: &ActiveEventLoop) -> Result<(), WindowError> {
        if self.runtime.is_some() {
            return Ok(());
        }

        let window = WindowController::new(event_loop, WindowConfig::default())?;

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface: wgpu::Surface<'static> = unsafe {
            // Safety: `GraphicsState` is dropped before `WindowController` in
            // `shutdown`, so the surface never outlives the winit window it was
            // created from. The widened lifetime only mirrors that ownership.
            std::mem::transmute(instance.create_surface(window.window())?)
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .ok_or_else(|| MkxpError::Init("no suitable GPU adapter".into()))?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))?;

        let surface_format = surface
            .get_capabilities(&adapter)
            .formats
            .first()
            .copied()
            .ok_or_else(|| MkxpError::Init("surface reported no supported formats".into()))?;

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: GAME_W,
            height: GAME_H,
            // Keep presentation synchronized by default. Immediate presentation
            // looked responsive on macOS but can tear during integer-scaled
            // fullscreen exits; config-driven vsync can be wired later.
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let runtime = Arc::new(SharedRuntime::new(device, queue, surface, surface_config));
        let script_thread = spawn_script_thread(
            Box::new(DemoScriptEngine),
            runtime.clone(),
            self.event_loop_proxy.clone(),
        );

        self.runtime = Some(runtime);
        self.script_thread = Some(script_thread);
        self.window = Some(window);

        Ok(())
    }
}

impl App {
    fn current_fps(&self) -> u32 {
        self.runtime
            .as_ref()
            .map(|runtime| runtime.graphics.lock().unwrap().target_fps())
            .unwrap_or(DEFAULT_FPS)
    }

    fn frame_duration(&self) -> Duration {
        Duration::from_nanos(1_000_000_000 / self.current_fps() as u64)
    }

    fn schedule_next_wake(&self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let wake_at = if self.next_frame_at > now {
            self.next_frame_at
        } else {
            now + self.frame_duration()
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake_at));
    }

    fn render_if_script_ready(&mut self, event_loop: &ActiveEventLoop) {
        if self.handle_script_exit(event_loop) {
            return;
        }

        if let Some(runtime) = &self.runtime {
            // A script frame can become ready before the scheduled frame time.
            // Keep it blocked until the FPS gate opens instead of rendering
            // early and accidentally speeding up the game loop.
            if !runtime.frame_sync.is_ready() || Instant::now() < self.next_frame_at {
                return;
            }

            let mut g = runtime.graphics.lock().unwrap();
            let fps = g.target_fps();
            let _ = g.update();
            runtime.frame_sync.render_finished();
            self.next_frame_at = Instant::now() + Duration::from_nanos(1_000_000_000 / fps as u64);
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
            Err(error) => {
                let error = WindowError::from(error);
                error!(%error, "script engine exited with error");
                self.fatal_error = Some(error);
            }
        }

        runtime.shutdown.store(true, Ordering::Release);
        runtime.frame_sync.wake_all();
        event_loop.exit();
        true
    }

    fn take_fatal_error(&mut self) -> Option<WindowError> {
        self.fatal_error.take()
    }

    fn apply_window_outputs(&mut self, event_loop: &ActiveEventLoop, outputs: Vec<WindowOutput>) {
        for output in outputs {
            match output {
                WindowOutput::SurfaceResized { width, height } => {
                    if let Some(runtime) = &self.runtime {
                        runtime.graphics.lock().unwrap().on_resize(width, height);
                    }
                }
                WindowOutput::ViewportScaleModeChanged(mode) => {
                    if let Some(runtime) = &self.runtime {
                        runtime
                            .graphics
                            .lock()
                            .unwrap()
                            .set_viewport_scale_mode(mode);
                    }
                }
                WindowOutput::QuitRequested => event_loop.exit(),
            }
        }
    }

    fn shutdown(&mut self) {
        if let Some(runtime) = &self.runtime {
            runtime.shutdown.store(true, Ordering::Release);
            runtime.frame_sync.wake_all();
        }

        // Dropping a JoinHandle only detaches the thread. Join explicitly so the
        // script cannot keep using `GraphicsState` while the window/surface are
        // being torn down.
        if let Some(handle) = self.script_thread.take() {
            let _ = handle.join();
        }

        self.runtime.take();
        self.window.take();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn frame_sync_blocks_script_until_render_finishes() {
        let sync = Arc::new(FrameSync::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (wake_tx, wake_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let script_sync = sync.clone();
        let script_shutdown = shutdown.clone();
        let handle = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let keep_running = script_sync.script_frame_ready_and_wait(&script_shutdown, || {
                wake_tx.send(()).unwrap();
            });
            done_tx.send(keep_running).unwrap();
        });

        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        wake_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        while !sync.is_ready() {
            thread::yield_now();
        }
        assert!(done_rx.try_recv().is_err());

        sync.render_finished();
        assert!(done_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        handle.join().unwrap();
    }

    #[test]
    fn frame_sync_shutdown_releases_blocked_script() {
        let sync = Arc::new(FrameSync::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (wake_tx, wake_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let script_sync = sync.clone();
        let script_shutdown = shutdown.clone();
        let handle = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let keep_running = script_sync.script_frame_ready_and_wait(&script_shutdown, || {
                wake_tx.send(()).unwrap();
            });
            done_tx.send(keep_running).unwrap();
        });

        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        wake_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        while !sync.is_ready() {
            thread::yield_now();
        }

        shutdown.store(true, Ordering::Release);
        sync.wake_all();
        assert!(!done_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        handle.join().unwrap();
    }

    #[test]
    fn panic_payload_to_string_preserves_string_messages() {
        assert_eq!(
            panic_payload_to_string(Box::new("boom")),
            "boom".to_string()
        );
        assert_eq!(
            panic_payload_to_string(Box::new("kaboom".to_string())),
            "kaboom".to_string()
        );
    }

    #[test]
    fn shared_runtime_records_script_success_once() {
        let slot = ScriptOutcomeSlot::default();

        slot.record(Ok(ScriptExit::Finished));

        assert_eq!(slot.take(), Some(Ok(ScriptExit::Finished)));
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn script_outcome_slot_records_script_error() {
        let slot = ScriptOutcomeSlot::default();

        slot.record(Err(ScriptError::Message("ruby failed".to_string())));

        assert_eq!(
            slot.take(),
            Some(Err(ScriptError::Message("ruby failed".to_string())))
        );
    }

    #[test]
    fn script_panic_payload_is_stored_as_error_result() {
        let slot = ScriptOutcomeSlot::default();

        slot.record(Err(ScriptError::Panic(panic_payload_to_string(Box::new(
            "boom",
        )))));
        assert_eq!(
            slot.take(),
            Some(Err(ScriptError::Panic("boom".to_string())))
        );
    }

    #[test]
    fn window_error_displays_script_panic() {
        let err = WindowError::ScriptPanic("boom".to_string());

        assert_eq!(err.to_string(), "script thread panicked: boom");
    }

    #[test]
    fn window_error_transparently_forwards_mkxp_error() {
        let err = WindowError::from(mkxp_types::MkxpError::Runtime("bad state".to_string()));

        assert_eq!(err.to_string(), "runtime error: bad state");
    }

    #[test]
    fn script_error_converts_to_window_error() {
        let err = WindowError::from(ScriptError::Message("ruby failed".to_string()));

        assert_eq!(err.to_string(), "script error: ruby failed");
    }
}
