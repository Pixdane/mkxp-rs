//! mkxp-graphics — 纯渲染库。
//!
//! 不依赖 winit。接收 [`wgpu::Surface`] 作为外部参数。
//! 提供场景图、精灵、视口、位图、着色器、后处理。
//!
//! # 最小示例
//!
//! ```no_run
//! use mkxp_graphics::GraphicsState;
//!
//! # /*
//! let mut graphics = GraphicsState::new(
//!     device, queue, surface, surface_config, 640, 480
//! );
//! graphics.set_debug_clear_color(0.2, 0.6, 0.9);
//! graphics.update().expect("surface error");
//! # */
//! ```

pub mod scene;
pub mod element;
pub mod texture;
pub mod pipeline;
pub mod geometry;
pub mod context;
pub mod post;

use tracing::{debug, error, info, instrument};
use wgpu::{Device, Queue, Surface, SurfaceConfiguration};

/// 渲染层的顶层状态。
///
/// 不持有窗口。由外部（binary crate）传入 wgpu 资源后创建。
///
/// # 示例
///
/// ```no_run
/// use mkxp_graphics::GraphicsState;
///
/// # /*
/// let mut gfx = GraphicsState::new(
///     device, queue, surface, surface_config,
///     640, 480,
/// );
/// gfx.update().unwrap();
/// # */
/// ```
pub struct GraphicsState {
    /// wgpu 设备句柄。GPU 资源的创建入口。
    pub device: Device,

    /// 命令提交队列。渲染命令通过它发给 GPU。
    pub queue: Queue,

    /// 渲染输出目标。从 winit 窗口的 surface 创建。
    pub surface: Surface<'static>,

    /// surface 配置。`on_resize` 时更新尺寸后重新 configure。
    surface_config: SurfaceConfiguration,

    /// 当前窗口物理尺寸（像素）。
    pub window_size: (u32, u32),

    /// 临时调试背景色。测试线程写，`update()` 读。
    /// 场景图和精灵就绪后删除。
    debug_clear_color: [f64; 3],
}

impl GraphicsState {
    /// 创建渲染状态。
    ///
    /// `device`、`queue`、`surface`、`surface_config` 由二进制入口
    /// 在启动时创建并传入。`screen_width` 和 `screen_height` 是
    /// 游戏的逻辑分辨率（XP 为 640×480，VX/Ace 为 544×416）。
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
        Self {
            device,
            queue,
            surface,
            surface_config,
            window_size: (screen_width, screen_height),
            debug_clear_color: [0.0, 0.0, 0.0],
        }
    }

    /// 合成并显示一帧。
    ///
    /// 用当前的调试背景色清屏，然后提交到屏幕。
    /// 场景图和精灵就绪后，此方法将改为完整的合成管线。
    ///
    /// # 错误
    ///
    /// 当 surface 不可用时返回 [`wgpu::SurfaceError`]。
    /// [`SurfaceError::Lost`] 时已记录 `error!` 日志。
    #[instrument(skip(self))]
    pub fn update(&mut self) -> Result<(), wgpu::SurfaceError> {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(e @ wgpu::SurfaceError::Lost) => {
                error!("surface lost, device may need recreation");
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let [r, g, b] = self.debug_clear_color;

        let mut encoder = self.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("mkxp-frame"),
            },
        );

        {
            let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r,
                            g,
                            b,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();

        Ok(())
    }

    /// 窗口缩放时调用。更新内部缓存尺寸和 surface 配置。
    ///
    /// 由 winit 事件循环中的 `WindowEvent::Resized` 触发。
    /// 尺寸为零时（窗口最小化）跳过，避免 wgpu panic。
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
    }

    /// 设置调试背景色（临时 API）。
    ///
    /// 由测试线程调用，改变下一帧的清屏颜色。每个通道取值范围 0.0–1.0，
    /// 超出范围的值会被裁剪。
    ///
    /// 场景图和精灵就绪后此方法将被删除。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// # /*
    /// let mut gfx = GraphicsState::new(device, queue, surface, config, 640, 480);
    /// gfx.set_debug_clear_color(0.2, 0.6, 0.9);
    /// // 下一帧 update() 时背景变成蓝色
    /// # */
    /// ```
    pub fn set_debug_clear_color(&mut self, r: f64, g: f64, b: f64) {
        self.debug_clear_color = [clamp_color(r), clamp_color(g), clamp_color(b)];
    }
}

// ── 纯函数（可单测，不依赖 GPU） ──

/// 将颜色通道值裁剪到 [0.0, 1.0] 范围。
fn clamp_color(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// 检查 surface 尺寸是否有效（宽高均大于 0）。
///
/// 窗口最小化时 winit 可能传出 (0,0)，wgpu 拒绝零尺寸 surface。
fn valid_surface_size(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_color_in_range() {
        assert_eq!(clamp_color(0.5), 0.5);
        assert_eq!(clamp_color(0.0), 0.0);
        assert_eq!(clamp_color(1.0), 1.0);
    }

    #[test]
    fn clamp_color_below_zero() {
        assert_eq!(clamp_color(-0.5), 0.0);
        assert_eq!(clamp_color(-100.0), 0.0);
    }

    #[test]
    fn clamp_color_above_one() {
        assert_eq!(clamp_color(1.5), 1.0);
        assert_eq!(clamp_color(999.0), 1.0);
    }

    #[test]
    fn valid_surface_size_positive() {
        assert!(valid_surface_size(640, 480));
        assert!(valid_surface_size(1, 1));
        assert!(valid_surface_size(3840, 2160));
    }

    #[test]
    fn valid_surface_size_zero_rejected() {
        assert!(!valid_surface_size(0, 480));
        assert!(!valid_surface_size(640, 0));
        assert!(!valid_surface_size(0, 0));
    }
}
