pub mod vertex;
pub mod quad;

pub use vertex::Vertex;
pub use quad::Quad;

/// 矩形（像素坐标，单精度浮点）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
