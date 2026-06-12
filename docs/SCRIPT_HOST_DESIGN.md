# Script host design

This document records the current script host boundary. It is the internal
`mkxp-window` interface for replacing the demo script thread with a real
RGSS/Ruby engine without leaking winit internals into the script layer.

## Decision

Do not create a standalone `mkxp-scripts` crate yet.

Keep the script host abstraction inside `mkxp-window`, in
`crates/mkxp-window/src/script_host.rs`. The current window binary still owns
winit, `EventLoopProxy`, `FrameSync`, `GraphicsState`, and shutdown ordering,
so an internal module is the honest boundary.

`mkxp-scripts` is also an imprecise name: it could mean script files,
`Scripts.rxdata` loading, RGSS bytecode, or scripting runtime. When the boundary
is stable, the more likely crates are:

- `mkxp-runtime`: subsystem handles and script-facing runtime services.
- `mkxp-binding`: Ruby/MRI lifecycle and RGSS API bindings.

## Non-goals

- Do not expose `Arc<SharedRuntime>` to script engines as the public interface.
- Do not expose `EventLoopProxy<RuntimeEvent>` or `RuntimeEvent` to script engines.
- Do not introduce a generic service registry, `Any`, or `Arc<Mutex<dyn Service>>`.
- Do not guess audio, filesystem, or input APIs before their real bindings need them.

## Interface shape

The host exposes a small `ScriptEngine` trait:

```rust
trait ScriptEngine: Send + 'static {
    fn run(self: Box<Self>, ctx: ScriptContext) -> ScriptRunResult;
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
spawn_script_thread(Box::new(DemoScriptEngine), runtime, proxy);
spawn_script_thread(Box::new(RubyScriptEngine::new(...)), runtime, proxy);
```

## ScriptContext

`ScriptContext` is the script-facing facade. Internally it may hold
`Arc<SharedRuntime>` and `EventLoopProxy<RuntimeEvent>`, but those fields should
stay private.

Initial methods:

```rust
impl ScriptContext {
    fn is_shutdown_requested(&self) -> bool;
    fn with_graphics<T>(&self, f: impl FnOnce(&mut GraphicsState) -> T) -> T;
    fn graphics_update(&self) -> bool;
}
```

`graphics_update()` is the important boundary: it sets the frame-ready flag,
sends `RuntimeEvent::ScriptFrameReady`, blocks on `FrameSync`, and returns
`false` when shutdown was requested. A real Ruby `Graphics.update` binding
should call this method rather than directly touching `FrameSync` or winit.

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
`ScriptError::Panic`. The winit thread then converts script errors into
`WindowError`, logs them through `tracing`, requests `event_loop.exit()`, and
returns the fatal error through the binary `anyhow::Result` path after
`run_app()` exits.

This keeps panic, Ruby exception, normal script completion, and shutdown on one
host-owned path.

## Current implementation

Implemented in `crates/mkxp-window/src/script_host.rs`:

- `ScriptEngine` owns the script entry point.
- `ScriptContext` hides `Arc<SharedRuntime>` and
  `EventLoopProxy<RuntimeEvent>`.
- `DemoScriptEngine` contains the old demo loop and calls
  `ctx.with_graphics(...)` plus `ctx.graphics_update()`.
- `spawn_script_thread` owns thread spawn, panic capture, result recording, and
  the final `RuntimeEvent::ScriptExited` wakeup.

`App` still owns winit, graphics bootstrap, `SharedRuntime`, `WindowController`,
and shutdown/drop ordering. It only chooses which engine to spawn:

```rust
spawn_script_thread(Box::new(DemoScriptEngine), runtime, proxy);
```

## Remaining migration work

1. Keep all script host types private to `mkxp-window` until Ruby bindings need
   them across crate boundaries.
2. Replace `DemoScriptEngine` construction with `RubyScriptEngine::new(...)`
   when `mkxp-binding` owns the real RGSS runtime.
3. When audio/fs/input bindings land, promote only stable service handles to
   `mkxp-runtime` or a similarly named crate.

The goal is that replacing the demo with the real script engine changes engine
construction, not the winit frame loop.
