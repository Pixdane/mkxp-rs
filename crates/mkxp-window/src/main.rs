use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes};

use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

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

struct App {
    window: Option<Window>,
    graphics: Option<Arc<Mutex<GraphicsState>>>,
    _menu: Option<Menu>,
    menu_receiver: Option<crossbeam_channel::Receiver<MenuEvent>>,

    scale_locked: bool,
    scale_factor: u32,
    aspect_locked: bool,
    resizing: bool,
    internal_resize: bool,

    /// 菜单项引用（用于更新 checkmark 状态）。
    mi_scale_1x: Option<CheckMenuItem>,
    mi_scale_2x: Option<CheckMenuItem>,
    mi_scale_3x: Option<CheckMenuItem>,
    mi_scale_4x: Option<CheckMenuItem>,
    mi_lock_aspect: Option<CheckMenuItem>,
    mi_lock_scale: Option<CheckMenuItem>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            window: None,
            graphics: None,
            _menu: None,
            menu_receiver: None,
            scale_locked: false,
            scale_factor: 1,
            aspect_locked: false,
            resizing: false,
            internal_resize: false,
            mi_scale_1x: None,
            mi_scale_2x: None,
            mi_scale_3x: None,
            mi_scale_4x: None,
            mi_lock_aspect: None,
            mi_lock_scale: None,
        }
    }
}

impl std::ops::Drop for App {
    fn drop(&mut self) {
        // Menu 需要显式 drop 来清理平台资源（macOS NSMenu）
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() {
            return;
        }

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title("mkxp-rs")
                    .with_resizable(true)
                    .with_inner_size(PhysicalSize::new(GAME_W, GAME_H)),
            )
            .expect("failed to create window");

        // ── 菜单栏 ──
        let (menu, menu_receiver, items) = Self::build_menu().expect("failed to create menu");

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
        self._menu = Some(menu);
        self.menu_receiver = Some(menu_receiver);
        self.mi_scale_1x = Some(items.scale_1x);
        self.mi_scale_2x = Some(items.scale_2x);
        self.mi_scale_3x = Some(items.scale_3x);
        self.mi_scale_4x = Some(items.scale_4x);
        self.mi_lock_aspect = Some(items.lock_aspect);
        self.mi_lock_scale = Some(items.lock_scale);
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
                // 自己触发的 resize，不清 checkmark
                if self.internal_resize {
                    self.internal_resize = false;
                    if let Some(ref graphics) = self.graphics {
                        graphics.lock().unwrap().on_resize(size.width, size.height);
                    }
                    return;
                }

                let (w, h) = (size.width, size.height);
                let constrained = self.constrain_size(w, h);

                if constrained != (w, h) {
                    // 窗口尺寸不符合约束（手动拖的），主动修正
        if let Some(ref window) = self.window {
                        let _ = window.request_inner_size(PhysicalSize::new(
                            constrained.0,
                            constrained.1,
                        ));
                    }
                    // 不渲染这帧，等修正后的 Resized 事件
                    return;
                }

                // 手动缩放且无约束 → 清除 scale checkmark
                if !self.scale_locked && !self.aspect_locked {
                    self.clear_scale_checkmark();
                }

                if let Some(ref graphics) = self.graphics {
                    graphics.lock().unwrap().on_resize(w, h);
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
                    self.update_lock_checkmarks();
                    self.clear_scale_checkmark();
                    self.reapply_constraints();
                }
                KeyCode::KeyS => {
                    self.scale_locked = !self.scale_locked;
                    self.aspect_locked = false;
                    self.update_lock_checkmarks();
                    self.clear_scale_checkmark();
                    self.reapply_constraints();
                }
                KeyCode::Equal | KeyCode::NumpadAdd => {
                    if self.scale_factor < 8 {
                        self.scale_factor += 1;
                    }
                    if self.scale_locked {
                        self.update_scale_checkmark(self.scale_factor);
                        self.reapply_constraints();
                    }
                }
                KeyCode::Minus | KeyCode::NumpadSubtract => {
                    if self.scale_factor > 1 {
                        self.scale_factor -= 1;
                    }
                    if self.scale_locked {
                        self.update_scale_checkmark(self.scale_factor);
                        self.reapply_constraints();
                    }
                }
                KeyCode::Digit0 | KeyCode::Numpad0 => {
                    self.aspect_locked = false;
                    self.scale_locked = false;
                    self.scale_factor = 1;
                    self.update_lock_checkmarks();
                    self.clear_scale_checkmark();
                    self.reapply_constraints();
                }
                _ => {}
            },

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 处理菜单事件
        self.poll_menu_events(event_loop);

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

struct MenuItems {
    scale_1x: CheckMenuItem,
    scale_2x: CheckMenuItem,
    scale_3x: CheckMenuItem,
    scale_4x: CheckMenuItem,
    lock_aspect: CheckMenuItem,
    lock_scale: CheckMenuItem,
}

impl App {
    fn build_menu() -> Result<(Menu, crossbeam_channel::Receiver<MenuEvent>, MenuItems), muda::Error> {
        let menu = Menu::new();

        // ── View ──
        let view_menu = Submenu::new("View", true);
        let scale_1x = CheckMenuItem::with_id("scale_1x", "1x (640\u{d7}480)", true, false, None);
        let scale_2x = CheckMenuItem::with_id("scale_2x", "2x (1280\u{d7}960)", true, false, None);
        let scale_3x = CheckMenuItem::with_id("scale_3x", "3x (1920\u{d7}1440)", true, false, None);
        let scale_4x = CheckMenuItem::with_id("scale_4x", "4x (2560\u{d7}1920)", true, false, None);
        let lock_aspect = CheckMenuItem::with_id("lock_aspect", "Lock Aspect Ratio", true, false, None);
        let lock_scale = CheckMenuItem::with_id("lock_scale", "Lock Integer Scale", true, false, None);
        view_menu.append_items(&[&scale_1x, &scale_2x, &scale_3x, &scale_4x])?;
        view_menu.append(&PredefinedMenuItem::separator())?;
        view_menu.append_items(&[&lock_aspect, &lock_scale])?;

        // ── Help ──
        let help_menu = Submenu::new("Help", true);
        let about = MenuItem::with_id("about", "About mkxp-rs", true, None);
        help_menu.append(&about)?;

        // ── 组装 ──
        menu.append(&view_menu)?;
        menu.append(&help_menu)?;

        // ── macOS 应用菜单（Quit 等）──
        let app_menu = Submenu::new("mkxp-rs", true);
        app_menu.append(&PredefinedMenuItem::quit(None))?;
        menu.insert(&app_menu, 0)?;

        #[cfg(target_os = "macos")]
        menu.init_for_nsapp();

        let items = MenuItems {
            scale_1x: scale_1x.clone(),
            scale_2x: scale_2x.clone(),
            scale_3x: scale_3x.clone(),
            scale_4x: scale_4x.clone(),
            lock_aspect: lock_aspect.clone(),
            lock_scale: lock_scale.clone(),
        };

        Ok((menu, MenuEvent::receiver().clone(), items))
    }

    fn poll_menu_events(&mut self, event_loop: &ActiveEventLoop) {
        let receiver = match &self.menu_receiver {
            Some(r) => r,
            None => return,
        };

        while let Ok(event) = receiver.try_recv() {
match event.id.0.as_str() {
                "scale_1x" => {
                    self.scale_locked = true;
                    self.aspect_locked = false;
                    self.scale_factor = 1;
                    self.reapply_constraints();
                }
                "scale_2x" => {
                    self.scale_locked = true;
                    self.aspect_locked = false;
                    self.scale_factor = 2;
                    self.reapply_constraints();
                }
                "scale_3x" => {
                    self.scale_locked = true;
                    self.aspect_locked = false;
                    self.scale_factor = 3;
                    self.reapply_constraints();
                }
                "scale_4x" => {
                    self.scale_locked = true;
                    self.aspect_locked = false;
                    self.scale_factor = 4;
                    self.reapply_constraints();
                }
                "lock_aspect" => {
                    self.aspect_locked = !self.aspect_locked;
                    self.scale_locked = false;
                    self.reapply_constraints();
                }
                "lock_scale" => {
                    self.scale_locked = !self.scale_locked;
                    self.aspect_locked = false;
                    self.reapply_constraints();
                }
                "quit" | "about" => {
                    // quit: Cmd+Q  
                    // about: placeholder
                    if event.id.0 == "quit" {
                        event_loop.exit();
                    }
                }
                _ => {}
            }
        }
    }

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

        fn update_scale_checkmark(&self, n: u32) {
        self.set_checked(&self.mi_scale_1x, n == 1);
        self.set_checked(&self.mi_scale_2x, n == 2);
        self.set_checked(&self.mi_scale_3x, n == 3);
        self.set_checked(&self.mi_scale_4x, n == 4);
    }

    fn clear_scale_checkmark(&self) {
        self.set_checked(&self.mi_scale_1x, false);
        self.set_checked(&self.mi_scale_2x, false);
        self.set_checked(&self.mi_scale_3x, false);
        self.set_checked(&self.mi_scale_4x, false);
    }

    fn update_lock_checkmarks(&self) {
        self.set_checked(&self.mi_lock_aspect, self.aspect_locked);
        self.set_checked(&self.mi_lock_scale, self.scale_locked);
    }

    fn set_checked(&self, item: &Option<CheckMenuItem>, checked: bool) {
        if let Some(item) = item {
            item.set_checked(checked);
        }
    }

    fn reapply_constraints(&self) {
        let Some(ref window) = self.window else { return };
        let size = window.inner_size();
        let constrained = self.constrain_size(size.width, size.height);
        if constrained != (size.width, size.height) {
            let _ = window.request_inner_size(PhysicalSize::new(
                constrained.0,
                constrained.1,
            ));
        } else if let Some(ref graphics) = self.graphics {
            graphics
                .lock()
                .unwrap()
                .on_resize(constrained.0, constrained.1);
        }
    }
}
