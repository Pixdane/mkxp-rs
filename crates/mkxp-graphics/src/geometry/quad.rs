use super::vertex::Vertex;
use wgpu::util::DeviceExt;

/// 一个可渲染的四边形 = 4 个顶点 + 6 个索引 = 2 个三角形。
///
/// 顶点数据同时存两份：CPU 端 `vertices`（快速修改）和 GPU 端
/// `vertex_buffer`（draw 时 GPU 读取）。`dirty` 标记表示 CPU 端
/// 有未上传的修改。
pub struct Quad {
    pub vertices: [Vertex; 4],
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    dirty: bool,
}

impl Quad {
    /// 创建一个新的四边形。
    ///
    /// `rect` 指定位置和尺寸（像素坐标），`color` 指定 RGBA 颜色。
    pub fn new(device: &wgpu::Device, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Self {
        let vertices = [
            Vertex::new(x, y, color[0], color[1], color[2], color[3]),
            Vertex::new(x + w, y, color[0], color[1], color[2], color[3]),
            Vertex::new(x + w, y + h, color[0], color[1], color[2], color[3]),
            Vertex::new(x, y + h, color[0], color[1], color[2], color[3]),
        ];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad vertex buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let index_data: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad index buffer"),
            contents: bytemuck::cast_slice(&index_data),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            vertices,
            vertex_buffer,
            index_buffer,
            index_count: 6,
            dirty: false,
        }
    }

    /// 移动四边形到新的位置。
    pub fn set_pos(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.vertices[0].position = [x, y];
        self.vertices[1].position = [x + w, y];
        self.vertices[2].position = [x + w, y + h];
        self.vertices[3].position = [x, y + h];
        self.dirty = true;
    }

    /// 改变四边形的颜色。
    pub fn set_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        let c = [r, g, b, a];
        for v in &mut self.vertices {
            v.color = c;
        }
        self.dirty = true;
    }

    /// 将脏的顶点数据从 CPU 上传到 GPU buffer。
    pub fn flush(&mut self, queue: &wgpu::Queue) {
        if !self.dirty {
            return;
        }
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        self.dirty = false;
    }

    /// 绘制四边形。
    ///
    /// 调用方需先设置好 pipeline 和 bind group。
    pub fn draw<'rp>(&self, render_pass: &mut wgpu::RenderPass<'rp>) {
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
