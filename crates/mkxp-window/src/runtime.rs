use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mkxp_graphics::GraphicsState;

use crate::error::ScriptRunResult;
use crate::frame_sync::FrameSync;
use crate::render_host::RenderError;
use crate::window_control::{GAME_H, GAME_W};

const DEFAULT_FPS: u32 = 60;

// ── Runtime config ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeConfig {
    pub(crate) window_title: String,
    pub(crate) window_size: (u32, u32),
    pub(crate) target_fps: u32,
    pub(crate) vsync: bool,
    pub(crate) enable_reset: bool,
    pub(crate) scripts_path: Option<String>,
    pub(crate) rgss_version: Option<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            window_title: "mkxp-rs".into(),
            window_size: (GAME_W, GAME_H),
            target_fps: DEFAULT_FPS,
            vsync: true,
            enable_reset: true,
            scripts_path: None,
            rgss_version: None,
        }
    }
}

impl From<mkxp_config::Config> for RuntimeConfig {
    fn from(config: mkxp_config::Config) -> Self {
        let mut runtime = Self::default();

        if let Some(title) = config.window.title {
            runtime.window_title = title;
        }
        if let Some((width, height)) = config.window.size
            && width > 0
            && height > 0
        {
            runtime.window_size = (width as u32, height as u32);
        }
        if let Some(frame_rate) = config.graphics.frame_rate {
            runtime.target_fps = frame_rate.clamp(1, 240);
        }
        if let Some(vsync) = config.graphics.vsync {
            runtime.vsync = vsync;
        }
        if let Some(enable_reset) = config.input.enable_reset {
            runtime.enable_reset = enable_reset;
        }

        runtime.scripts_path = config.ruby.scripts_path;
        runtime.rgss_version = config.ruby.rgss_version;

        runtime
    }
}

// ── Runtime events ──

#[derive(Debug, Clone, Copy)]
pub(crate) enum RuntimeEvent {
    ScriptExited,
    RenderExited,
}

// ── Outcome slots ──

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
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
    ) -> Self {
        Self::with_config(
            device,
            queue,
            surface,
            surface_config,
            RuntimeConfig::default(),
        )
    }

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
                GAME_W,
                GAME_H,
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

        assert_eq!(config.window_title, "mkxp-rs");
        assert_eq!(config.window_size, (640, 480));
        assert_eq!(config.target_fps, 60);
        assert!(config.vsync);
        assert!(config.enable_reset);
        assert_eq!(config.scripts_path, None);
        assert_eq!(config.rgss_version, None);
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
                vsync: Some(false),
                ..Default::default()
            },
            input: mkxp_config::config::Input {
                enable_reset: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };

        let config = RuntimeConfig::from(raw);

        assert_eq!(config.window_title, "Configured Game");
        assert_eq!(config.window_size, (1280, 960));
        assert_eq!(config.target_fps, 120);
        assert!(!config.vsync);
        assert!(!config.enable_reset);
        assert_eq!(config.scripts_path.as_deref(), Some("Data/Scripts.rvdata2"));
        assert_eq!(config.rgss_version.as_deref(), Some("3"));
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
