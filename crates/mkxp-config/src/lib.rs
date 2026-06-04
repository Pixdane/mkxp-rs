//! mkxp-config — layered configuration loading for mkxp-rs.
//!
//! Reads configuration from five sources in priority order and merges them:
//!
//! 1. Command-line arguments (`--xxx` flags)
//! 2. Environment variables (`MKXP_*`)
//! 3. User config (`~/.config/mkxp-rs/mkxp.ron`)
//! 4. Game directory config (`mkxp.ron`)
//! 5. `Game.ini` (Title and Scripts only)
//!
//! Uses the `config` crate for RON, INI, and environment variable parsing,
//! `clap` for command-line argument parsing, and `merge` for layering sources.
//!
//! # Quick start
//!
//! ```rust,no_run
//! let cfg = mkxp_config::load(std::env::args().collect()).unwrap();
//! ```
//!
//! # Logging
//!
//! When the `tracing` subscriber is active (`mkxp_log::init()`), `load()`
//! emits the following structured events:
//!
//! | Event | Level | Content |
//! |-------|-------|---------|
//! | CLI args parsed | `debug` | `rgss_version`, `window.size`, `debug.mode`, `debug.log_level` |
//! | Env vars detected | `info` | when any `MKXP_*` variable is set |
//! | User config loaded | `info` | `path` to the user's `mkxp.ron` |
//! | Game config loaded | `info` | `"loaded game config from mkxp.ron"` |
//! | Game.ini loaded | `info` | `title` and `scripts` from `[Game]` section |
//! | Configuration done | `info` | final `rgss_version` |

pub mod config;
mod command_line;

use merge::Merge;

pub use config::Config;

/// Errors that can occur while loading configuration from any source.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("failed to build config: {0}")]
    Build(#[from] ::config::ConfigError),
    #[error("failed to parse CLI args: {0}")]
    Cli(String),
    #[error(transparent)]
    Mkxp(#[from] mkxp_types::MkxpError),
}

/// Load configuration from all sources and merge them.
///
/// Sources are applied from highest to lowest priority:
/// CLI args → environment → user config → game config → Game.ini.
/// Within the `merge` crate semantics, the first-merged source wins;
/// subsequent sources only fill fields that are still `None`.
///
/// # Example
///
/// ```rust,no_run
/// let cfg = mkxp_config::load(std::env::args().collect()).unwrap();
/// ```
pub fn load(cli_args: Vec<String>) -> Result<Config, SourceError> {
    // Start with CLI config (highest priority — merged first wins).
    let mut cfg = command_line::parse(&cli_args).map_err(SourceError::Cli)?;
    tracing::debug!(?cfg.ruby.rgss_version, ?cfg.window.size, ?cfg.window.fullscreen,
        ?cfg.debug.mode, ?cfg.debug.log_level,
        "CLI args parsed");

    // --- 2. Environment variables ---
    if let Ok(env_builder) = ::config::Config::builder()
        .add_source(::config::Environment::with_prefix("MKXP").separator("__"))
        .build()
        && let Ok(env_cfg) = env_builder.try_deserialize::<Config>() {
            let had_env = env_cfg.window.title.is_some()
                || env_cfg.debug.mode.is_some()
                || env_cfg.debug.log_level.is_some();
            cfg.merge(env_cfg);
            if had_env {
                tracing::info!("loaded env config (MKXP_* variables)");
            }
        }

    // --- 3. User config ---
    if let Some(user_path) = user_config_path()
        && let Ok(user_builder) = ::config::Config::builder()
            .add_source(::config::File::with_name(&user_path).required(false))
            .build()
            && let Ok(user_cfg) = user_builder.try_deserialize::<Config>() {
                cfg.merge(user_cfg);
                tracing::info!(path = %user_path, "loaded user config");
            }

    // --- 4. Game directory mkxp.ron ---
    if let Ok(ron_builder) = ::config::Config::builder()
        .add_source(::config::File::with_name("mkxp").required(false))
        .build()
        && let Ok(ron_cfg) = ron_builder.try_deserialize::<Config>() {
            cfg.merge(ron_cfg);
            tracing::info!("loaded game config from mkxp.ron");
        }

    // --- 5. Game.ini (lowest priority — fills remaining gaps) ---
    if let Ok(ini_builder) = ::config::Config::builder()
        .add_source(::config::File::with_name("Game").required(false))
        .build()
        && let Ok(ini_cfg) = ini_builder.try_deserialize::<IniHelper>() {
            apply_ini_to_config(&mut cfg, &ini_cfg);
            if let (Some(title), Some(scripts)) = (&ini_cfg.game.title, &ini_cfg.game.scripts) {
                tracing::info!(title, scripts, "loaded Game.ini");
            } else {
                tracing::info!("loaded Game.ini");
            }
        }

    tracing::info!(rgss_version = ?cfg.ruby.rgss_version, "configuration loaded");
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Game.ini support
// ---------------------------------------------------------------------------

/// Deserialization helper for Game.ini.
#[derive(Debug, serde::Deserialize)]
struct IniHelper {
    #[serde(rename = "Game")]
    game: IniGame,
}

#[derive(Debug, serde::Deserialize)]
struct IniGame {
    #[serde(rename = "Title")]
    title: Option<String>,
    #[serde(rename = "Scripts")]
    scripts: Option<String>,
}

/// Apply Game.ini values to a Config. Only `window.title` and
/// `ruby.scripts_path` are set; all other fields are left alone.
fn apply_ini_to_config(cfg: &mut Config, ini: &IniHelper) {
    if let Some(title) = &ini.game.title
        && !title.is_empty() {
            cfg.window.title = Some(title.clone());
        }
    if let Some(scripts) = &ini.game.scripts {
        cfg.ruby.scripts_path = Some(scripts.clone());
    }
}

/// Return the path to the user config file, if the home directory can be
/// determined.
fn user_config_path() -> Option<String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(format!("{home}/.config/mkxp-rs/mkxp.ron"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use merge::Merge;

    // ----------------------------------------------------------------
    // Defaults
    // ----------------------------------------------------------------

    #[test]
    fn default_config_has_all_fields_none() {
        let cfg = Config::default();
        assert!(cfg.ruby.rgss_version.is_none());
        assert!(cfg.window.title.is_none());
        assert!(cfg.window.size.is_none());
        assert!(cfg.graphics.vsync.is_none());
        assert!(cfg.audio.master_volume.is_none());
        assert!(cfg.debug.mode.is_none());
    }

    // ----------------------------------------------------------------
    // Merge semantics
    // ----------------------------------------------------------------

    /// `a.merge(b)` means "a keeps its value when both have one, b fills gaps".
    /// So to make the higher-priority source win, merge it first,
    /// then merge lower-priority sources to fill in missing fields.
    #[test]
    fn merge_higher_priority_first_wins() {
        let cli  = Config { window: config::Window { title: Some("CLI".into()), ..Default::default() }, ..Default::default() };
        let game = Config { window: config::Window { title: Some("Game".into()), ..Default::default() }, ..Default::default() };
        let mut cfg = cli;
        cfg.merge(game);
        // cli was merged first, so its value wins
        assert_eq!(cfg.window.title, Some("CLI".into()));
    }

    #[test]
    fn merge_none_keeps_existing_value() {
        let mut cfg = Config::default();
        let game = Config { window: config::Window { title: Some("Game".into()), ..Default::default() }, ..Default::default() };
        let empty = Config::default();
        cfg.merge(game);
        cfg.merge(empty);
        assert_eq!(cfg.window.title, Some("Game".into()));
    }

    #[test]
    fn merge_different_fields_coexist() {
        // source_a provides title, source_b provides size.
        // Merge order: a first (title wins), then b (fills size gap).
        let source_a = Config { window: config::Window { title: Some("Title".into()), ..Default::default() }, ..Default::default() };
        let source_b = Config { window: config::Window { size: Some((800, 600)), ..Default::default() }, ..Default::default() };
        let mut cfg = source_a;
        cfg.merge(source_b);
        assert_eq!(cfg.window.title, Some("Title".into()));
        assert_eq!(cfg.window.size, Some((800, 600)));
    }

    // ----------------------------------------------------------------
    // RON deserialization via the config crate
    // ----------------------------------------------------------------

    #[test]
    fn ron_deserialize_is_roundtrip() {
        let ron_content = r#"(
            ruby: (rgss_version: "1"),
            window: (title: "My Game", size: (800, 600), fullscreen: true),
            debug: (mode: true, show_fps: "titlebar"),
        )"#;

        let dir = std::env::temp_dir().join("mkxp_test_ron2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("mkxp.ron");
        let _ = std::fs::write(&path, ron_content);

        let result = ::config::Config::builder()
            .add_source(::config::File::with_name(path.to_str().unwrap()).required(false))
            .build();

        match result {
            Ok(b) => {
                match b.try_deserialize::<Config>() {
                    Ok(cfg) => {
                        assert_eq!(cfg.ruby.rgss_version, Some("1".into()));
                        assert_eq!(cfg.window.title, Some("My Game".into()));
                        assert_eq!(cfg.window.size, Some((800, 600)));
                    }
                    Err(e) => panic!("deserialize failed: {}", e),
                }
            }
            Err(e) => panic!("build failed: {}", e),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ----------------------------------------------------------------
    // Game.ini application
    // ----------------------------------------------------------------

    #[test]
    fn ini_sets_title_and_scripts() {
        let ini = IniHelper {
            game: IniGame {
                title: Some("Test Game".into()),
                scripts: Some("Data/Scripts.rxdata".into()),
            }
        };
        let mut cfg = Config::default();
        apply_ini_to_config(&mut cfg, &ini);
        assert_eq!(cfg.window.title, Some("Test Game".into()));
        assert_eq!(cfg.ruby.scripts_path, Some("Data/Scripts.rxdata".into()));
        assert!(cfg.graphics.vsync.is_none());
    }

    #[test]
    fn ini_empty_title_is_ignored() {
        let ini = IniHelper {
            game: IniGame { title: Some("".into()), scripts: None }
        };
        let mut cfg = Config::default();
        apply_ini_to_config(&mut cfg, &ini);
        assert_eq!(cfg.window.title, None);
        assert!(cfg.ruby.scripts_path.is_none());
    }

    // ----------------------------------------------------------------
    // Full pipeline simulation
    // ----------------------------------------------------------------

    /// Merge semantics: `a.merge(b)` keeps a's values and fills gaps from b.
    /// To get correct priority, merge from highest priority down to lowest.
    #[test]
    fn full_pipeline_respects_merge_order() {
        // Simulate CLI (1st, highest)
        let cli = Config {
            window: config::Window { size: Some((1920, 1080)), ..Default::default() },
            ..Default::default()
        };
        // Simulate game RON (4th)
        let ron = Config {
            window: config::Window { title: Some("RON".into()), size: Some((640, 480)), ..Default::default() },
            ..Default::default()
        };
        // Simulate INI (5th, lowest)
        let ini = Config {
            window: config::Window { title: Some("INI".into()), ..Default::default() },
            ruby: config::Ruby { scripts_path: Some("Scripts.rxdata".into()), ..Default::default() },
            ..Default::default()
        };

        // Merge from highest to lowest
        let mut cfg = cli;
        cfg.merge(ron);
        cfg.merge(ini);

        // CLI size wins (merged first)
        assert_eq!(cfg.window.size, Some((1920, 1080)));
        // RON title wins over INI title
        assert_eq!(cfg.window.title, Some("RON".into()));
        // INI scripts_path fills the gap
        assert_eq!(cfg.ruby.scripts_path, Some("Scripts.rxdata".into()));
    }
}
