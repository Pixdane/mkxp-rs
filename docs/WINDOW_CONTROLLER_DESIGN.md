# WindowController 架构设计

本文是窗口控制层的架构规范。窗口行为细节见
[`WINDOW_CONSTRAINTS.md`](WINDOW_CONSTRAINTS.md)，frame loop 和线程协作见
[`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md)。

## 目标

`WindowController` 把窗口、菜单、快捷键、全屏状态和 resize 策略从 `App`
中隔离出来。外部宿主只关心它输出的结果事件，不需要理解菜单勾选、
`Fit`、整数倍缩放、宽高比锁定或 pending resize 的内部规则。

本阶段不引入通用 service registry，也不把脚本、音频、文件系统等子系统塞进
同一个窗口 trait。窗口控制层只负责平台窗口和窗口策略。

## 当前模块图

```text
binary main
  calls mkxp_gui::run_demo()

mkxp_gui::run_demo()
  loads config
  initializes logging
  creates EventLoop<RuntimeEvent>
  creates App<DemoScriptEngine>
  enters winit event loop

App<E: ScriptEngine> / winit main thread
  owns WindowController
  owns SharedRuntime
  owns script/render JoinHandle
  owns RenderCommand sender
  translates WindowOutput into RenderCommand or lifecycle actions

WindowController
  owns winit::Window
  owns muda menu and menu event receiver
  owns modifier/fullscreen/aspect/pending-resize state
  emits WindowOutput

SharedRuntime
  owns RuntimeConfig
  owns Mutex<GraphicsState>
  owns FrameSync
  owns RuntimeControl
  owns script/render outcome slots

render thread
  waits on FrameSync
  drains RenderCommand
  mutates GraphicsState for resize/viewport/render

script thread
  owns E
  mutates script-facing GraphicsState
  blocks at ScriptContext::submit_frame_and_wait()
```

## 所有权边界

`WindowController` 创建并持有 `winit::Window`。`App` 在 controller 创建后借用
`controller.window()` 创建 wgpu surface，然后把 surface、device、queue
放入 `GraphicsState`。controller 不创建 wgpu 资源，也不持有 `GraphicsState`。

```text
WindowController
  owns window and platform UI

GraphicsState
  owns wgpu::Surface
  owns wgpu::Device / Queue
  owns surface config and viewport computation

App
  owns both
  guarantees shutdown and drop order
```

这个边界让窗口策略和渲染后端保持单向通信：窗口线程输出事件，render thread
在下一帧前应用渲染命令。

## 生命周期和 Drop 顺序

`wgpu::Surface` 逻辑上依赖 `winit::Window`。因此 shutdown 必须保证
`GraphicsState` 和 render thread 先结束，`WindowController` 和 window 后 drop。

当前宿主使用显式 shutdown：

```text
App::exiting / QuitRequested / fatal error
  RuntimeControl::request_shutdown()
  FrameSync::wake_all()
  send RenderCommand::Shutdown
  join script thread
  join render thread
  drop SharedRuntime / GraphicsState
  drop WindowController / Window
```

不要只依赖 `JoinHandle` drop；它会 detach 线程。窗口销毁前必须 join
script/render 线程，避免后台线程继续访问 surface 或 graphics state。

## WindowController 内部状态

controller 内部维护所有窗口策略状态：

```rust
WindowController {
    window,
    menu,
    menu_receiver,
    menu_items,
    aspect_locked,
    fullscreen_scale_mode,
    resize_requests,
    modifiers,
    game_size,
    window_mode,
    enable_reset,
}
```

这些状态不应散落在 `App`：

- `aspect_locked`
- `fullscreen_scale_mode`
- `ResizeRequestTracker`
- menu item/checkmark handles
- modifier key状态
- 全屏和宽高比修正规则
- reset/restart 菜单启用状态
- 从 `RuntimeConfig.game_size` 传入的逻辑游戏尺寸

纯策略 helper，例如 `fit_aspect_size`、`window_scale_mark`、`classify_resize`
和 `ResizeRequestTracker`，必须能用普通单元测试覆盖，不依赖真实窗口或菜单。
这些 helper 必须接收 `game_size` 参数，不应读取窗口模块内的全局游戏尺寸常量。

## 输入

controller 接收 winit 和菜单输入：

```rust
WindowController::on_window_event(event)
WindowController::on_about_to_wait()
```

它在内部消费窗口级命令：

- `Alt+Enter`：切换全屏
- `F12`：当 reset 启用时请求脚本 restart
- `Game > Restart`：当 reset 启用时请求脚本 restart
- `Game > Quit` 或关闭窗口：请求退出
- `Screen > Fit` / `1x`-`4x` / `Lock Aspect Ratio`：更新窗口或 viewport 策略

普通游戏输入不在本层解释。未来接入 input service 时，controller 可以把未消费的
键盘、鼠标、手柄输入转发给 input 子系统；这不应改变窗口控制边界。

## 输出

controller 输出结果事件：

```rust
WindowOutput::SurfaceResized { width, height }
WindowOutput::ViewportScaleModeChanged(mode)
WindowOutput::RestartRequested
WindowOutput::QuitRequested
```

它不输出“请帮我 resize 窗口”这种内部策略命令。程序化 resize、全屏切换和菜单
勾选都是 controller 自己的副作用。

`SurfaceResized` 必须携带真实窗口尺寸，而不是修正目标尺寸。macOS live resize
和程序化 resize 都可能让窗口短暂停在 off-ratio 尺寸；render thread 必须立即
用真实尺寸 reconfigure surface，避免 surface config 过期。

`ViewportScaleModeChanged` 只表达渲染 viewport 状态变化：

- 全屏 `Fit` -> `ViewportScaleMode::Fit`
- 全屏 `1x`-`4x` -> `ViewportScaleMode::Integer(n)`
- 从全屏回到窗口 -> 恢复 `ViewportScaleMode::Fit`

窗口模式下的 `Fit` 和 `1x`-`4x` 主要改变窗口 inner size，不改变 graphics
viewport mode。

## App 协作

`App` 是 winit `ApplicationHandler`，负责生命周期和跨线程编排：

```text
window_event / about_to_wait
  WindowController -> Vec<WindowOutput>
  App translates output:
    SurfaceResized -> RenderCommand::SurfaceResized
    ViewportScaleModeChanged -> RenderCommand::ViewportScaleModeChanged
    RestartRequested -> RuntimeControl::request_restart()
    QuitRequested -> shutdown + event_loop.exit()

user_event
  ScriptExited -> inspect ScriptExit / ScriptError
  RenderExited -> inspect RenderError
```

`App` 不直接调用 `GraphicsState::update()`，也不在 winit main thread 执行每帧
render。渲染只在 render thread 发生。

## Restart 边界

restart 是脚本生命周期事件，不是窗口生命周期事件。

当 controller 输出 `RestartRequested` 时：

```text
App
  sets RuntimeControl.restart
  resets FrameSync pending frame
  wakes blocked script/render waiters

script thread
  returns ScriptExit::RestartRequested from Graphics.update boundary

App
  joins old script thread
  clears restart flag and old script outcome
  resets script-owned demo graphics state and target FPS from RuntimeConfig
  spawns E::default() as a fresh script engine
```

restart 不重建 `WindowController`、winit window、wgpu surface 或 render thread。
窗口尺寸、全屏状态、菜单状态和 viewport scale mode 仍由窗口层持有。

## 非目标和未来扩展

当前非目标：

- 不在 controller 内创建 wgpu device/surface。
- 不让 script thread 直接操作 winit window。
- 不把普通游戏输入实现为窗口命令。
- 不在窗口文档里规定 Ruby binding、audio 或 file system 的内部实现。

未来可能添加：

- `WindowOutput::Input(...)`，转发未消费输入到 input service。
- script-originated window commands，例如改标题、居中、显示/隐藏光标。
- 更小的 `WindowPolicy` 纯策略类型，用于进一步隔离菜单和 winit 依赖。
