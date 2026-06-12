use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;

use winit::event_loop::EventLoopProxy;

use mkxp_graphics::GraphicsState;

use crate::window_control::{GAME_H, GAME_W};
use crate::{
    RuntimeEvent, ScriptError, ScriptExit, ScriptRunResult, SharedRuntime, panic_payload_to_string,
};

pub(crate) trait ScriptEngine: Send + 'static {
    fn run(self: Box<Self>, ctx: ScriptContext) -> ScriptRunResult;
}

pub(crate) struct ScriptContext {
    runtime: Arc<SharedRuntime>,
    proxy: EventLoopProxy<RuntimeEvent>,
}

impl ScriptContext {
    fn new(runtime: Arc<SharedRuntime>, proxy: EventLoopProxy<RuntimeEvent>) -> Self {
        Self { runtime, proxy }
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.runtime.shutdown.load(Ordering::Acquire)
    }

    pub(crate) fn with_graphics<T>(&self, f: impl FnOnce(&mut GraphicsState) -> T) -> T {
        f(&mut self.runtime.graphics.lock().unwrap())
    }

    pub(crate) fn graphics_update(&self) -> bool {
        self.runtime
            .frame_sync
            .script_frame_ready_and_wait(&self.runtime.shutdown, || {
                let _ = self.proxy.send_event(RuntimeEvent::ScriptFrameReady);
            })
    }
}

pub(crate) struct DemoScriptEngine;

impl ScriptEngine for DemoScriptEngine {
    fn run(self: Box<Self>, ctx: ScriptContext) -> ScriptRunResult {
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

            if !ctx.graphics_update() {
                return Ok(ScriptExit::ShutdownRequested);
            }
        }

        if ctx.is_shutdown_requested() {
            Ok(ScriptExit::ShutdownRequested)
        } else {
            Ok(ScriptExit::Finished)
        }
    }
}

pub(crate) fn spawn_script_thread(
    engine: Box<dyn ScriptEngine>,
    runtime: Arc<SharedRuntime>,
    proxy: EventLoopProxy<RuntimeEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let ctx = ScriptContext::new(runtime.clone(), proxy.clone());
        let result = catch_script_unwind(|| engine.run(ctx));
        runtime.record_script_result(result);

        // Wake winit even if it is sleeping until the next frame; the main
        // thread should observe and report script completion promptly.
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
    use crate::{ScriptError, ScriptExit};

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
}
