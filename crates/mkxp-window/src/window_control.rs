//! Window control boundary: owns the window, menu, resize policy, and fullscreen/scale state.
//!
//! `WindowController` exposes a clean event-based API so the runtime only needs to
//! apply [`WindowOutput`] events to [`GraphicsState`]. It does **not** own any wgpu
//! resources or know about rendering.
//!
//! ## Architecture
//!
//! ```text
//! WindowController
//!   owns: winit::Window, muda::Menu, receiver, menu items, policy state
//!   outputs: WindowOutput events for the runtime to apply
//!
//! Runtime (main)
//!   owns: WindowController, GraphicsState
//!   applies: WindowOutput → graphics.resize / graphics.set_viewport_scale_mode
//! ```
//!
//! See `docs/WINDOW_CONTROLLER_DESIGN.md` and `docs/WINDOW_CONSTRAINTS.md`.

use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowAttributes};

use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use mkxp_graphics::ViewportScaleMode;
use tracing::debug;

// ── Constants ──

/// Default game logical width.
pub(crate) const GAME_W: u32 = 640;
/// Default game logical height.
pub(crate) const GAME_H: u32 = 480;
const RESIZE_REQUEST_TIMEOUT: Duration = Duration::from_millis(100);

// ── Public configuration ──

/// Configuration used to create a [`WindowController`].
#[derive(Debug, Clone)]
pub(crate) struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Initial window inner size in physical pixels.
    pub inner_size: (u32, u32),
    /// Whether the window is resizable by the user.
    pub resizable: bool,
    /// Whether restart/reset commands are enabled.
    pub enable_reset: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "mkxp-rs".into(),
            inner_size: (GAME_W, GAME_H),
            resizable: true,
            enable_reset: true,
        }
    }
}

// ── Error type ──

/// Errors that can occur during [`WindowController`] creation.
#[derive(Debug)]
pub(crate) enum WindowControllerError {
    /// Failed to create the winit window.
    Window(winit::error::OsError),
    /// Failed to build the platform menu.
    Menu(muda::Error),
}

impl std::fmt::Display for WindowControllerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(e) => write!(f, "window creation failed: {e}"),
            Self::Menu(e) => write!(f, "menu creation failed: {e}"),
        }
    }
}

impl std::error::Error for WindowControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Window(e) => Some(e),
            Self::Menu(e) => Some(e),
        }
    }
}

// ── Output events ──

/// Events emitted by [`WindowController`] for the runtime to handle.
///
/// The runtime is expected to forward these to the appropriate subsystem
/// (e.g. `GraphicsState::on_resize` for `SurfaceResized`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowOutput {
    /// The window surface was resized. The runtime must reconfigure the wgpu surface.
    /// Size is the *actual* window size, which may be off-ratio.
    SurfaceResized {
        /// Actual window inner width.
        width: u32,
        /// Actual window inner height.
        height: u32,
    },
    /// The viewport scale mode should change (only meaningful in fullscreen).
    ViewportScaleModeChanged(ViewportScaleMode),
    /// The user requested the application to quit.
    QuitRequested,
    /// The user requested a script restart/reset.
    RestartRequested,
}

// ── Public pure-function API (testable without platform resources) ──

/// Outcome of classifying a resize event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeDecision {
    /// Off-ratio while aspect-locked; a correction request was (or should be) issued.
    /// Caller must still update the surface for the actual size.
    NeedsCorrection,
    /// Proceed normally — on-ratio, fullscreen, or not aspect-locked.
    Proceed,
}

/// Describes what integer scale the current window size represents, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowScaleMark {
    /// The window is exactly `n × GAME_W` by `n × GAME_H`.
    Integer(u32),
}

// ── Internal types ──

/// How a resize request was initiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeRequestMode {
    /// Automatic aspect-ratio correction; may be suppressed by a pending request.
    Coalesced,
    /// Explicit user command (menu, key); overrides pending Coalesced requests.
    Explicit,
}

/// A resize request that has been sent to the platform but not yet acknowledged.
#[derive(Debug, Clone, Copy)]
struct PendingResize {
    target: (u32, u32),
    requested_at: Instant,
}

/// Tracks pending programmatic resize requests to avoid request storms during live resize.
#[derive(Debug, Default)]
pub(crate) struct ResizeRequestTracker {
    pending: Option<PendingResize>,
}

impl ResizeRequestTracker {
    /// Whether a new request with the given mode can be sent.
    pub(crate) fn can_request(&self, mode: ResizeRequestMode, now: Instant) -> bool {
        match mode {
            ResizeRequestMode::Explicit => true,
            ResizeRequestMode::Coalesced => self
                .pending
                .map(|p| now.duration_since(p.requested_at) > RESIZE_REQUEST_TIMEOUT)
                .unwrap_or(true),
        }
    }

    /// Record that a resize target has been requested.
    pub(crate) fn mark_requested(&mut self, target: (u32, u32), now: Instant) {
        self.pending = Some(PendingResize {
            target,
            requested_at: now,
        });
    }

    /// Observe an incoming `Resized` event; clears the pending state if it matches.
    pub(crate) fn observe_resized(&mut self, size: (u32, u32)) {
        if self.pending.map(|p| p.target == size).unwrap_or(false) {
            self.pending = None;
        }
    }

    /// Returns `true` when there is a pending resize request that hasn't been
    /// fulfilled yet and hasn't timed out.
    #[cfg(test)]
    pub(crate) fn has_live_pending(&self, now: Instant) -> bool {
        self.pending
            .map(|p| now.duration_since(p.requested_at) <= RESIZE_REQUEST_TIMEOUT)
            .unwrap_or(false)
    }
}

// ── Fullscreen scale mode ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullscreenScaleMode {
    Fit,
    Integer(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowMode {
    Windowed,
    Fullscreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuMarks {
    fit: bool,
    scale_1x: bool,
    scale_2x: bool,
    scale_3x: bool,
    scale_4x: bool,
    lock_aspect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowModeSync {
    mode: WindowMode,
    output: Option<WindowOutput>,
}

// ── MenuItems ──

struct MenuItems {
    _restart: MenuItem,
    fit: CheckMenuItem,
    scale_1x: CheckMenuItem,
    scale_2x: CheckMenuItem,
    scale_3x: CheckMenuItem,
    scale_4x: CheckMenuItem,
    lock_aspect: CheckMenuItem,
}

// ── Pure helpers (tests can use these without platform) ──

/// Compute the largest 4:3 inner size that fits within `(w, h)`.
pub(crate) fn fit_aspect_size(w: u32, h: u32) -> (u32, u32) {
    if (w as u64) * 3 > (h as u64) * 4 {
        (((h as f64 * 4.0 / 3.0).round()) as u32, h)
    } else {
        (w, ((w as f64 * 3.0 / 4.0).round()) as u32)
    }
}

/// Returns `Some(WindowScaleMark)` when `(w, h)` exactly matches a windowed integer scale.
pub(crate) fn window_scale_mark(w: u32, h: u32) -> Option<WindowScaleMark> {
    if w == 0 || h == 0 {
        return None;
    }

    if w.is_multiple_of(GAME_W) && h.is_multiple_of(GAME_H) && w / GAME_W == h / GAME_H {
        let n = w / GAME_W;
        if (1..=4).contains(&n) {
            return Some(WindowScaleMark::Integer(n));
        }
    }
    None
}

/// Return the pixel size for `n` times the game resolution.
fn integer_size(n: u32) -> (u32, u32) {
    (GAME_W * n, GAME_H * n)
}

/// Classify a resize event: does it need aspect-ratio correction?
pub(crate) fn classify_resize(
    w: u32,
    h: u32,
    is_fullscreen: bool,
    aspect_locked: bool,
) -> ResizeDecision {
    if is_fullscreen || !aspect_locked {
        return ResizeDecision::Proceed;
    }
    let c = fit_aspect_size(w, h);
    if c != (w, h) {
        ResizeDecision::NeedsCorrection
    } else {
        ResizeDecision::Proceed
    }
}

/// Whether toggling aspect-lock should immediately request a windowed fit.
pub(crate) fn should_request_windowed_fit_after_aspect_toggle(
    aspect_locked: bool,
    is_fullscreen: bool,
) -> bool {
    aspect_locked && !is_fullscreen
}

fn menu_marks(
    window_mode: WindowMode,
    fullscreen_scale_mode: FullscreenScaleMode,
    aspect_locked: bool,
    inner_size: PhysicalSize<u32>,
) -> MenuMarks {
    let (fit, int_n) = match window_mode {
        WindowMode::Fullscreen => match fullscreen_scale_mode {
            FullscreenScaleMode::Fit => (true, 0),
            FullscreenScaleMode::Integer(n) => (false, n),
        },
        WindowMode::Windowed => {
            let int_n = match window_scale_mark(inner_size.width, inner_size.height) {
                Some(WindowScaleMark::Integer(n)) => n,
                _ => 0,
            };
            (false, int_n)
        }
    };

    MenuMarks {
        fit,
        scale_1x: !fit && int_n == 1,
        scale_2x: !fit && int_n == 2,
        scale_3x: !fit && int_n == 3,
        scale_4x: !fit && int_n == 4,
        lock_aspect: aspect_locked,
    }
}

fn viewport_mode_for_fullscreen_scale(mode: FullscreenScaleMode) -> ViewportScaleMode {
    match mode {
        FullscreenScaleMode::Fit => ViewportScaleMode::Fit,
        FullscreenScaleMode::Integer(n) => ViewportScaleMode::Integer(n),
    }
}

fn key_requests_restart(key: KeyCode) -> bool {
    key == KeyCode::F12
}

fn sync_window_mode(
    current_mode: WindowMode,
    platform_fullscreen: bool,
    fullscreen_scale_mode: FullscreenScaleMode,
) -> WindowModeSync {
    let platform_mode = if platform_fullscreen {
        WindowMode::Fullscreen
    } else {
        WindowMode::Windowed
    };

    if platform_mode == current_mode {
        return WindowModeSync {
            mode: current_mode,
            output: None,
        };
    }

    let output = match platform_mode {
        WindowMode::Fullscreen => Some(WindowOutput::ViewportScaleModeChanged(
            viewport_mode_for_fullscreen_scale(fullscreen_scale_mode),
        )),
        WindowMode::Windowed => Some(WindowOutput::ViewportScaleModeChanged(
            ViewportScaleMode::Fit,
        )),
    };

    WindowModeSync {
        mode: platform_mode,
        output,
    }
}

// ── WindowController ──

/// Owns the platform window, menu, and window-control policy.
///
/// Does **not** own wgpu resources or [`GraphicsState`](mkxp_graphics::GraphicsState).
/// The runtime accesses the window via [`WindowController::window`] to create a
/// wgpu surface, then forwards winit events to the controller and applies the
/// returned [`WindowOutput`] events.
pub(crate) struct WindowController {
    window: Window,
    _menu: Menu,
    menu_receiver: crossbeam_channel::Receiver<MenuEvent>,
    menu_items: MenuItems,

    window_mode: WindowMode,
    aspect_locked: bool,
    fullscreen_scale_mode: FullscreenScaleMode,
    resize_requests: ResizeRequestTracker,
    modifiers: ModifiersState,
    enable_reset: bool,
}

impl WindowController {
    /// Create a new window and menu, returning a controller.
    ///
    /// After this succeeds the runtime should create its wgpu surface via
    /// `instance.create_surface(controller.window())`.
    pub(crate) fn new(
        event_loop: &ActiveEventLoop,
        config: WindowConfig,
    ) -> Result<Self, WindowControllerError> {
        let initial_size = PhysicalSize::new(config.inner_size.0, config.inner_size.1);

        let window = event_loop
            .create_window(
                WindowAttributes::default()
                    .with_title(&config.title)
                    .with_resizable(config.resizable)
                    .with_inner_size(initial_size),
            )
            .map_err(WindowControllerError::Window)?;

        let (menu, receiver, menu_items) =
            Self::build_menu(config.enable_reset).map_err(WindowControllerError::Menu)?;
        debug!(
            title = %config.title,
            width = config.inner_size.0,
            height = config.inner_size.1,
            reset_enabled = config.enable_reset,
            "window controller created"
        );

        Ok(Self {
            window,
            _menu: menu,
            menu_receiver: receiver,
            menu_items,
            window_mode: WindowMode::Windowed,
            aspect_locked: false,
            fullscreen_scale_mode: FullscreenScaleMode::Fit,
            resize_requests: ResizeRequestTracker::default(),
            modifiers: ModifiersState::default(),
            enable_reset: config.enable_reset,
        })
    }

    /// Borrow the inner window (for surface creation, etc.).
    pub(crate) fn window(&self) -> &Window {
        &self.window
    }

    /// Handle a single [`WindowEvent`] from winit.
    ///
    /// Returns zero or more [`WindowOutput`] events that the runtime should apply.
    pub(crate) fn on_window_event(&mut self, event: WindowEvent) -> Vec<WindowOutput> {
        let mut outputs = self.sync_window_mode_from_platform();

        match event {
            WindowEvent::CloseRequested => outputs.push(WindowOutput::QuitRequested),
            WindowEvent::Resized(size) => {
                outputs.extend(self.handle_resized(size.width, size.height))
            }
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
            } => outputs.extend(self.handle_key(key)),
            _ => {}
        }

        outputs
    }

    /// Drain pending menu events and return any resulting [`WindowOutput`]s.
    pub(crate) fn poll_menu_events(&mut self) -> Vec<WindowOutput> {
        let mut outputs = Vec::new();
        while let Ok(event) = self.menu_receiver.try_recv() {
            match event.id.0.as_str() {
                "fit" => outputs.extend(self.handle_menu_fit()),
                "scale_1x" => outputs.extend(self.handle_menu_integer_scale(1)),
                "scale_2x" => outputs.extend(self.handle_menu_integer_scale(2)),
                "scale_3x" => outputs.extend(self.handle_menu_integer_scale(3)),
                "scale_4x" => outputs.extend(self.handle_menu_integer_scale(4)),
                "lock_aspect" => outputs.extend(self.handle_menu_lock_aspect()),
                "restart" if self.enable_reset => outputs.push(WindowOutput::RestartRequested),
                "quit" => outputs.push(WindowOutput::QuitRequested),
                _ => {}
            }
        }
        outputs
    }

    /// Called once per frame before rendering.
    ///
    /// Drains menu events and retries aspect-lock correction if the window is
    /// still off-ratio and the pending request has timed out.
    pub(crate) fn on_about_to_wait(&mut self) -> Vec<WindowOutput> {
        let mut outputs = self.sync_window_mode_from_platform();
        outputs.extend(self.poll_menu_events());

        // Retry aspect-lock correction if window is still off-ratio
        // and the pending request has timed out.
        if self.aspect_locked && !self.is_fullscreen() {
            let size = self.window.inner_size();
            let c = fit_aspect_size(size.width, size.height);
            if c != (size.width, size.height) {
                self.request_single_resize(c, ResizeRequestMode::Coalesced);
                self.refresh_menu_marks();
            }
        }

        outputs
    }
}

// ── Private helpers ──

impl WindowController {
    /// Build the platform menu structure.
    fn build_menu(
        enable_reset: bool,
    ) -> Result<(Menu, crossbeam_channel::Receiver<MenuEvent>, MenuItems), muda::Error> {
        let menu = Menu::new();

        let fit = CheckMenuItem::with_id("fit", "Fit", true, false, None);
        let scale_1x = CheckMenuItem::with_id("scale_1x", "1x (640\u{d7}480)", true, false, None);
        let scale_2x = CheckMenuItem::with_id("scale_2x", "2x (1280\u{d7}960)", true, false, None);
        let scale_3x = CheckMenuItem::with_id("scale_3x", "3x (1920\u{d7}1440)", true, false, None);
        let scale_4x = CheckMenuItem::with_id("scale_4x", "4x (2560\u{d7}1920)", true, false, None);
        let lock_aspect =
            CheckMenuItem::with_id("lock_aspect", "Lock Aspect Ratio", true, false, None);
        let restart = MenuItem::with_id("restart", "Restart", enable_reset, None);

        let view_menu = Submenu::new("View", true);
        view_menu.append_items(&[&fit, &scale_1x, &scale_2x, &scale_3x, &scale_4x])?;
        view_menu.append(&PredefinedMenuItem::separator())?;
        view_menu.append_items(&[&lock_aspect])?;

        let help_menu = Submenu::new("Help", true);
        help_menu.append(&MenuItem::with_id("about", "About mkxp-rs", true, None))?;

        let game_menu = Submenu::new("Game", true);
        game_menu.append(&restart)?;

        menu.append(&view_menu)?;
        menu.append(&game_menu)?;
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
                _restart: restart,
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

    fn sync_window_mode_from_platform(&mut self) -> Vec<WindowOutput> {
        let sync = sync_window_mode(
            self.window_mode,
            self.window.fullscreen().is_some(),
            self.fullscreen_scale_mode,
        );

        if sync.mode != self.window_mode {
            debug!(
                from = ?self.window_mode,
                to = ?sync.mode,
                output = ?sync.output,
                "window mode synchronized from platform"
            );
            self.window_mode = sync.mode;
            self.refresh_menu_marks();
        }

        sync.output.into_iter().collect()
    }

    fn request_single_resize(&mut self, size: (u32, u32), mode: ResizeRequestMode) {
        let now = Instant::now();
        if !self.resize_requests.can_request(mode, now) {
            return;
        }
        if self
            .window
            .request_inner_size(PhysicalSize::new(size.0, size.1))
            .is_none()
        {
            self.resize_requests.mark_requested(size, now);
        }
    }

    fn handle_resized(&mut self, w: u32, h: u32) -> Vec<WindowOutput> {
        self.resize_requests.observe_resized((w, h));

        let decision = classify_resize(w, h, self.is_fullscreen(), self.aspect_locked);
        if decision == ResizeDecision::NeedsCorrection {
            let c = fit_aspect_size(w, h);
            self.request_single_resize(c, ResizeRequestMode::Coalesced);
        }

        self.refresh_menu_marks();

        vec![WindowOutput::SurfaceResized {
            width: w,
            height: h,
        }]
    }

    // ── keyboard ──

    fn handle_key(&mut self, key: KeyCode) -> Vec<WindowOutput> {
        if self.modifiers.alt_key() && key == KeyCode::Enter {
            return self.toggle_fullscreen();
        }
        if self.enable_reset && key_requests_restart(key) {
            return vec![WindowOutput::RestartRequested];
        }

        Vec::new()
    }

    // ── fullscreen toggle ──

    fn toggle_fullscreen(&mut self) -> Vec<WindowOutput> {
        if self.is_fullscreen() {
            self.window.set_fullscreen(None);
        } else {
            self.window
                .set_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        self.sync_window_mode_from_platform()
    }

    // ── menu actions ──

    fn handle_menu_fit(&mut self) -> Vec<WindowOutput> {
        let outputs = if self.is_fullscreen() {
            self.fullscreen_scale_mode = FullscreenScaleMode::Fit;
            vec![WindowOutput::ViewportScaleModeChanged(
                ViewportScaleMode::Fit,
            )]
        } else {
            self.request_windowed_fit(ResizeRequestMode::Explicit);
            Vec::new()
        };
        self.refresh_menu_marks();
        outputs
    }

    fn handle_menu_integer_scale(&mut self, n: u32) -> Vec<WindowOutput> {
        let outputs = if self.is_fullscreen() {
            self.fullscreen_scale_mode = FullscreenScaleMode::Integer(n);
            vec![WindowOutput::ViewportScaleModeChanged(
                ViewportScaleMode::Integer(n),
            )]
        } else {
            self.request_single_resize(integer_size(n), ResizeRequestMode::Explicit);
            Vec::new()
        };
        self.refresh_menu_marks();
        outputs
    }

    fn handle_menu_lock_aspect(&mut self) -> Vec<WindowOutput> {
        self.aspect_locked = !self.aspect_locked;
        if should_request_windowed_fit_after_aspect_toggle(self.aspect_locked, self.is_fullscreen())
        {
            self.request_windowed_fit(ResizeRequestMode::Explicit);
        }
        self.refresh_menu_marks();
        Vec::new()
    }

    fn request_windowed_fit(&mut self, mode: ResizeRequestMode) {
        let size = self.window.inner_size();
        let c = fit_aspect_size(size.width, size.height);
        if c != (size.width, size.height) {
            self.request_single_resize(c, mode);
        }
    }

    // ── checkmark sync ──

    fn refresh_menu_marks(&self) {
        let marks = menu_marks(
            self.window_mode,
            self.fullscreen_scale_mode,
            self.aspect_locked,
            self.window.inner_size(),
        );

        self.menu_items.fit.set_checked(marks.fit);
        self.menu_items.scale_1x.set_checked(marks.scale_1x);
        self.menu_items.scale_2x.set_checked(marks.scale_2x);
        self.menu_items.scale_3x.set_checked(marks.scale_3x);
        self.menu_items.scale_4x.set_checked(marks.scale_4x);
        self.menu_items.lock_aspect.set_checked(marks.lock_aspect);
    }

    fn is_fullscreen(&self) -> bool {
        self.window_mode == WindowMode::Fullscreen
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── fit_aspect_size ──

    #[test]
    fn fit_aspect_size_shrinks_width_for_wide_window() {
        assert_eq!(fit_aspect_size(1000, 700), (933, 700));
    }

    #[test]
    fn fit_aspect_size_shrinks_height_for_tall_window() {
        assert_eq!(fit_aspect_size(800, 700), (800, 600));
    }

    // ── window_scale_mark ──

    #[test]
    fn window_scale_mark_detects_integer_scale() {
        assert_eq!(
            window_scale_mark(1280, 960),
            Some(WindowScaleMark::Integer(2))
        );
    }

    #[test]
    fn window_scale_mark_does_not_treat_fit_as_windowed_state() {
        assert_eq!(window_scale_mark(933, 700), None);
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

    // ── ResizeRequestTracker ──

    #[test]
    fn resize_request_tracker_blocks_repeated_requests_until_target_arrives() {
        let start = Instant::now();
        let mut tracker = ResizeRequestTracker::default();

        assert!(tracker.can_request(ResizeRequestMode::Coalesced, start));
        tracker.mark_requested((1067, 800), start);

        tracker.observe_resized((1100, 800));
        assert!(!tracker.can_request(
            ResizeRequestMode::Coalesced,
            start + Duration::from_millis(16)
        ));

        tracker.observe_resized((1067, 800));
        assert!(tracker.can_request(
            ResizeRequestMode::Coalesced,
            start + Duration::from_millis(32)
        ));
    }

    #[test]
    fn resize_request_tracker_allows_retry_after_timeout() {
        let start = Instant::now();
        let mut tracker = ResizeRequestTracker::default();

        tracker.mark_requested((1067, 800), start);

        assert!(!tracker.can_request(
            ResizeRequestMode::Coalesced,
            start + RESIZE_REQUEST_TIMEOUT / 2
        ));
        assert!(tracker.can_request(
            ResizeRequestMode::Coalesced,
            start + RESIZE_REQUEST_TIMEOUT + Duration::from_millis(1)
        ));
    }

    #[test]
    fn resize_request_tracker_allows_explicit_request_while_coalesced_request_is_pending() {
        let start = Instant::now();
        let mut tracker = ResizeRequestTracker::default();

        tracker.mark_requested((1067, 800), start);

        assert!(!tracker.can_request(
            ResizeRequestMode::Coalesced,
            start + Duration::from_millis(16)
        ));
        assert!(tracker.can_request(
            ResizeRequestMode::Explicit,
            start + Duration::from_millis(16)
        ));
    }

    #[test]
    fn resize_request_tracker_has_live_pending() {
        let start = Instant::now();
        let mut tracker = ResizeRequestTracker::default();

        assert!(!tracker.has_live_pending(start));
        tracker.mark_requested((1067, 800), start);
        assert!(tracker.has_live_pending(start));
        assert!(tracker.has_live_pending(start + RESIZE_REQUEST_TIMEOUT / 2));
        assert!(
            !tracker.has_live_pending(start + RESIZE_REQUEST_TIMEOUT + Duration::from_millis(1))
        );
    }

    // ── classify_resize ──

    #[test]
    fn classify_resize_needs_correction_when_aspect_locked_and_off_ratio() {
        assert_eq!(
            classify_resize(1100, 800, false, true),
            ResizeDecision::NeedsCorrection
        );
    }

    #[test]
    fn classify_resize_proceeds_when_aspect_locked_and_on_ratio() {
        assert_eq!(
            classify_resize(
                fit_aspect_size(1067, 800).0,
                fit_aspect_size(1067, 800).1,
                false,
                true
            ),
            ResizeDecision::Proceed
        );
    }

    #[test]
    fn classify_resize_proceeds_when_not_aspect_locked() {
        assert_eq!(
            classify_resize(1100, 800, false, false),
            ResizeDecision::Proceed
        );
    }

    #[test]
    fn classify_resize_proceeds_when_fullscreen() {
        assert_eq!(
            classify_resize(1100, 800, true, true),
            ResizeDecision::Proceed
        );
    }

    // ── should_request_windowed_fit_after_aspect_toggle ──

    #[test]
    fn aspect_toggle_requests_windowed_fit_only_when_locked_outside_fullscreen() {
        assert!(should_request_windowed_fit_after_aspect_toggle(true, false));
        assert!(!should_request_windowed_fit_after_aspect_toggle(true, true));
        assert!(!should_request_windowed_fit_after_aspect_toggle(
            false, false
        ));
    }

    #[test]
    fn f12_requests_restart() {
        assert!(key_requests_restart(KeyCode::F12));
        assert!(!key_requests_restart(KeyCode::F2));
    }

    #[test]
    fn menu_marks_clear_fit_when_window_mode_is_windowed() {
        let marks = menu_marks(
            WindowMode::Windowed,
            FullscreenScaleMode::Fit,
            false,
            PhysicalSize::new(1000, 700),
        );

        assert!(!marks.fit);
        assert!(!marks.scale_1x);
        assert!(!marks.scale_2x);
        assert!(!marks.scale_3x);
        assert!(!marks.scale_4x);
        assert!(!marks.lock_aspect);
    }

    #[test]
    fn sync_window_mode_detects_native_fullscreen_exit() {
        let sync = sync_window_mode(
            WindowMode::Fullscreen,
            false,
            FullscreenScaleMode::Integer(3),
        );

        assert_eq!(
            sync,
            WindowModeSync {
                mode: WindowMode::Windowed,
                output: Some(WindowOutput::ViewportScaleModeChanged(
                    ViewportScaleMode::Fit
                )),
            }
        );
    }

    #[test]
    fn sync_window_mode_detects_native_fullscreen_enter() {
        let sync = sync_window_mode(WindowMode::Windowed, true, FullscreenScaleMode::Integer(3));

        assert_eq!(
            sync,
            WindowModeSync {
                mode: WindowMode::Fullscreen,
                output: Some(WindowOutput::ViewportScaleModeChanged(
                    ViewportScaleMode::Integer(3)
                )),
            }
        );
    }
}
