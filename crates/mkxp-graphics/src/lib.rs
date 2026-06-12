//! mkxp-graphics — 纯渲染库。
//!
//! 不依赖 winit。接收 [`wgpu::Surface`] 作为外部参数。
//! 提供场景图、精灵、视口、位图、着色器、后处理。

pub mod context;
pub mod element;
pub mod geometry;
pub mod pipeline;
pub mod post;
pub mod scene;
pub mod texture;

use tracing::{debug, error, info, instrument};
use wgpu::{Device, Queue, Surface, SurfaceConfiguration};

use geometry::Quad;
use pipeline::PipelineSet;

/// Viewport 缩放模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportScaleMode {
    /// 保持游戏宽高比，完整显示在窗口/全屏 surface 内，居中，可能有黑边。
    Fit,
    /// 使用 n 倍游戏分辨率作为 viewport 尺寸，居中，可能有黑边。
    /// n=0 等价于 n=1。
    Integer(u32),
}

/// 渲染层的顶层状态。
///
/// 不持有窗口。由外部（binary crate）传入 wgpu 资源后创建。
pub struct GraphicsState {
    pub device: Device,
    pub queue: Queue,
    pub surface: Surface<'static>,
    surface_config: SurfaceConfiguration,

    /// 当前窗口物理尺寸。
    #[allow(dead_code)]
    window_size: (u32, u32),

    /// 游戏内容的固定分辨率。XP = (640,480), VX/Ace = (544,416)。
    game_size: (u32, u32),

    /// 目标帧率。可通过 `set_target_fps` 运行时修改。
    target_fps: u32,

    /// 窗口内游戏画面的区域（保持宽高比、居中、其余黑色填充）。
    /// 每次 on_resize 时重新计算。
    game_viewport: (u32, u32, u32, u32), // (x, y, w, h)

    /// 当前 viewport 缩放模式。
    viewport_scale_mode: ViewportScaleMode,

    /// 预编译的 pipeline。
    pub pipelines: PipelineSet,

    /// uniform buffer + bind group（flat_color pipeline 用）。
    #[allow(dead_code)]
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    /// 游戏画面区域背景四边形（区别于黑边）。
    bg_quad: Quad,

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
        target_fps: u32,
    ) -> Self {
        info!("graphics state initialized");

        surface.configure(&device, &surface_config);

        let game_size = (screen_width, screen_height);
        let viewport_scale_mode = ViewportScaleMode::Fit;
        let game_viewport = viewport_for_mode(
            surface_config.width,
            surface_config.height,
            game_size.0,
            game_size.1,
            viewport_scale_mode,
        );

        let pipelines = PipelineSet::new(&device, surface_config.format);
        // 渲染坐标系始终以游戏分辨率为准
        let (uniform_buffer, uniform_bind_group) =
            PipelineSet::create_uniform_bind_group(&device, game_size.0, game_size.1);

        // 初始位置：画面中央一个 200×150 的红色方块
        let win_w = surface_config.width;
        let win_h = surface_config.height;

        // 游戏画面背景：暗蓝色，填满整个游戏区域
        let bg_quad = Quad::new(
            &device,
            0.0,
            0.0,
            screen_width as f32,
            screen_height as f32,
            [0.05, 0.05, 0.15, 1.0],
        );

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
            window_size: (win_w, win_h),
            game_size,
            target_fps,
            game_viewport,
            viewport_scale_mode,
            bg_quad,
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("mkxp-frame"),
            });

        {
            let (vpx, vpy, vpw, vph) = self.game_viewport;

            // 清屏为黑色（全窗口），viewport 限制游戏画面范围
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

            // 将渲染限制在游戏画面区域内
            render_pass.set_viewport(vpx as f32, vpy as f32, vpw as f32, vph as f32, 0.0, 1.0);

            render_pass.set_pipeline(&self.pipelines.flat_color);
            render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);

            // 游戏区域背景（暗蓝色，区别于黑边）
            self.bg_quad.draw(&mut render_pass);

            // 测试四边形
            self.test_quad.flush(&self.queue);
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
        self.game_viewport = viewport_for_mode(
            width,
            height,
            self.game_size.0,
            self.game_size.1,
            self.viewport_scale_mode,
        );
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        // 不更新 uniform——shader 始终以游戏分辨率为坐标系
    }

    /// 设置 viewport 缩放模式并立即重新计算 game_viewport。
    pub fn set_viewport_scale_mode(&mut self, mode: ViewportScaleMode) {
        self.viewport_scale_mode = mode;
        let (w, h) = self.window_size;
        self.game_viewport = viewport_for_mode(w, h, self.game_size.0, self.game_size.1, mode);
    }

    /// 移动测试四边形（临时 API，场景图就绪后删除）。
    /// 目标帧率。
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }

    /// 设置目标帧率。运行时可变，下一帧生效。
    pub fn set_target_fps(&mut self, fps: u32) {
        self.target_fps = fps.clamp(1, 240);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_test_quad(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, g: f32, b: f32) {
        self.test_quad.set_pos(x, y, w, h);
        self.test_quad.set_color(r, g, b, 1.0);
    }
}

/// 计算窗口内游戏画面的显示区域。
///
/// 保持 `game_w:game_h` 宽高比不变，一个方向填满，另一个方向居中留黑边。
/// 返回 `(x, y, w, h)` 像素坐标。
fn letterbox_viewport(
    window_w: u32,
    window_h: u32,
    game_w: u32,
    game_h: u32,
) -> (u32, u32, u32, u32) {
    let window_ratio = window_w as f32 / window_h as f32;
    let game_ratio = game_w as f32 / game_h as f32;

    let (vpw, vph) = if window_ratio > game_ratio {
        // 窗口更宽：高度填满，宽度按比例缩小
        let h = window_h;
        let w = (window_h as f32 * game_ratio) as u32;
        (w, h)
    } else {
        // 窗口更高：宽度填满，高度按比例缩小
        let w = window_w;
        let h = (window_w as f32 / game_ratio) as u32;
        (w, h)
    };

    let x = (window_w.saturating_sub(vpw)) / 2;
    let y = (window_h.saturating_sub(vph)) / 2;

    (x, y, vpw, vph)
}

fn valid_surface_size(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

/// 根据缩放模式计算当前窗口/全屏 surface 内的游戏 viewport。
///
/// `Fit` 模式复用 `letterbox_viewport` 行为。
/// `Integer(n)` 使用 `n.max(1) * game_size` 居中。
pub fn viewport_for_mode(
    window_w: u32,
    window_h: u32,
    game_w: u32,
    game_h: u32,
    mode: ViewportScaleMode,
) -> (u32, u32, u32, u32) {
    match mode {
        ViewportScaleMode::Fit => letterbox_viewport(window_w, window_h, game_w, game_h),
        ViewportScaleMode::Integer(n) => {
            let scale = n.max(1);
            let vpw = game_w * scale;
            let vph = game_h * scale;
            if vpw > window_w || vph > window_h {
                debug!(
                    requested_scale = scale,
                    target_w = vpw,
                    target_h = vph,
                    surface_w = window_w,
                    surface_h = window_h,
                    "integer viewport does not fit surface; falling back to fit"
                );
                return letterbox_viewport(window_w, window_h, game_w, game_h);
            }
            let x = window_w.saturating_sub(vpw) / 2;
            let y = window_h.saturating_sub(vph) / 2;
            (x, y, vpw, vph)
        }
    }
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

    #[test]
    fn letterbox_exact_match() {
        // 窗口和游戏宽高比一致，填满
        let (x, y, w, h) = letterbox_viewport(640, 480, 640, 480);
        assert_eq!((x, y, w, h), (0, 0, 640, 480));
    }

    #[test]
    fn letterbox_window_wider() {
        // 窗口比游戏宽，左右留黑
        let (x, y, w, h) = letterbox_viewport(960, 480, 640, 480);
        assert_eq!(h, 480); // 高度填满
        assert_eq!(w, 640); // 宽度保持 4:3
        assert_eq!(x, 160); // 左边黑边
        assert_eq!(y, 0);
    }

    #[test]
    fn letterbox_window_taller() {
        // 窗口比游戏高，上下留黑
        let (x, y, w, h) = letterbox_viewport(640, 640, 640, 480);
        assert_eq!(w, 640); // 宽度填满
        assert_eq!(h, 480); // 高度保持 4:3
        assert_eq!(x, 0);
        assert_eq!(y, 80); // 上边黑边
    }

    #[test]
    fn letterbox_doubled() {
        let (x, y, w, h) = letterbox_viewport(1280, 960, 640, 480);
        assert_eq!((x, y, w, h), (0, 0, 1280, 960));
    }

    #[test]
    fn viewport_fit_matches_letterbox() {
        let viewport = viewport_for_mode(1920, 1080, 640, 480, ViewportScaleMode::Fit);
        assert_eq!(viewport, (240, 0, 1440, 1080));
    }

    #[test]
    fn viewport_integer_scale_is_centered() {
        let viewport = viewport_for_mode(1920, 1080, 640, 480, ViewportScaleMode::Integer(2));
        assert_eq!(viewport, (320, 60, 1280, 960));
    }

    #[test]
    fn viewport_integer_scale_clamps_to_at_least_one() {
        let viewport = viewport_for_mode(800, 600, 640, 480, ViewportScaleMode::Integer(0));
        assert_eq!(viewport, (80, 60, 640, 480));
    }

    #[test]
    fn viewport_integer_scale_falls_back_to_fit_when_surface_is_too_small() {
        let viewport = viewport_for_mode(2040, 1174, 640, 480, ViewportScaleMode::Integer(3));
        assert_eq!(viewport, (237, 0, 1565, 1174));
    }
}
