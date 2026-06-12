# mkxp-rs Configuration System

## Overview

mkxp-rs reads configuration from 5 sources, merged in priority order. The highest-priority source overrides all lower ones.

```
MKXP_* environment variables    highest priority
    |
--xxx CLI arguments
    |
~/.config/mkxp-rs/mkxp.ron      user-level config
    |
game-dir/mkxp.ron               engine config
    |
game-dir/Game.ini               game metadata
```

The `mkxp-config` crate handles reading and merging, producing a single `Config` struct consumed by downstream crates.

Libraries used: `ron` + `serde` for the config file format, `rust-ini` for Game.ini parsing, `clap` for CLI argument parsing.

Reference examples are located under `crates/mkxp-config/`: `mkxp.ron` and `Game.ini`.

---

## Engine Config (mkxp.ron)

The engine config uses [RON](https://github.com/ron-rs/ron) format. Every field is optional; omitted fields fall back to the Rust `Default` value.

### ruby

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `rgss_version` | `"1"` `"2"` `"3"` | `"3"` | Which RGSS version to target (1 = XP, 2 = VX, 3 = VX Ace). |
| `preload_scripts` | `[String]` | `[]` | Ruby scripts to load before the game scripts are executed. |
| `postload_scripts` | `[String]` | `[]` | Ruby scripts to load immediately before rgss_main (RGSS3 only). |
| `custom_script` | `Option<String>` | `None` | If set, run only this script instead of loading the full game. |
| `launch_args` | `[String]` | `[]` | Arguments forwarded to the Ruby script as `ARGV`. |
| `use_script_names` | `bool` | `true` | When true, Ruby backtraces show script filenames rather than internal indices. |

### window

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `title` | `String` | `""` | Window title. When empty, the value from `Game.ini` Title is used. |
| `size` | `(i32, i32)` | `(640, 480)` | Logical resolution in pixels. `(0, 0)` uses the RGSS version default. |
| `fullscreen` | `bool` | `false` | Whether to start in fullscreen mode. Alt+Enter toggles at runtime regardless of this setting. |
| `resizable` | `bool` | `true` | Whether the user can drag the window edges to change its size. |
| `fixed_aspect_ratio` | `bool` | `true` | When the window is resized, preserve the game screen aspect ratio with letterboxing. |
| `integer_scaling` | `bool` | `false` | Scale the game screen by an integer factor before filling remaining space. |
| `frame_skip` | `bool` | `false` | Skip rendering a frame when the engine is running behind schedule. |

### graphics

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `vsync` | `bool` | `true` | Wait for the display vertical blank before swapping buffers to prevent screen tearing. |
| `sync_to_refresh_rate` | `bool` | `false` | Match frame timing to the display refresh rate, and report the true frame rate back to Ruby scripts. Force-disabled if the refresh rate cannot be determined. |
| `frame_rate` | `u32` | `60` | Cap the frame rate to this value. The runtime clamps configured values to `1..=240`. |
| `scale_mode` | ScaleMode | `"bilinear"` | Default scaling algorithm for screen upscale, downscale, and bitmap scaling. One of `"nearest"` `"bilinear"` `"bicubic"` `"lanczos3"` `"xbrz"`. |
| `scale_up` | `Option<ScaleMode>` | `None` | Override the screen upscale algorithm. `None` inherits from `scale_mode`. |
| `scale_down` | `Option<ScaleMode>` | `None` | Override the screen downscale algorithm. |
| `bitmap_scale_up` | `Option<ScaleMode>` | `None` | Override the bitmap upscale algorithm. |
| `bitmap_scale_down` | `Option<ScaleMode>` | `None` | Override the bitmap downscale algorithm. |
| `mipmaps` | `bool` | `false` | Enable mipmap interpolation when downscaling. Only effective when `scale_down` is bilinear. |
| `bicubic_sharpness` | `u32` | `100` | Sharpness parameter for bicubic scaling. Range 0 to 200. |
| `xbrz_factor` | `f64` | `4.0` | Scale factor for the xBRZ algorithm. |
| `hires.enabled` | `bool` | `false` | When enabled, load higher-resolution versions of bitmaps from the `Hires` subdirectory. |
| `hires.factor` | `f64` | `4.0` | Scale factor of hi-res textures relative to the original bitmaps. |
| `enable_blitting` | `bool` | `true` | Use hardware framebuffer blitting when supported. Force-disabled when using non-nearest/non-bilinear scaling. |
| `max_texture_size` | `u32` | `0` | Maximum texture dimension. `0` means use the hardware limit. |
| `pixel_snap` | `bool` | `false` | Lock all rendering to integer pixel boundaries. When disabled, sprites get sub-pixel precision. |

### paths

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `game_folder` | `Option<String>` | `None` | Path to the game root directory. `None` means the current working directory. |
| `rtp` | `[String]` | `[]` | RPG Maker RTP archive paths (directories, zip files, or encrypted archives). |
| `patches` | `[String]` | `[]` | Mod or patch paths, searched before `game_folder`. |
| `icon_path` | `Option<String>` | `None` | Path to a custom window icon (Linux only). `None` uses the built-in default. |

### fonts

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_family` | `String` | `"Arial"` | Font family to use when a script requests a non-existent font. |
| `scale` | `f64` | `1.0` | Global multiplier applied to all font sizes. |
| `hinting` | Hinting | `"none"` | Hinting level: `"normal"` `"light"` `"mono"` `"none"`. RGSS does not use hinting; `"none"` gives the most accurate appearance. |
| `kerning` | `bool` | `false` | Whether to enable character kerning. RGSS does not use kerning. |
| `outline_crop` | `bool` | `true` | Crop the top row and left column of outlined text. Matches RGSS behaviour. |
| `substitutions` | `[{from, to}]` | `[]` | Font family substitution rules, mapping requested names to actual fonts. |
| `solid` | `[String]` | `[]` | Font families rendered without alpha blending (cached as bitmaps for performance). |
| `height_reporting` | HeightMode | `"nominal"` | How text height is calculated: `"nominal"` uses the font metric (RGSS), `"rendered"` uses the actual pixel height. |

### input

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `key_bindings` | `[{key, action}]` | `[]` | Custom keyboard bindings, mapping physical key names to game action names. |
| `gamepad_bindings` | `[{key, action}]` | `[]` | Custom gamepad bindings, same format as keyboard bindings. |
| `binding_names` | `{action: name}` | `{}` | Display names for actions shown in the F1 key configuration menu. |
| `enable_reset` | `bool` | `true` | Whether pressing F12 resets the game. |
| `enable_settings` | `bool` | `true` | Whether pressing F1 opens the key binding settings menu. |

### audio

> **Changes from mkxp-z:** The `midi_synth` config option has been removed (rustysynth is the only MIDI backend). The `soundfont` option has been renamed to `midi_soundfont`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `master_volume` | `f64` | `1.0` | Master volume multiplier. Range 0.0 to 1.0. |
| `bgm_volume` | `f64` | `1.0` | BGM (background music) volume multiplier. |
| `se_volume` | `f64` | `1.0` | SE (sound effect) volume multiplier. |
| `bgs_volume` | `f64` | `1.0` | BGS (background sound) volume multiplier. |
| `me_volume` | `f64` | `1.0` | ME (music effect) volume multiplier. |
| `midi_soundfont` | `Option<String>` | `None` | Path to a SoundFont file for MIDI playback. If empty, MIDI plays silently (no crash). |
| `midi_chorus` | `bool` | `false` | Enable the MIDI chorus effect (rustysynth). |
| `midi_reverb` | `bool` | `false` | Enable the MIDI reverb effect (rustysynth). |
| `se_source_count` | `u32` | `6` | Maximum number of simultaneous SE sources. Capped at 64. |
| `bgm_track_count` | `u32` | `1` | Maximum number of simultaneous BGM tracks. Capped at 16. |

### debug

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mode` | `bool` | `false` | Enable debug logging, which prints engine internals to standard output. |
| `console` | `bool` | `false` | Launch a standalone console window for log output (Windows only). |
| `show_fps` | FpsDisplay | `"none"` | Where to show the frame rate: `"none"`, `"titlebar"`, `"console"`, or `"both"`. |
| `log_level` | LogLevel | `None` | Override the log verbosity level: `"error"`, `"warn"`, `"info"`, `"debug"`, or `"trace"`. When set, takes precedence over `mode` for log output. `None` means use `mode` as a shortcut (`false` = info, `true` = debug). |

**Log level precedence.** The `log_level` field provides fine-grained control over log verbosity independent of the binary `mode` flag. When `log_level` is set (e.g. `"trace"`), it always wins. When `log_level` is `None`, the `mode` flag acts as a shortcut: `false` → `info`, `true` → `debug`. This design preserves backward compatibility while allowing CI / development workflows to dial log output up or down without toggling the entire debug mode.

---



## Game.ini

`Game.ini` is an INI file automatically generated by RPG Maker. mkxp-z reads two fields from it (source: `config.cpp:408-409`), and mkxp-rs does the same.

The `Title` field provides the game name, which is used as the window title when `window.title` is empty. The `Scripts` field specifies the path to the Ruby script archive.

mkxp-z does not use the `Library` field for RGSS version detection. Instead, it inspects the `Scripts` file extension: `.rxdata` indicates RGSS1, `.rvdata` indicates RGSS2, and `.rvdata2` indicates RGSS3. RTP paths are loaded from `mkxp.json`, not from `Game.ini`. Both `Library` and `RTP` are reserved fields for the RPG Maker editor and are ignored at runtime.

Example:
```ini
[Game]
Title=My Game
Scripts=Data\Scripts.rvdata
Library=RGSS300.dll
RTP=Standard
```

---

## Environment Variables

Environment variables use the `MKXP_` prefix with `__` (double underscore) as the hierarchy separator. For example, `MKXP_WINDOW__TITLE` maps to `window.title`. They have the highest priority among all configuration sources. Boolean values use `"1"` for true and `"0"` for false.

mkxp-z defines 3 environment variables: `MKXPZ_WINDOWS_CONSOLE` (enable a console window), `MKXPZ_MACOS_METAL` (force the Metal renderer), and `MKXPZ_FOLDER_SELECT` (show a folder picker on macOS). All three are platform-specific features. mkxp-rs defines its own set covering common launch parameters.

| Variable | Overrides |
|----------|-----------|
| `MKXP_RUBY__RGSS_VERSION` | `ruby.rgss_version` |
| `MKXP_RUBY__CUSTOM_SCRIPT` | `ruby.custom_script` |
| `MKXP_WINDOW__TITLE` | `window.title` |
| `MKXP_WINDOW__SIZE` | `window.size` (format `640x480`) |
| `MKXP_WINDOW__FULLSCREEN` | `window.fullscreen` |
| `MKXP_WINDOW__RESIZABLE` | `window.resizable` |
| `MKXP_GRAPHICS__SCALE_MODE` | `graphics.scale_mode` |
| `MKXP_GRAPHICS__VSYNC` | `graphics.vsync` |
| `MKXP_GRAPHICS__FRAME_RATE` | `graphics.frame_rate` |
| `MKXP_PATHS__GAME_FOLDER` | `paths.game_folder` |
| `MKXP_FONTS__SCALE` | `fonts.scale` |
| `MKXP_FONTS__HINTING` | `fonts.hinting` |
| `MKXP_FONTS__KERNING` | `fonts.kerning` |
| `MKXP_FONTS__OUTLINE_CROP` | `fonts.outline_crop` |
| `MKXP_AUDIO__MASTER_VOLUME` | `audio.master_volume` |
| `MKXP_AUDIO__BGM_VOLUME` | `audio.bgm_volume` |
| `MKXP_AUDIO__MIDI_SOUNDFONT` | `audio.midi_soundfont` |
| `MKXP_DEBUG__MODE` | `debug.mode` |
| `MKXP_DEBUG__CONSOLE` | `debug.console` |
| `MKXP_DEBUG__SHOW_FPS` | `debug.show_fps` |
| `MKXP_DEBUG__LOG_LEVEL` | `debug.log_level` |

---

## Command-Line Arguments

Command-line arguments use `--kebab-case` format and have the second highest priority after environment variables.

mkxp-z recognises only three arguments: `debug`, `test`, and `btest` (source: `config.cpp:225-235`). These are editor integration flags for RPG Maker XP. All other arguments are forwarded to Ruby `ARGV`. mkxp-rs defines its own set of arguments covering the same scope as the environment variables described above.

| Argument | Overrides |
|----------|-----------|
| `--rgss-version` | `ruby.rgss_version` |
| `--custom-script <path>` | `ruby.custom_script` |
| `--game-folder <path>` | `paths.game_folder` |
| `--window-title` | `window.title` |
| `--window-size <WxH>` | `window.size` |
| `--fullscreen` | `window.fullscreen` (flag, no value) |
| `--no-resizable` | `window.resizable` (flag) |
| `--scale-mode` | `graphics.scale_mode` |
| `--no-vsync` | `graphics.vsync` (flag) |
| `--frame-rate <n>` | `graphics.frame_rate` |
| `--font-scale <n>` | `fonts.scale` |
| `--font-hinting` | `fonts.hinting` |
| `--no-kerning` | `fonts.kerning` (flag) |
| `--no-outline-crop` | `fonts.outline_crop` (flag) |
| `--master-volume <n>` | `audio.master_volume` |
| `--bgm-volume <n>` | `audio.bgm_volume` |
| `--midi-soundfont <path>` | `audio.midi_soundfont` |
| `--debug` | `debug.mode` (flag) |
| `--console` | `debug.console` (flag) |
| `--show-fps` | `debug.show_fps` |
| `--log-level <level>` | `debug.log_level` |

Examples:

```bash
mkxp-rs --rgss-version 3 --fullscreen --show-fps titlebar
mkxp-rs --debug --frame-rate 30
mkxp-rs --custom-script benchmark.rb
```

---

## Differences from mkxp-z

**Format.** mkxp-z uses JSON5 (`mkxp.json`); mkxp-rs uses RON (`mkxp.ron`).

**Structure.** mkxp-z places all configuration keys at the JSON top level, resulting in roughly 60 keys in a flat namespace. mkxp-rs groups them into 8 sections: `ruby`, `window`, `graphics`, `paths`, `fonts`, `input`, `audio`, and `debug`.

**Scaling parameters.** mkxp-z uses 6 independent parameters to control screen and bitmap scaling (`smoothScaling`, `smoothScalingDown`, `bitmapSmoothScaling`, `bitmapSmoothScalingDown`, `smoothScalingMipmaps`, `bicubicSharpness`). mkxp-rs uses an override hierarchy: set `scale_mode` as the default, then override individual cases with `scale_up`, `scale_down`, `bitmap_scale_up`, and `bitmap_scale_down` when finer control is needed.

**Removed configuration options.** The following mkxp-z options are not present in mkxp-rs:

| Option | Reason |
|--------|--------|
| `JITEnable` `YJITEnable` `JITMaxCache` `JITMinCalls` `JITVerboseLevel` | Ruby JIT configuration is controlled through Ruby's own environment variables, not through the engine config file. |
| `preferMetalRenderer` | wgpu selects the rendering backend automatically. |
| `subImageFix` | A workaround targeting a specific older GPU model. |
| `anyAltToggleFS` | Allows either left or right Alt key with Enter to toggle fullscreen. Alt+Enter is a global shortcut regardless. |
| `execName` | mkxp-z needs this field to handle cases where the game executable has been renamed. mkxp-rs does not have this legacy. |
| `rubyLoadpath` | Replaced by static linking or `bundle install --standalone`. |
| `pathCache` `allowSymlinks` | These are filesystem implementation details belonging to `mkxp-filesystem`. |
| `dataPathOrg` `dataPathApp` | mkxp-z uses these to construct XDG data paths. mkxp-rs uses standard XDG conventions directly. |
| `editor` | RPG Maker XP editor integration flags, not needed for mkxp-rs. |
| `titleLanguage` | Controls the window title language. Rarely used. |
| `manualFolderSelect` | A macOS-specific folder picker triggered at startup. Rarely used. |
| `dumpAtlas` | A debugging tool that dumps the tile atlas, deferred to a later development stage. |

**Added configuration options.** The following options are present in mkxp-rs but have no equivalent in mkxp-z:

| Option | Description |
|--------|-------------|
| `graphics.pixel_snap` | Controls whether rendering is locked to integer pixel boundaries. When disabled, sprites have sub-pixel precision. |
| `audio.master_volume` and per-channel volumes | Independent volume control for each audio channel. mkxp-z relies on the system mixer for volume adjustment. |
