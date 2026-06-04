//! mkxp-window — winit 测试入口。
//!
//! 打开一个 640×480 的窗口，启动测试线程循环改变背景色，
//! 验证 winit ↔ wgpu ↔ GraphicsState 全链路。

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowAttributes;

use mkxp_graphics::GraphicsState;

use tracing_subscriber::EnvFilter;

fn main() {
    // 初始化日志：默认 info 级别，可通过 RUST_LOG 环境变量覆盖
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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
        if self.graphics.is_none() {
            let window = event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("mkxp-rs test (winit→wgpu→graphics)")
                        .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
                )
                .expect("failed to create window");

            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::PRIMARY,
                ..Default::default()
            });

            // surface 接管 window 的所有权，window 托管给 wgpu
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
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };

            let graphics = Arc::new(Mutex::new(GraphicsState::new(
                device, queue, surface, surface_config, 640, 480,
            )));

            // 测试线程：循环改变背景色
            let gfx = graphics.clone();
            thread::spawn(move || {
                let mut t = 0.0_f64;
                loop {
                    t = (t + 0.02) % 1.0;
                    let r = (0.6 + 0.4 * (t * std::f64::consts::TAU).cos()).clamp(0.0, 1.0);
                    let g =
                        (0.6 + 0.4 * ((t + 0.33) * std::f64::consts::TAU).cos()).clamp(0.0, 1.0);
                    let b =
                        (0.6 + 0.4 * ((t + 0.66) * std::f64::consts::TAU).cos()).clamp(0.0, 1.0);

                    gfx.lock().unwrap().set_debug_clear_color(r, g, b);
                    thread::sleep(Duration::from_millis(30));
                }
            });

            self.graphics = Some(graphics);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(ref graphics) = self.graphics {
                    graphics.lock().unwrap().on_resize(size.width, size.height);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(ref graphics) = self.graphics
            && let Err(e) = graphics.lock().unwrap().update() {
                eprintln!("graphics update error: {e:?}");
            }
    }
}
