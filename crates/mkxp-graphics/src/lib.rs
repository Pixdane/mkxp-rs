//! mkxp-graphics — 纯渲染库。
//!
//! 不依赖 winit。接收 [`wgpu::Surface`] 作为外部参数。
//! 提供场景图、精灵、视口、位图、着色器、后处理。

pub mod scene;
pub mod element;
pub mod texture;
pub mod pipeline;
pub mod geometry;
pub mod context;
pub mod post;

use tracing::{debug, error, info, instrument};
use wgpu::{Device, Queue, Surface, SurfaceConfiguration};

use geometry::Quad;
use pipeline::PipelineSet;

/// 渲染层的顶层状态。
///
/// 不持有窗口。由外部（binary crate）传入 wgpu 资源后创建。
pub struct GraphicsState {
    pub device: Device,
    pub queue: Queue,
    pub surface: Surface<'static>,
    surface_config: SurfaceConfiguration,

    pub window_size: (u32, u32),

    /// 预编译的 pipeline。
    pub pipelines: PipelineSet,

    /// uniform buffer + bind group（flat_color pipeline 用）。
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    /// 测试用四边形。
    test_quad: Quad,
}

impl GraphicsState {
    /// 创建渲染状态。
    #[instrument(skip(device, queue, surface, surface_config), fields(w = screen_width, h = screen_height))]
    pub fn new(
        device: Device,
        queue: Queue,
        surface: Surface<'static>,
        surface_config: SurfaceConfiguration,
        screen_width: u32,
        screen_height: u32,
    ) -> Self {
        info!("graphics state initialized");

        surface.configure(&device, &surface_config);

        let pipelines = PipelineSet::new(&device, surface_config.format);
        let (uniform_buffer, uniform_bind_group) = PipelineSet::create_uniform_bind_group(
            &device, surface_config.width, surface_config.height,
        );

        // 初始位置：画面中央一个 200×150 的红色方块
        let test_quad = Quad::new(
            &device,
            (surface_config.width as f32 - 200.0) / 2.0,
            (surface_config.height as f32 - 150.0) / 2.0,
            200.0,
            150.0,
            [1.0, 0.2, 0.2, 1.0],
        );

        Self {
            device,
            queue,
            surface,
            surface_config,
            window_size: (screen_width, screen_height),
            pipelines,
            uniform_buffer,
            uniform_bind_group,
            test_quad,
        }
    }

    /// 合成并显示一帧。
    #[instrument(skip(self))]
    pub fn update(&mut self) -> Result<(), wgpu::SurfaceError> {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Timeout) | Err(wgpu::SurfaceError::Outdated) => {
                // swapchain 没空闲纹理或 surface 过期，静默跳过
                return Ok(());
            }
            Err(e @ wgpu::SurfaceError::Lost) => {
                error!("surface lost, device may need recreation");
                return Err(e);
            }
            Err(e) => {
                error!("unexpected surface error: {e:?}");
                return Err(e);
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("mkxp-frame"),
            },
        );

        {
            // 清屏为黑色
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // 画测试四边形
            self.test_quad.flush(&self.queue);
            render_pass.set_pipeline(&self.pipelines.flat_color);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            self.test_quad.draw(&mut render_pass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();

        Ok(())
    }

    /// 窗口缩放时调用。
    #[instrument(skip(self), fields(w = width, h = height))]
    pub fn on_resize(&mut self, width: u32, height: u32) {
        if !valid_surface_size(width, height) {
            debug!("surface resize skipped: zero-size ({width}x{height})");
            return;
        }
        debug!("surface resized");
        self.window_size = (width, height);
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);

        // 更新 uniform 里的屏幕尺寸，让 shader 用新的投影
        PipelineSet::update_uniforms(&self.queue, &self.uniform_buffer, width, height);
    }

    /// 移动测试四边形（临时 API，场景图就绪后删除）。
    #[allow(clippy::too_many_arguments)]
    pub fn set_test_quad(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, g: f32, b: f32) {
        self.test_quad.set_pos(x, y, w, h);
        self.test_quad.set_color(r, g, b, 1.0);
    }
}

fn valid_surface_size(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_surface_size_positive() {
        assert!(valid_surface_size(640, 480));
        assert!(valid_surface_size(1, 1));
    }

    #[test]
    fn valid_surface_size_zero_rejected() {
        assert!(!valid_surface_size(0, 480));
        assert!(!valid_surface_size(640, 0));
        assert!(!valid_surface_size(0, 0));
    }
}
