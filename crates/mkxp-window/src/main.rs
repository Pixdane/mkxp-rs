mod window_control;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};

use mkxp_graphics::GraphicsState;

use crate::window_control::{GAME_H, GAME_W, WindowConfig, WindowController, WindowOutput};

const DEFAULT_FPS: u32 = 60;

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop
        .run_app(&mut App::default())
        .expect("event loop error");
}

#[derive(Default)]
struct App {
    graphics: Option<Arc<Mutex<GraphicsState>>>,
    window: Option<WindowController>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }

        let window = WindowController::new(event_loop, WindowConfig::default())
            .expect("failed to create window controller");

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface: wgpu::Surface<'static> = unsafe {
            std::mem::transmute(
                instance
                    .create_surface(window.window())
                    .expect("failed to create surface"),
            )
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no suitable GPU adapter");

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("failed to create GPU device");

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface.get_capabilities(&adapter).formats[0],
            width: GAME_W,
            height: GAME_H,
            #[cfg(target_os = "macos")]
            present_mode: wgpu::PresentMode::Immediate,
            #[cfg(not(target_os = "macos"))]
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let graphics = Arc::new(Mutex::new(GraphicsState::new(
            device,
            queue,
            surface,
            surface_config,
            GAME_W,
            GAME_H,
            DEFAULT_FPS,
        )));

        let gfx = graphics.clone();
        thread::spawn(move || {
            let mut x = 220.0_f32;
            let mut y = 165.0_f32;
            let mut dx = 2.0_f32;
            let mut dy = 1.5_f32;
            loop {
                x += dx;
                y += dy;
                if x <= 0.0 || x + 200.0 >= GAME_W as f32 {
                    dx = -dx;
                }
                if y <= 0.0 || y + 150.0 >= GAME_H as f32 {
                    dy = -dy;
                }
                let r = (x / GAME_W as f32).clamp(0.0, 1.0);
                let g = (y / GAME_H as f32).clamp(0.0, 1.0);
                let b = 0.5;
                gfx.lock()
                    .unwrap()
                    .set_test_quad(x, y, 200.0, 150.0, r, g, b);
                thread::sleep(Duration::from_millis(16));
            }
        });

        self.graphics = Some(graphics);
        self.window = Some(window);
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &mut self.window {
            let outputs = window.on_about_to_wait();
            self.apply_window_outputs(event_loop, outputs);
        }

        let frame_start = Instant::now();
        let fps = if let Some(graphics) = &self.graphics {
            let mut g = graphics.lock().unwrap();
            let _ = g.update();
            g.target_fps()
        } else {
            DEFAULT_FPS
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            (frame_start + Duration::from_nanos(1_000_000_000 / fps as u64)).max(Instant::now()),
        ));
    }
}

impl App {
    fn apply_window_outputs(&mut self, event_loop: &ActiveEventLoop, outputs: Vec<WindowOutput>) {
        for output in outputs {
            match output {
                WindowOutput::SurfaceResized { width, height } => {
                    if let Some(graphics) = &self.graphics {
                        graphics.lock().unwrap().on_resize(width, height);
                    }
                }
                WindowOutput::ViewportScaleModeChanged(mode) => {
                    if let Some(graphics) = &self.graphics {
                        graphics.lock().unwrap().set_viewport_scale_mode(mode);
                    }
                }
                WindowOutput::QuitRequested => event_loop.exit(),
            }
        }
    }
}
