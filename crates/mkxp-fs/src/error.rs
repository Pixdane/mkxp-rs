// Crate-specific error type for the virtual file system.
//
// Follows the workspace convention: each crate defines its own `thiserror`
// enum with crate-local variants plus a `#[from]` forwarding variant for
// the shared `mkxp_types::MkxpError` vocabulary.  The binary entry-point
// ultimately captures everything with `anyhow`.

use mkxp_types::MkxpError;

/// Errors that can occur during file-system operations.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    /// The requested path was not found in any mounted source.
    #[error("file not found: {path}")]
    NotFound { path: String },

    /// The path exists but is not a directory.
    #[error("not a directory: {path}")]
    NotADirectory { path: String },

    /// The virtual path attempted to escape a mount's root directory.
    #[error("path escapes mount root: {path}")]
    PathEscape { path: String },

    /// The supplied path string is not a valid virtual path.
    #[error("invalid path: {reason}")]
    InvalidPath { reason: String },

    /// An archive format is unknown or unsupported.
    #[error("unsupported archive format: {0}")]
    UnsupportedArchive(String),

    /// Shared error vocabulary (I/O, parse, init, runtime, unsupported).
    #[error(transparent)]
    Mkxp(#[from] MkxpError),
}

impl FsError {
    /// Convenience constructor for I/O errors.
    ///
    /// Wraps a `std::io::Error` into `MkxpError::Io` and forwards it.
    pub fn io(err: std::io::Error) -> Self {
        MkxpError::Io(err).into()
    }

    /// Convenience constructor for parse errors.
    pub fn parse(msg: impl Into<String>) -> Self {
        MkxpError::Parse(msg.into()).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display() {
        let e = FsError::NotFound { path: "Graphics/Title.png".into() };
        assert_eq!(e.to_string(), "file not found: Graphics/Title.png");
    }

    #[test]
    fn not_found_pattern_match() {
        let e = FsError::NotFound { path: "x.png".into() };
        if let FsError::NotFound { path } = e {
            assert_eq!(path, "x.png");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn invalid_path_display() {
        let e = FsError::InvalidPath { reason: "contains backslash".into() };
        assert!(e.to_string().contains("invalid path:"));
        assert!(e.to_string().contains("backslash"));
    }

    #[test]
    fn from_mkxp_parse_error() {
        let e: FsError = MkxpError::Parse("bad header".into()).into();
        assert_eq!(e.to_string(), "parse error: bad header");
    }

    #[test]
    fn from_mkxp_io_error() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let e: FsError = MkxpError::Io(io).into();
        let msg = e.to_string();
        assert!(msg.starts_with("IO error: "));
        assert!(msg.contains("file missing"));
    }

    #[test]
    fn convenience_io() {
        let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let e = FsError::io(io);
        let msg = e.to_string();
        assert!(msg.starts_with("IO error: "));
    }

    #[test]
    fn convenience_parse() {
        let e = FsError::parse("bad header");
        assert_eq!(e.to_string(), "parse error: bad header");
    }
}
