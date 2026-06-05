use bytemuck::{Pod, Zeroable};

/// 单个顶点——GPU 管线的最小输入单元。
///
/// 用 `#[repr(C)]` 保证内存布局和 WGSL 的 `@location` 一一对应。
/// `Pod + Zeroable` 允许安全地通过 `bytemuck::cast_slice` 转换为字节数组。
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// 像素坐标 (x, y)。对应 WGSL `@location(0) vec2<f32>`。
    pub position: [f32; 2],
    /// RGBA 颜色，0.0–1.0。对应 WGSL `@location(1) vec4<f32>`。
    pub color: [f32; 4],
}

impl Vertex {
    pub const fn new(x: f32, y: f32, r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            position: [x, y],
            color: [r, g, b, a],
        }
    }
}
