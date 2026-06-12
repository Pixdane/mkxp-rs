//! Shared runtime state used by the winit, render, and script hosts.
//!
//! `SharedRuntime` is the narrow cross-thread boundary for the current window
//! host. The winit thread owns window lifecycle, the render thread owns frame
//! presentation, and the script thread owns script execution. Shared state lives
//! here only when it must be visible across those boundaries.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mkxp_graphics::GraphicsState;

use crate::error::ScriptRunResult;
use crate::frame_sync::FrameSync;
use crate::render_host::RenderError;

// ── Runtime config ──

/// Runtime settings consumed by the window host after `mkxp_config` has been
/// normalized.
///
/// The original config object can contain optional values and sections for
/// future subsystems. This type stores the concrete values needed by the current
/// window/render/script host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeConfig {
    pub(crate) window_title: String,
    pub(crate) window_size: (u32, u32),
    pub(crate) game_size: (u32, u32),
    pub(crate) target_fps: u32,
    pub(crate) vsync: bool,
    pub(crate) enable_reset: bool,
    pub(crate) scripts_path: Option<String>,
    pub(crate) rgss_version: Option<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::from(mkxp_config::Config::default())
    }
}

impl From<mkxp_config::Config> for RuntimeConfig {
    fn from(config: mkxp_config::Config) -> Self {
        let mut config = config;
        config.fill_defaults();
        let defaults = mkxp_config::Config::default();

        Self {
            window_title: config.window.title.unwrap_or_default(),
            window_size: positive_size(config.window.size, defaults.window.size),
            game_size: positive_size(config.graphics.game_size, defaults.graphics.game_size),
            target_fps: normalize_frame_rate(config.graphics.frame_rate.unwrap_or_default()),
            vsync: config.graphics.vsync.unwrap_or(true),
            enable_reset: config.input.enable_reset.unwrap_or(true),
            scripts_path: config.ruby.scripts_path,
            rgss_version: config.ruby.rgss_version,
        }
    }
}

fn positive_size(value: Option<(i32, i32)>, fallback: Option<(i32, i32)>) -> (u32, u32) {
    let (width, height) = value
        .filter(|(width, height)| *width > 0 && *height > 0)
        .or(fallback)
        .expect("reference defaults must include a positive size");
    (width as u32, height as u32)
}

fn normalize_frame_rate(frame_rate: u32) -> u32 {
    if frame_rate == 0 {
        0
    } else {
        frame_rate.clamp(1, 240)
    }
}

// ── Runtime events ──

/// User events sent back to the winit event loop by host threads.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RuntimeEvent {
    /// The script thread recorded an outcome and exited.
    ScriptExited,
    /// The render thread recorded a fatal error or stopped unexpectedly.
    RenderExited,
}

// ── Outcome slots ──

/// Single-consumer storage for the last script thread outcome.
#[derive(Default)]
pub(crate) struct ScriptOutcomeSlot {
    result: Mutex<Option<ScriptRunResult>>,
}

impl ScriptOutcomeSlot {
    pub(crate) fn record(&self, result: ScriptRunResult) {
        *self.result.lock().unwrap() = Some(result);
    }

    pub(crate) fn take(&self) -> Option<ScriptRunResult> {
        self.result.lock().unwrap().take()
    }
}

/// Single-consumer storage for the last render thread error.
#[derive(Default)]
pub(crate) struct RenderOutcomeSlot {
    result: Mutex<Option<RenderError>>,
}

impl RenderOutcomeSlot {
    pub(crate) fn record(&self, error: RenderError) {
        *self.result.lock().unwrap() = Some(error);
    }

    pub(crate) fn take(&self) -> Option<RenderError> {
        self.result.lock().unwrap().take()
    }
}

// ── Runtime control ──

/// Cross-thread lifecycle flags.
///
/// Shutdown is terminal for the whole runtime. Restart is a script-only control
/// path: it wakes the script out of `Graphics.update`, joins that script thread,
/// and spawns a fresh engine instance without rebuilding the window or render
/// host.
#[derive(Default)]
pub(crate) struct RuntimeControl {
    shutdown: AtomicBool,
    restart: AtomicBool,
}

impl RuntimeControl {
    pub(crate) fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    pub(crate) fn request_restart(&self) {
        self.restart.store(true, Ordering::Release);
    }

    pub(crate) fn clear_restart(&self) {
        self.restart.store(false, Ordering::Release);
    }

    pub(crate) fn is_restart_requested(&self) -> bool {
        self.restart.load(Ordering::Acquire)
    }
}

// ── SharedRuntime ──

/// Shared state for the current window runtime.
///
/// `GraphicsState` is protected by a mutex because the script thread mutates
/// script-facing demo graphics before submitting a frame, while the render thread
/// applies window commands and presents the frame after `FrameSync` marks it
/// ready. The frame protocol prevents normal script/render mutation overlap.
pub(crate) struct SharedRuntime {
    #[allow(
        dead_code,
        reason = "ScriptContext exposes runtime config before later tasks consume it"
    )]
    pub(crate) config: Arc<RuntimeConfig>,
    pub(crate) graphics: Mutex<GraphicsState>,
    pub(crate) frame_sync: FrameSync,
    script_outcome: ScriptOutcomeSlot,
    render_outcome: RenderOutcomeSlot,
    pub(crate) control: RuntimeControl,
}

impl SharedRuntime {
    pub(crate) fn with_config(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        config: RuntimeConfig,
    ) -> Self {
        let config = Arc::new(config);
        Self {
            config: config.clone(),
            graphics: Mutex::new(GraphicsState::new(
                device,
                queue,
                surface,
                surface_config,
                config.game_size.0,
                config.game_size.1,
                config.target_fps,
            )),
            frame_sync: FrameSync::default(),
            script_outcome: ScriptOutcomeSlot::default(),
            render_outcome: RenderOutcomeSlot::default(),
            control: RuntimeControl::default(),
        }
    }

    pub(crate) fn record_script_result(&self, result: ScriptRunResult) {
        if result.is_err() {
            self.control.request_shutdown();
            self.frame_sync.wake_all();
        }
        self.script_outcome.record(result);
    }

    pub(crate) fn take_script_result(&self) -> Option<ScriptRunResult> {
        self.script_outcome.take()
    }

    pub(crate) fn record_render_error(&self, error: RenderError) {
        self.render_outcome.record(error);
    }

    pub(crate) fn take_render_error(&self) -> Option<RenderError> {
        self.render_outcome.take()
    }

    pub(crate) fn prepare_script_restart(&self) {
        self.script_outcome.take();
        self.control.clear_restart();
        self.frame_sync.reset();
        let mut graphics = self.graphics.lock().unwrap();
        graphics.set_target_fps(self.config.target_fps);
        graphics.reset_demo_state();
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use crate::error::{ScriptError, ScriptExit, panic_payload_to_string};
    use crate::render_host::RenderError;

    use super::{RenderOutcomeSlot, RuntimeConfig, RuntimeControl, ScriptOutcomeSlot};

    #[test]
    fn runtime_config_default_values_match_demo_defaults() {
        let config = RuntimeConfig::from(mkxp_config::Config::default());

        assert_eq!(config.window_title, "");
        assert_eq!(config.window_size, (640, 480));
        assert_eq!(config.game_size, (640, 480));
        assert_eq!(config.target_fps, 60);
        assert!(config.vsync);
        assert!(config.enable_reset);
        assert_eq!(config.scripts_path, None);
        assert_eq!(config.rgss_version.as_deref(), Some("3"));
    }

    #[test]
    fn runtime_config_uses_mkxp_config_overrides() {
        let raw = mkxp_config::Config {
            ruby: mkxp_config::config::Ruby {
                rgss_version: Some("3".into()),
                scripts_path: Some("Data/Scripts.rvdata2".into()),
                ..Default::default()
            },
            window: mkxp_config::config::Window {
                title: Some("Configured Game".into()),
                size: Some((1280, 960)),
                ..Default::default()
            },
            graphics: mkxp_config::config::Graphics {
                frame_rate: Some(120),
                game_size: Some((320, 240)),
                vsync: Some(false),
                ..Default::default()
            },
            input: mkxp_config::config::Input {
                enable_reset: Some(false),
                ..Default::default()
            },
            ..mkxp_config::Config::empty()
        };

        let config = RuntimeConfig::from(raw);

        assert_eq!(config.window_title, "Configured Game");
        assert_eq!(config.window_size, (1280, 960));
        assert_eq!(config.game_size, (320, 240));
        assert_eq!(config.target_fps, 120);
        assert!(!config.vsync);
        assert!(!config.enable_reset);
        assert_eq!(config.scripts_path.as_deref(), Some("Data/Scripts.rvdata2"));
        assert_eq!(config.rgss_version.as_deref(), Some("3"));
    }

    #[test]
    fn runtime_config_keeps_zero_frame_rate_uncapped() {
        let raw = mkxp_config::Config {
            graphics: mkxp_config::config::Graphics {
                frame_rate: Some(0),
                ..Default::default()
            },
            ..mkxp_config::Config::empty()
        };

        let config = RuntimeConfig::from(raw);

        assert_eq!(config.target_fps, 0);
    }

    #[test]
    fn runtime_config_clamps_nonzero_frame_rate() {
        let raw = mkxp_config::Config {
            graphics: mkxp_config::config::Graphics {
                frame_rate: Some(500),
                ..Default::default()
            },
            ..mkxp_config::Config::empty()
        };

        let config = RuntimeConfig::from(raw);

        assert_eq!(config.target_fps, 240);
    }

    #[test]
    fn runtime_control_keeps_restart_distinct_from_shutdown() {
        let control = RuntimeControl::default();

        control.request_restart();

        assert!(control.is_restart_requested());
        assert!(!control.is_shutdown_requested());

        control.clear_restart();

        assert!(!control.is_restart_requested());
        assert!(!control.is_shutdown_requested());
    }

    #[test]
    fn runtime_control_shutdown_can_coexist_with_restart() {
        let control = RuntimeControl::default();

        control.request_restart();
        control.request_shutdown();

        assert!(control.is_restart_requested());
        assert!(control.is_shutdown_requested());
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
    fn render_outcome_slot_records_error_once() {
        let slot = RenderOutcomeSlot::default();

        assert!(slot.take().is_none());

        slot.record(RenderError::Panic("gpu died".into()));
        assert!(matches!(slot.take(), Some(RenderError::Panic(_))));
        assert!(slot.take().is_none());
    }
}
