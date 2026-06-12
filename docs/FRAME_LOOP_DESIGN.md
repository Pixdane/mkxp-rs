# Frame Loop 架构设计

本文定义 `mkxp-window` 当前 frame loop 的线程模型。窗口行为见
[`WINDOW_CONTROLLER_DESIGN.md`](WINDOW_CONTROLLER_DESIGN.md) 和
[`WINDOW_CONSTRAINTS.md`](WINDOW_CONSTRAINTS.md)。

## 背景

旧设计把每帧 render 放在 winit main thread。macOS 快速切换输入源
（例如 `CapsLock` 或 `Cmd+Space`）时，AppKit/IMK 可能让 main thread 长时间不回到
`ApplicationHandler`，导致 script thread 已经提交 frame，却要等待数百到数千毫秒
才被 main thread 观察到。

诊断曾出现：

```text
script frame was ready before the main loop observed it script_wait_ms=5072
```

问题不是 `wgpu` present，也不是 `request_redraw()` 交付后太慢；问题是 main thread
可能暂时完全不进入 winit handler。当前架构因此把每帧 render 驱动移到独立
render thread。

## 目标

- 保持 RGSS 语义：`Graphics.update` 阻塞到当前 frame 被 render，shutdown/restart
  时返回对应动作。
- winit main thread 只拥有窗口、菜单和平台 UI。
- render thread 直接等待 `FrameSync`，不依赖 `UserEvent`、`AboutToWait` 或
  `RedrawRequested` 作为每帧驱动。
- resize、fullscreen、viewport mode 等窗口输出在下一帧 render 前生效。
- shutdown 必须显式 signal 并 join script/render thread。
- 未来 Ruby engine 只替换 `ScriptEngine`，不重写 window/render/frame loop。

## 非目标

- script thread 不 present `wgpu::Surface`。
- 窗口策略不放进 `mkxp-graphics`。
- 不创建通用 service registry。
- 不把 macOS 输入源切换暴露成游戏输入事件。
- 不依赖用户避免系统输入法快捷键。

## 线程模型

```text
winit main thread / App<E>
  owns WindowController
  owns SharedRuntime
  owns script/render JoinHandle
  owns RenderCommand sender
  handles RuntimeEvent

script thread
  owns E: ScriptEngine
  mutates script-facing GraphicsState before Graphics.update
  calls ScriptContext::submit_frame_and_wait()
  blocks until render thread completes or control requests shutdown/restart

render thread
  owns frame timing
  waits on FrameSync
  drains RenderCommand
  calls GraphicsState::update()
  wakes script after render
```

## SharedRuntime

`SharedRuntime` 是 script/render/main 三方共享边界：

```text
SharedRuntime
  config: Arc<RuntimeConfig>
  graphics: Mutex<GraphicsState>
  frame_sync: FrameSync
  script_outcome: ScriptOutcomeSlot
  render_outcome: RenderOutcomeSlot
  control: RuntimeControl
```

`GraphicsState` 暂时放在 `Mutex` 后面。正常 frame flow 仍由 `FrameSync` 保证
script mutation 和 render mutation 不并发：

```text
script updates game state
script mutates GraphicsState
script marks frame ready and blocks
render drains window commands
render mutates GraphicsState for resize/viewport
render presents frame
render marks frame finished
script continues
```

winit main thread 不直接改 `GraphicsState`；它把会影响 render 的窗口输出转成
`RenderCommand`。

## RenderCommand

```rust
enum RenderCommand {
    SurfaceResized { width: u32, height: u32 },
    ViewportScaleModeChanged(ViewportScaleMode),
    Shutdown,
}
```

映射关系：

```text
WindowOutput::SurfaceResized
  -> RenderCommand::SurfaceResized

WindowOutput::ViewportScaleModeChanged
  -> RenderCommand::ViewportScaleModeChanged

WindowOutput::QuitRequested
  -> App::initiate_shutdown()
  -> RenderCommand::Shutdown
  -> event_loop.exit()

WindowOutput::RestartRequested
  -> RuntimeControl::request_restart()
  -> FrameSync::reset() + wake_all()
```

render thread 每帧至少 drain 两次 command：一次在 frame ready 后、一次在 FPS gate
等待后。这样 resize/fullscreen 命令即使在等待帧率门期间到达，也能在 present 前
应用。

## FrameSync

`FrameSync` 用一个 mutex state 和 condvar 同步 script/render：

```rust
FrameSyncState {
    ready: bool,
    ready_at: Option<Instant>,
}
```

关键操作：

```text
script_frame_ready_and_wait(control) -> ScriptFrameAction
wait_for_ready_or_shutdown(control) -> FrameWait
render_finished()
reset()
wake_all()
```

script side：

```text
set ready = true
record ready_at
wake render waiter
wait while ready and no shutdown/restart
return Continue / Shutdown / Restart
```

render side：

```text
wait until ready and not restart, or shutdown
return Ready { ready_at } / Shutdown
```

restart 时 render side 不消费旧 frame；`FrameSync::reset()` 清掉 pending frame，
script side 观察到 `Restart` 并退出当前 engine。

## Render Timing

render thread 拥有 `next_frame_at`。时间线按固定帧间隔推进，而不是从上一帧结束后
再等待完整 interval。

```text
loop:
  wait for script-ready frame or shutdown
  drain render commands
  sleep until next_frame_at if needed
  drain render commands again
  if shutdown: exit
  if restart: reset FrameSync and continue
  GraphicsState::update()
  FrameSync::render_finished()
  next_frame_at = advance_frame_deadline(next_frame_at, frame_duration, now)
```

语义：

- 正常 frame 目标是 `t`, `t + frame_duration`, `t + 2 * frame_duration`。
- 小抖动不永久拖慢时间线。
- 大 stall 丢弃历史债务，不 burst-render 追赶旧 frame。
- `Graphics.update` 仍阻塞到计划 frame 被 present。

如果 target FPS 变化，应从当前时间重建后续节奏，避免新旧 frame duration 混用。

## App 职责

`App<E>` 是 `ApplicationHandler<RuntimeEvent>`：

```text
resumed
  create WindowController
  create wgpu Instance / Surface / Adapter / Device / Queue
  create SharedRuntime
  create RenderCommand channel
  spawn script thread with E::default()
  spawn render thread

window_event / about_to_wait
  route events to WindowController
  translate WindowOutput

user_event
  ScriptExited -> inspect script outcome
  RenderExited -> inspect render outcome

exiting
  explicit shutdown and join
```

`App` 的泛型参数决定当前脚本引擎类型：`App<DemoScriptEngine>` 用 demo engine，
未来 `App<RubyScriptEngine>` 应只替换 script engine 边界。

## Script Host

`ScriptEngine` 是脚本运行边界：

```rust
trait ScriptEngine: Default + Send + 'static {
    fn run(self, ctx: ScriptContext) -> ScriptRunResult;
}
```

restart 创建新 engine 实例：

```text
old engine returns ScriptExit::RestartRequested
App joins old script thread
SharedRuntime::prepare_script_restart()
spawn_script_thread(E::default(), ...)
```

因此 engine 内部状态必须属于 engine 实例；跨 restart 保留的状态必须显式放到
runtime/config/input 等共享对象里。

## Restart Flow

```text
F12 / menu restart
  WindowController emits RestartRequested

App
  RuntimeControl::request_restart()
  FrameSync::reset()
  FrameSync::wake_all()

script thread
  submit_frame_and_wait() returns Restart
  engine returns ScriptExit::RestartRequested
  records outcome
  sends RuntimeEvent::ScriptExited

App::handle_script_exit
  joins old script thread
  clears restart flag and script outcome
  resets pending frame
  restores script-owned demo graphics state and target FPS from RuntimeConfig
  spawns fresh E::default()
```

restart 不重建 window、surface、device、render thread 或菜单状态。

## Error Flow

script thread：

```text
Ok(ScriptExit::Finished)
Ok(ScriptExit::ShutdownRequested)
Ok(ScriptExit::RestartRequested)
Err(ScriptError)
panic -> ScriptError::Panic
```

script outcome 记录在 `SharedRuntime`，再通过 `RuntimeEvent::ScriptExited` 回到
winit main thread。

render thread：

```text
GraphicsState::update() fatal SurfaceError
panic
  -> RenderError
  -> SharedRuntime render outcome
  -> RuntimeControl::request_shutdown()
  -> RuntimeEvent::RenderExited
```

`ApplicationHandler` 回调不能直接返回 `Result`，因此 fatal error 先记录到 outcome
slot 或 `App::fatal_error`，最后由 `run_demo()` 在 `run_app()` 返回后转成
`anyhow::Result`。

## Shutdown 顺序

```text
App::exiting / fatal error / QuitRequested
  RuntimeControl::request_shutdown()
  FrameSync::wake_all()
  send RenderCommand::Shutdown
  join script thread
  join render thread
  drop SharedRuntime / GraphicsState
  drop WindowController / winit Window
```

这个顺序是 surface/window lifetime 的安全边界：`GraphicsState` 必须先于
`WindowController` drop。

## 验证策略

自动验证：

```text
cargo fmt -p mkxp-window --check
cargo test -p mkxp-window
cargo test -p mkxp-window --doc
cargo check -p mkxp-window
cargo clippy -p mkxp-window --all-targets -- -D warnings
cargo doc -p mkxp-window --no-deps
git diff --check
```

手动 smoke：

- 快速切换 macOS 输入源至少 30 秒，确认动画不出现多秒停顿。
- resize 窗口，确认 aspect lock、防抖和真实 surface size 行为正确。
- 切换 fullscreen/windowed，确认 `Fit` 和整数倍菜单勾选正确。
- 使用 `F12` 和 `Game > Restart`，确认 script 重启但窗口和 render host 不重建。

## 已完成迁移记录

早期实现计划曾包含 “从 `main.rs` 创建 render host”、“移动 frame timing”、
“把 `WindowOutput` 路由为 `RenderCommand`” 等任务。当前代码已经完成这些迁移：

- binary 入口已变为薄 `main.rs`。
- `mkxp_window::run_demo()` 是 library 入口。
- `App<E>` 持有 winit lifecycle。
- `render_host.rs` 持有 render loop 和 frame timing。
- `script_host.rs` 持有 `ScriptEngine` 边界。
- `runtime.rs` 持有 `SharedRuntime`、config、control 和 outcome slots。
