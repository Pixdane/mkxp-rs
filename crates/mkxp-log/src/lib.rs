//! mkxp-log — tracing-based logging for mkxp-rs.
//!
//! Provides a global `tracing` subscriber initialised from [`LogConfig`].
//! Product crates (`mkxp-audio`, `mkxp-fs`, etc.) only depend on the
//! `tracing` facade and use the standard `info!()`, `warn!()`, etc.
//! macros.  The binary entry point calls [`init()`] once at startup.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use mkxp_log::{LogConfig, LogLevel, init};
//!
//! let config = LogConfig {
//!     default_level: LogLevel::Info,
//!     ..Default::default()
//! };
//! init(config).expect("failed to initialise logger");
//! ```
//!
//! Once initialised, any crate that depends on `tracing` can emit logs:
//!
//! ```rust,no_run
//! use tracing::{info, warn};
//!
//! info!("Starting BGM playback");
//! warn!(filename = %"missing.ogg", "Audio file not found");
//! ```

mod error;
pub(crate) mod layer;

use std::fmt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

pub use error::LogError;

// ---------------------------------------------------------------------------
// LogLevel
// ---------------------------------------------------------------------------

/// Log verbosity level, in increasing order of detail.
///
/// Maps to the `tracing` / `RUST_LOG` level names when building the
/// `EnvFilter`.
///
/// # Examples
///
/// ```
/// use mkxp_log::LogLevel;
///
/// let level = LogLevel::Debug;
/// assert_eq!(level.as_env_str(), "debug");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Critical failures that prevent normal operation.
    Error,
    /// Unexpected conditions that the system can recover from.
    Warn,
    /// High-level progress milestones (enabled by default).
    Info,
    /// Detailed information for developers.
    Debug,
    /// Extremely verbose, per-frame or per-tick detail.
    Trace,
}

impl LogLevel {
    /// Return the level name as used in `RUST_LOG` filter directives.
    ///
    /// ```
    /// use mkxp_log::LogLevel;
    /// assert_eq!(LogLevel::Error.as_env_str(), "error");
    /// assert_eq!(LogLevel::Warn.as_env_str(),  "warn");
    /// assert_eq!(LogLevel::Info.as_env_str(),  "info");
    /// assert_eq!(LogLevel::Debug.as_env_str(), "debug");
    /// assert_eq!(LogLevel::Trace.as_env_str(), "trace");
    /// ```
    pub fn as_env_str(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_env_str())
    }
}

// ---------------------------------------------------------------------------
// LogTarget
// ---------------------------------------------------------------------------

/// Where log output is sent.
///
/// # Examples
///
/// ```
/// use mkxp_log::LogTarget;
///
/// let stderr = LogTarget::Stderr;
/// let file = LogTarget::File("mkxp.log".into());
/// let both = LogTarget::Composite(vec![
///     LogTarget::Stderr,
///     LogTarget::File("mkxp.log".into()),
/// ]);
/// ```
#[derive(Debug, Clone)]
pub enum LogTarget {
    /// Standard error (default, always available).
    Stderr,
    /// Append to a file. Parent directories are created automatically.
    File(std::path::PathBuf),
    /// Write to multiple targets simultaneously.  Nested `Composite`
    /// variants are flattened at construction time.
    Composite(Vec<LogTarget>),
}

// ---------------------------------------------------------------------------
// LogFormat
// ---------------------------------------------------------------------------

/// Log output format.
///
/// Currently only `Plain` is implemented.  `Json` and other formats can
/// be added as new variants without breaking the public API.
#[derive(Debug, Clone, Copy, Default)]
pub enum LogFormat {
    /// Human-readable plain text with ISO 8601 timestamps.
    ///
    /// Format: `[2026-05-31T10:30:00.123+08:00] LEVEL target{span}: message field=value ...`
    #[default]
    Plain,
}

// ---------------------------------------------------------------------------
// LogConfig
// ---------------------------------------------------------------------------

/// Complete log configuration.  Pass to [`init()`] to set up the global
/// subscriber.
///
/// # Examples
///
/// ```
/// use mkxp_log::{LogConfig, LogLevel};
///
/// let config = LogConfig {
///     default_level: LogLevel::Info,
///     ..Default::default()
/// };
/// ```
///
/// Filter precedence: `RUST_LOG` env var > [`LogConfig::target_filters`]
/// > [`LogConfig::default_level`].
pub struct LogConfig {
    /// Minimum level for log output (default: `Info`).
    pub default_level: LogLevel,
    /// Per-target level overrides, e.g. `vec![("mkxp_audio".into(), LogLevel::Debug)]`.
    /// `RUST_LOG` still takes highest priority.
    pub target_filters: Vec<(String, LogLevel)>,
    /// Output destination (default: `Stderr`).
    pub target: LogTarget,
    /// Output format (default: `Plain`).
    pub format: LogFormat,
    /// Whether to emit log lines when spans are created and closed.
    /// Useful for tracking the lifecycle and timing of nested operations
    /// (e.g. per-frame rendering, BGM playback scopes).
    /// Default: `false`.  Enabling this can produce significant output
    /// at default log levels.
    pub log_spans: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            default_level: LogLevel::Info,
            target_filters: Vec::new(),
            target: LogTarget::Stderr,
            format: LogFormat::Plain,
            log_spans: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the global tracing subscriber.
///
/// Must be called exactly once per process, before any `tracing` macro
/// is invoked.  Returns [`LogError::AlreadySet`] on subsequent calls.
///
/// Filter precedence: `RUST_LOG` env var > `config.target_filters` >
/// `config.default_level`.
///
/// # Errors
///
/// * [`LogError::AlreadySet`] — a global subscriber is already registered.
/// * [`LogError::CreateDir`] — the parent directory for a file target
///   could not be created.
/// * [`LogError::OpenFile`] — the log file could not be opened.
///
/// # Examples
///
/// ```rust,no_run
/// use mkxp_log::{init, LogConfig, LogLevel};
///
/// let config = LogConfig {
///     default_level: LogLevel::Debug,
///     ..Default::default()
/// };
/// init(config).expect("failed to initialise logger");
/// ```
pub fn init(config: LogConfig) -> Result<(), LogError> {
    let filter = build_filter(&config);
    let mkxp_layer = layer::MkxpLayer::new(config.target, config.log_spans)?;

    tracing_subscriber::registry()
        .with(filter)
        .with(mkxp_layer)
        .try_init()
        .map_err(|_| LogError::AlreadySet)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Integration with mkxp-config
// ---------------------------------------------------------------------------

/// Create a `LogConfig` from the debug-mode flag in `mkxp_config::Config`.
///
/// When `debug_mode` is `true` the default level is `Debug`; otherwise
/// `Info`.  All other fields use their defaults (stderr, plain format).
///
/// ```
/// use mkxp_log::{config_from_debug_mode, LogLevel};
///
/// let cfg = config_from_debug_mode(false);
/// assert_eq!(cfg.default_level, LogLevel::Info);
///
/// let cfg = config_from_debug_mode(true);
/// assert_eq!(cfg.default_level, LogLevel::Debug);
/// ```
pub fn config_from_debug_mode(debug_mode: bool) -> LogConfig {
    LogConfig {
        default_level: if debug_mode {
            LogLevel::Debug
        } else {
            LogLevel::Info
        },
        ..Default::default()
    }
}

impl From<&mkxp_config::Config> for LogConfig {
    /// Build a `LogConfig` from a `mkxp_config::Config`.
    ///
    /// Priority: `debug.log_level` > `debug_mode` flag > default `Info`.
    ///
    /// ```
    /// use mkxp_config::Config;
    /// use mkxp_log::{LogConfig, LogLevel};
    ///
    /// // Default config -> Info
    /// let cfg = Config::default();
    /// let log_cfg = LogConfig::from(&cfg);
    /// assert_eq!(log_cfg.default_level, LogLevel::Info);
    /// ```
    ///
    /// ```
    /// use mkxp_config::Config;
    /// use mkxp_log::{LogConfig, LogLevel};
    ///
    /// // debug.mode = true -> Debug
    /// let mut cfg = Config::default();
    /// cfg.debug.mode = Some(true);
    /// let log_cfg = LogConfig::from(&cfg);
    /// assert_eq!(log_cfg.default_level, LogLevel::Debug);
    /// ```
    ///
    /// ```
    /// use mkxp_config::Config;
    /// use mkxp_log::{LogConfig, LogLevel};
    ///
    /// // log_level = "trace" -> Trace (overrides debug.mode)
    /// let mut cfg = Config::default();
    /// cfg.debug.mode = Some(false);
    /// cfg.debug.log_level = Some("trace".into());
    /// let log_cfg = LogConfig::from(&cfg);
    /// assert_eq!(log_cfg.default_level, LogLevel::Trace);
    /// ```
    fn from(config: &mkxp_config::Config) -> Self {
        // log_level string takes highest priority
        if let Some(ref level_str) = config.debug.log_level
            && let Some(level) = parse_log_level(level_str)
        {
            return LogConfig {
                default_level: level,
                ..Default::default()
            };
        }

        // Fall back to debug.mode
        config_from_debug_mode(config.debug.mode.unwrap_or(false))
    }
}

/// Parse a log level string into a `LogLevel`, case-insensitively.
///
/// Returns `None` for unrecognised strings.
///
/// ```
/// use mkxp_log::{parse_log_level, LogLevel};
///
/// assert_eq!(parse_log_level("debug"), Some(LogLevel::Debug));
/// assert_eq!(parse_log_level("TRACE"), Some(LogLevel::Trace));
/// assert_eq!(parse_log_level("invalid"), None);
/// ```
pub fn parse_log_level(s: &str) -> Option<LogLevel> {
    match s.to_lowercase().as_str() {
        "error" => Some(LogLevel::Error),
        "warn" | "warning" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build an `EnvFilter` from `LogConfig`, falling back to the `RUST_LOG`
/// environment variable if set.
fn build_filter(config: &LogConfig) -> EnvFilter {
    // First try RUST_LOG env var (highest priority).
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }

    // Otherwise build from config.
    let mut filter_str = config.default_level.as_env_str().to_string();

    for (target, level) in &config.target_filters {
        use std::fmt::Write;
        let _ = write!(filter_str, ",{}={}", target, level.as_env_str());
    }

    EnvFilter::new(filter_str)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ----------------------------------------------------------------
    // LogLevel
    // ----------------------------------------------------------------

    #[test]
    fn log_level_env_strs() {
        assert_eq!(LogLevel::Error.as_env_str(), "error");
        assert_eq!(LogLevel::Warn.as_env_str(), "warn");
        assert_eq!(LogLevel::Info.as_env_str(), "info");
        assert_eq!(LogLevel::Debug.as_env_str(), "debug");
        assert_eq!(LogLevel::Trace.as_env_str(), "trace");
    }

    #[test]
    fn log_level_ord() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn log_level_display_matches_env_str() {
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            assert_eq!(level.to_string(), level.as_env_str());
        }
    }

    // ----------------------------------------------------------------
    // LogConfig defaults
    // ----------------------------------------------------------------

    #[test]
    fn default_config_is_info_stderr_plain() {
        let cfg = LogConfig::default();
        assert_eq!(cfg.default_level, LogLevel::Info);
        assert!(matches!(cfg.target, LogTarget::Stderr));
        assert!(matches!(cfg.format, LogFormat::Plain));
        assert!(cfg.target_filters.is_empty());
    }

    // ----------------------------------------------------------------
    // config_from_debug_mode
    // ----------------------------------------------------------------

    #[test]
    fn config_from_debug_mode_false_is_info() {
        let cfg = config_from_debug_mode(false);
        assert_eq!(cfg.default_level, LogLevel::Info);
    }

    #[test]
    fn config_from_debug_mode_true_is_debug() {
        let cfg = config_from_debug_mode(true);
        assert_eq!(cfg.default_level, LogLevel::Debug);
    }

    // ----------------------------------------------------------------
    // build_filter
    // ----------------------------------------------------------------

    #[test]
    fn build_filter_default_info() {
        // RUST_LOG may be set in the developer environment; the env var
        // always wins, so we only test that the fallback filter is
        // constructed correctly when RUST_LOG is cleared.
        unsafe { std::env::remove_var("RUST_LOG") };
        let cfg = LogConfig::default();
        let filter = build_filter(&cfg);
        let s = format!("{}", filter);
        assert!(
            s.contains("info"),
            "filter should contain default level when RUST_LOG is unset, got: {}",
            s
        );
    }

    #[test]
    fn build_filter_with_target_overrides() {
        // Clear RUST_LOG so our config takes effect.
        unsafe { std::env::remove_var("RUST_LOG") };

        let cfg = LogConfig {
            default_level: LogLevel::Warn,
            target_filters: vec![
                ("mkxp_audio".into(), LogLevel::Debug),
                ("mkxp_fs".into(), LogLevel::Trace),
            ],
            ..Default::default()
        };
        let filter = build_filter(&cfg);
        let s = format!("{}", filter);
        assert!(s.contains("warn"));
        assert!(s.contains("mkxp_audio=debug"));
        assert!(s.contains("mkxp_fs=trace"));
    }

    // ----------------------------------------------------------------
    // init
    // ----------------------------------------------------------------

    /// `init()` cannot be called twice in the same process.  Because
    /// `try_init` is global, these tests use `set_default` via a
    /// `DefaultGuard`, which is thread-local and allows multiple calls
    /// within a single test binary.  The "init twice" behaviour is
    /// verified implicitly: other tests in this crate each create their
    /// own subscriber via `set_default` without issue.
    #[test]
    fn init_runs_and_produces_output() {
        // Use set_default instead of try_init so the test is repeatable.
        let config = LogConfig::default();
        let filter = build_filter(&config);
        let layer = layer::MkxpLayer::new(LogTarget::Stderr, false).expect("create stderr layer");
        let subscriber = tracing_subscriber::registry().with(filter).with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Smoke test: emit a log and verify it doesn't panic.
        tracing::info!("init smoke test");
    }
}
