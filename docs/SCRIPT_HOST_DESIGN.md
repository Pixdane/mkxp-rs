# Script host design

This document records the current script host boundary. It is the internal
`mkxp-window` interface for replacing the demo script thread with a real
RGSS/Ruby engine without leaking winit internals into the script layer.

## Decision

Do not create a standalone `mkxp-scripts` crate yet.

Keep the script host abstraction inside `mkxp-window`, in
`crates/mkxp-window/src/script_host.rs`. The current window binary still owns
winit, `FrameSync`, `GraphicsState`, and shutdown ordering, so an internal
module is the honest boundary. The frame-loop redesign in
[`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md) moves normal per-frame rendering
to a render host thread; script host should still expose the same script-facing
boundary.

`mkxp-scripts` is also an imprecise name: it could mean script files,
`Scripts.rxdata` loading, RGSS bytecode, or scripting runtime. When the boundary
is stable, the more likely crates are:

- `mkxp-runtime`: subsystem handles and script-facing runtime services.
- `mkxp-binding`: Ruby/MRI lifecycle and RGSS API bindings.

## Non-goals

- Do not expose `Arc<SharedRuntime>` to script engines as the public interface.
- Do not expose `EventLoopProxy<RuntimeEvent>`, render host internals, or
  `RuntimeEvent` to script engines.
- Do not introduce a generic service registry, `Any`, or `Arc<Mutex<dyn Service>>`.
- Do not guess audio, filesystem, or input APIs before their real bindings need them.

## Interface shape

The host exposes a small `ScriptEngine` trait:

```rust
trait ScriptEngine: Default + Send + 'static {
    fn run(self, ctx: ScriptContext) -> ScriptRunResult;
}
```

The demo implementation is one engine:

```rust
struct DemoScriptEngine;
```

The future Ruby implementation should become another:

```rust
struct RubyScriptEngine;
```

The thread spawn path should not know which engine it runs:

```rust
spawn_script_thread(DemoScriptEngine::default(), runtime, proxy);
spawn_script_thread(RubyScriptEngine::default(), runtime, proxy);
```

## ScriptContext

`ScriptContext` is the script-facing facade. Internally it may hold
`Arc<SharedRuntime>` and host notification handles, but those fields should stay
private.

Initial methods:

```rust
impl ScriptContext {
    fn is_shutdown_requested(&self) -> bool;
    fn with_graphics<T>(&self, f: impl FnOnce(&mut GraphicsState) -> T) -> T;
    fn submit_frame_and_wait(&self) -> ScriptFrameAction;
}
```

`submit_frame_and_wait()` is the important boundary: it sets the frame-ready
flag, blocks on `FrameSync`, and returns a `ScriptFrameAction` when runtime
control should continue, shut down, or restart. A real Ruby `Graphics.update`
binding should call this method rather than directly touching `FrameSync`,
render host internals, or winit.

In the old winit-main-thread render loop, the script-facing frame boundary also
sent `RuntimeEvent::ScriptFrameReady` so the main thread would wake and render.
In the render-host design, normal per-frame wakeup is the `FrameSync` condvar
consumed by the render thread. Winit user events should remain for script exit,
fatal errors, and explicit host notifications, not for every rendered frame.

## Future service growth

Yes, this context will naturally grow toward audio, filesystem, input, config,
and reset/shutdown services. Add those only when the corresponding RGSS binding
needs them:

```rust
ctx.audio().play_bgm(...);
ctx.fs().read(...);
ctx.input().snapshot();
ctx.request_reset();
ctx.request_shutdown();
```

If the context starts representing multiple subsystems rather than only the
window demo host, that is the point to consider extracting a `mkxp-runtime`
crate. Ruby-specific ownership and binding registration should still live in
`mkxp-binding`.

## Error and exit flow

Script engines return `ScriptRunResult`.

```rust
type ScriptRunResult = Result<ScriptExit, ScriptError>;
```

The script thread wrapper must catch Rust panics and convert them into
`ScriptError::Panic`. On `ScriptExit::RestartRequested`, the winit thread joins
the old script thread, clears the restart/frame state, restores the temporary
demo graphics state and target FPS, and spawns a fresh `E::default()` while
keeping the window and render host alive. On script errors, the winit thread
converts script errors into `WindowError`, logs them through `tracing`, requests
`event_loop.exit()`, and returns the fatal error through the binary
`anyhow::Result` path after `run_app()` exits.

This keeps panic, Ruby exception, normal script completion, restart, and shutdown
on one host-owned path.

## Current implementation

Implemented in `crates/mkxp-window/src/script_host.rs`:

- `ScriptEngine` owns the script entry point.
- `ScriptContext` hides `Arc<SharedRuntime>` and any host notification handles.
- `DemoScriptEngine` contains the old demo loop and calls
  `ctx.with_graphics(...)` plus `ctx.submit_frame_and_wait()`.
- `spawn_script_thread` owns thread spawn, panic capture, result recording, and
  the final `RuntimeEvent::ScriptExited` wakeup.

`App<E>` still owns winit, graphics bootstrap, `SharedRuntime`,
`WindowController`, and shutdown/drop ordering. The binary selects the engine
type at startup:

```rust
let mut app = App::<DemoScriptEngine>::new(proxy, runtime_config);
```

Each script launch uses `E::default()`, so restart can create a fresh engine
instance without changing the window/render host. `ScriptContext::config()`
exposes the same runtime config to script engines without requiring engine
construction arguments.

## Remaining migration work

1. Keep all script host types private to `mkxp-window` until Ruby bindings need
   them across crate boundaries.
2. Replace the binary's `App::<DemoScriptEngine>` selection with the real Ruby
   engine type when `mkxp-binding` owns the real RGSS runtime.
3. When audio/fs/input bindings land, promote only stable service handles to
   `mkxp-runtime` or a similarly named crate.

The goal is that replacing the demo with the real script engine changes engine
construction, not the frame synchronization or render host architecture.
