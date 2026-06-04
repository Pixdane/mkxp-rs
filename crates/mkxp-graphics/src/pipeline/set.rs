/// 预编译的 pipeline 集合。
///
/// 对应 mkxp-z 的 `ShaderSet`。
/// 当前为空骨架。
pub struct PipelineSet;

impl Default for PipelineSet {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineSet {
    pub fn new() -> Self {
        Self
    }
}
