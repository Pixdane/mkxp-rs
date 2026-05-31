use clap::Parser;

use crate::Config;

#[derive(Parser, Debug)]
#[command(name = "mkxp-rs")]
struct Cli {
    /// RGSS version (1 = XP, 2 = VX, 3 = VX Ace)
    #[arg(long)]
    rgss_version: Option<String>,

    /// Run a single Ruby script instead of the full game
    #[arg(long)]
    custom_script: Option<String>,

    /// Path to the game root directory
    #[arg(long)]
    game_folder: Option<String>,

    /// Window title
    #[arg(long)]
    window_title: Option<String>,

    /// Logical resolution (WxH, e.g. 640x480)
    #[arg(long)]
    window_size: Option<String>,

    /// Start in fullscreen mode
    #[arg(long)]
    fullscreen: bool,

    /// Disable window resize
    #[arg(long)]
    no_resizable: bool,

    /// Scaling algorithm
    #[arg(long)]
    scale_mode: Option<String>,

    /// Disable vertical sync
    #[arg(long)]
    no_vsync: bool,

    /// Frame rate cap
    #[arg(long)]
    frame_rate: Option<u32>,

    /// Font size multiplier
    #[arg(long)]
    font_scale: Option<f64>,

    /// Font hinting level
    #[arg(long)]
    font_hinting: Option<String>,

    /// Disable font kerning
    #[arg(long)]
    no_kerning: bool,

    /// Disable outline text crop
    #[arg(long)]
    no_outline_crop: bool,

    /// Master volume (0.0-1.0)
    #[arg(long)]
    master_volume: Option<f64>,

    /// BGM volume
    #[arg(long)]
    bgm_volume: Option<f64>,

    /// SE volume
    #[arg(long)]
    se_volume: Option<f64>,

    /// MIDI synthesiser backend
    #[arg(long)]
    midi_synth: Option<String>,

    /// FluidSynth SoundFont path
    #[arg(long)]
    soundfont: Option<String>,

    /// Enable debug output
    #[arg(long)]
    debug: bool,

    /// Launch console window (Windows only)
    #[arg(long)]
    console: bool,

    /// FPS display mode (none / titlebar / console / both)
    #[arg(long)]
    show_fps: Option<String>,
}

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Config {
            ruby: crate::config::Ruby {
                rgss_version: cli.rgss_version,
                custom_script: cli.custom_script,
                ..Default::default()
            },
            window: crate::config::Window {
                title: cli.window_title,
                size: parse_size(&cli.window_size),
                fullscreen: if cli.fullscreen { Some(true) } else { None },
                resizable: if cli.no_resizable { Some(false) } else { None },
                ..Default::default()
            },
            graphics: crate::config::Graphics {
                vsync: if cli.no_vsync { Some(false) } else { None },
                frame_rate: cli.frame_rate,
                scale_mode: cli.scale_mode,
                ..Default::default()
            },
            paths: crate::config::Paths {
                game_folder: cli.game_folder,
                ..Default::default()
            },
            fonts: crate::config::Fonts {
                scale: cli.font_scale,
                hinting: cli.font_hinting,
                kerning: if cli.no_kerning { Some(false) } else { None },
                outline_crop: if cli.no_outline_crop { Some(false) } else { None },
                ..Default::default()
            },
            audio: crate::config::Audio {
                master_volume: cli.master_volume,
                bgm_volume: cli.bgm_volume,
                se_volume: cli.se_volume,
                midi_synth: cli.midi_synth,
                soundfont: cli.soundfont,
                ..Default::default()
            },
            debug: crate::config::Debug {
                mode: if cli.debug { Some(true) } else { None },
                console: if cli.console { Some(true) } else { None },
                show_fps: cli.show_fps,
            },
            ..Default::default()
        }
    }
}

fn parse_size(s: &Option<String>) -> Option<(i32, i32)> {
    s.as_ref().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    })
}

pub fn parse(args: &[String]) -> Result<Config, String> {
    let cli = Cli::try_parse_from(args).map_err(|e| e.to_string())?;
    Ok(Config::from(cli))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_args() {
        let args = vec![
            "mkxp-rs".into(),
            "--rgss-version".into(), "2".into(),
            "--fullscreen".into(),
            "--frame-rate".into(), "30".into(),
            "--window-size".into(), "640x480".into(),
        ];
        let cfg = parse(&args).unwrap();
        assert_eq!(cfg.ruby.rgss_version, Some("2".into()));
        assert_eq!(cfg.window.fullscreen, Some(true));
        assert_eq!(cfg.graphics.frame_rate, Some(30));
        assert_eq!(cfg.window.size, Some((640, 480)));
    }

    #[test]
    fn no_args_gives_empty_config() {
        let cfg = Config::from(Cli {
            rgss_version: None, custom_script: None, game_folder: None,
            window_title: None, window_size: None, fullscreen: false,
            no_resizable: false, scale_mode: None, no_vsync: false,
            frame_rate: None, font_scale: None, font_hinting: None,
            no_kerning: false, no_outline_crop: false, master_volume: None,
            bgm_volume: None, se_volume: None, midi_synth: None,
            soundfont: None, debug: false, console: false, show_fps: None,
        });
        assert!(cfg.ruby.rgss_version.is_none());
        assert!(cfg.window.size.is_none());
    }

    #[test]
    fn parse_size_format() {
        assert_eq!(parse_size(&Some("1920x1080".into())), Some((1920, 1080)));
        assert_eq!(parse_size(&Some("800x600".into())), Some((800, 600)));
        assert!(parse_size(&None).is_none());
        assert!(parse_size(&Some("invalid".into())).is_none());
    }

    #[test]
    fn debug_flag() {
        let args = vec!["mkxp-rs".into(), "--debug".into()];
        let cfg = parse(&args).unwrap();
        assert_eq!(cfg.debug.mode, Some(true));
        assert_eq!(cfg.debug.console, None);
    }
}
