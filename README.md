# mkxp-rs

A Rust reimplementation of [mkxp-z](https://github.com/mkxp-z/mkxp-z), a
cross-platform runtime for RPG Maker XP / VX / VX Ace games.  Licensed under
GPL-2.0.

## What it does

mkxp-rs lets you run RPG Maker games on modern systems without the original
RPG Maker editor.  It embeds Ruby MRI to execute the game's RGSS scripts,
renders graphics through wgpu, and plays audio through kira + rustysynth.

## Status

Early development.  The foundation crates are being built module by module.

| Crate | Description | Status |
|-------|-------------|--------|
| `mkxp-types` | Shared 2D math types (Vec2, Rect, Color, …) | Done |
| `mkxp-config` | Layered configuration (RON, Game.ini, env, CLI) | Done |
| `mkxp-fs` | Virtual filesystem (directories, RGSS archives) | Done |
| `mkxp-audio` | Audio playback (BGM/bgs/me/se, MIDI via rustysynth) | Design |
| `mkxp-graphics` | wgpu-based renderer (Bitmap, Sprite, Viewport, …) | Planned |
| `mkxp-binding` | Ruby MRI integration via magnus | Planned |

## Architecture

```
mkxp-rs/
├── docs/                  # User guides and design documents
│   ├── CONFIG.en.md       #   Configuration reference (English)
│   ├── CONFIG.zh.md       #   Configuration reference (Chinese)
│   ├── TYPES.md           #   Foundation type reference
│   ├── ERROR_HANDLING.md  #   Error handling strategy
│   ├── FS_DESIGN.md       #   Filesystem design notes
│   └── AUDIO_DESIGN.md    #   Audio system design (kira + rustysynth)
├── crates/
│   ├── mkxp-types/        #   Vec2, Rect, Color, BlendMode, MkxpError
│   ├── mkxp-config/       #   Config loading (RON + INI + env + CLI)
│   ├── mkxp-fs/           #   Virtual filesystem (mount, path cache, RGSS)
│   └── mkxp-audio-test/   #   Audio proof-of-concept (MIDI + kira + cpal)
├── Cargo.toml             # Workspace root
├── README.md
└── LICENSE
```

## Documentation

- [Foundation type reference](docs/TYPES.md)
- [Configuration reference (English)](docs/CONFIG.en.md)
- [Configuration reference (Chinese)](docs/CONFIG.zh.md)
- [Error handling strategy](docs/ERROR_HANDLING.md)
- [Filesystem design notes](docs/FS_DESIGN.md)
- [Audio system design](docs/AUDIO_DESIGN.md)
- [mkxp-z deepwiki](https://deepwiki.com/mkxp-z/mkxp-z) — upstream reference

## Tech stack

| Subsystem | mkxp-z (C++) | mkxp-rs (Rust) |
|-----------|-------------|----------------|
| Graphics | OpenGL 2.1 / ANGLE | wgpu (Vulkan/Metal/DX12/GL) |
| Audio back-end | OpenAL | kira (cpal/CoreAudio/ALSA/WASAPI) |
| Audio decoding | libvorbis, SDL_sound | symphonia |
| MIDI synthesis | FluidSynth (C .so/.dylib) | rustysynth (pure Rust) |
| Filesystem | PhysFS | Pure Rust (mkxp-fs) |
| Ruby | MRI | magnus |
| Config | json5pp + INI | config + RON + INI |

## Building

```bash
# Build all crates
cargo build

# Run all tests
cargo test

# Run audio proof-of-concept
cargo run -p mkxp-audio-test

# Build documentation
cargo doc --no-deps --open
```

## License

GNU General Public License v2.0.  See [LICENSE](LICENSE).
