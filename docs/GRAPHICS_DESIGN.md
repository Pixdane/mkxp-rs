# mkxp-rs 图形系统设计

本文档定义 `mkxp-graphics` crate 的架构规范。它对应 mkxp-z 中
`src/display/` 的全部功能：场景图、精灵、视口、位图、着色器、后处理、
帧循环。

## 依赖

| Crate | 版本 | 用途 |
|-------|------|------|
| `wgpu` | 24+ | GPU 抽象层（device、surface、pipeline、bind group） |
| `image` | 0.25+ | PNG/JPEG 解码 |
| `slotmap` | 1.0+ | 稳定句柄的 arena 分配器 |
| `mkxp-types` | workspace | Vec2、Color、Rect、BlendMode 等基础类型 |
| `mkxp-fs` | workspace | 虚拟文件系统和 RGSS 加密包读取 |

---

## 设计原则

### 1. 禁止全局可变状态

mkxp-z 大量使用 `shState->xxx()` 和 `glState.xxx()` 这种全局单例访问。
所有渲染代码通过宏直接捅进 `SharedState`。

mkxp-rs 中，任何需要外部资源的方法都通过参数接收，不依赖模块级 `static`。
入口函数组装好 `DrawContext`，逐层传下去。

```rust
// mkxp-z：全局访问
glState.blendMode.pushSet(blendType);          // 捅全局 GL 状态
shState->shaders().sprite.bind();               // 捅全局 ShaderSet
shState->config().enableHires                    // 捅全局 Config

// mkxp-rs：参数传递
ctx.blend.push(self.blend_type);                // ctx 来自参数
ctx.bind_pipeline(ctx.pipelines.sprite);        // pipelines 在 ctx 里
```

### 2. 元素画自己，场景管顺序

渲染循环遍历场景图的有序节点列表，每个节点调用自己的 `draw()`。
场景不关心精灵和视口具体怎么画——各画各的。

### 3. arena + 句柄 管理生命周期

所有场景元素存放在 `slotmap::SlotMap` 中，外部持有 `NodeId` 句柄。
Ruby 绑定层持有 `NodeId` 而非 Rust 对象的引用，对象的真实生命周期由
arena 管理。节点从场景中删除后在 arena 中留下空槽，不影响其他句柄。

### 4. 单一职责、组合优于继承

mkxp-z 的 `Viewport` 同时继承 `SceneElement` 和 `Scene`（多重继承）。
Rust 中用组合：`Viewport` 实现 `Drawable` trait，场景图负责维护
"这个节点有子节点"的元信息。

---

## 架构总览

```
mkxp-z (C++)                          mkxp-rs (Rust)
─────────────────────────────────     ─────────────────────────────────
SharedState (全局单例)                  GraphicsState (局部拥有)
  ├── Graphics                           ├── SceneGraph
  ├── GLState          ──────────→       ├── RenderTarget (画布)
  ├── ShaderSet                          ├── PipelineSet (约5个 pipeline)
  ├── TexPool                            ├── TexPool
  └── Config                             └── Config (只读引用)

Scene + SceneElement (侵入式链表)        SceneGraph (slotmap arena, 树结构)
  ├── Viewport : Scene + SceneElement      ├── Node (element + 父子关系 + z)
  ├── Sprite : ViewportElement               ├── Sprite : Drawable
  ├── Plane : ViewportElement                ├── Viewport : Drawable
  ├── Window : ViewportElement               ├── Plane : Drawable
  └── Tilemap                               ├── Window : Drawable
                                            └── Tilemap : Drawable

Quad / QuadArray                       Quad / QuadArray
  Vertex {pos, texPos, color}            Vertex {pos, tex_coord, color}
  VBO + VAO + glDrawElements             wgpu Buffer + BindGroup + draw_indexed

glState.blendMode.push/pop             ctx.blend.push / ctx.blend.pop
glState.scissorBox.push/pop            ctx.scissor.push / ctx.scissor.pop
```

---

## binary crate 的窗口层（GameWindow）

窗口创建和控制位于 binary crate 中，不在 `mkxp-graphics` 内。
它不参与渲染，只管理 winit 窗口本身，并在初始化时向渲染层提供 surface。

对应 mkxp-z 中 `EventThread` 负责的窗口部分（SDL_Window 创建、
全屏切换、光标显隐、缩放）以及 `main.cpp` 中的窗口初始化。

```rust
pub struct GameWindow {
    window: winit::window::Window,
}

impl GameWindow {
    pub fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        title: &str,
        width: u32,
        height: u32,
    ) -> Self;

    // ── 查询 ──
    pub fn size(&self) -> (u32, u32);
    pub fn scale_factor(&self) -> f64;

    // ── 控制 ──
    pub fn set_title(&self, title: &str);
    pub fn set_fullscreen(&mut self, fullscreen: bool);
    pub fn is_fullscreen(&self) -> bool;
    pub fn set_cursor_visible(&self, visible: bool);
    pub fn center(&self);
    pub fn request_resize(&self, width: u32, height: u32);

    // ── 给渲染层用的唯一入口 ──
    pub fn create_surface(&self, instance: &wgpu::Instance) -> wgpu::Surface<'static>;
    pub fn inner(&self) -> &winit::window::Window;
}
```

### 与渲染层的关系

```
winit main thread
  │
  ├── Resized 事件 → RenderCommand::SurfaceResized
  ├── 全屏/viewport 菜单 → RenderCommand::ViewportScaleModeChanged
  └─→ GameWindow / WindowController (管理 winit 窗口，不碰 GPU)

render thread
  │
  ├── drain RenderCommand
  ├── if script frame ready && FPS gate open:
  │      graphics.update() → 画一帧
  └─→ GraphicsState (持有 surface、device、场景图)
```

窗口层只做两件事：管理 winit 窗口本身 + 为渲染层提供 surface。
渲染层从 `create_surface()` 拿到 surface 后，自己负责 device、queue、
surface 配置和 swapchain 管理。窗口尺寸变化时，winit 主线程发送
`RenderCommand::SurfaceResized`；render thread 在下一帧前调用
`GraphicsState::on_resize()`，渲染层更新内部状态。

### 为什么分开

- **可单独测试**：窗口层不依赖 GPU，直接测全屏/光标/尺寸。渲染层构造假尺寸参数做单元测试，不需要真实窗口。
- **可换后端**：将来不用 winit（比如 wasm），只换 `GameWindow` 实现，渲染代码不动。
- **和 mkxp-z 一致**：mkxp-z 里窗口事件在 EventThread（独立线程），渲染在 RGSS 线程。分离窗口对应这个边界。
- **依赖方向正确**：`GraphicsState` 依赖 `GameWindow`（拿 surface），`GameWindow` 不知道任何渲染细节。单向。

---

## 核心抽象


### 1. NodeId

```rust
use slotmap::new_key_type;

new_key_type! {
    /// 场景图中节点的稳定标识符。
    ///
    /// 类比 mkxp-z 中 `SceneElement*` 指针，但不会悬空。
    /// 节点被删除后，其 NodeId 变为无效，其他节点不受影响。
    pub struct NodeId;
}
```

Ruby 绑定层把 `NodeId` 存在 Ruby 对象的 ivar 里。当 Ruby GC 回收
Sprite/Viewport 时，调用 `scene_graph.remove(node_id)` 即可。

### 2. Drawable trait

```rust
use mkxp_types::{Color, Rect, Tone, Vec2};

/// 场景图中可以绘制自己的元素。
///
/// 对应 mkxp-z 的 `SceneElement`。
pub trait Drawable {
    /// 在当前的 DrawContext 上绘制自身。
    ///
    /// `id` 是自身在 SceneGraph 中的句柄。需要访问子节点时通过
    /// `ctx.graph` 查询。
    fn draw(&self, id: NodeId, ctx: &mut DrawContext<'_>);

    /// 此元素是否有子节点需要在它之前/之后绘制。
    ///
    /// 默认返回 `ChildrenMode::None`（如 Sprite、Plane）。
    /// Viewport 返回 `ChildrenMode::BeforeSelf`。
    fn children_mode(&self) -> ChildrenMode {
        ChildrenMode::None
    }

    /// 父节点几何信息变更时回调。
    ///
    /// 对应 mkxp-z 的 `onGeometryChange`。
    fn on_geometry_change(&mut self, parent_geo: Geometry);
}

/// 子节点绘制策略。
pub enum ChildrenMode {
    /// 没有子节点，直接 draw(self)。
    None,
    /// 先递归绘制子节点，再 draw(self)。
    ///
    /// Viewport 使用此模式：先画子内容，再叠上 color/tone/flash 效果。
    BeforeSelf,
}
```

### 3. DrawContext

```rust
/// 渲染操作上下文。
///
/// 对应 mkxp-z 中分散在 `glState`、`shState->shaders()`、
/// `FBO::bind()` 等全局接口的全部能力。
///
/// 持有当前帧的 GPU encoder、所有可用的 pipeline、当前画布引用、
/// 以及混合/裁剪的状态栈。
pub struct DrawContext<'a> {
    /// 当前绑定的渲染目标（画布或 FBO 纹理）。
    pub target: &'a mut RenderTarget,

    /// 预编译的 pipeline 集合（约 5 个 ubershader）。
    pub pipelines: &'a PipelineSet,

    /// 场景图引用，用于递归绘制（Viewport 画子节点时需要）。
    pub graph: &'a SceneGraph,

    /// 混合模式栈。push/pop 对应 `glBlendFunc` 的切换。
    pub blend: BlendStack,

    /// 裁剪区域栈。push/pop 对应 `glScissor` 的切换。
    pub scissor: ScissorStack,
}

impl<'a> DrawContext<'a> {
    /// 在当前位置画一个四边形。
    ///
    /// 使用当前绑定 pipeline 和纹理，顶点数据来自 `quad`。
    /// 对应 mkxp-z 的 `Quad::draw()`。
    pub fn draw_quad(&mut self, quad: &Quad);

    /// 画一个 QuadArray（批量四边形，如 wave 效果或 tilemap 块）。
    ///
    /// 对应 mkxp-z 的 `QuadArray::draw()`。
    pub fn draw_quad_array(&mut self, array: &QuadArray);
}
```

---

## 场景图（SceneGraph）

### 数据结构

```rust
use slotmap::SlotMap;

pub struct SceneGraph {
    /// 所有节点的 arena。
    nodes: SlotMap<NodeId, Node>,

    /// 根节点的子节点列表，按 z 序排好。
    root_children: Vec<NodeId>,

    /// 创建时间戳计数器，用于同 z 时的稳定排序。
    stamp_counter: u64,
}

struct Node {
    /// 具体的可绘制对象。
    element: Box<dyn Drawable>,

    /// 父节点。
    parent: Parent,

    /// 直接子节点列表。不是始终保持有序——仅在需要绘制时排一次。
    children: Vec<NodeId>,

    /// children 是否已经有序。
    children_sorted: bool,

    /// Z 排序键。
    z: i32,
    sprite_y: i32,
    stamp: u64,

    /// 是否可见。
    visible: bool,
}

enum Parent {
    /// 根节点。
    Root,
    /// 父节点是另一个 Node。
    Node(NodeId),
}
```

### Z 序规则

与 mkxp-z 完全一致：

1. z 值小的先画（在下面）。
2. z 值相同，sprite_y 小的先画（RGSS2+）。
3. z 和 sprite_y 都相同，创建时间早的先画（stamp 小的）。

排序发生在 `SceneGraph::mark_children_dirty(node_id)` 调用后的下一次绘制前。

```rust
impl SceneGraph {
    /// 标记某节点的子节点顺序需要重排。
    ///
    /// 在 z、sprite_y 变化或新节点插入时调用。
    pub fn mark_children_dirty(&mut self, parent: NodeId);

    /// 确保某节点的子节点已排序（惰性排序）。
    fn ensure_children_sorted(&mut self, parent: NodeId);
}
```

### 节点操作

```rust
impl SceneGraph {
    /// 创建新节点，挂到指定父节点下。
    ///
    /// `parent: None` 表示挂在根节点下。
    pub fn insert(
        &mut self,
        parent: Option<NodeId>,
        element: Box<dyn Drawable>,
        z: i32,
        sprite_y: i32,
    ) -> NodeId;

    /// 删除节点及其所有子节点（递归）。
    pub fn remove(&mut self, id: NodeId);

    /// 修改节点的 z 值，自动在父节点中重排。
    pub fn set_z(&mut self, id: NodeId, z: i32);

    /// 修改节点的 sprite_y，自动重排。
    pub fn set_sprite_y(&mut self, id: NodeId, y: i32);

    /// 设置可见性。
    pub fn set_visible(&mut self, id: NodeId, visible: bool);
}
```

### 绘制遍历

```rust
impl SceneGraph {
    /// 合成一帧：从根节点开始，深度优先递归绘制所有可见节点。
    ///
    /// 对应 mkxp-z 的 `Scene::composite()`。
    pub fn composite(&self, ctx: &mut DrawContext<'_>) {
        self.draw_children(ctx, &self.root_children);
    }

    fn draw_children(&self, ctx: &mut DrawContext<'_>, children: &[NodeId]) {
        for &id in children {
            let node = &self.nodes[id];
            if !node.visible {
                continue;
            }

            // 确保子节点排好序
            // (在实际实现中需要 interior mutability)
            // self.ensure_children_sorted(id);

            match node.element.children_mode() {
                ChildrenMode::None => {
                    // 普通元素：直接画自己
                    node.element.draw(id, ctx);
                }
                ChildrenMode::BeforeSelf => {
                    // Viewport：先画子节点，再画自己（叠加效果）
                    self.draw_children(ctx, &self.nodes[id].children);
                    node.element.draw(id, ctx);
                }
            }
        }
    }
}
```

> **关于内部可变性**：`composite()` 需要 `&self` 但 `ensure_children_sorted()`
> 需要 `&mut self`。v1 中在 `composite()` 之前一次性排好所有脏节点，
> 遍历期间只读。后续可引入 `RefCell` 做真正的惰性排序。

---

## 可绘制对象

### Sprite

对应 mkxp-z 的 `Sprite` 类 + `SpritePrivate`。

```rust
pub struct Sprite {
    /// 引用的位图。
    bitmap: Option<BitmapHandle>,

    /// 源矩形（裁剪位图的哪个区域）。
    src_rect: Rect,

    /// 几何变换（位置、原点、缩放、旋转）。
    transform: Transform,

    /// 是否水平镜像。
    mirrored: bool,

    /// RGSS 效果。
    color: Color,
    tone: Tone,
    opacity: f32,     // 0.0 - 1.0，对应 0-255

    /// Bush 效果（草丛遮挡）。
    bush_depth: u32,
    bush_opacity: f32,

    /// 混合类型（正常/加法/减法）。
    blend_type: BlendMode,

    /// Pattern 叠加纹理（非标扩展）。
    pattern: Option<TextureHandle>,
    pattern_blend: BlendMode,
    pattern_tile: bool,
    pattern_opacity: f32,
    pattern_scroll: Vec2,
    pattern_zoom: Vec2,

    /// 反色（非标扩展）。
    invert: bool,

    /// Wave 波浪效果。
    wave: Option<WaveData>,

    /// 缓存：推断出的位图尺寸。
    bitmap_size: Vec2,

    /// 缓存：上一帧的父节点几何，用于脏检测。
    last_parent_geo: Geometry,
}

impl Drawable for Sprite {
    fn draw(&self, _id: NodeId, ctx: &mut DrawContext<'_>) {
        // 1. 选 pipeline（simple_sprite / sprite_with_effects）
        // 2. 填充 SpriteUniforms
        // 3. ctx.blend.push(self.blend_type)
        // 4. ctx.bind_texture(self.bitmap)
        // 5. ctx.draw_quad(&self.quad) 或 ctx.draw_quad_array(&self.wave_quads)
        // 6. ctx.blend.pop()
    }

    fn children_mode(&self) -> ChildrenMode { ChildrenMode::None }
}
```

### Viewport

对应 mkxp-z 的 `Viewport` 类。它在场景图中扮演双重角色：
- 作为普通节点的父节点（裁剪和坐标偏移其子节点）
- 作为可绘制对象（叠加 color、tone、flash 效果）

```rust
pub struct Viewport {
    /// 视口矩形（相对于父节点）。
    rect: Rect,

    /// 内容原点偏移（ox, oy）。
    origin: Vec2,

    /// 叠加效果。
    color: Color,
    tone: Tone,

    /// 闪烁状态。
    flash_color: Color,
    flash_duration: u32,
}

impl Drawable for Viewport {
    fn draw(&self, _id: NodeId, ctx: &mut DrawContext<'_>) {
        // 1. ctx.scissor.push(self.rect)     ← 裁剪子节点
        //    （子节点已在 children_mode 触发的递归中画完）
        // 2. 如有 color/tone/flash 效果，画全屏四边形叠加
        // 3. ctx.scissor.pop()
    }

    fn children_mode(&self) -> ChildrenMode {
        ChildrenMode::BeforeSelf  // ← 关键：先画子节点
    }
}
```

绘制 Viewport 的完整流程：

```
1. 场景遍历到 Viewport 节点
2. 检查 children_mode == BeforeSelf
3. 递归绘制 Viewport 的所有子节点：
   a. 每个子节点 Sprite::draw() 时，ctx.scissor 是激活的
   b. 子节点的坐标相对于 Viewport 的 geometry
4. 回到 Viewport::draw()：
   a. 如果 color/tone/flash 有效果，画叠加四边形
   b. ctx.scissor.pop()
```

### Plane

对应 mkxp-z 的 `Plane` 类。一个可平铺、可滚动的背景层。

```rust
pub struct Plane {
    bitmap: Option<BitmapHandle>,
    /// 滚动偏移。
    ox: f32,
    oy: f32,
    /// 缩放。
    zoom_x: f32,
    zoom_y: f32,
    /// 效果。
    color: Color,
    tone: Tone,
    opacity: f32,
    blend_type: BlendMode,
}

impl Drawable for Plane {
    fn children_mode(&self) -> ChildrenMode { ChildrenMode::None }
}
```

### Window

对应 mkxp-z 的 `Window` 类。九宫格皮肤 UI，带文本绘制。

v1 不做，原因：Window 依赖于 Bitmap 上的软件文本渲染（FreeType → SDL_Surface），
这与纯 GPU 纹理管道有较大差异。等 Bitmap 的软件绘制能力就绪后再实现。

### Tilemap

对应 mkxp-z 的 `Tilemap` 类。网格地图渲染。

v1 暂不做完整实现。Tilemap 的复杂度主要在数据层面（几千个四边形、自动瓦片动画、
优先级层级），渲染层面只是批量 QuadArray 绘制。等 SceneGraph 和 Sprite 稳定后再展开。

---

## 画布和渲染目标

### RenderTarget

```rust
/// 渲染目标——可以是窗口 swapchain 的一帧，也可以是 FBO 纹理。
///
/// 对应 mkxp-z 的 `TEXFBO`（纹理+帧缓冲）和屏幕 FBO。
pub struct RenderTarget {
    /// wgpu 纹理视图。
    view: wgpu::TextureView,

    /// 尺寸。
    size: Vec2,

    /// 是否为主屏幕（swapchain 帧）。
    is_screen: bool,
}
```

### PingPong

```rust
/// 乒乓双缓冲，用于后处理的多 pass 渲染。
///
/// 对应 mkxp-z 的 `PingPong` 内部类。
pub struct PingPong {
    buffers: [RenderTarget; 2],
    src: usize, // 当前"源"缓冲区的索引
    dst: usize, // 当前"目标"缓冲区的索引
}

impl PingPong {
    pub fn new(size: Vec2, device: &wgpu::Device) -> Self;

    /// 获取当前目标缓冲区（接下来要画的）。
    pub fn target(&mut self) -> &mut RenderTarget;

    /// 获取当前源缓冲区（已经画完的）。
    pub fn source(&self) -> &RenderTarget;

    /// 交换源和目标。
    pub fn swap(&mut self);

    /// 清空两个缓冲区。
    pub fn clear(&mut self, encoder: &mut wgpu::CommandEncoder);
}
```

---

## 四边形（最小绘制单元）

```rust
/// 一个四边形（4 顶点，2 三角形）。
///
/// 对应 mkxp-z 的 `Quad`——有自己的 VBO + VAO。
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub tex_coord: [f32; 2],
    pub color: [f32; 4],
}

pub struct Quad {
    pub vertices: [Vertex; 4],
    /// GPU 端顶点缓冲区。
    buffer: wgpu::Buffer,
    /// 缓冲区是否脏，需要重新上传。
    dirty: bool,
}

impl Quad {
    /// 设置位置矩形。
    pub fn set_pos_rect(&mut self, rect: Rect);

    /// 设置纹理坐标矩形。
    pub fn set_tex_rect(&mut self, rect: Rect);

    /// 同时设置纹理坐标和位置矩形。
    pub fn set_tex_pos_rect(&mut self, tex: Rect, pos: Rect);

    /// 设置四个顶点的统一颜色。
    pub fn set_color(&mut self, color: Color);

    /// 上传脏数据到 GPU。
    pub fn flush(&mut self, queue: &wgpu::Queue);
}
```

`QuadArray` 是 `Quad` 的批量版本——一个大的顶点缓冲区包含 N 个四边形的顶点，
一次 draw call 画全部。对应 mkxp-z 的 `QuadArray<Vertex>`。

---

## 纹理系统

### Bitmap

```rust
/// 位图——GPU 纹理 + 可选的 CPU 端像素缓冲区。
///
/// 对应 mkxp-z 的 `Bitmap` 类。
///
/// 大多数情况下，Bitmap 只是一个 wgpu 纹理句柄。
/// 当需要软件像素操作（get_pixel, set_pixel, fill_rect, draw_text）时，
/// 额外维护一个 CPU 端的像素缓冲区。
pub struct Bitmap {
    /// GPU 纹理。
    texture: wgpu::Texture,
    /// 纹理视图（用于绑定到 shader）。
    view: wgpu::TextureView,
    /// 尺寸。
    size: Vec2,

    /// 高分辨率替换纹理（enableHires 时有效）。
    hires: Option<Box<Bitmap>>,

    /// CPU 端像素缓冲区。仅在需要软件操作时分配。
    pixels: Option<Vec<u8>>,
    /// pixels 是否脏（需要重新上传到 GPU）。
    pixels_dirty: bool,

    /// 是否是"超大纹理"（超出 GPU 限制）。
    /// 对应 mkxp-z 的 MegaSurface 概念。现代 GPU 的上限足够大，
    /// 此字段大概率永远是 false，保留作为安全网。
    is_mega: bool,

    /// 纹理的来源文件路径（用于 hires 替换逻辑）。
    /// `None` 表示纯色/动态创建的 Bitmap。
    source_path: Option<String>,
}
```

### Bitmap 操作

```rust
impl Bitmap {
    /// 从文件加载。
    pub fn from_file(fs: &impl FileSystem, path: &VPath) -> Result<Self>;

    /// 创建纯色空白位图。
    pub fn new(width: u32, height: u32) -> Self;

    /// 创建带初始像素数据的位图。
    pub fn from_pixels(width: u32, height: u32, data: &[u8]) -> Self;

    // ── 需要 CPU 缓冲区的操作 ──

    /// 获取指定坐标的像素颜色。
    /// 触发 CPU 缓冲区的惰性分配（如果尚未分配，从 GPU 读回）。
    pub fn get_pixel(&mut self, x: u32, y: u32) -> Color;

    /// 设置指定坐标的像素颜色。
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color);

    /// 填充矩形。
    pub fn fill_rect(&mut self, rect: Rect, color: Color);

    /// 清除矩形区域（设为透明）。
    pub fn clear_rect(&mut self, rect: Rect);

    /// 将 CPU 缓冲区的修改上传到 GPU 纹理。
    /// 在下次 draw 调用前自动调用（惰性上传）。
    pub fn flush(&mut self, queue: &wgpu::Queue);

    // ── GPU 端操作（通过 blit）──

    /// 从另一 Bitmap 拷贝区域到当前 Bitmap。
    /// 使用 Blt pipeline，不走 CPU。
    pub fn blt(
        &self,
        x: i32, y: i32,
        src: &Bitmap,
        src_rect: Rect,
        opacity: u8,
        ctx: &mut DrawContext,
    );

    /// 拉伸拷贝。
    pub fn stretch_blt(
        &self,
        dest_rect: Rect,
        src: &Bitmap,
        src_rect: Rect,
        opacity: u8,
        ctx: &mut DrawContext,
    );
}
```

### TexPool

```rust
/// 纹理缓存池。
///
/// 对应 mkxp-z 的 `TexPool`。
/// 缓存上限 20MB 或 `pool_size` 配置项指定的值。
///
/// 游戏频繁创建/销毁临时 Bitmap 时（如战斗特效），
/// 避免重复分配 GPU 内存。
pub struct TexPool {
    /// 空闲纹理列表，按尺寸分组以便快速匹配。
    free: Vec<PoolEntry>,
    /// 当前已缓存的总字节数。
    current_bytes: u64,
    /// 缓存上限。
    max_bytes: u64,
}

impl TexPool {
    /// 请求一个指定尺寸的纹理。优先从池中取现成的。
    pub fn request(&mut self, width: u32, height: u32, device: &wgpu::Device)
        -> wgpu::Texture;

    /// 归还纹理到池中（而不是销毁）。
    pub fn release(&mut self, texture: wgpu::Texture);
}
```

---

## Pipeline 和 Shader 系统

### Ubershader 策略

mkxp-z 有 25 个独立的 shader 类。它们在绘制时动态选择：
"有特效吗？用 SpriteShader。只有透明度？用 AlphaSpriteShader。
啥特效都没有？用 SimpleSpriteShader。"

在 wgpu 里，每个 shader 变体对应一个 `wgpu::RenderPipeline`。
25 个 pipeline 对象可以接受，但更好的做法是用 ubershader——
一个 WGSL 文件，用编译期 `#define` 或运行时 uniform 标志来控制
哪些效果路径激活。

但不是所有效果都能简单地塞进 ubershader。
有效的做法是**适度合并**，按"渲染阶段"分组：

| Pipeline | 用途 | 对应 mkxp-z shader |
|----------|------|-------------------|
| `simple` | 纯纹理四边形，无效果 | SimpleShader, SimpleSpriteShader |
| `sprite` | 带 tone/color/opacity/bush/pattern/invert | SpriteShader, AlphaSpriteShader |
| `flat_color` | 纯色四边形（后处理用） | FlatColorShader |
| `blit` | 纹理间拷贝 | BltShader, KglSubtractShader |
| `blur` | 高斯模糊（两 pass） | BlurShader (HPass + VPass) |

v1 只实现 `simple`、`sprite`、`flat_color` 三个 pipeline。
blit 和 blur 在后续版本追加。

### PipelineSet

```rust
/// 预编译的 pipeline 集合。
///
/// 对应 mkxp-z 的 `ShaderSet`。
pub struct PipelineSet {
    /// 基础纹理四边形。
    pub simple: wgpu::RenderPipeline,
    /// 带效果的精灵（tone / color / opacity / bush / pattern / invert）。
    pub sprite: wgpu::RenderPipeline,
    /// 纯色四边形（后处理用）。
    pub flat_color: wgpu::RenderPipeline,
}

impl PipelineSet {
    /// 从 wgpu Device 创建所有 pipeline。
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
    ) -> Self;
}
```

### Uniform 布局

```rust
/// 所有 shader 共享的帧级 uniform（每帧更新一次）。
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameUniforms {
    /// 正交投影矩阵，将像素坐标映射到 NDC。
    pub proj_mat: [[f32; 4]; 4],
    /// 纹理尺寸的倒数（1/w, 1/h），用于 texel 精度相关计算。
    pub tex_size_inv: [f32; 2],
    /// 视口平移（用于 Viewport origin 偏移）。
    pub translation: [f32; 2],
    pub _padding: [f32; 2],
}

/// 精灵绘制时的 per-draw uniform。
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteUniforms {
    /// 精灵变换矩阵（4x4，包含 translate/rotate/scale/origin）。
    pub sprite_mat: [[f32; 4]; 4],

    // 效果参数
    pub tone: [f32; 4],
    pub color: [f32; 4],
    pub opacity: f32,

    // Bush 效果
    pub bush_depth: f32,
    pub bush_slope: f32,
    pub bush_intercept: f32,
    pub bush_opacity: f32,

    // Pattern
    pub pattern_size_inv: [f32; 2],
    pub pattern_blend: i32,
    pub pattern_tile: i32,   // bool as i32 for alignment
    pub pattern_opacity: f32,
    pub pattern_scroll: [f32; 2],
    pub pattern_zoom: [f32; 2],

    // Invert
    pub invert: i32,
    pub _pad: [i32; 3],
}
```

---

## DrawContext 的状态栈

### 混合模式栈

```rust
/// 混合模式栈。
///
/// 对应 mkxp-z 的 `GLBlendMode` + `GLBlend`。
pub struct BlendStack {
    stack: Vec<BlendMode>,
}

impl BlendStack {
    /// 保存当前混合模式，切换到新值。
    pub fn push(&mut self, mode: BlendMode);

    /// 恢复到上一个混合模式。
    pub fn pop(&mut self);
}
```

### 裁剪栈

```rust
/// 裁剪区域栈。
///
/// 对应 mkxp-z 的 `GLScissorBox` + `GLScissorTest`。
pub struct ScissorStack {
    stack: Vec<Option<Rect>>,
}

impl ScissorStack {
    /// 启用在指定区域的裁剪。
    ///
    /// 如果已有激活的裁剪区域，新区域取交集（对应 `setIntersect`）。
    pub fn push(&mut self, rect: Rect);

    /// 恢复到上一个裁剪状态。
    pub fn pop(&mut self);

    /// 当前生效的裁剪区域。`None` 表示不裁剪。
    pub fn current(&self) -> Option<Rect>;
}
```

---

## 后处理

### 处理链

每一帧场景合成完成后，叠加以下效果（与 mkxp-z 完全一致）：

```
1. 场景合成结果 → PingPong.src
2. [可选] 灰度化（tone.w != 0）
   PingPong.swap()
   用 GrayShader 读取 src，写入 dst
3. [可选] 色调 RGB（tone.xyz != 0）
   用 FlatColorShader + 加法/减法混合 叠加
4. [可选] 颜色 + 闪光
   用 FlatColorShader + alpha blend 叠加
5. [可选] 亮度
   用 FlatColorShader 画黑色半透明四边形（alpha = 1 - brightness）
6. PingPong 的最终内容 → blit 到屏幕 swapchain
```

```rust
/// 后处理管线。
pub struct PostProcess {
    /// 乒乓缓冲。
    pingpong: PingPong,
    /// 全屏四边形（后处理 pass 共用）。
    screen_quad: Quad,
    /// 亮度四边形（黑色，透明度可变）。
    brightness_quad: Quad,
}
```

---

## 帧循环

### GraphicsState

```rust
/// mkxp-graphics 的渲染层顶层状态。
///
/// 对应 mkxp-z 的 `Graphics` 类 + `GraphicsPrivate`。
///
/// 不持有窗口，不依赖 winit。接收一个 `wgpu::Surface` 作为参数，
/// 所有 GPU 资源由 GraphicsState 自行管理。
///
/// 构造时由 binary crate 传入已创建好的 device、queue、surface。
pub struct GraphicsState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,

    /// 缓存的窗口尺寸。
    window_size: Vec2,
    pub scene: SceneGraph,
    pub pipelines: PipelineSet,
    pub tex_pool: TexPool,
    pub post: PostProcess,
    pub config: Config,
}

impl GraphicsState {
    /// 由 binary crate 调用。传入已初始化好的 wgpu 资源。
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        surface_config: wgpu::SurfaceConfiguration,
        config: &Config,
    ) -> Self;
}
```

### on_resize

窗口缩放事件由 binary crate 的 winit 事件循环捕获，
然后调用 `GraphicsState::on_resize()`。`mkxp-graphics` 不知道
事件的来源——它只管更新内部状态。

```rust
impl GraphicsState {
    /// 窗口缩放时调用。更新内部缓存尺寸和 surface 配置。
    /// 由 binary crate 的事件循环调用。
    pub fn on_resize(&mut self, width: u32, height: u32) {
        self.window_size = Vec2::new(width as f32, height as f32);
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }
}
```

### Graphics::update

```rust
impl GraphicsState {
    /// 合成并显示一帧。
    ///
    /// 对应 mkxp-z 的 `Graphics::update()` → `redrawScreen()`。
    pub fn update(&mut self) -> Result<(), wgpu::SurfaceError> {
        // 1. 获取 swapchain 的当前帧
        let frame = self.surface.get_current_texture()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 2. 创建 command encoder
        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("frame") }
        );

        // 3. 场景合成 → 后处理的目标缓冲区
        self.post.pingpong.clear(&mut encoder);

        {
            let mut ctx = DrawContext {
                target: self.post.pingpong.target(),
                pipelines: &self.pipelines,
                graph: &self.scene,
                blend: BlendStack::new(),
                scissor: ScissorStack::new(),
            };
            self.scene.composite(&mut ctx);
        }

        // 4. 后处理链
        self.post.apply(&self.scene, &self.pipelines, &mut encoder);

        // 5. Blit 到屏幕（翻转 + 缩放）
        self.blit_to_screen(&view, &mut encoder);

        // 6. 提交
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();

        Ok(())
    }
}
```

---

---

## 公开 API 与共享状态

`mkxp-graphics` 只暴露一个公开入口——`GraphicsState`。内部的所有类型
（SceneGraph、PipelineSet、TexPool、DrawContext）对外不可见。

### API 分组

同一套 `GraphicsState` 的方法按使用者分成两组，互不重叠：

```
GraphicsState 公开方法
│
├─ 场景修改（Ruby 用）
│   create_sprite(parent, z) → NodeId
│   create_viewport(parent, rect, z) → NodeId
│   remove_node(id)
│   set_sprite_x/y/z/zoom/angle/opacity/ ...
│   set_sprite_color/tone/blend/visible/ ...
│   set_viewport_rect/origin/color/tone/ ...
│   load_bitmap(path) → Bitmap
│   create_bitmap(w, h) → Bitmap
│
├─ 查询（Ruby 用）
│   get_sprite_x/y/z/zoom/ ...
│
└─ 渲染和窗口（render host 用）
    update()        ← render thread 在 FrameSync ready 且 FPS gate 到期时画一帧
    on_resize()     ← render thread 收到 RenderCommand::SurfaceResized 后重新配置 surface
```

Ruby 绑定层只是把 Ruby 参数翻译成 Rust 类型，调用 `GraphicsState` 的方法。
它不需要知道 SceneGraph、SlotMap、Z 序——这些全是 `pub(crate)` 的内部实现。

```rust
// magnus 绑定 — 薄薄一层
fn sprite_set_x(ruby: &Ruby, shared: &Shared, id: NodeId, x: i32) {
    let mut gfx = shared.graphics.lock().unwrap();
    gfx.set_sprite_position(id, x, gfx.get_sprite_y(id));
}
```

### 怎么共享

```rust
/// 多线程共享的 runtime 状态。
/// 启动时创建，Arc 克隆给 render host 和 Ruby/script 双方。
struct Shared {
    graphics: Mutex<GraphicsState>,
    input: InputState,           // 内部全是 Atomic*，无需锁
    signals: ControlSignals,     // 内部 AtomicBool
    frame_sync: FrameSync,       // 内部 Mutex<FrameSyncState> + Condvar
    config: Config,              // 只读
}

// ── 启动 ──
let shared = Arc::new(Shared { ... });

// render 线程拿一份
let s = shared.clone();
thread::spawn(move || {
    loop {
        s.frame_sync.wait_for_ready_or_shutdown();
        drain_render_commands();
        s.graphics.lock().unwrap().update();
        s.frame_sync.render_finished();
    }
});

// Ruby 线程拿同一份
let s = shared.clone();
thread::spawn(move || {
    // $scene.update 中：
    // s.graphics.lock().unwrap().set_sprite_x(id, 100);
    // Graphics.update 中：
    // s.frame_sync.ruby_frame_ready_and_wait();
});
```

### 为什么 Mutex 是零竞争的

FrameSync 保证了 render thread 和 Ruby/script thread 永远不会同时调用
`graphics.lock()`。render thread 拿锁时 Ruby 正阻塞在 `frame_sync` 上；
Ruby 拿锁时 render thread 正在等待下一次 ready 或 FPS gate。Mutex 主要用于让
Rust 类型系统接受跨线程可变性，正常帧循环中不应长期竞争。

### 依赖方向

```
Ruby 绑定层                 winit 事件循环
     │                          │
     └──────┬───────────────────┘
            │ 都通过 GraphicsState 公开 API
            ▼
     mkxp-graphics 公开 API
       GraphicsState::{create_sprite, set_sprite_x, update, on_resize, ...}
       NodeId（透明句柄，外部只持有不解构）
            │
            ▼
     mkxp-graphics 内部实现（pub(crate)）
       SceneGraph, DrawContext, PipelineSet, TexPool, PostProcess, ...


## binary crate：事件循环集成

这是 binary crate 的 `main.rs`。它拥有所有子系统的实例，
在事件循环中协调它们。`mkxp-graphics` 是被调用者，不知道事件循环的存在。

```rust
// binary crate 的 main.rs
fn main() {
    // 加载配置
    let config = mkxp_config::load()?;

    // 初始化日志
    mkxp_log::init((&config).into())?;

    // 挂载文件系统
    let mut fs = mkxp_fs::FileSystem::new(&config);

    // 初始化音频
    let audio = mkxp_audio::AudioManager::new(&config)?;

    // ── 创建 winit 窗口（只有 binary crate 依赖 winit）──
    let event_loop = winit::event_loop::EventLoop::new()?;
    let window = GameWindow::new(&event_loop, &config.game_title, 640, 480);

    // ── 初始化 wgpu（只有 binary crate 做这件事）──
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
    let surface = window.create_surface(&instance);
    let adapter = pollster::block_on(instance.request_adapter(/* ... */))?;
    let (device, queue) = pollster::block_on(adapter.request_device(/* ... */))?;
    let surface_config = /* 配置 surface 格式和尺寸 */;

    // ── 创建渲染层，传入 GPU 资源（mkxp-graphics 开始介入）──
    let mut graphics = GraphicsState::new(
        device, queue, surface, surface_config, &config,
    );

    // ── 事件循环：binary crate 是唯一的协调者 ──
    event_loop.run_app(&mut app)?;
}
```

binary crate 做的事情 `mkxp-graphics` 一概不知：
- winit 窗口和事件循环
- wgpu Instance/Adapter/Device 初始化
- 选择 GPU 后端（Vulkan/Metal/DX12）
- 协调音频、输入、文件系统
- 脚本线程和 `Graphics.update` 的 `FrameSync` 调度
- render host、FPS gate、render command queue 和 present mode 选择

---

## 与 Ruby 绑定的交互

### 生命周期

```
Ruby 脚本                           Rust 引擎
─────────                          ─────────
sprite = Sprite.new(viewport)
  │
  ├─→ magnus 调用 Rust 函数
  │     scene_graph.insert(
  │       parent: viewport_node_id,
  │       element: Box::new(Sprite::new(...)),
  │       z: 0, sprite_y: 0
  │     ) → NodeId
  │
  ├─→ 把 NodeId 存到 Ruby 对象的 ivar 中
  │
sprite.x = 100
  │
  ├─→ magnus 调用 Rust 函数
  │     sprite.set_x(100)
  │     (没有场景图操作，只是改内部状态)
  │
sprite.z = 5
  │
  ├─→ magnus 调用 Rust 函数
  │     scene_graph.set_z(node_id, 5)
  │
Graphics.update
  │
  └─→ graphics_state.update()
        scene_graph.composite(&mut ctx)
          → 遍历根子节点
            → Sprite::draw()
```

### GC 回收

```ruby
sprite = nil  # Ruby GC 触发
```

```rust
// Sprite 的 Drop 实现中：
impl Drop for Sprite {
    fn drop(&mut self) {
        // 通过某种渠道通知 SceneGraph 删除自己的 NodeId
        // 方案 A：持有 Arc<Mutex<SceneGraph>> 的弱引用
        // 方案 B：持有 mpsc Sender，往场景线程发删除消息
        // 方案 C（推荐）：Sprite 的 Drop 不做任何事。
        //   场景在每帧开始前，检查哪些 NodeId 对应的 Ruby 对象已经死了。
        //   通过 GC guard / finalizer 回调来标记。
    }
}
```

具体方案取决于 magnus 的 GC 集成方式。v1 中可以用简单的引用计数：
Ruby 对象持有 `Arc<SpriteData>`，SceneGraph 持有 `Weak<SpriteData>`。
当 Ruby GC 回收 Sprite 时，`Arc` 引用计数归零，SceneGraph 的 `Weak` 失效，
绘制时跳过即可。

---

## 渐进实现路径

与"七个里程碑"一一对应：

| 步骤 | 做什么 | 涉及的模块 | 状态 |
|------|--------|-----------|------|
| 1. 开窗 | wgpu + winit 初始化，清屏为纯色 | `GraphicsState` + `mkxp-window` | ✅ |
| 2. 纯色矩形 | 一个 Quad + FlatColor pipeline | `Quad` + `PipelineSet.flat_color` | ✅ |
| 3. 贴图 | 加载 PNG，创建纹理，画纹理四边形 | `Bitmap` + `PipelineSet.simple` | |
| 4. 移动 | 每帧改 transform，画动态精灵 | `Sprite` + `scene_graph.composite()` | |
| 5. 重叠 | 两个精灵，z 序不同 | `SceneGraph` Z 序 + `ensure_children_sorted` | |
| 6. 裁剪 | 一个 Viewport，子精灵被切掉 | `Viewport` + `ChildrenMode::BeforeSelf` + `ScissorStack` | |
| 7. 滤镜 | 后处理灰度/色调/亮度 | `PingPong` + `PostProcess` | |

每一步都是在上一步的代码基础上加东西，每步可见。

### 步骤 1 实现摘要

`mkxp-graphics` crate：
- `GraphicsState::new(device, queue, surface, config, w, h)` — 初始化
- `GraphicsState::update()` — 清屏 + present
- `GraphicsState::on_resize(w, h)` — 窗口缩放
- 纯函数单元测试（表面尺寸验证）
- `#[instrument]` + `tracing` 日志

`mkxp-window` crate（二进制）：
- winit 事件循环 → wgpu surface → GraphicsState 全链路
- 测试线程通过 `Arc<Mutex<GraphicsState>>` 改背景色，验证跨线程通信

### 步骤 2 实现摘要

新增模块：

| 文件 | 内容 |
|---|---|
| `geometry/vertex.rs` | `Vertex` — GPU 管线最小输入（position + color），`Pod + Zeroable` |
| `geometry/quad.rs` | `Quad` — 4 顶点 + 6 索引（`[0,1,2, 0,2,3]`）拼 2 三角形 |
| `pipeline/set.rs` | `PipelineSet` — FlatColor WGSL 内联编译 + uniform bind group |

帧率控制：

目标 frame-loop 使用 render host thread 等待 `FrameSync`，再用固定时间轴
`next_frame_at += frame_duration` 执行 `target_fps` 门控；严重落后时丢弃历史 timing
debt，避免补帧风暴。present mode 默认使用 `wgpu::PresentMode::Fifo`，由显示同步避免
tearing；脚本线程不会因为 winit main thread 被 macOS 输入法切换阻塞而长期卡在
`Graphics.update`。完整帧循环见 [`FRAME_LOOP_DESIGN.md`](FRAME_LOOP_DESIGN.md)。

---

## 类型索引

| Rust 类型 | 对应 mkxp-z | 文件 |
|-----------|------------|------|
| `GameWindow` | EventThread（窗口部分）+ main.cpp 窗口初始化 | binary crate `src/window.rs` |
| `NodeId` | `SceneElement*` | `crates/mkxp-graphics/src/scene/id.rs` |
| `SceneGraph` | `Scene` + 全局排序逻辑 | `crates/mkxp-graphics/src/scene/graph.rs` |
| `Drawable` (trait) | `SceneElement` | `crates/mkxp-graphics/src/scene/drawable.rs` |
| `DrawContext` | `glState` + `shState->shaders()` + `FBO::bind()` | `crates/mkxp-graphics/src/context.rs` |
| `Sprite` | `Sprite` + `SpritePrivate` | `crates/mkxp-graphics/src/element/sprite.rs` |
| `Viewport` | `Viewport` + `ViewportPrivate` | `crates/mkxp-graphics/src/element/viewport.rs` |
| `Bitmap` | `Bitmap` + `BitmapPrivate` | `crates/mkxp-graphics/src/texture/bitmap.rs` |
| `TexPool` | `TexPool` | `crates/mkxp-graphics/src/texture/pool.rs` |
| `Quad` | `Quad` | `crates/mkxp-graphics/src/geometry/quad.rs` |
| `QuadArray` | `QuadArray` | `crates/mkxp-graphics/src/geometry/quad_array.rs` |
| `PipelineSet` | `ShaderSet` | `crates/mkxp-graphics/src/pipeline/set.rs` |
| `SpriteUniforms` | 各 shader 的 uniform setter | `crates/mkxp-graphics/src/pipeline/uniform.rs` |
| `PingPong` | `PingPong` (graphics.cpp 内部) | `crates/mkxp-graphics/src/post.rs` |
| `GraphicsState` | `Graphics` + `GraphicsPrivate` | `crates/mkxp-graphics/src/lib.rs` |
| `Shared` | 跨线程共享状态容器 | binary crate |
| `InputState` | EventThread 输入数组（keyStates + mouseState） | binary crate |
| `ControlSignals` | AtomicFlag（rqTerm / rqReset 等） | binary crate |
| `FrameSync` | 帧调度同步原语（Mutex + Condvar） | binary crate |

> 帧循环的完整设计见 [FRAME_LOOP_DESIGN.md](FRAME_LOOP_DESIGN.md)。
