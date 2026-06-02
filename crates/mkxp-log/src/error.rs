use mkxp_types::MkxpError;

/// Errors returned by `mkxp_log::init()`.
///
/// Follows the three-layer error model: shared vocabulary (`MkxpError`) →
/// crate-specific `LogError` → `anyhow` at the binary layer.
///
/// # Examples
///
/// ```
/// use mkxp_log::LogError;
///
/// let err = LogError::already_set();
/// assert_eq!(err.to_string(), "logger already initialised");
/// ```
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// The global subscriber has already been set. `init()` must be
    /// called exactly once per process.
    #[error("logger already initialised")]
    AlreadySet,

    /// Could not create the parent directory for a log file.
    #[error("failed to create log directory `{path}`")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Could not open or create the log file itself.
    #[error("failed to open log file `{path}`")]
    OpenFile {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Transparent pass-through for shared error vocabulary.
    #[error(transparent)]
    Mkxp(#[from] MkxpError),
}

impl LogError {
    /// Shorthand for an `AlreadySet` error.
    pub fn already_set() -> Self {
        LogError::AlreadySet
    }

    /// Shorthand for a directory-creation error with the source attached.
    pub fn create_dir(path: impl Into<String>, source: std::io::Error) -> Self {
        LogError::CreateDir {
            path: path.into(),
            source,
        }
    }

    /// Shorthand for a file-open error with the source attached.
    pub fn open_file(path: impl Into<String>, source: std::io::Error) -> Self {
        LogError::OpenFile {
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_already_set() {
        let err = LogError::already_set();
        assert_eq!(err.to_string(), "logger already initialised");
    }

    #[test]
    fn display_create_dir() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err = LogError::create_dir("/tmp/logs", io);
        let msg = err.to_string();
        assert!(msg.contains("/tmp/logs"));
    }

    #[test]
    fn display_open_file() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err = LogError::open_file("/tmp/logs/mkxp.log", io);
        let msg = err.to_string();
        assert!(msg.contains("mkxp.log"));
    }

    #[test]
    fn convenience_constructors() {
        let io = std::io::Error::from(std::io::ErrorKind::Other);
        let err = LogError::create_dir("/a", io);
        assert!(matches!(err, LogError::CreateDir { .. }));

        let io = std::io::Error::from(std::io::ErrorKind::Other);
        let err = LogError::open_file("/b", io);
        assert!(matches!(err, LogError::OpenFile { .. }));
    }
}
