use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tracing::error;
use winit::event_loop::EventLoopProxy;

use mkxp_graphics::ViewportScaleMode;

use crate::runtime::{RuntimeEvent, SharedRuntime};

// ── Constants ──

const DEFAULT_FPS: u32 = 60;

/// Advances the frame deadline from a fixed timeline.
///
/// `next_frame_at` is the previous scheduled deadline. The render loop advances
/// the schedule by exactly one `frame_duration`, producing a fixed cadence.
/// When the deadline falls far behind `now`, the debt is dropped and the
/// deadline resets to `now + frame_duration` so the system does not burst-render
/// historical frames.
fn advance_frame_deadline(
    next_frame_at: Instant,
    frame_duration: Duration,
    now: Instant,
) -> Instant {
    let next = next_frame_at + frame_duration;
    if next + frame_duration < now {
        now + frame_duration
    } else {
        next
    }
}

// ── Render commands ──

/// Commands sent from the winit main thread to the render thread.
#[derive(Debug)]
pub(crate) enum RenderCommand {
    /// The window surface was resized to the given pixel dimensions.
    SurfaceResized { width: u32, height: u32 },
    /// The viewport scale mode changed (fullscreen enter/exit, menu selection).
    ViewportScaleModeChanged(ViewportScaleMode),
    /// Shut down the render thread.
    Shutdown,
}

// ── Error type ──

/// Errors that can occur during rendering, logged and propagated to the
/// winit thread through `RuntimeEvent::RenderExited`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RenderError {
    /// A fatal surface error (Lost or unexpected) that was returned from
    /// `GraphicsState::update()`.
    #[error("surface error: {0}")]
    Surface(#[from] wgpu::SurfaceError),
    /// The render thread panicked.
    #[error("render thread panicked: {0}")]
    Panic(String),
}

// ── Spawn ──

/// Spawn the render thread and return its `JoinHandle`.
///
/// The render thread owns frame timing, waits for script-ready frames on
/// `FrameSync`, drains `RenderCommand`s before rendering, and calls
/// `GraphicsState::update()` from its own thread.
///
/// Fatal render errors are recorded in `SharedRuntime::render_error` and
/// signalled via `RuntimeEvent::RenderExited` on the event loop proxy.
pub(crate) fn spawn_render_thread(
    runtime: Arc<SharedRuntime>,
    commands: Receiver<RenderCommand>,
    proxy: EventLoopProxy<RuntimeEvent>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let result = catch_render_unwind(|| render_loop(&runtime, &commands));
        match result {
            Ok(()) => {}
            Err(error) => {
                error!(%error, "render thread exited with error");
                runtime.record_render_error(error);
                runtime.shutdown.store(true, Ordering::Release);
                runtime.frame_sync.wake_all();
                let _ = proxy.send_event(RuntimeEvent::RenderExited);
            }
        }
    })
}

// ── Render loop ──

fn render_loop(
    runtime: &SharedRuntime,
    commands: &Receiver<RenderCommand>,
) -> Result<(), RenderError> {
    let mut next_frame_at = Instant::now();
    #[allow(unused_assignments)]
    let mut frame_duration = Duration::from_nanos(1_000_000_000 / DEFAULT_FPS as u64);

    loop {
        // 1. Wait for a script-ready frame or shutdown.
        match runtime
            .frame_sync
            .wait_for_ready_or_shutdown(&runtime.shutdown)
        {
            crate::frame_sync::FrameWait::Shutdown => return Ok(()),
            crate::frame_sync::FrameWait::Ready { .. } => {}
        }

        // 2. Drain pending window commands before rendering.
        if drain_commands(runtime, commands) {
            return Ok(());
        }

        // 3. Wait until the FPS gate opens.
        let now = Instant::now();
        if now < next_frame_at {
            // Coarse sleep; a future improvement could use Condvar::wait_timeout.
            thread::sleep(next_frame_at - now);
        }

        // 4. Drain commands again (resize/viewport commands may arrive while sleeping).
        if drain_commands(runtime, commands) {
            return Ok(());
        }

        // 5. Re-check shutdown after the sleep window.
        if runtime.shutdown.load(Ordering::Acquire) {
            return Ok(());
        }

        // 6. Render the frame.
        match runtime.graphics.lock().unwrap().update() {
            Ok(()) => {}
            Err(e) => return Err(RenderError::Surface(e)),
        }

        runtime.frame_sync.render_finished();

        // 7. Refresh fps if the game changed it.
        let current_fps = runtime.graphics.lock().unwrap().target_fps();
        frame_duration = Duration::from_nanos(1_000_000_000 / current_fps as u64);

        // 8. Advance the fixed timeline.
        next_frame_at = advance_frame_deadline(next_frame_at, frame_duration, Instant::now());
    }
}

// ── Command drain ──

/// Drains all pending `RenderCommand`s from the channel and applies them.
/// Returns `true` if a `Shutdown` command was received or the channel closed.
fn drain_commands(runtime: &SharedRuntime, commands: &Receiver<RenderCommand>) -> bool {
    loop {
        match commands.try_recv() {
            Ok(RenderCommand::SurfaceResized { width, height }) => {
                runtime.graphics.lock().unwrap().on_resize(width, height);
            }
            Ok(RenderCommand::ViewportScaleModeChanged(mode)) => {
                runtime
                    .graphics
                    .lock()
                    .unwrap()
                    .set_viewport_scale_mode(mode);
            }
            Ok(RenderCommand::Shutdown) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return true;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
        }
    }
}

// ── Catch unwind ──

fn catch_render_unwind(run: impl FnOnce() -> Result<(), RenderError>) -> Result<(), RenderError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(result) => result,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic payload".to_string()
            };
            Err(RenderError::Panic(msg))
        }
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_command_channel_receives_shutdown() {
        let (tx, rx) = std::sync::mpsc::channel::<RenderCommand>();

        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        tx.send(RenderCommand::Shutdown).unwrap();
        assert!(matches!(rx.try_recv(), Ok(RenderCommand::Shutdown)));
    }

    #[test]
    fn render_command_channel_reports_disconnection() {
        let (tx, rx) = std::sync::mpsc::channel::<RenderCommand>();
        drop(tx);

        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn advance_frame_deadline_keeps_fixed_timeline_and_drops_large_debt() {
        let now = Instant::now();
        let frame_duration = Duration::from_millis(16);

        assert_eq!(
            advance_frame_deadline(now, frame_duration, now + Duration::from_millis(2)),
            now + frame_duration
        );
        assert_eq!(
            advance_frame_deadline(now, frame_duration, now + Duration::from_millis(100)),
            now + Duration::from_millis(100) + frame_duration
        );
    }
}
