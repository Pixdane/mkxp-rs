use std::sync::Mutex;
use std::time::Instant;

use crate::runtime::RuntimeControl;
use crate::script_host::ScriptFrameAction;

/// Outcome of waiting for a script frame on the render side.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum FrameWait {
    /// A frame is ready for rendering, recorded at the given instant.
    Ready { ready_at: Instant },
    /// The runtime has been asked to shut down.
    Shutdown,
}

#[derive(Default)]
struct FrameSyncState {
    ready: bool,
    ready_at: Option<Instant>,
}

/// Synchronizes one script-produced frame with one render pass.
///
/// The script side sets `ready = true` at the `Graphics.update` boundary and
/// then blocks. The render side consumes exactly one frame, flips `ready` back
/// to false, and wakes the script so the next game update can begin.
#[derive(Default)]
pub(crate) struct FrameSync {
    state: Mutex<FrameSyncState>,
    cv: std::sync::Condvar,
}

#[allow(dead_code)]
impl FrameSync {
    /// Called from the script thread to signal that a frame is ready.
    ///
    /// Sets `ready = true`, records the instant, wakes the render waiter via
    /// the internal Condvar, then blocks until the render side calls
    /// `render_finished()` or shutdown is requested.
    ///
    /// The former `wake_event_loop` callback has been removed; the
    /// render thread waits on a separate Condvar in `wait_for_ready_or_shutdown`.
    ///
    /// Returns the next script-side action after the frame is rendered or the
    /// runtime control state changes.
    pub(crate) fn script_frame_ready_and_wait(
        &self,
        control: &RuntimeControl,
    ) -> ScriptFrameAction {
        let mut state = self.state.lock().unwrap();
        state.ready = true;
        state.ready_at = Some(Instant::now());
        self.cv.notify_one();

        while state.ready && !control.is_shutdown_requested() && !control.is_restart_requested() {
            state = self.cv.wait(state).unwrap();
        }

        if control.is_shutdown_requested() {
            ScriptFrameAction::Shutdown
        } else if control.is_restart_requested() {
            ScriptFrameAction::Restart
        } else {
            ScriptFrameAction::Continue
        }
    }

    /// Called from the render thread to block until a frame is ready for
    /// rendering or shutdown is requested.
    ///
    /// Returns [`FrameWait::Ready`] when the script has signalled a frame, or
    /// [`FrameWait::Shutdown`] when the runtime has been asked to shut down.
    pub(crate) fn wait_for_ready_or_shutdown(&self, control: &RuntimeControl) -> FrameWait {
        let mut state = self.state.lock().unwrap();
        while (!state.ready || control.is_restart_requested()) && !control.is_shutdown_requested() {
            state = self.cv.wait(state).unwrap();
        }

        if control.is_shutdown_requested() {
            FrameWait::Shutdown
        } else {
            let ready_at = state.ready_at.take().unwrap_or_else(Instant::now);
            FrameWait::Ready { ready_at }
        }
    }

    /// Returns `true` when a script frame is ready and waiting for rendering.
    pub(crate) fn is_ready(&self) -> bool {
        self.state.lock().unwrap().ready
    }

    /// Returns the instant the current ready frame was submitted, if any.
    pub(crate) fn ready_since(&self) -> Option<Instant> {
        self.state.lock().unwrap().ready_at
    }

    /// Called from the render side after the frame has been rendered.
    ///
    /// Resets `ready` to `false` and wakes the script thread so it can
    /// begin the next update cycle.
    pub(crate) fn render_finished(&self) {
        let mut state = self.state.lock().unwrap();
        state.ready = false;
        state.ready_at = None;
        self.cv.notify_one();
    }

    /// Clears any pending frame without waking the script side as if a render
    /// completed.
    pub(crate) fn reset(&self) {
        let mut state = self.state.lock().unwrap();
        state.ready = false;
        state.ready_at = None;
        self.cv.notify_all();
    }

    /// Wakes all threads waiting on this condvar.
    ///
    /// Used during shutdown so blocked script or render threads can observe
    /// the shutdown flag and exit.
    pub(crate) fn wake_all(&self) {
        self.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn blocks_script_until_render_finishes() {
        let sync = Arc::new(FrameSync::default());
        let control = Arc::new(RuntimeControl::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let script_sync = sync.clone();
        let script_control = control.clone();
        let handle = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let action = script_sync.script_frame_ready_and_wait(&script_control);
            done_tx.send(action).unwrap();
        });

        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        while !sync.is_ready() {
            thread::yield_now();
        }
        assert!(done_rx.try_recv().is_err());

        sync.render_finished();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            ScriptFrameAction::Continue
        );
        handle.join().unwrap();
    }

    #[test]
    fn shutdown_releases_blocked_script() {
        let sync = Arc::new(FrameSync::default());
        let control = Arc::new(RuntimeControl::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let script_sync = sync.clone();
        let script_control = control.clone();
        let handle = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let action = script_sync.script_frame_ready_and_wait(&script_control);
            done_tx.send(action).unwrap();
        });

        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        while !sync.is_ready() {
            thread::yield_now();
        }

        control.request_shutdown();
        sync.wake_all();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            ScriptFrameAction::Shutdown
        );
        handle.join().unwrap();
    }

    #[test]
    fn wakes_render_waiter_when_script_frame_is_ready() {
        let sync = Arc::new(FrameSync::default());
        let control = Arc::new(RuntimeControl::default());
        let (render_wait_tx, render_wait_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let render_sync = sync.clone();
        let render_control = control.clone();
        let render_handle = thread::spawn(move || {
            render_wait_tx
                .send(render_sync.wait_for_ready_or_shutdown(&render_control))
                .unwrap();
        });

        assert!(
            render_wait_rx
                .recv_timeout(Duration::from_millis(20))
                .is_err()
        );

        let script_sync = sync.clone();
        let script_control = control.clone();
        let script_handle = thread::spawn(move || {
            done_tx
                .send(script_sync.script_frame_ready_and_wait(&script_control))
                .unwrap();
        });

        match render_wait_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
            FrameWait::Ready { ready_at } => {
                assert!(ready_at <= Instant::now());
            }
            FrameWait::Shutdown => panic!("render waiter should observe a ready frame"),
        }

        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        sync.render_finished();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            ScriptFrameAction::Continue
        );

        render_handle.join().unwrap();
        script_handle.join().unwrap();
    }

    #[test]
    fn restart_releases_blocked_script() {
        let sync = Arc::new(FrameSync::default());
        let control = Arc::new(RuntimeControl::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();

        let script_sync = sync.clone();
        let script_control = control.clone();
        let handle = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let action = script_sync.script_frame_ready_and_wait(&script_control);
            done_tx.send(action).unwrap();
        });

        ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        while !sync.is_ready() {
            thread::yield_now();
        }

        control.request_restart();
        sync.wake_all();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            ScriptFrameAction::Restart
        );
        handle.join().unwrap();
    }

    #[test]
    fn reset_clears_pending_frame() {
        let sync = FrameSync::default();
        let control = RuntimeControl::default();

        let action = std::thread::scope(|scope| {
            let handle = scope.spawn(|| sync.script_frame_ready_and_wait(&control));
            while !sync.is_ready() {
                thread::yield_now();
            }
            sync.reset();
            handle.join().unwrap()
        });

        assert_eq!(action, ScriptFrameAction::Continue);
        assert!(!sync.is_ready());
        assert_eq!(sync.ready_since(), None);
    }

    #[test]
    fn restart_prevents_render_waiter_from_consuming_old_ready_frame() {
        let sync = Arc::new(FrameSync::default());
        let control = Arc::new(RuntimeControl::default());
        let (wait_tx, wait_rx) = mpsc::channel();

        control.request_restart();

        let render_sync = sync.clone();
        let render_control = control.clone();
        let render_handle = thread::spawn(move || {
            wait_tx
                .send(render_sync.wait_for_ready_or_shutdown(&render_control))
                .unwrap();
        });

        let script_sync = sync.clone();
        let script_control = control.clone();
        let script_handle =
            thread::spawn(move || script_sync.script_frame_ready_and_wait(&script_control));

        assert!(wait_rx.recv_timeout(Duration::from_millis(20)).is_err());

        control.clear_restart();
        sync.reset();

        assert!(wait_rx.recv_timeout(Duration::from_millis(20)).is_err());

        control.request_shutdown();
        sync.wake_all();
        assert!(matches!(
            wait_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            FrameWait::Shutdown
        ));
        assert_eq!(script_handle.join().unwrap(), ScriptFrameAction::Restart);
        render_handle.join().unwrap();
    }
}
