// Drawable trait + ChildrenMode + Geometry
// v1 骨架，待实现。

use super::id::NodeId;
use crate::context::DrawContext;

pub enum ChildrenMode {
    None,
    BeforeSelf,
}

pub trait Drawable {
    fn draw(&self, _id: NodeId, _ctx: &mut DrawContext<'_>) {}
    fn children_mode(&self) -> ChildrenMode {
        ChildrenMode::None
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Geometry {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}
