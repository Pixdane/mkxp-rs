# mkxp-types — 基础类型

`mkxp-types` 是 mkxp-rs workspace 的基础设施 crate。提供所有下游模块共用的数学类型、颜色类型、混合模式和错误词汇表。零必选依赖，可选 `serde` feature。

## 类型一览

| 类型 | 对应 RGSS | 用途 |
|------|----------|------|
| `Vec2` | — | 2D 浮点向量（纹理坐标、缩放） |
| `Vec2i` | — | 2D 整数向量（像素坐标、窗口尺寸） |
| `Color` | `Color` | RGBA 颜色（`f64` 分量） |
| `Tone` | `Tone` | 色调调整（per-sprite 色彩偏移） |
| `Rect` | `Rect` | 整数矩形（viewport、blit 区域） |
| `FloatRect` | — | 浮点矩形（GPU 归一化坐标映射） |
| `BlendMode` | `BlendType` | 像素混合模式（Normal / Addition / Subtraction / Multiply） |
| `MkxpError` | — | 全 workspace 共享错误词汇 |

---

## Vec2 / Vec2i

```rust
pub struct Vec2  { pub x: f32, pub y: f32 }
pub struct Vec2i { pub x: i32, pub y: i32 }
```

`Vec2` 用于纹理坐标、shader uniform 和缩放因子。支持 `+`、`-`、`*f32`、`/f32` 运算符，以及 `dot()` 和 `length()`。

`Vec2i` 用于像素坐标、窗口尺寸和鼠标位置。`Vec2` ↔ `Vec2i` 可以互相转换（`Vec2i → Vec2` 无损，`Vec2 → Vec2i` 向零截断）。

```rust
let a = Vec2::new(1.0, 2.0);
let b = Vec2::new(3.0, 4.0);
assert_eq!(a + b, Vec2::new(4.0, 6.0));
assert_eq!(Vec2::new(3.0, 4.0).length(), 5.0);

let p = Vec2i::new(10, 20);
let f: Vec2 = p.into();              // Vec2i → Vec2
let v: Vec2i = Vec2::new(3.7, -1.2).into();  // 截断 → Vec2i(3, -1)
```

---

## Color / Tone

```rust
pub struct Color { pub r: f64, pub g: f64, pub b: f64, pub a: f64 }
pub struct Tone  { pub r: f64, pub g: f64, pub b: f64, pub gray: f64 }
```

`Color` 对应 RGSS 的 `Color` 类。分量用 `f64`（不是 `u8`），因为 RGSS 允许值超出 `0..255` 范围（shader 负责 clamp）。

常用构造器：`Color::black()`、`Color::white()`、`Color::transparent()`。线性插值：`c1.lerp(c2, t)`。

`Tone` 对应 RGSS 的 `Tone` 类，分量范围 `-255.0..255.0`。`gray` 控制亮度偏移，`r`/`g`/`b` 控制独立通道偏移。

```rust
let red = Color::new(255.0, 0.0, 0.0, 255.0);
let half = black.lerp(white, 0.5);  // Color(127.5, 127.5, 127.5, 255.0)

let sepia = Tone::new(50.0, -30.0, -80.0, 10.0);
let neutral = Tone::neutral();      // Tone(0, 0, 0, 0)
```

---

## Rect / FloatRect

```rust
pub struct Rect      { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }
pub struct FloatRect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }
```

`Rect` 对应 RGSS 的 `Rect` 类。提供 `contains_point()`（左/上边界包含，右/下边界不包含）、`intersection()`（不相交返回空矩形）、`translate()` 和 `set()`。

`FloatRect` 是内部类型，用于将 RGSS 像素坐标映射到 GPU 归一化纹理坐标。`Rect → FloatRect` 可无损转换。

```rust
let r = Rect::new(10, 20, 100, 50);
assert!(r.contains_point(Vec2i::new(50, 40)));

let a = Rect::new(0, 0, 100, 100);
let b = Rect::new(50, 50, 100, 100);
assert_eq!(a.intersection(b), Rect::new(50, 50, 50, 50));
```

---

## BlendMode

```rust
#[repr(u8)]
pub enum BlendMode {
    Normal = 0,
    Addition = 1,
    Subtraction = 2,
    Multiply = 3,
}
```

对应 RGSS 的 `BlendType`（用于 `Sprite#blend_type` 和 `Bitmap#blt`）。提供 `From<BlendMode> for u8` 和 `TryFrom<u8>`（未知值返回 `MkxpError::Unsupported`）。

```rust
assert_eq!(BlendMode::Normal as u8, 0);
assert_eq!(BlendMode::try_from(1u8).unwrap(), BlendMode::Addition);
assert!(BlendMode::try_from(99).is_err());
```

---

## MkxpError

```rust
pub enum MkxpError {
    Io(std::io::Error),
    Parse(String),
    Init(String),
    Runtime(String),
    Unsupported(String),
}
```

全 workspace 共享的错误词汇表。每个下游 crate 通过 `#[from]` 转发，形成分层错误模型。详见 [ERROR_HANDLING.md](ERROR_HANDLING.md)。

`MkxpError::Io` 包装完整的 `std::io::Error`（保留 `kind()` 和 source chain），并通过 `From<std::io::Error>` 支持 `std::io::Result` 的直接 `?` 传播。

---

## Cargo features

| Feature | 效果 |
|---------|------|
| （默认，无 feature） | 零依赖，纯值类型 |
| `serde` | 为所有类型派生 `Serialize` / `Deserialize` |

## 测试统计

| 模块 | 单元测试 | 文档测试 |
|------|---------|---------|
| vec | 3 | 2 |
| color | 1 | 2 |
| rect | 5 | 2 |
| blend_mode | 2 | 1 |
| error | 4 | 1 |
| **合计** | **15** | **8** |
