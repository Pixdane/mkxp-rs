# mkxp-fs — 虚拟文件系统设计

## mkxp-z 实现分析

mkxp-z 的文件系统基于 [PhysicsFS](https://icculus.org/physfs/)，共计 ~800 行 C++
（`src/filesystem/` 692 行 + `src/crypto/rgssad.cpp` 232 行）。

### 核心机制

**挂载点系统。** `addPath("/path/to/game")` 将游戏目录或加密档案挂载到虚拟目录树
的 `/` 根。PhysFS 在读取时按挂载顺序查找，后挂载的优先。

**RGSS 加密档案。** 三种格式（rgssad / rgss2a / rgss3a）注册为 PhysFS archiver。
`FileSystem` 构造时调用 `PHYSFS_registerArchiver()` 注册。

**openRead 流程。** 这是 mkxp-z 最复杂的方法：
1. 规范化路径（反斜杠→正斜杠、`.`/`..` 折叠）
2. 如果 path cache 激活：转小写、拆分为目录+文件名
3. 用 path cache 的 `fileLists[dir]` 做前缀匹配（快路径），或无 cache 时调
   `PHYSFS_enumerate()`（慢路径）
4. **扩展名补全**：前缀匹配后检查下一字符是 `.` 还是 `\0`，决定是否命中
5. 对每个候选文件调 `OpenHandler::tryRead()`，handler 返回 true 时停止

**path cache。** 两层结构：
- `pathCache`：`小写全路径 → 原始大小写路径`
- `fileLists`：`小写目录路径 → [小写文件名列表]`（加速枚举，避免调 PhysFS）

**RGSS XOR 加密（`crypto/rgssad.cpp`）。** 不是固定 key，而是一个 LCG 序列：

```cpp
#define RGSS_MAGIC 0xDEADCAFE

static inline uint32_t advanceMagic(uint32_t &magic) {
    uint32_t old = magic;
    magic = magic * 7 + 3;
    return old;  // 返回旧值用于 XOR
}
```

每个 4 字节 dword 与 LCG 当前输出值 XOR，未对齐的剩余字节与当前 magic 的低字节 XOR。
RGSS1/2 和 RGSS3 共用同一套 `RGSS_ioRead` 解密逻辑。

**RGSS1 / RGSS2（交织格式）。** 两者共享完全相同的内部结构，仅文件扩展名不同。
Entry header 和 file data 交替排列：

```
[Header 8 bytes: "RGSSAD\0\x01"]
[Entry 1: nameLen(4)+name(n)+size(4)] ← 全部与连续 LCG XOR
[Entry 1 data: size bytes]            ← 从 data 起始 LCG 快照开始 XOR
[Entry 2: ...]
[Entry 2 data: ...]
...
[EOF 结束]
```

**RGSS3（分离格式）。** 索引用 baseKey XOR 包裹，数据有绝对偏移：

```
[Header 8 bytes: "RGSSAD\0\x03"]
[baseKey: 4 bytes, 读后做 *9+3 变换]
[Entry: offset(4)+size(4)+key(4)+nameLen(4)+name(n)] ← 全与 baseKey XOR
...
[offset=0 → 索引结束]
[数据区：各文件以绝对偏移排列]
```

## mkxp-fs 实现对照

### 设计决策

| 特性 | mkxp-z | mkxp-fs |
|------|--------|---------|
| 依赖 | PhysFS (C) | 纯 Rust，零 C 依赖 |
| 路径规范化 | 接受并折叠 `..`、反斜杠 | VPath 构造时严格拒绝不合规路径 |
| 扩展名补全 | `openRead` 自动前缀匹配 | **不实现**，由调用方自行处理 |
| 文件列表缓存 | `fileLists` 加速枚举 | 无（`Mountable::enumerate` 本身已够快） |
| I/O 模型 | streaming（seek/tell/partial read） | 全量读取（`Vec<u8>`） |
| mount 语义 | PhysFS mount 优先级 | `Vec` 逆序遍历，后挂载优先 |
| 错误处理 | C 风格：字符串 + 异常 | 三层 thiserror 模型 |

### 与 mkxp-z 行为一致的模块

| 模块 | 一致性 |
|------|--------|
| RGSS XOR | **100% 一致** — `advance_lcg` 逻辑、种子、dword 对齐/未对齐处理 |
| RGSS1 解析 | **100% 一致** — 连续 LCG 流、entry/data 交织 |
| RGSS3 解析 | **100% 一致** — baseKey XOR、absolute offset |
| path_cache 映射 | **一致** — `lower → real` 映射，同样逆序覆盖 |
| mount 优先级 | **一致** — 后挂载的源先搜索 |

### 与 mkxp-z 有意不同的模块

**`exists()` 实现。** mkxp-z 调 `PHYSFS_exists()` 判断存在性。我们最初错误地将
`exists()` 委托给 `try_read()`（读完整文件），现已修正为单独的 `try_exists()`，
仅检查 `Mountable::exists()`。

**无 `fileLists`。** mkxp-z 在 path cache 中额外保存 `directory → [lowercase filenames]`
映射，目的是避免每次 `openRead` 都调 `PHYSFS_enumerate()`。我们的 `Mountable::enumerate()`
不涉及 PhysFS 开销（`RealDirectory` 是 OS read_dir，`RgssArchive` 是 HashMap 遍历），
所以不需要额外缓存。

**无 `desensitize()`。** mkxp-z 提供单独的大小写转换公开方法。我们的 `resolve_path()`
内联了此逻辑，不单独暴露。

## 当前实现结构

```
crates/mkxp-fs/src/
├── lib.rs           (16 行)   — 模块声明 + 重导出
├── error.rs         (107 行)  — FsError（4 个 crate 独有变体 + #[from] MkxpError）
├── vpath.rs         (500 行)  — VPath newtype（构造验证 + 7 个方法 + 33 tests）
├── mountable.rs     (249 行)  — Mountable trait + RealDirectory + 10 tests
├── filesystem.rs    (362 行)  — FileSystem（mount/read/exists/read_dir/path_cache）+ 11 tests
├── path_cache.rs    (175 行)  — PathCache（build/resolve）+ 4 tests
└── rgss.rs          (~650 行) — RgssArchive + LCG XOR + 21 tests
```

### API 概览

```rust
// ---- VPath ----
pub struct VPath(String);
impl VPath {
    pub fn new(raw: &str) -> Result<Self, FsError>;
    pub fn as_str(&self) -> &str;
    pub fn is_root(&self) -> bool;
    pub fn parent(&self) -> Option<&str>;
    pub fn file_name(&self) -> Option<&str>;
    pub fn extension(&self) -> Option<&str>;
    pub fn join(&self, child: &str) -> Result<Self, FsError>;
}

// ---- Mountable trait ----
pub trait Mountable: Send + Sync {
    fn read(&self, path: &VPath) -> Result<Vec<u8>, FsError>;
    fn exists(&self, path: &VPath) -> bool;
    fn enumerate(&self, dir: &VPath) -> Result<Vec<String>, FsError>;
}

// ---- FileSystem ----
pub struct FileSystem { /* mounts, path_cache */ }
impl FileSystem {
    pub fn new() -> Self;
    pub fn mount(&mut self, source: Box<dyn Mountable>, mountpoint: &VPath);
    pub fn read(&self, path: &str) -> Result<Vec<u8>, FsError>;
    pub fn exists(&self, path: &str) -> bool;
    pub fn read_dir(&self, dir: &str) -> Result<Vec<String>, FsError>;
    pub fn build_path_cache(&mut self) -> Result<(), FsError>;
}

// ---- RgssArchive ----
pub struct RgssArchive { /* data, entries, directories */ }
impl RgssArchive {
    pub fn parse(raw: Vec<u8>) -> Result<Self, FsError>;
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError>;
    pub fn file_exists(&self, path: &str) -> bool;
    pub fn enumerate_dir(&self, dir: &str) -> Result<Vec<String>, FsError>;
}

// ---- PathCache ----
pub struct PathCache { /* lower_to_real */ }
impl PathCache {
    pub fn build(mounts: &[(VPath, Box<dyn Mountable>)]) -> Result<Self, FsError>;
    pub fn resolve(&self, lower: &str) -> Option<&str>;
}
```

### 测试统计

| 模块 | 单元测试 | 文档测试 |
|------|---------|---------|
| error | 8 | 0 |
| vpath | 33 | 7 |
| mountable | 10 | 0 |
| filesystem | 11 | 0 |
| path_cache | 4 | 1 |
| rgss | 21 | 0 |
| **合计** | **87** | **8** |

### 依赖

- `mkxp-types` — 共享错误词汇 `MkxpError` + `From<std::io::Error>`
- `encoding_rs` — Shift_JIS 文件名解码（RGSS 档案）
- `thiserror` — FsError derive
