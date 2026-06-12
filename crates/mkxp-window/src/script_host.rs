//! Script-thread host and engine boundary.
//!
//! A `ScriptEngine` owns one script run. Restart is implemented by letting the
//! current engine return `ScriptExit::RestartRequested`, joining that thread, and
//! spawning a fresh `E::default()` instance against the same window/render
//! runtime.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;

use winit::event_loop::EventLoopProxy;

use mkxp_graphics::GraphicsState;
use tracing::{debug, error, info};

use crate::error::{ScriptError, ScriptExit, ScriptRunResult, panic_payload_to_string};
use crate::runtime::{RuntimeConfig, RuntimeEvent, SharedRuntime};
use crate::window_control::{GAME_H, GAME_W};

/// A script engine that can run on the script host thread.
///
/// Implementations should keep per-run script state inside the engine instance.
/// `App<E>` creates a fresh `E::default()` for each restart.
pub(crate) trait ScriptEngine: Default + Send + 'static {
    /// Run the engine until it finishes, fails, shuts down, or requests restart.
    fn run(self, ctx: ScriptContext) -> ScriptRunResult;
}

/// Result of the script-side `Graphics.update` boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptFrameAction {
    /// The frame was rendered and the script may continue updating.
    Continue,
    /// Runtime shutdown was requested.
    Shutdown,
    #[allow(
        dead_code,
        reason = "restart is wired in the next runtime-control task"
    )]
    Restart,
}

/// Script-facing access to shared runtime services.
///
/// The context intentionally exposes a small API: read lifecycle/config state,
/// mutate graphics state before submitting a frame, and block at the
/// `Graphics.update` boundary.
pub(crate) struct ScriptContext {
    runtime: Arc<SharedRuntime>,
}

impl ScriptContext {
    fn new(runtime: Arc<SharedRuntime>) -> Self {
        Self { runtime }
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.runtime.control.is_shutdown_requested()
    }

    /// Mutate script-facing graphics state.
    ///
    /// Callers should keep this closure small. The render thread also needs the
    /// graphics mutex after the script submits a frame, so script code must not
    /// hold the lock across `submit_frame_and_wait`.
    pub(crate) fn with_graphics<T>(&self, f: impl FnOnce(&mut GraphicsState) -> T) -> T {
        f(&mut self.runtime.graphics.lock().unwrap())
    }

    #[allow(
        dead_code,
        reason = "script engines start reading config in the generic engine/config tasks"
    )]
    pub(crate) fn config(&self) -> &RuntimeConfig {
        &self.runtime.config
    }

    /// Submit the current script-produced frame and block until the render host
    /// presents it or lifecycle control requests shutdown/restart.
    pub(crate) fn submit_frame_and_wait(&self) -> ScriptFrameAction {
        self.runtime
            .frame_sync
            .script_frame_ready_and_wait(&self.runtime.control)
    }
}

/// Demo engine used by the default binary entry point.
#[derive(Default)]
pub(crate) struct DemoScriptEngine;

impl ScriptEngine for DemoScriptEngine {
    fn run(self, ctx: ScriptContext) -> ScriptRunResult {
        let mut x = 220.0_f32;
        let mut y = 165.0_f32;
        let mut dx = 2.0_f32;
        let mut dy = 1.5_f32;

        while !ctx.is_shutdown_requested() {
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

            ctx.with_graphics(|graphics| {
                graphics.set_test_quad(x, y, 200.0, 150.0, r, g, b);
            });

            match ctx.submit_frame_and_wait() {
                ScriptFrameAction::Continue => {}
                ScriptFrameAction::Shutdown => return Ok(ScriptExit::ShutdownRequested),
                ScriptFrameAction::Restart => return Ok(ScriptExit::RestartRequested),
            }
        }

        if ctx.is_shutdown_requested() {
            Ok(ScriptExit::ShutdownRequested)
        } else {
            Ok(ScriptExit::Finished)
        }
    }
}

pub(crate) fn spawn_script_thread<E: ScriptEngine>(
    engine: E,
    runtime: Arc<SharedRuntime>,
    proxy: EventLoopProxy<RuntimeEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        debug!("script thread started");
        let ctx = ScriptContext::new(runtime.clone());
        let result = catch_script_unwind(|| engine.run(ctx));
        match &result {
            Ok(exit) => info!(?exit, "script thread finished"),
            Err(error) => error!(%error, "script thread failed"),
        }
        runtime.record_script_result(result);

        let _ = proxy.send_event(RuntimeEvent::ScriptExited);
    })
}

fn catch_script_unwind(run: impl FnOnce() -> ScriptRunResult) -> ScriptRunResult {
    match panic::catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result,
        Err(payload) => Err(ScriptError::Panic(panic_payload_to_string(payload))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ScriptError, ScriptExit};

    #[test]
    fn catch_script_unwind_preserves_normal_result() {
        let result = catch_script_unwind(|| Ok(ScriptExit::Finished));

        assert_eq!(result, Ok(ScriptExit::Finished));
    }

    #[test]
    fn catch_script_unwind_converts_panic_payload_to_script_error() {
        let result = catch_script_unwind(|| panic!("boom"));

        assert_eq!(result, Err(ScriptError::Panic("boom".to_string())));
    }

    #[test]
    fn demo_script_engine_can_be_created_for_each_run() {
        fn new_engine<E: ScriptEngine>() -> E {
            E::default()
        }

        let _first = DemoScriptEngine;
        let _second: DemoScriptEngine = new_engine();
    }
}
