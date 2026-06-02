use std::collections::HashMap;

use merge::Merge;
use serde::Deserialize;

/// Top-level configuration for mkxp-rs.
///
/// All sections and fields are `Option<T>` so that partial configurations
/// from different sources can be layered together with the `merge` crate.
/// A `None` value means "not set by this source", allowing lower-priority
/// sources to fill it.
///
/// When deserialized from RON or environment variables via the `config`
/// crate, missing sections are filled with their `Default` impls thanks to
/// `#[serde(default)]`.
/// # Example
///
/// ```rust
/// use mkxp_config::Config;
/// let cfg = Config::default();
/// assert!(cfg.window.title.is_none());
/// ```
#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Config {
    #[serde(default)]
    pub ruby: Ruby,
    #[serde(default)]
    pub window: Window,
    #[serde(default)]
    pub graphics: Graphics,
    #[serde(default)]
    pub paths: Paths,
    #[serde(default)]
    pub fonts: Fonts,
    #[serde(default)]
    pub input: Input,
    #[serde(default)]
    pub audio: Audio,
    #[serde(default)]
    pub debug: Debug,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Ruby {
    #[merge(strategy = merge::option::overwrite_none)]
    pub rgss_version: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub preload_scripts: Option<Vec<String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub postload_scripts: Option<Vec<String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub custom_script: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub launch_args: Option<Vec<String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub use_script_names: Option<bool>,

    /// Filled by Game.ini Scripts field, not present in RON.
    #[serde(skip)]
    #[merge(strategy = merge::option::overwrite_none)]
    pub scripts_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Window {
    #[merge(strategy = merge::option::overwrite_none)]
    pub title: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub size: Option<(i32, i32)>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub fullscreen: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub resizable: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub fixed_aspect_ratio: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub integer_scaling: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub frame_skip: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Graphics {
    #[merge(strategy = merge::option::overwrite_none)]
    pub vsync: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub sync_to_refresh_rate: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub frame_rate: Option<u32>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub scale_mode: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub scale_up: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub scale_down: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub bitmap_scale_up: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub bitmap_scale_down: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub mipmaps: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub bicubic_sharpness: Option<u32>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub xbrz_factor: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub hires: Option<Hires>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub enable_blitting: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub max_texture_size: Option<u32>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub pixel_snap: Option<bool>,
}


#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Hires {
    #[merge(strategy = merge::option::overwrite_none)]
    pub enabled: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub factor: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Paths {
    #[merge(strategy = merge::option::overwrite_none)]
    pub game_folder: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub rtp: Option<Vec<String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub patches: Option<Vec<String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Fonts {
    #[merge(strategy = merge::option::overwrite_none)]
    pub default_family: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub scale: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub hinting: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub kerning: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub outline_crop: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub substitutions: Option<Vec<FontSubstitution>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub solid: Option<Vec<String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub height_reporting: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FontSubstitution {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Input {
    #[merge(strategy = merge::option::overwrite_none)]
    pub key_bindings: Option<Vec<KeyBinding>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub gamepad_bindings: Option<Vec<KeyBinding>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub binding_names: Option<HashMap<String, String>>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub enable_reset: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub enable_settings: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct KeyBinding {
    pub key: String,
    pub action: String,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Audio {
    #[merge(strategy = merge::option::overwrite_none)]
    pub master_volume: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub bgm_volume: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub se_volume: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub bgs_volume: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub me_volume: Option<f64>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub midi_soundfont: Option<String>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub midi_chorus: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub midi_reverb: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub se_source_count: Option<u32>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub bgm_track_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Debug {
    #[merge(strategy = merge::option::overwrite_none)]
    pub mode: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub console: Option<bool>,
    #[merge(strategy = merge::option::overwrite_none)]
    pub show_fps: Option<String>,
    /// Log level override: `"error"`, `"warn"`, `"info"`, `"debug"`, or `"trace"`.
    /// When set, takes precedence over the `mode` flag for log verbosity.
    #[merge(strategy = merge::option::overwrite_none)]
    pub log_level: Option<String>,
}
