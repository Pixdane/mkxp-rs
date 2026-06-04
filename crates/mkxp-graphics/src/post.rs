/// 后处理管线（乒乓缓冲、色调、亮度、颜色叠加）。
///
/// 对应 mkxp-z 的 PingPong + ScreenScene 的后处理部分。
/// 当前为空骨架。
pub struct PostProcess;

impl Default for PostProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl PostProcess {
    pub fn new() -> Self {
        Self
    }
}
