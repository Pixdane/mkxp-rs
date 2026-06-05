use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowAttributes;

use mkxp_graphics::GraphicsState;



fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop
        .run_app(&mut App::default())
        .expect("event loop error");
}

#[derive(Default)]
struct App {
    graphics: Option<Arc<Mutex<GraphicsState>>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("mkxp-rs test")
                    .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
            )
            .expect("failed to create window");

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance
            .create_surface(window)
            .expect("failed to create surface");

        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            }),
        )
        .expect("no suitable GPU adapter");

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .expect("failed to create GPU device");

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface.get_capabilities(&adapter).formats[0],
            width: 640,
            height: 480,
            #[cfg(target_os = "macos")]
            present_mode: wgpu::PresentMode::Immediate,
            #[cfg(not(target_os = "macos"))]
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let target_fps = 60;  // XP = 40, VX/Ace = 60
        let graphics = Arc::new(Mutex::new(GraphicsState::new(
            device, queue, surface, surface_config, 640, 480, target_fps,
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
                if x <= 0.0 || x + 200.0 >= 640.0 {
                    dx = -dx;
                }
                if y <= 0.0 || y + 150.0 >= 480.0 {
                    dy = -dy;
                }

                let r = (x / 640.0).clamp(0.0, 1.0);
                let g = (y / 480.0).clamp(0.0, 1.0);
                let b = 0.5;

                gfx.lock()
                    .unwrap()
                    .set_test_quad(x, y, 200.0, 150.0, r, g, b);
                thread::sleep(Duration::from_millis(16));
            }
        });

        self.graphics = Some(graphics);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(ref graphics) = self.graphics {
                    graphics.lock().unwrap().on_resize(size.width, size.height);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let frame_start = Instant::now();

        let fps = if let Some(ref graphics) = self.graphics {
            let mut g = graphics.lock().unwrap();
            let _ = g.update();
            g.target_fps()
        } else {
            60
        };

        let frame_duration = Duration::from_nanos(1_000_000_000 / fps as u64);

        // 从帧开始计时，渲染时间包含在帧预算内。
        // 超过预算时 max 取 Instant::now()，下一帧不等待。
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            (frame_start + frame_duration).max(Instant::now()),
        ));
    }
}
