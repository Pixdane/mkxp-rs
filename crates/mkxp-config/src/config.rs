
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
    pub rgss_version: Option<String>,
    pub preload_scripts: Option<Vec<String>>,
    pub postload_scripts: Option<Vec<String>>,
    pub custom_script: Option<String>,
    pub launch_args: Option<Vec<String>>,
    pub use_script_names: Option<bool>,

    /// Filled by Game.ini Scripts field, not present in RON.
    #[serde(skip)]
    pub scripts_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Window {
    pub title: Option<String>,
    pub size: Option<(i32, i32)>,
    pub fullscreen: Option<bool>,
    pub resizable: Option<bool>,
    pub fixed_aspect_ratio: Option<bool>,
    pub integer_scaling: Option<bool>,
    pub frame_skip: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Graphics {
    pub vsync: Option<bool>,
    pub sync_to_refresh_rate: Option<bool>,
    pub frame_rate: Option<u32>,
    pub scale_mode: Option<String>,
    pub scale_up: Option<String>,
    pub scale_down: Option<String>,
    pub bitmap_scale_up: Option<String>,
    pub bitmap_scale_down: Option<String>,
    pub mipmaps: Option<bool>,
    pub bicubic_sharpness: Option<u32>,
    pub xbrz_factor: Option<f64>,
    pub hires: Option<Hires>,
    pub enable_blitting: Option<bool>,
    pub max_texture_size: Option<u32>,
    pub pixel_snap: Option<bool>,
}


#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Hires {
    pub enabled: Option<bool>,
    pub factor: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Paths {
    pub game_folder: Option<String>,
    pub rtp: Option<Vec<String>>,
    pub patches: Option<Vec<String>>,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Fonts {
    pub default_family: Option<String>,
    pub scale: Option<f64>,
    pub hinting: Option<String>,
    pub kerning: Option<bool>,
    pub outline_crop: Option<bool>,
    pub substitutions: Option<Vec<FontSubstitution>>,
    pub solid: Option<Vec<String>>,
    pub height_reporting: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FontSubstitution {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Input {
    pub key_bindings: Option<Vec<KeyBinding>>,
    pub gamepad_bindings: Option<Vec<KeyBinding>>,
    pub binding_names: Option<HashMap<String, String>>,
    pub enable_reset: Option<bool>,
    pub enable_settings: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct KeyBinding {
    pub key: String,
    pub action: String,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Audio {
    pub master_volume: Option<f64>,
    pub bgm_volume: Option<f64>,
    pub se_volume: Option<f64>,
    pub bgs_volume: Option<f64>,
    pub me_volume: Option<f64>,
    pub midi_synth: Option<String>,
    pub soundfont: Option<String>,
    pub midi_chorus: Option<bool>,
    pub midi_reverb: Option<bool>,
    pub se_source_count: Option<u32>,
    pub bgm_track_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Merge)]
pub struct Debug {
    pub mode: Option<bool>,
    pub console: Option<bool>,
    pub show_fps: Option<String>,
}
