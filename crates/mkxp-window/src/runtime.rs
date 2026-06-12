use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use mkxp_graphics::GraphicsState;

use crate::error::ScriptRunResult;
use crate::frame_sync::FrameSync;
use crate::render_host::RenderError;
use crate::window_control::{GAME_H, GAME_W};

const DEFAULT_FPS: u32 = 60;

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

// ── SharedRuntime ──

pub(crate) struct SharedRuntime {
    pub(crate) graphics: Mutex<GraphicsState>,
    pub(crate) frame_sync: FrameSync,
    script_outcome: ScriptOutcomeSlot,
    render_outcome: RenderOutcomeSlot,
    pub(crate) shutdown: AtomicBool,
}

impl SharedRuntime {
    pub(crate) fn new(
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
            render_outcome: RenderOutcomeSlot::default(),
            shutdown: AtomicBool::new(false),
        }
    }

    pub(crate) fn record_script_result(&self, result: ScriptRunResult) {
        if result.is_err() {
            self.shutdown.store(true, Ordering::Release);
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
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use crate::error::{ScriptError, ScriptExit, panic_payload_to_string};
    use crate::render_host::RenderError;

    use super::{RenderOutcomeSlot, ScriptOutcomeSlot};

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
