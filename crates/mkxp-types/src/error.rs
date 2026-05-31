// Workspace-wide error vocabulary.

/// Shared error variants used by all downstream crates.
///
/// Each crate defines its own `thiserror` enum and uses `#[from]` to
/// transparently forward `MkxpError` variants. The binary entry-point
/// captures everything with `anyhow`.
///
/// # Examples
///
/// ```
/// use mkxp_types::MkxpError;
///
/// let e = MkxpError::Init("could not create window".into());
/// assert_eq!(e.to_string(), "init error: could not create window");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum MkxpError {
    /// File-system or I/O operation failed.
    Io(String),
    /// A file or byte-stream could not be parsed.
    Parse(String),
    /// A subsystem failed to initialise (window, audio, graphics).
    Init(String),
    /// An unexpected condition at runtime.
    Runtime(String),
    /// A feature or format that is not (yet) supported.
    Unsupported(String),
}

impl std::fmt::Display for MkxpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MkxpError::Io(msg) => write!(f, "IO error: {msg}"),
            MkxpError::Parse(msg) => write!(f, "parse error: {msg}"),
            MkxpError::Init(msg) => write!(f, "init error: {msg}"),
            MkxpError::Runtime(msg) => write!(f, "runtime error: {msg}"),
            MkxpError::Unsupported(msg) => write!(f, "unsupported: {msg}"),
        }
    }
}

impl std::error::Error for MkxpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        let e = MkxpError::Init("could not create window".into());
        assert_eq!(e.to_string(), "init error: could not create window");
    }
}
