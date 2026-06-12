# Script Host 架构设计

本文记录当前 `mkxp-window` 内部的 script host 边界。它的目标是把临时
demo script thread 替换成真正的 RGSS/Ruby engine 时，不把 winit、render host
或窗口生命周期细节泄漏到脚本层。

相关线程模型见 [`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md)。

## 设计决策

暂时不创建独立的 `mkxp-scripts` crate。

script host 抽象继续放在 `crates/mkxp-window/src/script_host.rs`。当前 window
runtime 仍然拥有 winit、`FrameSync`、`GraphicsState`、render host、shutdown/drop
顺序和 restart 流程，所以把 script host 留在 `mkxp-window` 内部是更诚实的边界。

`mkxp-scripts` 这个名字也不够精确：它可能指 script 文件、`Scripts.rxdata` 加载、
RGSS 字节码、Ruby runtime，或 RGSS API binding。等边界稳定后，更可能拆出的 crate
是：

- `mkxp-runtime`：脚本侧可见的 subsystem handle 和 runtime service。
- `mkxp-binding`：Ruby/MRI 生命周期、RGSS API 注册和 Ruby binding。

## 非目标

- 不把 `Arc<SharedRuntime>` 作为公开接口暴露给 script engine。
- 不把 `EventLoopProxy<RuntimeEvent>`、render host 内部结构或 `RuntimeEvent` 暴露给
  script engine。
- 不引入通用 service registry、`Any` 或 `Arc<Mutex<dyn Service>>`。
- 不在 audio、filesystem、input 的真实 binding 需要之前猜它们的 API。
- 不为了 demo engine 提前冻结未来 Ruby engine 的 crate 边界。

## 当前模块边界

当前实现分布在几个 `mkxp-window` 内部模块：

```text
app.rs
  owns App<E>
  owns winit lifecycle
  owns window/render/script thread handles
  chooses E: ScriptEngine
  handles restart/shutdown/error propagation

script_host.rs
  defines ScriptEngine
  defines ScriptContext
  defines DemoScriptEngine
  owns script thread spawn and panic capture

runtime.rs
  defines RuntimeConfig
  defines SharedRuntime
  owns cross-thread lifecycle flags and outcome slots

frame_sync.rs
  synchronizes Graphics.update with render thread
```

`App<E>` 是 host 侧组合根。泛型参数 `E` 决定本次启动使用哪个 script engine：

```rust
let mut app = App::<DemoScriptEngine>::new(proxy, runtime_config);
```

未来接入 Ruby 时，预期入口变成类似：

```rust
let mut app = App::<RubyScriptEngine>::new(proxy, runtime_config);
```

替换 engine 类型不应该要求重写 window bootstrap、render thread、`FrameSync` 或
shutdown 顺序。

## ScriptEngine

host 暴露的核心接口是很小的 `ScriptEngine` trait：

```rust
trait ScriptEngine: Default + Send + 'static {
    fn run(self, ctx: ScriptContext) -> ScriptRunResult;
}
```

关键约束：

- 一个 engine instance 只表示一次 script run。
- 每次 restart 都创建新的 `E::default()`。
- 每次 run 的内部状态应该放在 engine instance 里。
- 跨 restart 保留的状态必须显式放到 runtime/config/input 等共享对象里。

当前 demo engine 是：

```rust
struct DemoScriptEngine;
```

未来 Ruby engine 应该是另一个实现：

```rust
struct RubyScriptEngine;
```

script thread spawn 路径不需要知道具体 engine：

```rust
spawn_script_thread(E::default(), runtime, proxy);
```

## ScriptContext

`ScriptContext` 是脚本侧 facade。它内部持有 `Arc<SharedRuntime>`，但字段保持私有。
脚本层只能通过明确的方法访问 host 能力。

当前方法：

```rust
impl ScriptContext {
    fn is_shutdown_requested(&self) -> bool;
    fn with_graphics<T>(&self, f: impl FnOnce(&mut GraphicsState) -> T) -> T;
    fn config(&self) -> &RuntimeConfig;
    fn submit_frame_and_wait(&self) -> ScriptFrameAction;
}
```

各方法的职责：

- `is_shutdown_requested()`：让 script loop 能主动观察 terminal shutdown。
- `with_graphics(...)`：在提交 frame 之前短暂修改 script-facing `GraphicsState`。
- `config()`：读取 normalized runtime config，例如 `game_size`、`target_fps`、
  `scripts_path` 和 `rgss_version`。
- `submit_frame_and_wait()`：实现脚本侧 `Graphics.update` 边界。

`with_graphics(...)` 的 closure 必须保持短小，不能跨 `submit_frame_and_wait()` 持有
graphics mutex。render thread 在 frame ready 后也需要拿同一个 mutex 来处理 resize、
viewport mode 和 present。

## Graphics.update 边界

`submit_frame_and_wait()` 是 script host 最重要的边界。它会：

```text
script side
  mark frame ready
  wake render thread
  block until render finishes or lifecycle control changes
  return Continue / Shutdown / Restart
```

真实 Ruby `Graphics.update` binding 应该调用这个方法，而不是直接接触：

- `FrameSync`
- render command queue
- render thread 内部结构
- winit `EventLoopProxy`
- winit `RuntimeEvent`

旧的 winit-main-thread render loop 曾经依赖 `RuntimeEvent::ScriptFrameReady` 唤醒主线程
来画每一帧。当前 render-host 设计里，正常每帧唤醒走 `FrameSync` condvar，由 render
thread 直接消费。winit user event 只保留给 script exit、render exit、fatal error 和
其他明确 host notification，不用于每帧驱动。

## Restart Flow

restart 是 script-only 生命周期路径。窗口和 render host 保持存活，只替换 script
engine instance。

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
  spawn wrapper records outcome
  sends RuntimeEvent::ScriptExited

App::handle_script_exit
  joins old script thread
  SharedRuntime::prepare_script_restart()
  spawns replacement script thread with E::default()
```

`SharedRuntime::prepare_script_restart()` 负责清理 script-side restart 状态：

- 丢弃旧 script outcome。
- 清掉 restart flag。
- reset `FrameSync`。
- 把 graphics target FPS 恢复为 runtime config 的目标值。
- reset 当前 demo graphics state。

这里的 demo graphics reset 是临时实现细节。真实 Ruby engine 接入后，需要把
engine-owned state 放进 engine instance，让新的 `E::default()` 自然得到干净状态；
真正需要跨 restart 保留的状态再提升到 runtime service。

## Error And Exit Flow

script engine 返回统一的 `ScriptRunResult`：

```rust
type ScriptRunResult = Result<ScriptExit, ScriptError>;
```

非错误退出：

```rust
enum ScriptExit {
    Finished,
    ShutdownRequested,
    RestartRequested,
}
```

错误：

```rust
enum ScriptError {
    Message(String),
    Panic(String),
}
```

script thread wrapper 必须捕获 Rust panic，并转换成 `ScriptError::Panic`。真实 Ruby
exception 接入后，应转换成 `ScriptError::Message` 或更细的 script error 类型，再由
`WindowError` 统一向 binary 入口返回。

host 侧流程：

```text
script thread exits
  record ScriptRunResult in SharedRuntime
  send RuntimeEvent::ScriptExited

App::handle_script_exit
  Finished / ShutdownRequested -> initiate shutdown and event_loop.exit()
  RestartRequested -> restart script thread
  Err -> convert to WindowError, store fatal_error, shutdown, event_loop.exit()

run_demo()
  after run_app() returns, take fatal_error()
  return anyhow::Result
```

这让 Rust panic、Ruby exception、正常结束、restart 和 shutdown 都走同一条
host-owned path。

## SharedRuntime 边界

`SharedRuntime` 是 main/render/script 三方共享状态，但它不是 script engine 的公开 API。
script engine 只能通过 `ScriptContext` 间接访问它。

当前内容：

```text
SharedRuntime
  config: Arc<RuntimeConfig>
  graphics: Mutex<GraphicsState>
  frame_sync: FrameSync
  script_outcome: ScriptOutcomeSlot
  render_outcome: RenderOutcomeSlot
  control: RuntimeControl
```

`RuntimeConfig` 是由 `mkxp_config::Config` normalize 后的运行时配置。它把可选配置转换成
window host 现在实际需要的具体值：

```text
window_title
window_size
game_size
target_fps
vsync
enable_reset
scripts_path
rgss_version
```

script engine 通过 `ScriptContext::config()` 读取同一份 `RuntimeConfig`。例如 demo
engine 用 `game_size` 计算移动边界，而 `GraphicsState::new` 和 `WindowController` 也使用
同一个 `game_size`。

## Future Service Growth

`ScriptContext` 将来会自然增长出 audio、filesystem、input、config、reset/shutdown 等
service。原则是只在对应 RGSS binding 真正需要时添加：

```rust
ctx.audio().play_bgm(...);
ctx.fs().read(...);
ctx.input().snapshot();
ctx.request_reset();
ctx.request_shutdown();
```

当 context 开始代表多个 subsystem，而不再只是 window demo host 的小 facade 时，就是
考虑抽出 `mkxp-runtime` 的时机。Ruby/MRI 生命周期和 RGSS API 注册仍应放在
`mkxp-binding`，不要混进 runtime service crate。

## 当前实现状态

已实现：

- `ScriptEngine` 定义一次 script run 的入口。
- `ScriptContext` 隐藏 `Arc<SharedRuntime>`，只暴露 lifecycle/config/graphics/frame
  boundary。
- `DemoScriptEngine` 保留当前 demo loop，并通过 `ctx.with_graphics(...)` 和
  `ctx.submit_frame_and_wait()` 工作。
- `spawn_script_thread` 负责创建 thread、构造 context、捕获 panic、记录结果，并发送
  `RuntimeEvent::ScriptExited`。
- `App<E>` 用泛型选择 engine 类型。
- restart 通过 `E::default()` 创建新的 engine instance。
- `ScriptContext::config()` 让 engine 不需要构造参数也能读取运行配置。

## 剩余迁移工作

1. 在 Ruby binding 真正需要跨 crate 使用之前，继续保持 script host 类型为
   `mkxp-window` 私有。
2. 接入真实 Ruby runtime 时，用 `RubyScriptEngine` 替换 binary/library 入口处的
   `App<DemoScriptEngine>`。
3. Ruby `Graphics.update` binding 必须调用 `ScriptContext::submit_frame_and_wait()`，
   不要绕过 frame protocol。
4. audio/fs/input binding 落地时，只提升稳定 service handle 到 `mkxp-runtime` 或同等
   crate；不要提前创建通用 registry。
5. 清理 `SharedRuntime::prepare_script_restart()` 中 demo-only graphics reset，把真实
   engine state 归还给 engine instance 所有。

目标是：从 demo engine 切到真实 Ruby engine 时，只替换 engine 构造和 RGSS binding，
不重写 frame synchronization、render host 或 window lifecycle。
