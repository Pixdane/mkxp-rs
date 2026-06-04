/// 子节点绘制策略。
pub enum ChildrenMode {
    /// 没有子节点，直接 draw(self)。
    None,
    /// 先递归绘制子节点，再 draw(self)。
    BeforeSelf,
}

/// 场景图中可以绘制自己的元素。
///
/// 对应 mkxp-z 的 `SceneElement`。
pub trait Drawable {
    fn children_mode(&self) -> ChildrenMode {
        ChildrenMode::None
    }
}

/// 父节点几何信息。
#[derive(Debug, Clone, Copy, Default)]
pub struct Geometry {
    pub rect: crate::geometry::Rect,
    pub origin: (f32, f32),
}
