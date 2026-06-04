use bytemuck::{Pod, Zeroable};

/// 单个顶点。对应 mkxp-z 的 `Vertex`。
///
/// wgpu 要求顶点数据实现 `Pod` 和 `Zeroable` 才能直接写入 GPU 缓冲区。
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub tex_coord: [f32; 2],
    pub color: [f32; 4],
}
