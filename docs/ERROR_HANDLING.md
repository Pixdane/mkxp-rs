# 错误处理策略

## 背景：mkxp-z 的做法

mkxp-z 是 C++ 项目，错误处理遵循经典的 C 风格——**所有错误都是字符串**。整个代码库没有错误类型、没有枚举、没有异常层级，就三种模式：

1. **初始化失败** → SDL 弹窗 + 退出
2. **运行期错误** → 字符串塞进共享内存 + 线程终止
3. **配置解析失败** → 静默吞掉，用默认值继续

```
所有错误 = 字符串
所有处理 = 弹窗 / 打日志 / 吞掉
```

这能跑是因为 mkxp-z 是一次性启动的桌面游戏运行时——要么初始化成功，要么进程退出。不存在需要从错误中恢复的业务逻辑，也没有调用方需要区分"文件不存在"和"格式错误"来走不同分支。

mkxp-rs 不同。Rust 的类型系统让我们能做更好的设计。

---

## 三层错误模型

```
anyhow::Result                     ← binary 入口（兜底，只管 ? 不管来源）
    ↑
    ├── mkxp_config::SourceError   ← 配置解析特有 + #[from] MkxpError
    ├── mkxp_fs::FsError           ← 文件系统特有 + #[from] MkxpError
    ├── (mkxp_graphics::GfxError)  ← 未来
    ├── (mkxp_audio::AudioError)   ← 未来
    └── (mkxp_binding::BindError)  ← 未来
            ↑
    mkxp_types::MkxpError          ← 共享词汇：Io / Parse / Init / Runtime / Unsupported
```

每一层的职责很清楚：

| 层级 | 类型 | 角色 |
|------|------|------|
| Shared | `MkxpError` | 所有 crate 都认的通用错误类别，消灭重复 |
| Crate | `FsError`, `SourceError`, ... | 本 crate 独有的错误变体 + 转发共享词汇 |
| Binary | `anyhow::Result` | 万能 `?`，从不 inspect 具体变体 |

---

## 第一层：共享词汇 — `MkxpError`

`MkxpError` 存在的唯一目的是**消灭重复**。没有它，每个 crate 都要重新定义 `Io`、`Parse` 这类通用变体。它只描述"发生了什么类型的错误"，不关心错误在哪个 crate 发生。

```rust
// crates/mkxp-types/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum MkxpError {
    #[error("IO error: {0}")]
    Io(std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("init error: {0}")]
    Init(String),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

// std::io::Result 可以直接 ? 到 MkxpError
impl From<std::io::Error> for MkxpError {
    fn from(e: std::io::Error) -> Self { MkxpError::Io(e) }
}
```

### 为什么 `Io` 包 `std::io::Error` 而不是 `String`

最初设计是 `Io(String)`——把 `io::Error` 的 display message 压成一个字符串。这丢失了关键信息：

- `io::Error::kind()` — `NotFound` / `PermissionDenied` / `WouldBlock` / …
- source chain — 底层错误原因链

没有这些，下游想做精细化错误处理就只能靠字符串比对，脆弱且不可维护。

改成 `Io(std::io::Error)` 之后，加上 `From<std::io::Error>` 的自动转换，`std::io::Result` 仍然可以直接 `?`：

```rust
fn read_config() -> Result<String, MkxpError> {
    let content = std::fs::read_to_string("config.ron")?;  // io::Error → MkxpError::Io
    Ok(content)
}
```

### 为什么 `Parse` / `Init` / `Runtime` / `Unsupported` 仍然是 `String`

这些变体的"原始错误"差异太大——`Parse` 可能来自 JSON 解析库、INI 解析库、字节流格式检测，每种库的错误类型都不同。用 `String` 是最通用的兜底方案。如果未来某个 crate 需要从这些变体中提取结构化信息（比如"是哪个字段解析失败了"），应该在该 crate 自己的错误枚举中加新变体，而不是改造共享层。

---

## 第二层：crate 专属错误

每个 crate 用 `thiserror` 定义自己的错误枚举，包含**本 crate 独有**的变体，并用 `#[from]` 透明转发共享词汇。

### mkxp-fs: `FsError`

```rust
// crates/mkxp-fs/src/error.rs

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("file not found: {path}")]
    NotFound { path: String },

    #[error("not a directory: {path}")]
    NotADirectory { path: String },

    #[error("path escapes mount root: {path}")]
    PathEscape { path: String },

    #[error("invalid path: {reason}")]
    InvalidPath { reason: String },

    #[error("unsupported archive format: {0}")]
    UnsupportedArchive(String),

    #[error(transparent)]
    Mkxp(#[from] MkxpError),
}

impl FsError {
    pub fn io(err: std::io::Error) -> Self { MkxpError::Io(err).into() }
    pub fn parse(msg: impl Into<String>) -> Self { MkxpError::Parse(msg.into()).into() }
}
```

几个设计点：

- **结构化字段**：`NotFound { path }` 而不是 `NotFound(String)`。调用方可以 pattern match 提取路径做 fallback。
- **`InvalidPath`**：独立变体而非复用 `Parse`，因为"路径不合法"和"数据格式不合法"在调用方看来是不同的恢复路径。
- **`#[error(transparent)]`**：`Mkxp` 变体不加前缀，直接透传共享层的 message。`FsError::NotFound { path: "a.png" }` 显示 `"file not found: a.png"`，而 `FsError::Mkxp(MkxpError::Io(e))` 显示 `"IO error: ..."`——没有重复包装。
- **convenience 构造器**：`FsError::io(e)` 和 `FsError::parse(msg)` 是 `MkxpError::Io/Parse → FsError` 的快捷写法，避免调用方自己写 `.into()`。

### mkxp-config: `SourceError`

```rust
// crates/mkxp-config/src/lib.rs

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("failed to build config: {0}")]
    Build(#[from] ::config::ConfigError),

    #[error("failed to parse CLI args: {0}")]
    Cli(String),

    #[error(transparent)]
    Mkxp(#[from] mkxp_types::MkxpError),
}
```

`SourceError` 最初没有 `#[from] MkxpError` 变体——当时 `load()` 函数只委托给 `config` crate，不自己做 I/O。后来加上是为未来扩展留余地：一旦 `load()` 需要自己读文件，`?` 能直接工作，不需要改 API。

---

## 第三层：binary 入口 — `anyhow`

`main.rs` 用 `anyhow::Result`。所有 crate 的 error enum 都实现了 `std::error::Error`（通过 `thiserror`），所以 `?` 自动转为 `anyhow::Error`：

```rust
fn main() -> anyhow::Result<()> {
    let config = mkxp_config::load(std::env::args().collect())?;  // SourceError
    let mut fs = mkxp_fs::FileSystem::new();
    // ...
    let data = fs.read("Data/Scripts.rxdata")?;                   // FsError
    Ok(())
}
```

入口层永远不需要 `match` 错误变体。

---

## 为什么不做一个全 workspace 的平铺枚举

如果把所有 crate 的错误塞进一个 `enum`：

- 每个 crate 要认识无关 crate 的变体（文件系统 crate 为什么需要知道图形着色器编译错误？）
- 加一个新变体是全 workspace 的 breaking change
- `match` 分支会爆炸
- 命名冲突（两个 crate 都可能想叫 `NotFound`）

分层模型让每个 crate 的错误面保持在领域范围内，共享层只放真正通用的类别。

---

## 给新 crate 加错误

如果你想新加一个 `mkxp-graphics` crate，错误类型应该长这样：

```rust
#[derive(Debug, thiserror::Error)]
pub enum GfxError {
    #[error("shader compile failed: {0}")]
    ShaderCompile(String),               // graphics 特有
    #[error("texture not power of two: {size}")]
    TextureSize { size: (u32, u32) },    // graphics 特有
    #[error(transparent)]
    Mkxp(#[from] mkxp_types::MkxpError), // 共享词汇
}
```

步骤：
1. 在 `Cargo.toml` 加 `mkxp-types` 和 `thiserror`
2. 定义自己的 `enum`，写明本 crate 独有的变体
3. 加 `Mkxp(#[from] MkxpError)` 变体
4. 所有函数返回 `Result<_, GfxError>`，直接用 `?` 传播
