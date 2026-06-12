# WindowController 模块设计

## 目标

`WindowController` 用来把窗口、菜单、快捷键和 resize 策略从 binary
入口的 `App` 中抽离出来。抽离后，外部不再直接维护窗口缩放相关状态，
也不再理解 `Fit`、整数倍缩放、宽高比锁定、全屏菜单状态和 pending resize
之间的细节。

本设计只抽象窗口控制层，不在本阶段引入通用 runtime service registry，
也不把脚本引擎、文件系统、音频等子系统纳入同一个模块 trait。

相关行为规格见 [`WINDOW_CONSTRAINTS.md`](WINDOW_CONSTRAINTS.md)。本文档只描述
模块边界、持有关系和通信方式。

## 设计原则

- `WindowController` 拥有窗口控制策略和平台菜单实现。
- `GraphicsState` 继续拥有 wgpu surface、device、queue 和渲染 viewport 计算。
- `App` 或未来的 `RuntimeHost` 只做生命周期和事件转发。
- 窗口 resize 请求是窗口控制器的内部副作用，不暴露给 runtime 再绕回来执行。
- graphics 必须继续接收真实 surface 尺寸。即使窗口短暂处于 off-ratio，
  wgpu surface 也要立即同步真实尺寸。
- 窗口模式下的 `Fit` 是一次性窗口命令；全屏模式下的 `Fit` 是 graphics
  viewport 状态。

## 模块关系

```text
App / RuntimeHost
  owns:
    WindowController
    Runtime or GraphicsState

WindowController
  owns:
    winit::Window
    muda::Menu
    menu receiver
    menu items
    window constraint state

GraphicsState
  owns:
    wgpu::Surface
    wgpu::Device
    wgpu::Queue
    wgpu::SurfaceConfiguration
    viewport state
```

`WindowController` 创建并持有 `winit::Window`。外部 bootstrap 在 controller
创建后通过 `controller.window()` 借用窗口来创建 wgpu surface：

```rust
let window_controller = WindowController::new(event_loop, config)?;

let instance = wgpu::Instance::new(...);
let surface = instance.create_surface(window_controller.window())?;
let adapter = request_adapter(&instance, &surface)?;
let (device, queue) = request_device(&adapter)?;

let graphics = GraphicsState::new(
    device,
    queue,
    surface,
    surface_config,
    GAME_W,
    GAME_H,
    DEFAULT_FPS,
);
```

`WindowController` 不创建 wgpu 资源，也不持有 `GraphicsState`。这样窗口控制
和渲染后端保持单向通信，避免窗口模块依赖 graphics 的锁、线程和渲染生命周期。

## 生命周期和 Drop 顺序

`wgpu::Surface` 逻辑上依赖 `winit::Window`。如果 `WindowController` 持有
window，而 `GraphicsState` 持有 surface，则宿主结构必须保证 graphics 先于
window drop。

推荐显式 shutdown；如果依赖字段 drop 顺序，则把 `graphics` 字段声明在
`window` 字段之前，让 graphics 先 drop：

```rust
struct RuntimeHost {
    graphics: Option<GraphicsState>,
    window: Option<WindowController>,
}

impl RuntimeHost {
    fn shutdown(&mut self) {
        self.graphics.take();
        self.window.take();
    }
}
```

显式 `take()` 更清楚，也更适合未来添加 audio、script 等 subsystem shutdown。

## WindowController 内部状态

`WindowController` 维护窗口控制所需的全部状态：

```rust
pub struct WindowController {
    window: Window,
    menu: Menu,
    menu_receiver: Receiver<MenuEvent>,
    menu_items: MenuItems,

    aspect_locked: bool,
    fullscreen_scale_mode: FullscreenScaleMode,
    resize_requests: ResizeRequestTracker,
    modifiers: ModifiersState,
    game_size: PhysicalSize<u32>,
}
```

这些状态不再散落在 `App` 中：

- `aspect_locked`
- `fullscreen_scale_mode`
- `ResizeRequestTracker`
- menu item/checkmark handles
- modifier key state
- resize correction policy

窗口控制器内部可以继续拆出更小的纯策略类型，例如：

```rust
struct WindowPolicy {
    aspect_locked: bool,
    fullscreen_scale_mode: FullscreenScaleMode,
    resize_requests: ResizeRequestTracker,
}
```

这样 `WindowPolicy` 可以用普通单元测试覆盖，不需要真实窗口或菜单。

## 输入

`WindowController` 接收来自 winit 和菜单系统的输入。第一阶段可以保留贴近
winit 的方法，避免先设计过泛的 input abstraction：

```rust
impl WindowController {
    pub fn on_window_event(&mut self, event: WindowEvent) -> Vec<WindowOutput>;
    pub fn poll_menu_events(&mut self) -> Vec<WindowOutput>;
    pub fn on_about_to_wait(&mut self) -> Vec<WindowOutput>;
}
```

内部可再转换成更小的 typed input：

```rust
enum WindowInput {
    Resized { width: u32, height: u32 },
    CloseRequested,
    KeyboardPressed(KeyCode),
    ModifiersChanged(ModifiersState),
    MenuCommand(WindowMenuCommand),
    AboutToWait,
}

enum WindowMenuCommand {
    Fit,
    IntegerScale(u32),
    ToggleAspectLock,
    Restart,
    Quit,
}
```

窗口级快捷键由 `WindowController` 消费：

- `Alt+Enter`：切换全屏
- `F12`：请求脚本重启/reset

普通游戏输入不应被窗口控制器解释为窗口命令。未来接入 input service 后，
窗口控制器可以把未消费的键盘输入作为 runtime input event 输出。

## 输出

`WindowController` 输出给 runtime 的是结果事件，不输出“请帮我 resize 窗口”
这种内部策略命令：

```rust
pub enum WindowOutput {
    SurfaceResized { width: u32, height: u32 },
    ViewportScaleModeChanged(ViewportScaleMode),
    Input(InputEvent),
    RestartRequested,
    QuitRequested,
}
```

第一阶段如果尚未接入 input service，可以先省略 `Input`。

`SurfaceResized` 必须使用真实窗口尺寸，而不是修正目标尺寸。原因是 macOS
live resize 和程序化 resize 都可能让窗口短暂停留在 off-ratio 尺寸；graphics
必须立即同步真实 surface，避免内容被错误拉伸或 surface config 过期。

`ViewportScaleModeChanged` 只表达 graphics viewport 状态变化：

- 全屏菜单 `Fit` -> `ViewportScaleMode::Fit`
- 全屏菜单 `1x`-`4x` -> `ViewportScaleMode::Integer(n)`
- 从全屏切回窗口 -> 通常恢复 `ViewportScaleMode::Fit`

窗口模式菜单 `Fit` 和 `1x`-`4x` 主要通过改变窗口尺寸完成，不需要切换
graphics viewport mode。

## 副作用边界

`WindowController` 内部直接执行这些副作用：

- `window.request_inner_size(...)`
- `window.set_fullscreen(...)`
- `menu_item.set_checked(...)`
- 从 menu receiver drain 菜单事件

这些不应交给 runtime 处理。否则 runtime 仍然需要理解窗口约束策略，
`WindowController` 的封装就会漏出来。

Runtime 只解释输出事件。当前目标 frame-loop 中，窗口线程不直接调用
`GraphicsState`；它把会影响渲染的输出转换成 `RenderCommand`，由 render host
在下一帧前应用：

```rust
fn translate_window_output(
    output: WindowOutput,
    render_tx: &Sender<RenderCommand>,
) -> Result<(), RenderCommandError> {
    match output {
        WindowOutput::SurfaceResized { width, height } => {
            render_tx.send(RenderCommand::SurfaceResized { width, height })?;
        }
        WindowOutput::ViewportScaleModeChanged(mode) => {
            render_tx.send(RenderCommand::ViewportScaleModeChanged(mode))?;
        }
        WindowOutput::QuitRequested => {
            // handled by App because it owns ActiveEventLoop
        }
        WindowOutput::RestartRequested => {
            // handled by App because it owns script thread lifecycle
        }
        WindowOutput::Input(event) => {
            // future InputService
        }
    }

    Ok(())
}
```

`QuitRequested` 是否直接调用 `event_loop.exit()` 可由宿主决定。推荐
`WindowController` 输出 `QuitRequested`，由 `App` 调用 `ActiveEventLoop::exit()`，
因为 `ActiveEventLoop` 是 winit 生命周期参数，不需要被 controller 长期持有。

## 事件流

### 手动 resize

```text
WindowEvent::Resized(1100, 800)
  -> WindowController
     observe pending resize
     if aspect_locked and windowed and off-ratio:
       request_inner_size(1067, 800, Coalesced)
     refresh menu checkmarks
  -> output SurfaceResized(1100, 800)
  -> RenderCommand::SurfaceResized(1100, 800)
  -> render host applies GraphicsState::on_resize before the next frame
```

注意输出的是 `1100x800`，不是修正目标 `1067x800`。

### 菜单 2x，窗口模式

```text
MenuCommand::IntegerScale(2)
  -> WindowController
     request_inner_size(1280, 960, Explicit)
     refresh menu checkmarks
  -> no graphics output until Resized arrives

WindowEvent::Resized(1280, 960)
  -> output SurfaceResized(1280, 960)
  -> RenderCommand::SurfaceResized(1280, 960)
  -> render host applies GraphicsState::on_resize before the next frame
```

窗口模式下整数倍缩放不是 graphics viewport 状态。它只是一次性窗口 resize
命令。

### 菜单 3x，全屏模式

```text
MenuCommand::IntegerScale(3)
  -> WindowController
     fullscreen_scale_mode = Integer(3)
     refresh menu checkmarks
  -> output ViewportScaleModeChanged(Integer(3))
  -> RenderCommand::ViewportScaleModeChanged(Integer(3))
  -> render host applies GraphicsState::set_viewport_scale_mode before the next frame
```

全屏模式下整数倍缩放是 graphics viewport 状态。

### 退出全屏

```text
Alt+Enter while fullscreen
  -> WindowController
     window.set_fullscreen(None)
     refresh menu checkmarks
  -> output ViewportScaleModeChanged(Fit)
```

窗口模式下 `Fit` 菜单项不会因为 `ViewportScaleMode::Fit` 打勾。窗口模式
`Fit` 是命令，不是状态。

## App / RuntimeHost 形状

抽象后，宿主只负责生命周期 glue：

```rust
struct App {
    window: Option<WindowController>,
    render_tx: Sender<RenderCommand>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = WindowController::new(event_loop, WindowConfig::default())?;
        let graphics = create_graphics(window.window())?;
        let (render_tx, render_rx) = channel();
        spawn_render_thread(runtime, render_rx);

        self.window = Some(window);
        self.render_tx = render_tx;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        let outputs = self.window.as_mut().unwrap().on_window_event(event);
        self.apply_window_outputs(event_loop, outputs);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let outputs = self.window.as_mut().unwrap().on_about_to_wait();
        self.apply_window_outputs(event_loop, outputs);
    }
}
```

这里的 `create_graphics(window.window())` 是 bootstrap 代码，不属于
`WindowController`。它可以暂时留在 binary crate 中，未来再移动到
`RuntimeHost` 或 `WgpuBootstrap`。

## 测试策略

优先测试纯策略，再测试 controller glue。

纯策略测试：

- `aspect_locked` 窗口 resize off-ratio 时产生 `Coalesced` 修正请求。
- pending 自动修正期间，显式菜单命令可以覆盖 pending。
- 全屏时切换 `Lock Aspect Ratio` 不请求 windowed fit。
- 窗口模式 `Fit` 不产生 checkmark 状态。
- 窗口模式 `1x`-`4x` 按真实尺寸同步 checkmark。
- 全屏模式 `Fit` 和 `Integer(n)` 按 `fullscreen_scale_mode` 同步 checkmark。

controller glue 测试可以通过小型 fake/window adapter 完成。如果真实
`winit::Window` 难以在单元测试中构造，第一阶段可以让 `WindowPolicy`
返回 `WindowSideEffect`，由 `WindowController` 负责应用：

```rust
enum WindowSideEffect {
    RequestInnerSize(u32, u32, ResizeRequestMode),
    SetFullscreen(bool),
    SetMenuChecked(MenuItemId, bool),
}
```

这样大部分行为可以无窗口测试，真实 controller 只保留薄薄的平台适配层。

## 非目标

本阶段不做这些事：

- 不引入通用 `Module` trait。
- 不设计全局 service locator。
- 不把 filesystem、script、audio 注册进 WindowController。
- 不让子系统直接持有 `muda::MenuItem` 或 `winit::Window`。
- 不让 WindowController 创建 wgpu device/surface。
- 不改变 graphics 的固定游戏坐标系。

## 未来可扩展方向

### Runtime services

当文件系统、音频、脚本引擎、input 都接入后，可以新增 runtime 层：

```text
Runtime
  owns:
    FileSystem
    GraphicsState
    AudioManager
    InputState
    ScriptEngine
```

推荐先使用强类型 `Runtime` struct 或 `RuntimeBuilder`，不要过早抽象成动态
registry：

```rust
let runtime = RuntimeBuilder::new(config)
    .with_file_system(file_system)
    .with_graphics(graphics)
    .with_audio(audio)
    .with_script_engine(script)
    .build()?;
```

这种方式能清楚表达初始化顺序和依赖关系。Rust 中过早使用
`Arc<Mutex<dyn Service>>`、`Any` 或 service locator 容易让生命周期、
错误传播和借用边界变复杂。

### Script bindings

脚本引擎可以有自己的 binding 注册系统：

```rust
script.register_bindings(GraphicsBindings::new(...));
script.register_bindings(AudioBindings::new(...));
script.register_bindings(FileSystemBindings::new(...));
script.register_bindings(InputBindings::new(...));
```

这和 `WindowController` 的菜单注册不是同一层抽象。脚本 bindings 面向 RGSS
API；窗口菜单面向平台 UI。

### Menu contributions

未来如果 debug 工具、脚本热重载或文件系统工具需要菜单，可以设计菜单贡献：

```rust
pub struct MenuContribution {
    id: CommandId,
    label: String,
    shortcut: Option<Shortcut>,
    checkable: bool,
    group: MenuGroup,
}
```

模块声明菜单项，`WindowController` 创建真实 `muda` 菜单，并把点击转换成：

```rust
WindowOutput::Command(CommandId)
```

Runtime 再把命令分发给脚本、文件系统或 debug service。模块不直接接触
`muda`、`winit` 或 `Window`。

这个扩展应在实际出现第二个菜单贡献者时再做。当前先把窗口级菜单和快捷键
收进 `WindowController`，保持实现小而明确。

## 当前实现状态

当前 `mkxp-window` 同时提供 library entry 和 thin binary。窗口控制边界已经落在
`crates/mkxp-window/src/window_control.rs`：

- `WindowController` 持有 `winit::Window`、`muda::Menu`、menu receiver、
  menu items、modifier 状态、宽高比锁定状态、全屏 scale mode 和 resize
  request tracker。
- `WindowController` 不持有 `GraphicsState`，也不创建 wgpu resource。
- `App` 负责 wgpu bootstrap、winit 事件转发和生命周期 glue。它把会影响渲染的
  `WindowOutput` 转换为 `RenderCommand`，不再直接调用 `GraphicsState` 的
  resize、viewport 或 render 方法。
- `render_host.rs` 持有 render command receiver 和固定时间轴，负责在 render
  thread 中应用 `SurfaceResized`、`ViewportScaleModeChanged` 并执行每帧
  `GraphicsState::update()`。
- `App` 在退出时设置 shutdown、唤醒 `FrameSync`、发送 `RenderCommand::Shutdown`，
  并显式 join script/render 线程，避免后台线程继续持有 `GraphicsState` 到
  `WindowController` drop 之后。
- `WindowOutput::SurfaceResized` 使用真实窗口尺寸；自动宽高比修正仍是
  controller 内部副作用。
- `window_mode` 由 `window.fullscreen()` 的平台状态统一同步。Alt+Enter 和
  macOS 原生全屏/退出全屏入口都收敛到同一套菜单勾选和 viewport 输出逻辑。
- 纯策略测试随 `window_control.rs` 编译，覆盖 resize tracker、fit 计算、
  windowed integer scale mark 和 resize 分类。

尚未单独抽出 `WindowPolicy`/`WindowSideEffect`。当前实现先保留轻量 helper
函数；等需要无窗口测试完整状态机转移时，再引入更明确的纯策略类型。

## 建议实施顺序

1. ✅ 新增 `window_control.rs`，移动纯函数和状态类型。
2. ✅ 新增 `WindowController`，持有 `Window`、`Menu`、receiver 和 menu items。
3. ✅ 让 `App` 将 `WindowOutput` 转换为 `RenderCommand`。
4. ✅ 保留 wgpu bootstrap 在 `App` 中，通过 `controller.window()` 创建 surface。
5. ✅ 新增 render host thread，由它应用 render commands 并执行 frame render。
6. 后续按需抽出 `WindowPolicy`/`WindowSideEffect`，覆盖更多无窗口状态机测试。
7. 后续再考虑 `RuntimeHost`、`RuntimeBuilder` 和菜单贡献系统。
