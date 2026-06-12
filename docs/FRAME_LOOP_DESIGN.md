# mkxp-rs Frame Loop Redesign: Script Thread, Render Thread, winit Main Thread

This document defines the target frame-loop architecture after the macOS input
source switching investigation on 2026-06-12.

The old design rendered on the winit main thread. That matched winit's
`RedrawRequested` guidance, but it made every frame depend on AppKit returning
control to `ApplicationHandler`. On macOS, rapid input-source switching
(`CapsLock`, `Cmd+Space`, or equivalent system shortcuts) can keep the main
thread inside AppKit/IMK long enough for script frames to wait hundreds or
thousands of milliseconds before the winit handler observes them. Diagnostics
showed delays such as:

```text
script frame was ready before the main loop observed it script_wait_ms=5072
```

No matching slow-render or slow-redraw-delivery warning appeared. The root
problem is therefore not `wgpu` present time and not `request_redraw()` delivery
after the app observes readiness. The problem is that the main thread may not
observe script readiness at all while macOS is processing input-source changes.

The new design removes per-frame rendering from the winit main loop. The winit
thread still owns window events and platform UI. A dedicated render thread owns
the render timing loop and consumes script-ready frames.

Related boundaries:

- [`WINDOW_CONTROLLER_DESIGN.md`](WINDOW_CONTROLLER_DESIGN.md): window, menu,
  fullscreen, resize, and `WindowOutput` ownership.
- [`WINDOW_CONSTRAINTS.md`](WINDOW_CONSTRAINTS.md): window scaling and menu
  state behavior.
- [`SCRIPT_HOST_DESIGN.md`](SCRIPT_HOST_DESIGN.md): script host interface and
  future Ruby engine replacement boundary.

## Goals

- Keep RGSS semantics: `Graphics.update` blocks until the prepared frame has
  been rendered, or returns `false` during shutdown.
- Keep window ownership on the winit main thread.
- Stop using `UserEvent/AboutToWait/RedrawRequested` as the only per-frame
  render driver.
- Keep resize, fullscreen, menu state, and viewport-scale commands ordered
  before the next rendered frame.
- Preserve deterministic shutdown: script thread and render thread must be
  explicitly signalled and joined before `WindowController` drops.
- Keep the future Ruby engine behind `ScriptEngine` / `ScriptContext`; replacing
  the demo script must not require rewriting the frame loop again.

## Non-Goals

- Do not let the script thread present the `wgpu::Surface`.
- Do not put window/menu/fullscreen policy into `mkxp-graphics`.
- Do not create a generic service registry.
- Do not make input-source switching a special game-facing input event.
- Do not depend on users avoiding macOS CapsLock input-source switching.

## Target Architecture

```text
winit main thread
  owns WindowController
  receives WindowEvent / menu events
  converts window effects to RenderCommand
  handles app exit and joins threads

script thread
  runs ScriptEngine through ScriptContext
  mutates script-facing graphics state
  calls Graphics.update -> FrameSync::script_frame_ready_and_wait
  blocks until render thread completes the frame

render thread
  owns frame timing
  waits for script-ready frame or shutdown
  drains RenderCommand queue
  if ready && FPS gate is due:
    GraphicsState::update()
    FrameSync::render_finished()
```

The important change is that the render thread waits directly on `FrameSync`.
It is not woken by winit user events and does not need the winit main loop to
reach `about_to_wait`.

## Ownership

```text
App / winit main thread
  owns:
    WindowController
    SharedRuntime
    ScriptHost JoinHandle
    RenderHost JoinHandle
    RenderCommand sender

SharedRuntime
  owns:
    Mutex<GraphicsState>
    FrameSync
    ScriptOutcomeSlot
    shutdown flag

RenderHost / render thread
  borrows SharedRuntime through Arc
  owns RenderCommand receiver
  owns next_frame_at / target frame timing

ScriptHost / script thread
  borrows SharedRuntime through Arc
  owns ScriptEngine
```

`GraphicsState` can remain inside `SharedRuntime` behind `Mutex<GraphicsState>`
for the first migration. `FrameSync` still guarantees that script mutation and
render submission are not concurrent during normal frame flow:

- Script thread mutates graphics state before `Graphics.update`.
- Script thread sets `ready = true` and blocks.
- Render thread drains window commands, renders, sets `ready = false`, and
  wakes the script.
- Script thread continues to the next game frame.

The winit thread should stop mutating `GraphicsState` directly. Instead it sends
commands to the render thread.

## Render Commands

`WindowController` continues to output `WindowOutput`. `App` translates outputs
into render commands:

```rust
enum RenderCommand {
    SurfaceResized { width: u32, height: u32 },
    ViewportScaleModeChanged(ViewportScaleMode),
    Shutdown,
}
```

Mapping:

```text
WindowOutput::SurfaceResized
  -> RenderCommand::SurfaceResized

WindowOutput::ViewportScaleModeChanged
  -> RenderCommand::ViewportScaleModeChanged

WindowOutput::QuitRequested
  -> event_loop.exit()
  -> RenderCommand::Shutdown during App shutdown
```

The render thread drains all pending commands before rendering a ready frame:

```text
drain RenderCommand
apply resize / viewport commands to GraphicsState
if shutdown:
  exit render loop
if frame ready and now >= next_frame_at:
  render
```

This preserves the old invariant that resize/fullscreen/viewport changes are
applied before the next frame is presented, without requiring winit to render.

## FrameSync

`FrameSync` should store both readiness and the time at which the script became
ready:

```rust
struct FrameSync {
    state: Mutex<FrameSyncState>,
    cv: Condvar,
}

struct FrameSyncState {
    ready: bool,
    ready_at: Option<Instant>,
}
```

Required operations:

```rust
impl FrameSync {
    fn script_frame_ready_and_wait(&self, control: &RuntimeControl) -> ScriptFrameAction;
    fn wait_for_ready_or_shutdown(&self, control: &RuntimeControl) -> FrameWait;
    fn render_finished(&self);
    fn reset(&self);
    fn wake_all(&self);
}

enum ScriptFrameAction {
    Continue,
    Shutdown,
    Restart,
}

enum FrameWait {
    Ready { ready_at: Instant },
    Shutdown,
}
```

The script side no longer needs to wake winit for normal per-frame rendering.
`ScriptContext::submit_frame_and_wait()` should call
`script_frame_ready_and_wait()` and block until the render thread calls
`render_finished()` or runtime control requests shutdown/restart.

Runtime control keeps shutdown and restart separate. Shutdown is terminal and
causes the render host to exit. Restart wakes the blocked script, clears any
pending frame through `FrameSync::reset()`, restores script-owned demo graphics
state and target FPS from runtime config, joins the old script after it reports
`ScriptExit::RestartRequested`, and spawns a fresh `E::default()` without
recreating the window or render host. Window state such as size, fullscreen, and
viewport scale mode stays owned by `WindowController` and is not reset here.

The render side blocks on the same `Condvar`. It wakes when:

- the script sets `ready = true`,
- shutdown is requested,
- a command path explicitly calls `wake_all()` because render-side state changed.

## Render Timing

The render thread owns `next_frame_at`. It must be a fixed target timeline, not
"one full frame duration after the previous render finished".

The old timing style:

```text
render finishes at now
next_frame_at = now + frame_duration
```

is intentionally not the target design. It adds render time and script update
time on top of every frame interval, so the visible frame cadence drifts slower
than `target_fps` whenever either side does real work.

The target design advances scheduled frame time by a fixed duration:

```text
loop:
  wait until script frame is ready or shutdown
  if shutdown:
    break

  drain render commands

  wait until next_frame_at if needed
  drain render commands again

  graphics.update()
  frame_sync.render_finished()
  next_frame_at = advance_frame_deadline(next_frame_at, frame_duration, Instant::now())
```

The second command drain after the FPS wait matters. Resize/fullscreen commands
can arrive while the render thread is sleeping until the frame gate opens.
They must still be applied before present.

Recommended deadline advancement:

```rust
fn advance_frame_deadline(
    mut next_frame_at: Instant,
    frame_duration: Duration,
    now: Instant,
) -> Instant {
    next_frame_at += frame_duration;

    if next_frame_at + frame_duration < now {
        now + frame_duration
    } else {
        next_frame_at
    }
}
```

Semantics:

- Normal frames target `t`, `t + frame_duration`, `t + 2 * frame_duration`, and
  so on.
- Small jitter does not permanently drift the timeline. A frame that renders a
  few milliseconds late does not force the next frame to wait a whole fresh
  duration from the late completion time.
- Large stalls drop timing debt. If the process is hundreds of milliseconds
  late, do not render a burst of historical frames to catch up; reset the next
  target near `now + frame_duration`.
- `Graphics.update` still blocks the script until the scheduled frame is
  rendered and presented.

`target_fps` remains read from `GraphicsState::target_fps()`. A future config
service may move this into a separate timing configuration object, but that is
not required for this migration.

If `target_fps` changes, rebuild the timeline from the current time rather than
mixing the old duration into the new cadence:

```text
frame_duration = 1 / new_target_fps
next_frame_at = Instant::now() + frame_duration
```

## winit Main Thread Responsibilities

The winit thread remains the only owner of platform window and menu objects:

```text
ApplicationHandler::resumed
  create WindowController
  create wgpu surface from WindowController::window()
  create GraphicsState
  create SharedRuntime
  spawn render thread
  spawn script thread

ApplicationHandler::window_event
  WindowController::on_window_event
  translate WindowOutput -> RenderCommand / exit

ApplicationHandler::about_to_wait
  WindowController::on_about_to_wait
  translate WindowOutput -> RenderCommand / exit

ApplicationHandler::user_event
  handle ScriptExited or future host notifications only
```

The winit thread should not call `GraphicsState::update()` and should not call
`Window::request_redraw()` for the game frame loop.

It may still call platform APIs required by winit itself, such as fullscreen,
window resize requests, menu checkmark updates, and event-loop exit.

## Script Thread Responsibilities

The script thread shape remains intentionally close to real RGSS:

```text
loop:
  update game state
  mutate graphics state
  Graphics.update
    -> ScriptContext::submit_frame_and_wait()
    -> block until render thread presents the frame
```

The `ScriptEngine` trait remains the replacement boundary:

```rust
trait ScriptEngine: Default + Send + 'static {
    fn run(self, ctx: ScriptContext) -> ScriptRunResult;
}
```

The binary selects the engine with `App::<DemoScriptEngine>::new(proxy)` today.
Restart and future engine swaps create fresh instances through `E::default()`,
so the future Ruby engine should replace the selected `App<E>` type, not the
render host or window host.

## Error and Exit Flow

Script thread exits through `ScriptRunResult`:

```text
ScriptExit::Finished
  -> record outcome
  -> send RuntimeEvent::ScriptExited

ScriptError::Message / ScriptError::Panic
  -> record outcome
  -> send RuntimeEvent::ScriptExited
```

The winit thread remains responsible for presenting/logging fatal script errors
and exiting the app.

Script restart is non-fatal:

```text
ScriptExit::RestartRequested
  -> join old script thread
  -> clear restart control, pending frame state, demo graphics state, and FPS
  -> spawn_script_thread(E::default(), ...)
```

Render thread errors should use a separate result slot or command back to the
winit thread:

```rust
type RenderRunResult = Result<RenderExit, RenderError>;

enum RuntimeEvent {
    ScriptExited,
    RenderExited,
}
```

Initial render errors can be fatal:

```text
wgpu::SurfaceError::Lost / unexpected error
  -> record RenderError
  -> set shutdown
  -> wake FrameSync
  -> send RuntimeEvent::RenderExited
  -> winit consumes error and exits
```

`SurfaceError::Timeout` and `Outdated` may continue to be non-fatal skips.

## Shutdown Order

Shutdown must be explicit:

```text
App::exiting / fatal error / QuitRequested
  control.shutdown = true
  send RenderCommand::Shutdown
  FrameSync::wake_all()
  join script thread
  join render thread
  drop SharedRuntime / GraphicsState
  drop WindowController / winit Window
```

`JoinHandle` drop is not enough; it detaches the thread. Both host threads must
be joined so that no thread can continue using `GraphicsState` or `wgpu::Surface`
while the window is being destroyed.

`WindowController` must outlive `GraphicsState` because `wgpu::Surface`
logically depends on the winit window. Keep the current explicit `take()` style
or ensure field order and shutdown code preserve this invariant.

## macOS Input-Source Switching Rationale

The observed failure mode:

```text
script thread sets ready = true
script thread blocks at Graphics.update
macOS handles rapid input-source switching on main thread
winit ApplicationHandler is not entered for hundreds/thousands of ms
script frame waits until main thread returns
```

Attempts that did not solve the issue:

- Move rendering from `about_to_wait` to `RedrawRequested`.
- Call `request_redraw()` when the script frame is ready and due.
- Temporarily use `ControlFlow::Poll` while a script frame is pending.
- Observe pending ready frames from ordinary `window_event`.

These attempts failed because all of them still require the winit main thread
to reach an `ApplicationHandler` callback.

The render thread design removes that dependency. It uses the script/render
`Condvar`, not the AppKit runloop, as the per-frame wake mechanism.

## Technical Assumptions to Verify

The migration depends on these assumptions:

1. `wgpu::Device`, `wgpu::Queue`, and `wgpu::Surface<'static>` can be moved to
   and used from the render thread while the winit `Window` remains alive on the
   main thread.
2. Presenting a `wgpu::Surface` from a non-main thread works on the supported
   backends, especially macOS/Metal.
3. Resize reconfiguration can safely happen on the render thread after the main
   thread sends the latest physical size.

These should be verified with a small implementation and macOS smoke test before
removing the old diagnostics.

If assumption 2 fails on macOS, the fallback is a platform-specific design:

- keep present on the main thread only for platforms that require it,
- or introduce a lower-level macOS display/link integration,
- or accept that macOS input-source switching can stall rendering when using
  winit/AppKit main-thread present.

Do not silently fall back to the old design without documenting the platform
constraint.

## Implementation Plan

### Task 1: Remove Failed Callback Workarounds

Files:

- Modify `crates/mkxp-window/src/main.rs`
- Modify this document if the observed behavior changes during testing

Steps:

1. Remove the ordinary `window_event` path that calls
   `request_redraw_if_script_ready()` for non-redraw events.
2. Keep `FrameSync.ready_at` and diagnostics until the render thread is proven.
3. Keep `request_redraw` code only until render-thread migration replaces the
   old path.
4. Run `cargo test -p mkxp-window`.

### Task 2: Introduce Render Host

Files:

- Create `crates/mkxp-window/src/render_host.rs`
- Modify `crates/mkxp-window/src/main.rs`

Initial types:

```rust
enum RenderCommand {
    SurfaceResized { width: u32, height: u32 },
    ViewportScaleModeChanged(ViewportScaleMode),
    Shutdown,
}

trait RenderHost {
    fn spawn(runtime: Arc<SharedRuntime>, commands: Receiver<RenderCommand>) -> JoinHandle<()>;
}
```

The first implementation can be a free function, mirroring
`spawn_script_thread()`.

### Task 3: Move Frame Rendering to Render Thread

Files:

- Modify `crates/mkxp-window/src/main.rs`
- Modify `crates/mkxp-window/src/render_host.rs`

Steps:

1. Move `next_frame_at`, `frame_duration`, and render timing into render host.
2. Render host waits on `FrameSync::wait_for_ready_or_shutdown()`.
3. Render host drains `RenderCommand` before each frame.
4. Render host calls `GraphicsState::update()`.
5. Render host calls `FrameSync::render_finished()`.
6. Remove per-frame `request_redraw()` and `render_if_script_ready()` from
   `ApplicationHandler`.

### Task 4: Route Window Outputs to Render Commands

Files:

- Modify `crates/mkxp-window/src/main.rs`
- Modify `crates/mkxp-window/src/render_host.rs`

Steps:

1. `SurfaceResized` sends `RenderCommand::SurfaceResized`.
2. `ViewportScaleModeChanged` sends `RenderCommand::ViewportScaleModeChanged`.
3. `QuitRequested` exits winit and triggers shutdown.
4. If the render command receiver is closed, record a fatal render host error.

### Task 5: Error Propagation

Files:

- Modify `crates/mkxp-window/src/main.rs`
- Modify `crates/mkxp-window/src/render_host.rs`

Steps:

1. Add `RuntimeEvent::RenderExited`.
2. Add a render outcome slot mirroring `ScriptOutcomeSlot`, or make a generic
   once-consumed outcome slot.
3. Convert render fatal errors into `WindowError`.
4. On render fatal error, set shutdown, wake script, exit event loop, and join
   both threads.

### Task 6: Documentation and Tests

Files:

- Modify `docs/FRAME_LOOP_DESIGN.md`
- Modify `docs/SCRIPT_HOST_DESIGN.md` if `ScriptContext::submit_frame_and_wait()`
  changes its runtime-control semantics.
- Modify `docs/WINDOW_CONTROLLER_DESIGN.md` if `App` no longer applies
  `WindowOutput` directly to `GraphicsState`.
- Modify tests in `crates/mkxp-window/src/main.rs` and new render host tests.

Required test coverage:

- `FrameSync` wakes render host when script frame becomes ready.
- `FrameSync` shutdown releases a blocked script frame.
- `FrameSync` restart releases a blocked script frame and clears pending frame state.
- render host applies resize commands before rendering a ready frame.
- render host releases the script thread after render.
- render host exits on shutdown without leaving script blocked.
- render fatal error is recorded once and reaches the winit-owned error path.

## Current Implementation Status

The render-host migration is implemented in `mkxp-window`:

- `main.rs` is the current binary entry: it initializes logging, creates the
  winit event loop, selects `App::<DemoScriptEngine>`, and runs `run_app()`.
- `app.rs` owns winit `ApplicationHandler`, wgpu bootstrap, event forwarding,
  shutdown, and thread joins.
- `render_host.rs` owns `RenderCommand`, render-thread spawn, render timing,
  command draining, `GraphicsState::update()`, and render error propagation.
- `frame_sync.rs` owns the script/render synchronization primitive; the render
  thread waits on `FrameSync::wait_for_ready_or_shutdown()`.
- `script_host.rs` no longer sends `RuntimeEvent::ScriptFrameReady` for normal
  per-frame rendering. The winit user event path is used for script exit.
- `RuntimeEvent::RenderExited` reports fatal render thread errors back to the
  winit-owned error path.

The old diagnostic path based on `RuntimeEvent::ScriptFrameReady`,
`request_redraw_if_script_ready`, `RedrawRequested -> render_if_script_ready`,
and `FrameDiagnostics` has been removed.

## Verification Strategy

Before considering the migration complete:

- Run `cargo test -p mkxp-window`.
- Run `cargo check -p mkxp-window`.
- Run `cargo clippy -p mkxp-window --no-deps -- -D warnings`.
- Run `git diff --check`.
- On macOS, rapidly switch input sources with CapsLock or the configured input
  shortcut for at least 30 seconds.
- Confirm no recurring `script frame was ready before the main loop observed it`
  warnings during render-thread mode.
- Confirm fullscreen enter/exit, integer fullscreen scaling, and window resize
  still update viewport and menu state correctly.

The macOS smoke test is required because this redesign exists to remove a
platform/runloop timing dependency that unit tests cannot reproduce.
