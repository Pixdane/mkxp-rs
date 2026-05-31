// Workspace-wide error vocabulary.

/// Shared error variants used by all downstream crates.
///
/// Each crate defines its own `thiserror` enum with crate-specific
/// variants and forwards `MkxpError` via `#[from]`.  The binary
/// entry-point captures everything with `anyhow`.
///
/// `MkxpError::Io` wraps a full `std::io::Error` so that callers can
/// inspect the error kind and source chain — not just a flattened string.
///
/// # Examples
///
/// ```rust
/// use mkxp_types::MkxpError;
///
/// let e = MkxpError::Init("could not create window".into());
/// assert_eq!(e.to_string(), "init error: could not create window");
/// ```
#[derive(Debug, thiserror::Error)]
pub enum MkxpError {
    /// File-system or I/O operation failed.
    #[error("IO error: {0}")]
    Io(std::io::Error),
    /// A file or byte-stream could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),
    /// A subsystem failed to initialise (window, audio, graphics).
    #[error("init error: {0}")]
    Init(String),
    /// An unexpected condition at runtime.
    #[error("runtime error: {0}")]
    Runtime(String),
    /// A feature or format that is not (yet) supported.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

// Let `std::io::Error` convert into `MkxpError::Io` automatically,
// which means any `std::io::Result<T>` can use `?` when the outer
// function returns `Result<_, MkxpError>` (or a crate error that
// forwards it via `#[from]`).
impl From<std::io::Error> for MkxpError {
    fn from(e: std::io::Error) -> Self {
        MkxpError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_init() {
        let e = MkxpError::Init("could not create window".into());
        assert_eq!(e.to_string(), "init error: could not create window");
    }

    #[test]
    fn display_io_wraps_std_error() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let e = MkxpError::Io(io);
        let msg = e.to_string();
        assert!(msg.starts_with("IO error: "));
        assert!(msg.contains("file missing"));
    }

    #[test]
    fn from_std_io_error() {
        let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let e: MkxpError = io.into();
        assert!(matches!(e, MkxpError::Io(_)));
    }

    #[test]
    fn question_mark_works() {
        fn read_thing() -> Result<(), MkxpError> {
            // `?` on a std::io::Error auto-converts via the From impl
            let _ = std::fs::read("/nonexistent/path/definitely/not/real")?;
            Ok(())
        }
        let err = read_thing().unwrap_err();
        assert!(matches!(err, MkxpError::Io(_)));
    }
}
