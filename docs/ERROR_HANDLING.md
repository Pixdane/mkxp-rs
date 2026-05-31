# 错误处理策略

## mkxp-z 怎么做

mkxp-z 是 C++ 项目，遵循经典的 C 风格：所有错误都是字符串。没有错误类型、没有枚举、没有异常层级。

整个代码库就三种模式：

### 1. 致命初始化错误 → SDL 弹窗退出

```cpp
// main.cpp
if (SDL_Init(...) < 0) {
    showInitError("Error initializing SDL: " + std::string(SDL_GetError()));
    return 0;
}
```

`showInitError` 弹一个 SDL 消息框然后退出。一个字符串承载全部信息。

### 2. RGSS 线程错误 → 字符串塞进共享内存 + 终止

```cpp
static void rgssThreadError(RGSSThreadData *rtData, const std::string &msg) {
    rtData->rgssErrorMsg = msg;
    rtData->ethread->requestTerminate();
    rtData->rqTermAck.set();
}
```

唯一的异常类（`Exception`，在 `src/util/exception.h`）就包了一个 `std::string msg`。没有子类，没有错误码。

### 3. 配置解析错误 → 静默吞掉

```cpp
try {
    ret = json::parse5(Encoding::convertString(cfg));
} catch (const std::exception &e) {
    Debug() << "Failed to parse " << path << ": " << e.what();
    // 什么都不做，用默认值继续
}
```

### 总结

```
所有错误 = 字符串
所有处理 = 弹窗 / 打日志 / 吞掉
没有 Result<>、Either、错误枚举、错误链
```

这能跑是因为 mkxp-z 是一次性启动的桌面游戏运行时——要么初始化成功，要么进程退出。不存在需要从错误中恢复的业务逻辑，也没有调用方需要区分"文件不存在"和"格式错误"来走不同分支。

## mkxp-rs 怎么做

Rust 的类型系统让我们能做更好的设计。整个 workspace 采用**三层错误模型**。

### 设计原则

分层模型的核心思想很简单：**不同的抽象层级应该有不同的错误粒度**。

- 在最底层，`MkxpError` 只描述"发生了什么类型的错误"——I/O 错了、解析错了、初始化错了。它不关心错误在哪个 crate 发生。
- 在 crate 层，每个 crate 定义**自己领域内**能精确描述的错误——文件没找到、目录不存在、路径逃逸。这些变体是调用方真正关心的。
- 在 binary 层，`anyhow` 把一切都吞掉。入口函数不需要 `match` 任何错误变体，只管 `?` 传播。

这样一来，下层不需要了解上层在干什么（`MkxpError` 不知道什么是"路径逃逸"），上层也不需要重新定义下层已经有的东西（`FsError` 不需要自己写一个 `Io` 变体）。

### 第一层：共享错误词汇 — `mkxp_types::MkxpError`

定义一套所有 crate 都认的通用错误类别：

```rust
#[derive(Debug, thiserror::Error)]
pub enum MkxpError {
    #[error("IO error: {0}")]
    Io(std::io::Error),         // 文件系统或 I/O 操作失败
    #[error("parse error: {0}")]
    Parse(String),              // 字节流无法解析
    #[error("init error: {0}")]
    Init(String),               // 子系统初始化失败
    #[error("runtime error: {0}")]
    Runtime(String),            // 运行时异常
    #[error("unsupported: {0}")]
    Unsupported(String),        // 暂不支持的功能或格式
}
```

它存在的唯一目的是**消灭重复**。没有它，每个 crate 都要重新定义一遍 `Io(String)`、`Parse(String)` 之类的东西。

**`Io` 为什么包 `std::io::Error` 而不是 `String`？** 因为 `std::io::Error` 比字符串能传递更多信息——它有 `kind()`（NotFound / PermissionDenied / …）、有 source chain。如果只保留 `io_error.to_string()`，这些信息就全丢了。下游想做精细化错误处理（比如"文件不存在时尝试另一个路径"）就只能靠字符串比对。

同时，`MkxpError` 实现了 `From<std::io::Error>`：

```rust
impl From<std::io::Error> for MkxpError {
    fn from(e: std::io::Error) -> Self { MkxpError::Io(e) }
}
```

这意味着任何返回 `std::io::Result` 的调用，在返回 `Result<_, MkxpError>` 的函数里可以直接 `?`：

```rust
fn read_config() -> Result<String, MkxpError> {
    let content = std::fs::read_to_string("config.ron")?;  // 自动转换
    Ok(content)
}
```

### 第二层：crate 专属错误 — `thiserror` + `#[from]`

每个 crate 定义自己的错误枚举，包含**本 crate 独有**的变体，同时用 `#[from]` 透明转发共享词汇：

```rust
// crates/mkxp-fs/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("file not found: {path}")]
    NotFound { path: String },              // fs 特有的——别处用不到
    #[error("not a directory: {path}")]
    NotADirectory { path: String },         // fs 特有的
    #[error("path escapes mount root: {path}")]
    PathEscape { path: String },            // fs 特有的
    #[error("unsupported archive format: {0}")]
    UnsupportedArchive(String),             // fs 特有的
    #[error(transparent)]
    Mkxp(#[from] MkxpError),               // 共享的：Io / Parse / Init / …
}
```

```rust
// crates/mkxp-config/src/lib.rs
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("failed to build config: {0}")]
    Build(#[from] ::config::ConfigError),   // config 特有的
    #[error("failed to parse CLI args: {0}")]
    Cli(String),                            // config 特有的
    #[error(transparent)]
    Mkxp(#[from] mkxp_types::MkxpError),   // 共享的
}
```

`#[from]` 的意思是：任何返回 `MkxpError` 的代码，在返回 `FsError`（或 `SourceError`）的函数里可以直接 `?`——转换是自动的。

### 第三层：binary 入口 — `anyhow`

`main.rs`（或独立的 binary crate）用 `anyhow::Result` 作为万能错误类型。所有 crate 的错误和所有 `MkxpError` 变体都通过 `Error` trait 自动转成 `anyhow::Error`：

```rust
fn main() -> anyhow::Result<()> {
    let config = mkxp_config::load(std::env::args().collect())?;  // SourceError
    let mut fs = mkxp_fs::FileSystem::new();
    fs.mount_dir("game")?;                                        // FsError
    // ...
    Ok(())
}
```

入口层永远不需要 `match` 错误变体。只管 `?`。

### 错误流转图

```
anyhow::Result                     ← main.rs（兜底）
    ↑
    ├── mkxp_config::SourceError   ← Build / Cli / #[from] MkxpError
    ├── mkxp_fs::FsError           ← NotFound / NotADirectory / PathEscape / … / #[from] MkxpError
    ├── mkxp_graphics::GfxError    ← (未来)
    ├── mkxp_audio::AudioError     ← (未来)
    └── mkxp_binding::BindError    ← (未来)
            ↑
    mkxp_types::MkxpError          ← Io / Parse / Init / Runtime / Unsupported
```

### 为什么不做单一 workspace 枚举？

如果把所有 crate 的错误塞进一个平铺的枚举：

- 每个 crate 都要知道无关 crate 的变体（文件系统 crate 为什么要认识图形着色器编译错误？）
- `match` 分支会炸（加一个图形错误变体，config crate 的 match 编译不过）
- 破坏封装——新 crate 增加错误变体是全 workspace 的 breaking change
- 命名冲突不可避免

分层模型让每个 crate 的错误面保持紧凑和相关，同时只在共享词汇层保留真正通用的类别。

### 错误信息的结构化程度

| 级别 | 类型 | 结构 |
|------|------|------|
| 共享 | `MkxpError::Io(std::io::Error)` | 保留 `std::io::Error` 的 kind 和 source chain |
| 共享 | `MkxpError::Parse(String)` | 简单字符串——不同的解析器报错格式差异太大，不适合统一结构化 |
| crate | `FsError::NotFound { path }` | 把路径作为独立字段，允许调用方做 pattern match 提取 |
| crate | `FsError::UnsupportedArchive(String)` | 简单字符串——archive 格式名本身就是描述信息 |

**原则**：能结构化的地方就结构化（`path` 作为独立字段），无法统一结构的地方至少保留完整信息（`std::io::Error` 不抹成字符串），真正无法分类的描述才用裸 `String`。
