use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes};

use mkxp_graphics::GraphicsState;

const DEFAULT_FPS: u32 = 60;
const GAME_W: u32 = 640;
const GAME_H: u32 = 480;

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop
        .run_app(&mut App::default())
        .expect("event loop error");
}

#[derive(Default)]
struct App {
    window: Option<Window>,
    graphics: Option<Arc<Mutex<GraphicsState>>>,

    scale_locked: bool,
    scale_factor: u32,
    aspect_locked: bool,

    /// 正在主动调整窗口尺寸，跳过 Resized 事件避免反弹。
    resizing: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("mkxp-rs — A:aspect S:scale +/-:zoom 0:reset")
                    .with_resizable(true)
                    .with_inner_size(PhysicalSize::new(640, 480)),
            )
            .expect("failed to create window");

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface: wgpu::Surface<'static> = unsafe {
            std::mem::transmute(
                instance
                    .create_surface(&window)
                    .expect("failed to create surface"),
            )
        };

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

        let graphics = Arc::new(Mutex::new(GraphicsState::new(
            device, queue, surface, surface_config, GAME_W, GAME_H, DEFAULT_FPS,
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

        self.scale_factor = 1;
        self.graphics = Some(graphics);
        self.window = Some(window);
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
                if self.resizing {
                    self.resizing = false;
                    return;
                }

                let (w, h) = (size.width, size.height);
                let constrained = self.constrain_size(w, h);

                if constrained != (w, h) {
                    self.resizing = true;
                    if let Some(ref window) = self.window {
                        let _ = window.request_inner_size(PhysicalSize::new(
                            constrained.0,
                            constrained.1,
                        ));
                    }
                }

                if let Some(ref graphics) = self.graphics {
                    graphics
                        .lock()
                        .unwrap()
                        .on_resize(constrained.0, constrained.1);
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match key {
                KeyCode::KeyA => {
                    self.aspect_locked = !self.aspect_locked;
                    self.scale_locked = false;
                    println!(
                        "aspect ratio lock: {}  scale lock: off",
                        self.aspect_locked
                    );
                    // 重新约束当前窗口
                    self.reapply_constraints();
                }
                KeyCode::KeyS => {
                    self.scale_locked = !self.scale_locked;
                    self.aspect_locked = false;
                    println!(
                        "integer scale lock: {} ({}×)  aspect lock: off",
                        self.scale_locked, self.scale_factor
                    );
                    self.reapply_constraints();
                }
                KeyCode::Equal | KeyCode::NumpadAdd => {
                    if self.scale_factor < 8 {
                        self.scale_factor += 1;
                    }
                    println!("scale: {}×", self.scale_factor);
                    if self.scale_locked {
                        self.reapply_constraints();
                    }
                }
                KeyCode::Minus | KeyCode::NumpadSubtract => {
                    if self.scale_factor > 1 {
                        self.scale_factor -= 1;
                    }
                    println!("scale: {}×", self.scale_factor);
                    if self.scale_locked {
                        self.reapply_constraints();
                    }
                }
                KeyCode::Digit0 | KeyCode::Numpad0 => {
                    self.aspect_locked = false;
                    self.scale_locked = false;
                    self.scale_factor = 1;
                    println!("reset all constraints");
                    self.reapply_constraints();
                }
                _ => {}
            },

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
            DEFAULT_FPS
        };

        let frame_duration = Duration::from_nanos(1_000_000_000 / fps as u64);
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            (frame_start + frame_duration).max(Instant::now()),
        ));
    }
}

impl App {
    fn constrain_size(&self, w: u32, h: u32) -> (u32, u32) {
        if self.scale_locked {
            let s = self.scale_factor.max(1);
            (s * GAME_W, s * GAME_H)
        } else if self.aspect_locked {
            let target = GAME_W as f32 / GAME_H as f32;
            let current = w as f32 / h as f32;
            if current > target {
                ((h as f32 * target) as u32, h)
            } else {
                (w, (w as f32 / target) as u32)
            }
        } else {
            (w, h)
        }
    }

    fn reapply_constraints(&self) {
        let window = match &self.window {
            Some(w) => w,
            None => return,
        };
        let size = window.inner_size();
        let constrained = self.constrain_size(size.width, size.height);
        if constrained != (size.width, size.height) {
            let _ = window.request_inner_size(PhysicalSize::new(constrained.0, constrained.1));
        } else if let Some(ref graphics) = self.graphics {
            graphics
                .lock()
                .unwrap()
                .on_resize(constrained.0, constrained.1);
        }
    }
}
