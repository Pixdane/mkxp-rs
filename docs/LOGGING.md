# 日志系统设计

## 目标

- 所有 crate 用统一 API 输出日志，零学习成本
- logger 在 binary 入口初始化一次，全局生效
- 可扩展：后端、格式、过滤规则都能组合替换
- 零开销过滤：低于当前日志级别的调用不产生分配
- 与 mkxp-z 的 `Debug() << "msg"` 风格兼容，同时支持结构化字段

## 技术选型：tracing

Rust 生态有三个主要的日志/追踪方案：

| 方案 | 结构化字段 | span 支持 | 动态过滤 | 生态兼容 |
|------|-----------|----------|----------|----------|
| `log` | 否 | 否 | 仅编译期 feature flag | 最广 |
| `tracing` | 是 | 是 | `EnvFilter` 运行时切换 | wgpu、bevy 等 |
| `slog` | 是 | 否 | 手动实现 | 较少 |

选 `tracing` 的理由：
1. **wgpu 直接在 tracing span 里输出渲染管线信息**，用同一套基础设施不用 bridge
2. **span 天然匹配游戏循环概念**：`frame`、`bgm_playback`、`script_exec` 都是嵌套上下文
3. **`EnvFilter` 运行时动态过滤** —— 排查问题时 `RUST_LOG=info,mkxp_audio=trace` 比编译期 feature flag 灵活得多
4. **`tracing-subscriber` 的 Layer 组合模型**天然可扩展 —— 不需要自己造 `CompositeBackend`

## 整体架构

```
+====================================================+
|  binary (future mkxp-rs binary)                    |
|  mkxp_log::init(&config)   <- 一次性初始化         |
+====================+===============================+
                     |
+====================+===============================+
|  mkxp-log crate                                    |
|  - config -> log level mapping                     |
|  - custom Layer: mkxp-style formatting             |
|  - output target: stderr / file / custom           |
+====================+===============================+
                     | depends on
+====================+===============================+
|  tracing-subscriber (Layer + EnvFilter)            |
|  tracing (facade macros)                           |
+====================+===============================+
                     | used by
     +---------------+--------------+
     v              v              v
 mkxp-audio    mkxp-fs     mkxp-config ...
 tracing = "0.1"  (facade only, no subscriber)
```

关键分层：
- **产品 crate**（`mkxp-audio`、`mkxp-fs` 等）只依赖 `tracing` facade，不依赖 `mkxp-log`
- **`mkxp-log`** 是唯一的 subscriber 实现，只在 binary 入口链接
- 这种分离确保了：换成其他 subscriber（如 `tracing-chrome` 输出 Chrome trace）不需要改产品 crate

## 依赖清单

```toml
# crates/mkxp-log/Cargo.toml
[dependencies]
mkxp-types = { path = "../mkxp-types" }
tracing = "0.1"
tracing-subscriber = { version = "0.3", default-features = false, features = ["env-filter", "registry", "std"] }
thiserror = "2"

# 其他 crate 的 Cargo.toml
[dependencies]
tracing = "0.1"
```

`tracing-subscriber` 只在 `mkxp-log` 里依赖，产品 crate 只需要 `tracing` 的宏。

`tracing-subscriber` 只在 `mkxp-log` 里依赖，产品 crate 只需要 `tracing` 的宏。

## 公共 API

### 初始化（binary 入口调用一次）

```rust
// crates/mkxp-log/src/lib.rs

/// 日志输出目标
pub enum LogTarget {
    /// 标准错误（默认）
    Stderr,
    /// 写入文件
    File(std::path::PathBuf),
    /// 同时输出到多个目标
    Composite(Vec<LogTarget>),
}

/// 日志格式
pub enum LogFormat {
    /// 人类可读的纯文本（默认）
    /// 格式: [2026-05-31T10:30:00.123+08:00] INFO  mkxp_audio::manager: BGM start "bgm_001.ogg"
    Plain,
    /// JSON 格式，适合结构化分析
    Json,
}

/// 日志配置，通常从 mkxp_config::Config 映射而来
pub struct LogConfig {
    /// 全局最低日志级别
    pub default_level: LogLevel,
    /// 按模块名覆盖级别，例如 [("mkxp_audio", Debug), ("mkxp_graphics", Warn)]
    pub target_filters: Vec<(String, LogLevel)>,
    /// 输出目标
    pub target: LogTarget,
    /// 输出格式
    pub format: LogFormat,
}

pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// 初始化全局 logger。必须在所有 tracing 宏调用之前执行，且只能调用一次。
///
/// # Errors
/// 如果 `LogTarget::File` 指定的路径无法创建父目录，返回 `LogError::InitFailed`
pub fn init(config: LogConfig) -> Result<(), LogError>;
```

### 产品 crate 中输出日志（零配置）

```rust
// 任何 crate，只需依赖 tracing = "0.1"
use tracing::{info, warn, error, debug, trace, instrument};

// 简单消息
info!("Starting BGM playback");
warn!(filename = %path, "Audio file not found, skipping");
error!(error = %e, "Failed to initialize renderer");
debug!(?config, "Configuration after merge");
trace!(sample_count = n, "Rendered MIDI tick");

// 带 span 的嵌套上下文
#[instrument(skip(data))]
fn decode_audio(data: &[u8]) -> Result<Decoded, Error> {
    info!("Decoding audio");  // 自动带上 span 上下文
    // ...
}

// 手动 span 配合敏感操作
let frame_span = tracing::info_span!("frame", number = frame_count);
let _guard = frame_span.enter();
// 这个 scope 内所有日志自动带上 frame number
graphics.update()?;
audio.update()?;
```

### 过滤规则（EnvFilter 语法）

`tracing-subscriber` 的 `EnvFilter` 支持两种方式设置过滤：

**静态（代码中）：**
```rust
let filter = EnvFilter::new("info,mkxp_audio=debug,mkxp_graphics=warn");
```

**动态（环境变量）：**
```bash
# 默认 info 级别，audio 模块开 debug
RUST_LOG=info,mkxp_audio=debug ./mkxp-rs

# 全部 trace（非常详细，仅开发用）
RUST_LOG=trace ./mkxp-rs

# 只看错误
RUST_LOG=error ./mkxp-rs
```

规则：`RUST_LOG` 环境变量优先级 > 代码中的 filter string > `LogConfig::default_level`。

## 与 mkxp-config 的集成

`mkxp-config` 现有 `debugMode: bool`。我们不再新增字段，而是用以下规则映射：

| `debugMode` | 默认日志级别 | 说明 |
|-------------|-------------|------|
| `false` | `Info` | 正式运行：只输出关键信息 |
| `true` | `Debug` | 调试模式：额外的内部状态 |

如果以后需要更细粒度控制，可以加 `logLevel` 字段（值 `"info" | "debug" | "trace"`），`debugMode` 仍作为快速开关。

映射函数（在 `mkxp-log` 或 binary 中实现）：
```rust
impl From<&mkxp_config::Config> for LogConfig {
    fn from(config: &mkxp_config::Config) -> Self {
        LogConfig {
            default_level: if config.debugMode { LogLevel::Debug } else { LogLevel::Info },
            target_filters: vec![],  // 留给 RUST_LOG 环境变量覆盖
            target: LogTarget::Stderr,
            format: LogFormat::Plain,
        }
    }
}
```

## 自定义 Layer 实现

`mkxp-log` 的核心是一个实现 `tracing_subscriber::layer::Layer` 的 `MkxpLayer`：

```rust
struct MkxpLayer {
    writer: Mutex<Box<dyn Write + Send>>,
    format: LogFormat,
    start_time: Instant,
}

impl<S> Layer<S> for MkxpLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        // 格式化事件：时间戳 + 级别 + target + span 链 + 消息
        // 输出到 writer
    }

    fn on_new_span(&self, _attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        // 可选：创建 span 时输出
    }

    fn on_close(&self, _id: Id, _ctx: Context<'_, S>) {
        // 可选：关闭 span 时输出（如记录 span 耗时）
    }
}
```

### Plain 格式示例

```
[2026-05-31T10:30:00.123+08:00] INFO  mkxp_audio::manager: BGM playback started path="Audio/BGM/001-Battle01.ogg"
[2026-05-31T10:30:00.456+08:00] DEBUG mkxp_audio::midi: CC reset{chan=3 ctrl=7}: inserted volume reset after first note
[2026-05-31T10:30:02.789+08:00] INFO  mkxp_graphics::renderer{frame=154}: frame complete
[2026-05-31T10:30:03.012+08:00] WARN  mkxp_fs::filesystem: case mismatch requested="data/actors.rxdata" actual="Data/Actors.rxdata"
[2026-05-31T10:30:05.000+08:00] ERROR mkxp_audio::manager: failed to decode file error="Unsupported format" target="Audio/SE/click.wma"
```

格式：`[时间戳] 级别  target{span 上下文}: 消息 field=value ...`

### JSON 格式示例

```json
{"timestamp":"2026-05-31T10:30:00.123000000+08:00","level":"INFO","target":"mkxp_audio::manager","span":{"name":"bgm_play","track":0},"fields":{"path":"Audio/BGM/001-Battle01.ogg"},"message":"BGM playback started"}
```

## 使用示例：改造 mkxp-audio

### 改造前（当前代码假设存在 `println!` 或类似）

```rust
// mkxp-audio/src/manager.rs
fn play_bgm(&mut self, path: &str, volume: f32) {
    // 没有日志
    self.bgm_stream.load(path);
    // ...
}
```

### 改造后

```rust
use tracing::{info, warn, instrument};

#[instrument(skip(self), fields(path = %path, volume))]
fn play_bgm(&mut self, path: &str, volume: f32) {
    info!("BGM playback started");
    self.bgm_stream.load(path);
    // ...
}
```

`#[instrument]` 自动为这个函数创建 span，入参 `path` 自动记录为 span 字段。函数内所有日志自动嵌套在这个 span 下。

## 日志级别使用约定

| 级别 | 用途 | 示例场景 |
|------|------|----------|
| `ERROR` | 不可恢复的错误，影响用户可见功能 | 渲染器初始化失败、音频设备丢失、核心配置缺失 |
| `WARN` | 异常但可降级继续运行 | 找不到字体用默认替代、文件名大小写不匹配、非关键文件加载失败 |
| `INFO` | 关键流程节点，正常运行时也能看到 | BGM 开始/结束、渲染器就绪、窗口创建、配置加载完毕 |
| `DEBUG` | 开发/调试信息，正常运行时隐藏 | 音量计算中间值、SE 缓存命中/驱逐、MIDI CC 事件注入 |
| `TRACE` | 极度详细的内部状态，排查深层 bug 用 | 单个 MIDI tick 渲染、每个 OpenGL 调用、每帧绘制列表 |

规则：默认 `Info` 级别时，单帧不应输出超过 1-2 条日志。`Debug` 级别可能每帧几条。`Trace` 可能每帧几十条——只在开发环境开。

## 扩展点

### 自定义输出后端

如果需要在网络发送日志（远程调试）、写入 SQLite、输出到游戏内控制台等：

```rust
// 实现 Writer trait，或者直接用 make_writer
let (layer, _) = tracing_subscriber::fmt::layer()
    .with_writer(my_custom_writer)
    .with_filter(filter);
```

tracing-subscriber 的 `MakeWriter` trait 是 `Fn() -> T: Write + Send` 的抽象。任何实现 `io::Write` 的类型都能当输出目标，不需要实现自定义 trait。

### Chrome trace 输出（性能分析）

加入 `tracing-chrome` crate：

```rust
let (chrome_layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
    .file("trace.json")?
    .build();
```

输出文件可以直接在 Chrome 的 `chrome://tracing` 中打开，看到 frame-by-frame 时间线。

### 结构化事件订阅（测试 / CI）

tracing 的 `Event` 可以被任何 `Layer` 捕获。测试代码可以注册一个 `Layer` 把日志收集到 `Vec` 中验证：

```rust
#[test]
fn test_se_cache_eviction_logs_warning() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let test_layer = TestLayer::new(events.clone());
    // ... 初始化 subscriber with test_layer
    // ... 触发 SE 缓存驱逐
    assert!(events.lock().unwrap().iter().any(|e| e.contains("evicting")));
}
```



### Span 生命周期日志

`LogConfig::log_spans` 设置为 `true` 后，每个 span 的创建和关闭都会产生一条日志：

```
[22:57:45.981+08:00:00] SPAN+ WARN  mkxp_graphics::renderer{frame}: number=154
[22:57:45.981+08:00:00] INFO  mkxp_graphics::renderer{frame}: draw calls=42
[22:57:46.005+08:00:00] SPAN- WARN  mkxp_graphics::renderer{frame} dur=24.123ms
```

- `SPAN+` — span 创建，附初始字段值
- `SPAN-` — span 关闭，附 `dur=` 耗时（毫秒，通过 `span.extensions()` 存储 `Instant`）
- 默认 `false`，因为开启后会产生大量输出

格式：`[timestamp] SPAN+/- LEVEL target{span_name}{parent_chain}: field=value ... dur=X.XXXms`

## 实现路线

分两个阶段：

### Phase 1：基础框架（现在）

1. 创建 `crates/mkxp-log/`，实现：
   - `LogConfig` + `LogLevel` + `LogTarget` + `LogFormat`
   - `init()` 函数，组合 `EnvFilter` + `MkxpLayer` + subscriber
   - `MkxpLayer`：Plain 和 Json 两种格式的 `on_event` 实现
2. 更新 root `Cargo.toml` 加入 workspace member
3. 给 `mkxp-config` 的 `Config` 加 `From<Config> for LogConfig`
4. 写集成测试（stderr 输出验证）

### Phase 2：全 crate 接入（逐步）

1. 每个产品 crate 加 `tracing = "0.1"` 依赖
2. 在关键路径上加 `info!` / `warn!` / `debug!` 宏
3. 用 `#[instrument]` 注解公共函数
4. 删除零散的 `println!` / `eprintln!`

## 不应做的事

- **不要在库 crate 中设全局 subscriber** —— `tracing_subscriber::init()` 只能调用一次，必须在 binary 入口
- **不要用 `println!` 代替日志** —— 无法关闭、无法过滤、无法重定向
- **不要在热路径上输出 `trace!` 级别的格式化字符串** —— 虽然宏会跳过被过滤的调用，但一旦开启会有性能开销
- **不要跨线程在 span 外记日志期望自动带上 span 上下文** —— span 是线程局部的

## 与 mkxp-z 的对比

| 特性 | mkxp-z (`Debug()`) | mkxp-rs (`tracing`) |
|------|---------------------|----------------------|
| 级别过滤 | 无（全有或全无） | 5 级 + target 过滤 + 运行时切换 |
| 输出目标 | 仅 stderr | stderr / file / 自定义 / 组合 |
| 格式 | 纯文本 | 纯文本 / JSON / Chrome trace |
| 上下文 | 手动拼接字符串 | span 自动嵌套 |
| 结构化字段 | 无 | `path = %filename` |
| 零开销过滤 | N/A | 跳过时不求值参数 |
| 性能分析集成 | 无 | `tracing-chrome` 输出可直接在 chrome://tracing 分析 |


## 实现状态

当前 (`Phase 1`) 已完成：

| 组件 | 状态 | 说明 |
|------|------|------|
| `LogConfig` / `LogLevel` / `LogTarget` / `LogFormat` | ✓ | 完整实现 |
| `LogError` | ✓ | 三层错误模型：`AlreadySet` / `CreateDir` / `OpenFile` / `Mkxp` |
| `init()` | ✓ | 全局 subscriber 初始化，`RUST_LOG` 优先 |
| `config_from_debug_mode()` | ✓ | 从 `Config::debug_mode` 快捷映射 |
| `MkxpLayer` (Plain) | ✓ | 自定义 `tracing_subscriber::Layer`，Plain 格式 |
| `MkxpLayer` (Json) | 未实现 | 需要 `serde_json` 依赖 |
| `LogTarget::Composite` | ✓ | 构造期扁平化，每个叶子目标独立 writer |
| 时间戳格式 | ✓ | ISO 8601 with local offset |
| 单元测试 | ✓ | 18 个单元测试 + 9 个 doctest |
| Phase 2（全 crate 接入） | ✓ | 4 个 crate 已接入，详见下方「Phase 2 记录」 |

### 与设计文档的差异

1. **`LogFormat::Json`**：暂未实现。`LogFormat` 枚举已保留该变体的扩展位置。

