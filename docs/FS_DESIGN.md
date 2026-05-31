# mkxp-fs — 虚拟文件系统设计

## mkxp-z 实现分析

mkxp-z 的文件系统基于 [PhysicsFS](https://icculus.org/physfs/)，共计 1063 行 C++（`src/filesystem/` + `src/crypto/rgssad.cpp`）。

### 核心机制

**挂载点系统（Mount Points）。** 调用 `addPath("/path/to/game")` 将游戏目录挂载到
虚拟目录树的 `/` 根。可以挂载多个目录和档案文件，PhysFS 在读取时按挂载顺序查找：

```
PHYSFS_mount("game/", "/", 1)
PHYSFS_mount("game/Data.rgssad", "/", 1)    // 加密档案也挂到根
PHYSFS_openRead("Graphics/Titles/title.png")  // 不关心文件在哪个 mount 里
```

挂载是两类来源：

- **真实目录** — `PHYSFS_mount(path, mountpoint, 1)`
- **RGSS 加密档案** — `.rgssad` / `.rgss2a` / `.rgss3a` 注册为 PhysFS archiver 插件。
  `PHYSFS_registerArchiver(&RGSS1_Archiver)` 在 `FileSystem::FileSystem()` 构造时调用。

**文件打开流程（openRead）。** mkxp-z 没有文件名→文件句柄的直接映射。`openRead` 接收
一个 `OpenHandler` 回调：PhysFS 枚举目录中所有文件名，逐个尝试匹配（前缀匹配 + 扩展名补全）。
每找到一个匹配项就调 handler 的 `tryRead`，handler 返回 true 时停止搜索。
这个设计是为了支持"同一文件名、多扩展名"的场景（RPG Maker 的资源引用有时不带扩展名）。

在启用 path cache 时，枚举改为遍历内存中的 `fileLists` map（目录→文件名列表），
避免每次都调 PhysFS 的文件系统枚举。

**path cache（大小写不敏感）。** Windows 游戏文件名大小写混用。
Linux/macOS 文件系统区分大小写。mkxp-z 在 `createPathCache()` 中遍历整个
虚拟目录树，建立 `全小写路径 → 真实大小写路径` 的映射。`openRead` 先将请求路径转小写，
再查表得到真实路径。

```cpp
// filesystem.cpp 简化逻辑
void FileSystem::createPathCache() {
    PHYSFS_enumerate("", cacheEnumCB, &data);
    // cacheEnumCB 回调中:
    //   strTolower(lowerCase);
    //   pathCache.insert(lowerCase, mixedCase);
    //   fileLists[directory].push_back(lowerFilename);
}
```

**RGSS 加密档案（`crypto/rgssad.cpp`）。** 三种格式的加密方式相同：XOR 加密。
密钥是固定的魔数，不同版本密钥不同。档案文件结构：

```
[4 bytes 魔数] [文件索引表] [文件数据区]
```

PhysFS archiver 接口要求实现 `openRead`/`read`/`seek`/`tell`/`close` 等底层 I/O 回调。
mkxp-z 将它们注册为 PhysFS archiver 后，后续文件访问对加密档案完全透明。

### 对外 API

mkxp-z 的 `FileSystem` 类暴露给引擎其他部分的方法：

| 方法 | 用途 |
|------|------|
| `addPath(path)` | 挂载目录或档案 |
| `openRead(handler, filename)` | 打开文件（扩展名补全 + 大小写不敏感） |
| `openReadRaw(ops, filename)` | 绕开扩展名补全，直接打开 |
| `exists(filename)` | 检查文件存在 |
| `createPathCache()` | 建立大小写映射 |

## Rust 实现方案对比

### 方案 A：PhysFS FFI 绑定

使用 `physfs` crate 或自己写 FFI，链接 PhysFS C 库。

```
[优点]
- 功能完整，行为和 mkxp-z 一致
- 开发快，直接复用现成的 archiver 机制

[缺点]
- 引入 C 编译依赖，破坏 pure Rust 承诺
- 不能编译到 WASM（PhysFS 依赖 POSIX）
- physfs crate 维护状态不确定（最后更新 2021）
```

### 方案 B：纯 Rust 实现（推荐）

从零实现 mount 系统 + RGSS 解码。

```
[优点]
- 零 C 依赖，编译到 WASM 无障碍
- 接口可以设计得更 Rust 惯用（Result、bytes、Read trait）
- 实现量不大（PhysFS 核心约 3000 行 C，RGSS 解码约 200 行）

[缺点]
- 需要自己实现 mount 优先级查找和 path cache
- 需要自己处理三种 RGSS 加密格式
```

### 方案 C：`include_dir` + `vfs` crate

使用 Rust 生态已有的虚拟文件系统 crate。

`include_dir` 是编译期嵌入目录树，不支持运行时 mount，不适用。
`vfs` crate 提供了 mount/read 抽象但只支持物理目录，不支持加密档案。

结论：**方案 B（纯 Rust）** 是正确选择。

## mkxp-fs API 设计

### 核心 trait

```rust
/// 可挂载到文件系统的数据源。
pub trait Mountable: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, FsError>;
    fn exists(&self, path: &str) -> bool;
    fn enumerate(&self, dir: &str) -> Result<Vec<String>, FsError>;
}
```

三个内置实现：

```rust
// 普通目录
impl Mountable for RealDirectory { ... }

// .zip 压缩包（标准库或 zip crate）
impl Mountable for ZipArchive { ... }

// .rgssad / .rgss2a / .rgss3a 加密档案
impl Mountable for RgssArchive { ... }
```

### 文件系统本体

```rust
pub struct FileSystem {
    mounts: Vec<(String, Box<dyn Mountable>)>,  // (mountpoint, source)
    path_cache: Option<HashMap<String, String>>, // 小写 → 真实路径
}

impl FileSystem {
    /// 挂载一个数据源到指定路径。
    pub fn mount(&mut self, path: &str) -> Result<(), FsError>;

    /// 打开文件并返回字节数据。自动查找第一个包含该路径的 mount 源。
    /// 查询顺序：后挂载的优先（覆写语义）。
    pub fn read(&self, path: &str) -> Result<Vec<u8>, FsError>;

    /// 检查文件是否存在。
    pub fn exists(&self, path: &str) -> bool;

    /// 枚举目录下的所有文件名。
    pub fn read_dir(&self, dir: &str) -> Result<Vec<String>, FsError>;

    /// 建立大小写不敏感的路径缓存。
    pub fn build_path_cache(&mut self) -> Result<(), FsError>;
}
```

### 设计要点

**mount 优先级。** 后挂载的源先查找。例如先 `mount("game/")` 再
`mount("game/Data.rgssad")`，加密档案中的文件会覆盖真实目录中的同名文件。

**path cache 是可选特性。** 通过 `build_path_cache()` 显式开启。开启后
`read("graphics/titles/title")` 可以匹配到 `Graphics/Titles/Title.png`。

**不实现扩展名补全。** mkxp-z 的 `openRead` 枚举目录搜索同名不同扩展名的文件。
这个行为耦合了 PhysFS 的底层回调。在 Rust 版中，由调用方（graphics/audio crate）
自己处理扩展名补全，fs 层只做精确路径匹配。

**错误类型。** 使用 `mkxp_types::MkxpError` 的 `Io` 变体，不引入新的 error enum。

### Crate 结构

```
crates/mkxp-fs/
├── Cargo.toml       # mkxp-types, zip (optional), thiserror
└── src/
    ├── lib.rs        # FileSystem struct + mount/read/exists API
    ├── mountable.rs  # Mountable trait + RealDirectory 实现
    ├── rgss.rs       # RGSS 加密档案解码 (rgssad/rgss2a/rgss3a)
    └── path_cache.rs # 大小写不敏感路径映射
```

### 依赖

- `mkxp-types` — `MkxpError`
- `zip` — zip 文件支持（可选 feature）
- `encoding_rs` — Shift_JIS 文件名编码转换（RGSS 档案中常见）
