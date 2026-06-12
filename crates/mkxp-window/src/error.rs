use mkxp_types::MkxpError;

use crate::render_host::RenderError;
use crate::window_control::WindowControllerError;

// ── Main result alias ──

pub(crate) type ScriptRunResult = Result<ScriptExit, ScriptError>;

// ── Error types ──

#[derive(Debug, thiserror::Error)]
pub(crate) enum WindowError {
    #[error(transparent)]
    WindowController(#[from] WindowControllerError),
    #[error("failed to create wgpu surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("failed to request GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("script error: {0}")]
    Script(String),
    #[error("script thread panicked: {0}")]
    ScriptPanic(String),
    #[error("render error: {0}")]
    Render(#[from] RenderError),
    #[error(transparent)]
    Mkxp(#[from] MkxpError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScriptExit {
    Finished,
    ShutdownRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScriptError {
    #[allow(
        dead_code,
        reason = "real Ruby exceptions will construct this once mkxp-binding is wired"
    )]
    Message(String),
    Panic(String),
}

impl std::fmt::Display for ScriptExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finished => f.write_str("script finished"),
            Self::ShutdownRequested => f.write_str("script shutdown requested"),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Panic(message) => write!(f, "script thread panicked: {message}"),
        }
    }
}

impl From<ScriptError> for WindowError {
    fn from(error: ScriptError) -> Self {
        match error {
            ScriptError::Message(message) => Self::Script(message),
            ScriptError::Panic(message) => Self::ScriptPanic(message),
        }
    }
}

// ── Utility ──

pub(crate) fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_payload_to_string_preserves_string_messages() {
        assert_eq!(
            panic_payload_to_string(Box::new("boom")),
            "boom".to_string()
        );
        assert_eq!(
            panic_payload_to_string(Box::new("kaboom".to_string())),
            "kaboom".to_string()
        );
    }

    #[test]
    fn window_error_displays_script_panic() {
        let err = WindowError::ScriptPanic("boom".to_string());

        assert_eq!(err.to_string(), "script thread panicked: boom");
    }

    #[test]
    fn window_error_transparently_forwards_mkxp_error() {
        let err = WindowError::from(mkxp_types::MkxpError::Runtime("bad state".to_string()));

        assert_eq!(err.to_string(), "runtime error: bad state");
    }

    #[test]
    fn script_error_converts_to_window_error() {
        let err = WindowError::from(ScriptError::Message("ruby failed".to_string()));

        assert_eq!(err.to_string(), "script error: ruby failed");
    }

    #[test]
    fn window_error_displays_render_panic() {
        let err = WindowError::Render(RenderError::Panic("gpu oom".into()));

        assert_eq!(
            err.to_string(),
            "render error: render thread panicked: gpu oom"
        );
    }

    #[test]
    fn window_error_displays_surface_error() {
        // wgpu::SurfaceError::Lost display is system-locale dependent on some platforms.
        let err = WindowError::Render(RenderError::Surface(wgpu::SurfaceError::Lost));
        let msg = err.to_string();
        assert!(
            msg.starts_with("render error: surface error: "),
            "msg: {msg}"
        );
    }
}
