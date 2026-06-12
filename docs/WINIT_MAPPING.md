# winit 对照 mkxp-z：迁移参考

本文是 mkxp-z/SDL 到 winit/wgpu 的迁移对照表，不是当前架构规范。当前窗口架构以
[`WINDOW_CONTROLLER_DESIGN.md`](WINDOW_CONTROLLER_DESIGN.md)、
[`WINDOW_CONSTRAINTS.md`](WINDOW_CONSTRAINTS.md) 和
[`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md) 为准。

## 定位

mkxp-z 使用 SDL event thread、SDL user event、OpenGL context 和 Ruby thread。
mkxp-rs 当前使用：

- winit main thread：窗口、菜单、平台 UI、winit lifecycle。
- render thread：frame timing、`wgpu::Surface` present、`GraphicsState::update()`。
- script thread：`ScriptEngine`、script-facing graphics mutation、
  `Graphics.update` 阻塞语义。
- `WindowController`：窗口策略和菜单命令。
- `SharedRuntime`：graphics、frame sync、runtime control、outcome slots。

## 启动阶段

| mkxp-z | mkxp-rs / winit |
|---|---|
| `Config::read(argc, argv)` | `mkxp_config::load()` -> `RuntimeConfig` |
| `SDL_Init(VIDEO|GAMECONTROLLER|TIMER)` | `EventLoop::new()`；winit 负责平台初始化 |
| `SDL_CreateWindow(...)` | `WindowController::new(...)` 创建 winit window 和平台菜单 |
| `initGL(win, conf)` | wgpu `Instance -> Surface -> Adapter -> Device + Queue` |
| `SDL_GL_GetDrawableSize` | wgpu surface configuration 使用 physical size |
| `alcOpenDevice` | 未来接 `mkxp-audio`，不属于窗口层 |
| `SDL_CreateThread(rgssThreadFun)` | `spawn_script_thread(E::default(), ...)` |
| `eventThread.process(rtData)` | `event_loop.run_app(&mut App<E>)` |

wgpu surface 从 `WindowController::window()` 创建，但 `WindowController` 不持有
wgpu 资源。`GraphicsState` 放在 `SharedRuntime` 中，并由 render thread 使用。

## 窗口事件

### Resize

mkxp-z：

```cpp
case SDL_WINDOWEVENT_SIZE_CHANGED:
    SDL_GL_GetDrawableSize(win, &drwW, &drwH);
    windowSizeMsg.post(Vec2i(winW, winH));
    drawableSizeMsg.post(Vec2i(drwW, drwH));
```

mkxp-rs 当前路径：

```text
WindowEvent::Resized
  -> WindowController::on_window_event
  -> WindowOutput::SurfaceResized { actual width, actual height }
  -> App translates to RenderCommand::SurfaceResized
  -> render thread applies GraphicsState::on_resize
```

注意：窗口约束修正可以在 controller 内触发 `request_inner_size`，但输出给 render
层的尺寸始终是真实 window inner size。

### Close

mkxp-z：

```cpp
case SDL_WINDOWEVENT_CLOSE:
    terminate = true;
```

mkxp-rs：

```text
WindowEvent::CloseRequested
  -> WindowOutput::QuitRequested
  -> App::initiate_shutdown()
  -> event_loop.exit()
```

### Focus 和输入

mkxp-z 在 focus lost 时清空输入状态，避免卡键。mkxp-rs 未来接 input service 时应
保留同样语义：

```text
WindowEvent::Focused(false)
  -> input.reset_all()
```

当前窗口层只消费窗口级快捷键；普通游戏输入仍是后续 input service 范围。

## 键盘和菜单命令

mkxp-z：

```cpp
if (Alt+Enter) toggleFullscreen();
if (F12) rqReset.set();
```

mkxp-rs：

```text
Alt+Enter
  -> WindowController toggles fullscreen

F12 / Game > Restart
  -> WindowOutput::RestartRequested
  -> App requests runtime restart

Game > Quit / CloseRequested
  -> WindowOutput::QuitRequested
```

reset 可以通过配置关闭；关闭后 F12 和菜单 restart 不应产生 restart 输出。

## SDL User Event 对照

mkxp-z 中 Ruby thread 通过 `SDL_PushEvent()` 请求窗口操作。winit 侧不能让非主线程
直接操作 window；窗口平台操作仍必须回到 winit main thread。

当前已实现的路径：

| mkxp-z 概念 | mkxp-rs 当前路径 |
|---|---|
| window resize notification | `WindowOutput::SurfaceResized -> RenderCommand::SurfaceResized` |
| fullscreen toggle | `WindowController` 在 winit main thread 直接调用 `window.set_fullscreen` |
| viewport scale change | `WindowOutput::ViewportScaleModeChanged -> RenderCommand` |
| reset | `WindowOutput::RestartRequested -> RuntimeControl::request_restart` |
| terminate | shutdown control + `RenderCommand::Shutdown` + join threads |

未来 script-originated window requests 需要显式设计 command 通道，例如改标题、
居中、显示/隐藏光标、message box。不要把 “winit 直接函数调用” 理解成
script thread 可以直接操作 window。

## 帧渲染

| mkxp-z | mkxp-rs |
|---|---|
| RGSS thread 调 `Graphics::update()` | script thread 调 `ScriptContext::submit_frame_and_wait()` |
| `fpsLimiter.delay()` | render thread 持有 fixed frame timeline |
| `SDL_GL_SwapWindow` | render thread 调 `GraphicsState::update()` / `present()` |
| SDL event loop 可参与 redraw | winit main thread 不驱动每帧 render |

早期草案尝试在 `AboutToWait` 或 `RedrawRequested` 中 render。macOS 输入源快速切换
会让 main thread 长时间不回到 handler，因此当前设计改为独立 render thread。

## 退出阶段

| mkxp-z | mkxp-rs |
|---|---|
| `terminate = true` | `RuntimeControl::request_shutdown()` |
| `rqTerm.set()` | `FrameSync::wake_all()` + `RenderCommand::Shutdown` |
| `SDL_WaitThread(rgssThread)` | join script thread |
| destroy GL/window | join render thread，drop `SharedRuntime`，再 drop `WindowController` |

`JoinHandle` 不能只 drop；必须 join。`GraphicsState`/surface 必须先于 window drop。

## 当前清单

```text
启动阶段：
  ✓ 加载 Config 并生成 RuntimeConfig
  ✓ 初始化 logging
  ✓ 创建 EventLoop<RuntimeEvent>
  ✓ 创建 WindowController
  ✓ 创建 wgpu Instance / Surface / Adapter / Device / Queue
  ✓ 创建 SharedRuntime
  ✓ 创建 RenderCommand channel
  ✓ 启动 script thread
  ✓ 启动 render thread

事件循环阶段：
  ✓ WindowEvent -> WindowController
  ✓ SurfaceResized -> RenderCommand::SurfaceResized
  ✓ ViewportScaleModeChanged -> RenderCommand::ViewportScaleModeChanged
  ✓ QuitRequested -> shutdown + event_loop.exit()
  ✓ RestartRequested -> runtime restart flow
  □ 普通 keyboard/mouse input service
  □ gamepad input service
  □ script-originated window commands

退出阶段：
  ✓ request shutdown
  ✓ wake FrameSync
  ✓ send RenderCommand::Shutdown
  ✓ join script thread
  ✓ join render thread
  ✓ drop graphics/runtime before window
```

## winit 不需要复刻的 SDL 机制

| mkxp-z | mkxp-rs 处理方式 |
|---|---|
| `EventThread::allocUserEvents()` | 不复刻 SDL user event；使用 typed Rust channels/events |
| `windowSizeMsg` / `drawableSizeMsg` | resize 经 `WindowOutput` 和 `RenderCommand` 到 render host |
| `SDL_GL_MakeCurrent` | wgpu device/queue/surface 由 render host 使用 |
| `IMG_Init` / `TTF_Init` / `Sound_Init` | 分别由 Rust crate 或未来子系统处理 |
| SDL controller API | 未来使用 `gilrs` 或独立 input backend |
