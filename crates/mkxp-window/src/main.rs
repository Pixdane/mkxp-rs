use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowAttributes};

use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use mkxp_graphics::{GraphicsState, ViewportScaleMode};

const DEFAULT_FPS: u32 = 60;
const GAME_W: u32 = 640;
const GAME_H: u32 = 480;

// ── public API for tests ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowScaleMark {
    Fit,
    Integer(u32),
}

/// Compute the largest 4:3 inner size that fits within (w, h).
pub fn fit_aspect_size(w: u32, h: u32) -> (u32, u32) {
    if (w as u64) * 3 > (h as u64) * 4 {
        (((h as f64 * 4.0 / 3.0).round()) as u32, h)
    } else {
        (w, ((w as f64 * 3.0 / 4.0).round()) as u32)
    }
}

/// Returns Some(WindowScaleMark) when (w, h) is recognisably 4:3.
pub fn window_scale_mark(w: u32, h: u32) -> Option<WindowScaleMark> {
    if w == 0 || h == 0 {
        return None;
    }

    if w.is_multiple_of(GAME_W) && h.is_multiple_of(GAME_H) && w / GAME_W == h / GAME_H {
        let n = w / GAME_W;
        if (1..=4).contains(&n) {
            return Some(WindowScaleMark::Integer(n));
        }
    }
    if fit_aspect_size(w, h) == (w, h) {
        return Some(WindowScaleMark::Fit);
    }
    None
}

fn integer_size(n: u32) -> (u32, u32) {
    (GAME_W * n, GAME_H * n)
}

fn main() {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop
        .run_app(&mut App::default())
        .expect("event loop error");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullscreenScaleMode {
    Fit,
    Integer(u32),
}

struct MenuItems {
    fit: CheckMenuItem,
    scale_1x: CheckMenuItem,
    scale_2x: CheckMenuItem,
    scale_3x: CheckMenuItem,
    scale_4x: CheckMenuItem,
    lock_aspect: CheckMenuItem,
}

struct App {
    window: Option<Window>,
    graphics: Option<Arc<Mutex<GraphicsState>>>,
    _menu: Option<Menu>,
    menu_receiver: Option<crossbeam_channel::Receiver<MenuEvent>>,

    aspect_locked: bool,
    resize_in_progress: bool,
    fullscreen_scale_mode: FullscreenScaleMode,
    modifiers: ModifiersState,

    mi_fit: Option<CheckMenuItem>,
    mi_scale_1x: Option<CheckMenuItem>,
    mi_scale_2x: Option<CheckMenuItem>,
    mi_scale_3x: Option<CheckMenuItem>,
    mi_scale_4x: Option<CheckMenuItem>,
    mi_lock_aspect: Option<CheckMenuItem>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            window: None,
            graphics: None,
            _menu: None,
            menu_receiver: None,
            aspect_locked: false,
            resize_in_progress: false,
            fullscreen_scale_mode: FullscreenScaleMode::Fit,
            modifiers: ModifiersState::default(),
            mi_fit: None,
            mi_scale_1x: None,
            mi_scale_2x: None,
            mi_scale_3x: None,
            mi_scale_4x: None,
            mi_lock_aspect: None,
        }
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

        let (menu, receiver, items) = Self::build_menu().expect("failed to create menu");
        self._menu = Some(menu);
        self.menu_receiver = Some(receiver);
        self.mi_fit = Some(items.fit);
        self.mi_scale_1x = Some(items.scale_1x);
        self.mi_scale_2x = Some(items.scale_2x);
        self.mi_scale_3x = Some(items.scale_3x);
        self.mi_scale_4x = Some(items.scale_4x);
        self.mi_lock_aspect = Some(items.lock_aspect);

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
            WindowEvent::Resized(size) => self.handle_resize(size.width, size.height),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => self.handle_key(key),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_menu_events(event_loop);
        let frame_start = Instant::now();
        let fps = if let Some(ref graphics) = self.graphics {
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

// ── private methods ──

impl App {
    fn build_menu() -> Result<(Menu, crossbeam_channel::Receiver<MenuEvent>, MenuItems), muda::Error>
    {
        let menu = Menu::new();

        let fit = CheckMenuItem::with_id("fit", "Fit", true, false, None);
        let scale_1x = CheckMenuItem::with_id("scale_1x", "1x (640\u{d7}480)", true, false, None);
        let scale_2x = CheckMenuItem::with_id("scale_2x", "2x (1280\u{d7}960)", true, false, None);
        let scale_3x = CheckMenuItem::with_id("scale_3x", "3x (1920\u{d7}1440)", true, false, None);
        let scale_4x = CheckMenuItem::with_id("scale_4x", "4x (2560\u{d7}1920)", true, false, None);
        let lock_aspect =
            CheckMenuItem::with_id("lock_aspect", "Lock Aspect Ratio", true, false, None);

        let view_menu = Submenu::new("View", true);
        view_menu.append_items(&[&fit, &scale_1x, &scale_2x, &scale_3x, &scale_4x])?;
        view_menu.append(&PredefinedMenuItem::separator())?;
        view_menu.append_items(&[&lock_aspect])?;

        let help_menu = Submenu::new("Help", true);
        help_menu.append(&MenuItem::with_id("about", "About mkxp-rs", true, None))?;

        menu.append(&view_menu)?;
        menu.append(&help_menu)?;

        let app_menu = Submenu::new("mkxp-rs", true);
        app_menu.append(&PredefinedMenuItem::quit(None))?;
        menu.insert(&app_menu, 0)?;

        #[cfg(target_os = "macos")]
        menu.init_for_nsapp();

        Ok((
            menu,
            MenuEvent::receiver().clone(),
            MenuItems {
                fit,
                scale_1x,
                scale_2x,
                scale_3x,
                scale_4x,
                lock_aspect,
            },
        ))
    }

    // ── resize ──

    fn request_single_resize(&mut self, size: (u32, u32)) {
        if self.resize_in_progress {
            return;
        }
        self.resize_in_progress = true;
        if let Some(ref window) = self.window {
            let _ = window.request_inner_size(PhysicalSize::new(size.0, size.1));
        }
    }

    fn handle_resize(&mut self, w: u32, h: u32) {
        self.resize_in_progress = false;

        if !self.is_fullscreen() && self.aspect_locked {
            let c = fit_aspect_size(w, h);
            if c != (w, h) {
                self.request_single_resize(c);
                self.refresh_menu_marks();
                return;
            }
        }

        if let Some(ref graphics) = self.graphics {
            graphics.lock().unwrap().on_resize(w, h);
        }
        self.refresh_menu_marks();
    }

    // ── keyboard ──

    fn handle_key(&mut self, key: KeyCode) {
        if self.modifiers.alt_key() && key == KeyCode::Enter {
            return self.toggle_fullscreen();
        }

        match key {
            KeyCode::KeyA => {
                self.aspect_locked = !self.aspect_locked;
                if self.aspect_locked {
                    self.request_windowed_fit();
                }
                self.refresh_menu_marks();
            }
            KeyCode::Digit0 | KeyCode::Numpad0 => {
                self.aspect_locked = false;
                self.fullscreen_scale_mode = FullscreenScaleMode::Fit;
                if let Some(ref graphics) = self.graphics {
                    graphics
                        .lock()
                        .unwrap()
                        .set_viewport_scale_mode(ViewportScaleMode::Fit);
                }
                self.refresh_menu_marks();
            }
            _ => {}
        }
    }

    // ── fullscreen toggle ──

    fn toggle_fullscreen(&mut self) {
        if let Some(ref window) = self.window {
            if self.is_fullscreen() {
                window.set_fullscreen(None);
                self.set_graphics_viewport_mode(ViewportScaleMode::Fit);
            } else {
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                self.apply_fullscreen_scale_mode();
            }
            self.refresh_menu_marks();
        }
    }

    // ── menu events ──

    fn poll_menu_events(&mut self, event_loop: &ActiveEventLoop) {
        let events: Vec<MenuEvent> = match &self.menu_receiver {
            Some(r) => {
                let mut v = Vec::new();
                while let Ok(e) = r.try_recv() {
                    v.push(e);
                }
                v
            }
            None => return,
        };
        for event in events {
            match event.id.0.as_str() {
                "fit" => self.menu_fit(),
                "scale_1x" => self.menu_integer_scale(1),
                "scale_2x" => self.menu_integer_scale(2),
                "scale_3x" => self.menu_integer_scale(3),
                "scale_4x" => self.menu_integer_scale(4),
                "lock_aspect" => {
                    self.aspect_locked = !self.aspect_locked;
                    if self.aspect_locked {
                        self.request_windowed_fit();
                    }
                    self.refresh_menu_marks();
                }
                "quit" => event_loop.exit(),
                _ => {}
            }
        }
    }

    fn menu_fit(&mut self) {
        if self.is_fullscreen() {
            self.fullscreen_scale_mode = FullscreenScaleMode::Fit;
            self.apply_fullscreen_scale_mode();
        } else if let Some(ref window) = self.window {
            let size = window.inner_size();
            let c = fit_aspect_size(size.width, size.height);
            if c != (size.width, size.height) {
                self.request_single_resize(c);
            }
        }
        self.refresh_menu_marks();
    }

    fn menu_integer_scale(&mut self, n: u32) {
        if self.is_fullscreen() {
            self.fullscreen_scale_mode = FullscreenScaleMode::Integer(n);
            self.apply_fullscreen_scale_mode();
        } else {
            self.request_single_resize(integer_size(n));
        }
        self.refresh_menu_marks();
    }

    fn request_windowed_fit(&mut self) {
        if let Some(ref window) = self.window {
            let size = window.inner_size();
            let c = fit_aspect_size(size.width, size.height);
            if c != (size.width, size.height) {
                self.request_single_resize(c);
            }
        }
    }

    // ── checkmark sync ──

    fn refresh_menu_marks(&self) {
        if self.is_fullscreen() {
            let (fit_checked, int_n) = match self.fullscreen_scale_mode {
                FullscreenScaleMode::Fit => (true, 0),
                FullscreenScaleMode::Integer(n) => (false, n),
            };
            self.set_checked(&self.mi_fit, fit_checked);
            self.set_checked(&self.mi_scale_1x, !fit_checked && int_n == 1);
            self.set_checked(&self.mi_scale_2x, !fit_checked && int_n == 2);
            self.set_checked(&self.mi_scale_3x, !fit_checked && int_n == 3);
            self.set_checked(&self.mi_scale_4x, !fit_checked && int_n == 4);
        } else {
            let inner = self.window.as_ref().map(|w| w.inner_size());
            match inner {
                None => {
                    self.set_checked(&self.mi_fit, false);
                    self.set_checked(&self.mi_scale_1x, false);
                    self.set_checked(&self.mi_scale_2x, false);
                    self.set_checked(&self.mi_scale_3x, false);
                    self.set_checked(&self.mi_scale_4x, false);
                }
                Some(size) => {
                    let mark = window_scale_mark(size.width, size.height);
                    let fit_checked = mark == Some(WindowScaleMark::Fit);
                    let int_n = match mark {
                        Some(WindowScaleMark::Integer(n)) => n,
                        _ => 0,
                    };
                    let int_checked = matches!(mark, Some(WindowScaleMark::Integer(_)));
                    self.set_checked(&self.mi_fit, fit_checked);
                    self.set_checked(&self.mi_scale_1x, int_checked && int_n == 1);
                    self.set_checked(&self.mi_scale_2x, int_checked && int_n == 2);
                    self.set_checked(&self.mi_scale_3x, int_checked && int_n == 3);
                    self.set_checked(&self.mi_scale_4x, int_checked && int_n == 4);
                }
            }
        }
        self.set_checked(&self.mi_lock_aspect, self.aspect_locked);
    }

    fn is_fullscreen(&self) -> bool {
        self.window.as_ref().and_then(Window::fullscreen).is_some()
    }

    fn apply_fullscreen_scale_mode(&self) {
        match self.fullscreen_scale_mode {
            FullscreenScaleMode::Fit => self.set_graphics_viewport_mode(ViewportScaleMode::Fit),
            FullscreenScaleMode::Integer(n) => {
                self.set_graphics_viewport_mode(ViewportScaleMode::Integer(n));
            }
        }
    }

    fn set_graphics_viewport_mode(&self, mode: ViewportScaleMode) {
        if let Some(ref graphics) = self.graphics {
            graphics.lock().unwrap().set_viewport_scale_mode(mode);
        }
    }

    fn set_checked(&self, item: &Option<CheckMenuItem>, checked: bool) {
        if let Some(item) = item {
            item.set_checked(checked);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_aspect_size_shrinks_width_for_wide_window() {
        assert_eq!(fit_aspect_size(1000, 700), (933, 700));
    }

    #[test]
    fn fit_aspect_size_shrinks_height_for_tall_window() {
        assert_eq!(fit_aspect_size(800, 700), (800, 600));
    }

    #[test]
    fn window_scale_mark_detects_integer_scale() {
        assert_eq!(
            window_scale_mark(1280, 960),
            Some(WindowScaleMark::Integer(2))
        );
    }

    #[test]
    fn window_scale_mark_detects_fit_when_aspect_matches_but_not_integer_scale() {
        assert_eq!(window_scale_mark(933, 700), Some(WindowScaleMark::Fit));
    }

    #[test]
    fn window_scale_mark_clears_when_aspect_does_not_match() {
        assert_eq!(window_scale_mark(1000, 700), None);
    }

    #[test]
    fn window_scale_mark_ignores_zero_size() {
        assert_eq!(window_scale_mark(0, 0), None);
        assert_eq!(window_scale_mark(640, 0), None);
        assert_eq!(window_scale_mark(0, 480), None);
    }
}
