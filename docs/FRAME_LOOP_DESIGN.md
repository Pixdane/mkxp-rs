# mkxp-rs 帧循环设计：脚本阻塞，winit 渲染

本文档定义 mkxp-rs 中脚本线程、winit 事件循环和 graphics 渲染之间的帧同步协议。
当前 `mkxp-window` demo 已经按这个形状运行；未来接入 magnus/RGSS 时，应把 demo
thread 替换成真实脚本线程，而不是改变帧同步模型。

相关边界见：

- [`WINDOW_CONTROLLER_DESIGN.md`](WINDOW_CONTROLLER_DESIGN.md)：窗口、菜单、全屏、
  resize 和 `WindowOutput` 的职责边界。
- [`WINDOW_CONSTRAINTS.md`](WINDOW_CONSTRAINTS.md)：窗口缩放、整数倍、全屏状态和
  菜单状态的行为规格。
- [`SCRIPT_HOST_DESIGN.md`](SCRIPT_HOST_DESIGN.md)：脚本线程替换接口、
  `ScriptContext` 和未来 runtime/binding crate 边界。

## 背景

mkxp-z 的游戏逻辑以 RGSS 脚本循环为中心：

```text
RGSS 线程
  loop do
    Graphics.update
    Input.update
    $scene.update
  end

事件线程
  SDL_WaitEvent
  处理窗口、输入和平台事件
```

`Graphics.update` 对脚本而言是“画面更新了一帧”的同步点。mkxp-z 内部通过
OpenGL 上下文切换、事件线程和 RGSS 线程之间的同步原语完成这一点。

mkxp-rs 不能把渲染直接放进脚本线程：

- `winit::EventLoop` 必须占用主线程，并且平台窗口事件必须由它驱动。
- `wgpu::Surface`、窗口 resize、present 和平台生命周期需要收敛在 winit 主线程。
- Ruby/MRI 需要在独立脚本线程中运行 RGSS loop。

因此 mkxp-rs 的兼容目标是：脚本仍然自然调用 `Graphics.update`，但
`Graphics.update` 不直接渲染。它通知 winit“脚本帧已经准备好”，然后阻塞，
直到 winit 主线程完成实际渲染。

## 当前方案

```text
script thread
  update game state
  write graphics state
  Graphics.update / FrameSync::script_frame_ready_and_wait
    set ready = true
    send RuntimeEvent::ScriptFrameReady
    block on Condvar
  when script engine returns:
    store ScriptRunResult
    send RuntimeEvent::ScriptExited

winit main thread
  EventLoop<RuntimeEvent>::run_app
  window_event / about_to_wait / user_event
    apply WindowOutput first
    if ready && now >= next_frame_at:
      GraphicsState::update
      ready = false
      wake script thread
      next_frame_at = now + frame_duration
```

脚本线程仍然是游戏逻辑的外层循环；winit 主线程仍然是平台事件循环的唯一主体。
二者在 `Graphics.update` 边界严格交替：

1. 脚本线程推进一帧游戏状态。
2. 脚本线程在 `Graphics.update` 设置 `ready = true`，发送 winit user event，然后阻塞。
3. winit 主线程先处理窗口输出和 resize，再在 FPS 门控允许时调用 `GraphicsState::update()`。
4. winit 主线程完成 present 后设置 `ready = false`，唤醒脚本线程。
5. 脚本线程从 `Graphics.update` 返回，进入下一帧游戏逻辑。

## 线程和所有权

```text
App / winit main thread
  owns:
    WindowController
    SharedRuntime
    demo/script JoinHandle

SharedRuntime
  owns:
    Mutex<GraphicsState>
    FrameSync
    script_outcome
    shutdown

WindowController
  owns:
    winit::Window
    menu state
    window mode / resize policy

GraphicsState
  owns:
    wgpu::Surface
    wgpu::Device
    wgpu::Queue
    viewport state

script thread
  borrows SharedRuntime through Arc
  writes graphics state
  blocks at FrameSync
```

`WindowController` 和 `GraphicsState` 不直接互相持有。`App` 是 glue 层：

- `WindowController` 输出 `WindowOutput`。
- `App` 把 `SurfaceResized` 和 `ViewportScaleModeChanged` 应用到 `GraphicsState`。
- `App` 在渲染前处理窗口输出，保证 resize/fullscreen 状态先进入 graphics。
- `App` 退出时先设置 `shutdown`，唤醒并 join 脚本线程，再 drop runtime 和 window。

`JoinHandle` 的 drop 只会 detach 线程，不会自动停止线程。必须显式 join，避免脚本线程
在窗口和 surface teardown 期间继续持有或访问 `GraphicsState`。

## FrameSync

当前同步原语是一个 bool 加一个 `Condvar`：

```rust
struct FrameSync {
    ready: Mutex<bool>,
    cv: Condvar,
}
```

语义：

- `ready = false`：脚本线程可以继续运行，winit 没有待渲染脚本帧。
- `ready = true`：脚本线程已经到达 `Graphics.update` 并阻塞，winit 应渲染一帧。

脚本侧：

```rust
fn script_frame_ready_and_wait(shutdown, wake_event_loop) -> bool {
    ready = true;
    wake_event_loop();      // send RuntimeEvent::ScriptFrameReady
    cv.notify_one();

    while ready && !shutdown {
        cv.wait();
    }

    !shutdown
}
```

winit 侧：

```rust
fn render_if_script_ready() {
    if script_outcome {
        handle script exit on main thread;
        return;
    }

    if !ready || now < next_frame_at {
        return;
    }

    graphics.update();
    ready = false;
    cv.notify_one();
    next_frame_at = now + frame_duration;
}
```

`Condvar` 负责让脚本线程睡眠和被唤醒。它不能可靠唤醒已经停在
`ControlFlow::WaitUntil` 的 winit 事件循环，所以脚本侧还必须发送
`RuntimeEvent::ScriptFrameReady`。这对 macOS 尤其重要：如果只依赖
`AboutToWait + WaitUntil`，在没有鼠标/窗口事件时可能出现刷新不及时。

## User Event 和 FPS 门控

`RuntimeEvent::ScriptFrameReady` 的作用只是唤醒 winit，让主线程尽快观察到
`ready = true`。它不是“立即渲染”的许可。

实际渲染仍由 `next_frame_at` 控制：

```text
if ready && Instant::now() >= next_frame_at:
  render
else:
  keep waiting until next_frame_at
```

这个分层很重要：

- 没有 user event：winit 在某些平台上可能睡到下一次输入/窗口事件才发现脚本 ready。
- user event 直接触发渲染：脚本线程会在每次 ready 后立刻被释放，游戏速度可能超过
  `Graphics.frame_rate` / `target_fps`。
- user event + `next_frame_at`：winit 能及时醒来，但仍保持稳定帧率。

`schedule_next_wake()` 用 `ControlFlow::WaitUntil(wake_at)` 安排下一次唤醒。
如果下一帧时间已经过了，就使用当前 `target_fps` 计算一个新的 wake time，避免忙等。

## Present Mode

当前 `mkxp-window` 默认使用：

```rust
present_mode: wgpu::PresentMode::Fifo
```

理由：

- `Fifo` 是跨平台的垂直同步 present mode，行为最接近默认 vsync。
- macOS 上 `Immediate` 曾让刷新看起来更主动，但在全屏整数缩放后通过系统方式退出全屏时，
  边缘可能出现 tearing。
- 帧率稳定性由 `next_frame_at` 管，显示同步由 `Fifo` 管，二者职责不同。

未来接入 `mkxp-config.graphics.vsync` 时，可以把 present mode 变成配置驱动。
在那之前，默认保持 `Fifo`，优先保证画面稳定和跨平台一致性。

## `Graphics.update` 绑定语义

未来 Ruby 绑定层应把 `Graphics.update` 实现为同步点，而不是直接调用 renderer：

```rust
fn graphics_update(runtime: Arc<SharedRuntime>) -> magnus::Value {
    if !runtime.frame_sync.script_frame_ready_and_wait(&runtime.shutdown, || {
        let _ = proxy.send_event(RuntimeEvent::ScriptFrameReady);
    }) {
        // raise/return through the chosen shutdown path
    }

    Qnil.into()
}
```

对 RGSS 脚本的可见语义保持不变：`Graphics.update` 返回时，上一帧画面已经由
winit 主线程提交。脚本不需要知道内部使用了 winit user event、wgpu present 或 condvar。

## 输入处理

输入事件由 winit 主线程捕获。推荐模型：

```text
winit window_event
  update shared InputState

Ruby Input.update
  read shared InputState snapshot
  compute trigger/repeat/release state
```

这和 mkxp-z 的共享 key state 思路一致。窗口级快捷键由 `WindowController` 消费；
普通游戏输入应进入未来的 input service，不应混进窗口控制策略。

目前窗口级快捷键只保留：

- `Alt+Enter`：切换全屏。

全屏状态必须以平台真实状态为准。Alt+Enter、菜单项和 macOS 原生全屏/退出全屏都应通过
同一套 `window_mode` 同步路径更新菜单勾选和 viewport 输出。

## Resize 和窗口输出顺序

窗口事件发生在 winit 主线程。每次 `window_event` 和 `about_to_wait` 都先让
`WindowController` 处理事件，再把输出应用到 graphics：

```text
WindowEvent::Resized(w, h)
  -> WindowController
  -> WindowOutput::SurfaceResized { width: w, height: h }
  -> GraphicsState::on_resize(w, h)
```

如果窗口锁定宽高比，自动修正 resize 是 `WindowController` 内部副作用；
`SurfaceResized` 仍然输出真实 surface 尺寸，而不是修正目标尺寸。这样即使窗口短暂处于
off-ratio，wgpu surface 也不会落后于平台真实大小。

渲染时序：

```text
winit wake
  drain/apply WindowOutput
  render_if_script_ready
  schedule_next_wake
```

因此 resize、全屏切换和 viewport mode 变化会在下一次 render 之前进入 graphics。

## Panic、退出和重置

脚本线程退出结果不应只停在后台线程里。脚本入口返回 `ScriptRunResult`：

```rust
type ScriptRunResult = Result<ScriptExit, ScriptError>;
```

线程结束时会把结果写入 `script_outcome`，并发送 `RuntimeEvent::ScriptExited` 唤醒
winit 主线程。主线程统一消费结果：

```text
ScriptExit::Finished
  -> log info
  -> event_loop.exit()

ScriptExit::ShutdownRequested
  -> log info
  -> event_loop.exit()

ScriptError::Message(...)
  -> convert to WindowError
  -> log/display error
  -> event_loop.exit()
```

脚本线程 panic 也走同一条路径。当前 demo thread 用 `catch_unwind` 捕获 panic：

```text
script panic
  -> record ScriptError::Panic(message)
  -> shutdown = true
  -> wake FrameSync
  -> send ScriptExited

winit next tick
  -> take script_outcome
  -> convert to WindowError::ScriptPanic
  -> log/display panic message
  -> event_loop.exit()
```

这样主线程会展示脚本错误并走正常程序收尾，而不是让后台线程静默结束。因为
`ApplicationHandler` 回调不能返回 `Result`，错误会暂存在 `App.fatal_error`；
`run_app()` 返回后，binary 入口再把它作为 `anyhow::Result` 返回。

正常退出：

```text
winit QuitRequested / event loop exiting
  -> shutdown = true
  -> FrameSync::wake_all()
  -> join script thread
  -> drop SharedRuntime / GraphicsState
  -> drop WindowController / winit Window
```

未来 F12 reset 可以作为独立 control signal：

```text
winit detects reset shortcut
  -> reset = true
  -> wake script if needed

script checks at Graphics.update boundary
  -> raise Reset
  -> rgss_main handles reset and restarts script entry
```

reset 不应绕过 `Graphics.update` 边界直接打断 renderer。

## 与 mkxp-z 的对比

| 项目 | mkxp-z | mkxp-rs |
|---|---|---|
| 平台事件循环 | SDL event thread | winit main thread |
| 脚本线程 | RGSS thread | Ruby/demo script thread |
| `Graphics.update` | 同步并触发渲染 | 设置 ready、发送 user event、阻塞 |
| 渲染线程 | OpenGL context 可切换 | winit main thread owns wgpu render/present |
| 唤醒机制 | SDL/user messages + sync points | `EventLoopProxy` + `Condvar` |
| 帧率控制 | Graphics frame rate | `next_frame_at` + `target_fps` |
| 显示同步 | OpenGL/driver vsync | `wgpu::PresentMode::Fifo` |
| 窗口 resize | SDL event thread | `WindowController` -> `WindowOutput` -> `GraphicsState` |

## 当前实现状态

当前落地位置：

- `crates/mkxp-window/src/main.rs`
  - `RuntimeEvent::ScriptFrameReady`
  - `FrameSync`
  - `SharedRuntime`
  - `App::render_if_script_ready`
  - `App::schedule_next_wake`
  - `run_demo_script`
- `crates/mkxp-window/src/window_control.rs`
  - `WindowController`
  - `WindowOutput`
  - `window_mode` 同步和窗口级命令处理
- `crates/mkxp-graphics/src/`
  - `GraphicsState::update`
  - viewport scale modes
  - resize/surface reconfigure

当前 demo 线程不是最终脚本引擎，但它刻意模拟真实 RGSS 运行形状：

```text
mutate graphics state
Graphics.update boundary
block until render
continue next script frame
```

因此后续接入 magnus 时，应优先替换 `run_demo_script` 和 binding 层，而不是重写
winit/graphics 的帧循环。

## 测试策略

已覆盖的关键行为：

- `FrameSync` 会阻塞脚本线程直到 render finished。
- `shutdown` 会释放阻塞中的脚本线程。
- 脚本正常结束、脚本错误和 panic payload 会被记录成 `ScriptRunResult` 并只消费一次。
- `WindowController` 覆盖 resize tracker、窗口模式同步、全屏进入/退出和菜单勾选状态。
- `mkxp-graphics` 覆盖 fixed game coordinate 和 viewport scale mode 计算。

后续接入真实 Ruby 后应补充：

- `Graphics.update` 绑定在 render 完成前不会返回。
- Ruby exception 能转成 `ScriptError::Message` 并进入主线程统一展示/退出路径。
- reset signal 在 `Graphics.update` 边界抛出，并不会破坏 graphics/window 状态。
- 无输入、鼠标不动、窗口不动时，脚本 ready 仍能通过 user event 驱动稳定渲染。
- 不同平台上默认 `Fifo` present mode 的窗口/全屏切换行为一致。

## 非目标和暂缓项

当前设计不做这些事：

- 不让脚本线程直接持有或 present `wgpu::Surface`。
- 不在 `WindowController` 中创建或持有 `GraphicsState`。
- 不把普通游戏输入解释为窗口命令。
- 不引入通用 runtime service registry。
- 不启用场景图双缓冲。

双缓冲可以作为未来性能优化：

```text
script writes back scene
winit renders front scene
Graphics.update swaps front/back
```

代价是一帧显示延迟和更复杂的资源生命周期。RPG Maker 典型场景下，当前严格交替模型更简单，
也更接近 `Graphics.update` 作为同步点的脚本语义。
