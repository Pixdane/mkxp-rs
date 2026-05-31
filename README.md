# mkxp-rs

A Rust reimplementation of [mkxp-z](https://github.com/mkxp-z/mkxp-z), a
cross-platform runtime for RPG Maker XP / VX / VX Ace games.  Licensed under
GPL-2.0.

## What it does

mkxp-rs lets you run RPG Maker games on modern systems without the original
RPG Maker editor.  It embeds Ruby MRI to execute the game's RGSS scripts,
renders graphics through wgpu, and plays audio through rodio.

## Status

Early development.  The foundation crates are being built module by module.

| Crate | Description | Status |
|-------|-------------|--------|
| `mkxp-types` | Shared 2D math types (Vec2, Rect, Color, …) | Done |
| `mkxp-config` | Layered configuration (RON, Game.ini, env, CLI) | Done |
| `mkxp-fs` | Virtual filesystem (directories, RGSS archives, ZIP) | In design |
| `mkxp-graphics` | wgpu-based renderer (Bitmap, Sprite, Viewport, …) | Planned |
| `mkxp-audio` | Audio playback (BGM, SE, MIDI) | Planned |
| `mkxp-binding` | Ruby MRI integration via magnus | Planned |

## Architecture

```
mkxp-rs/
├── docs/                  # User guides and design documents
│   ├── CONFIG.en.md       #   Configuration reference (English)
│   ├── CONFIG.zh.md       #   Configuration reference (Chinese)
│   └── FS_DESIGN.md       #   Filesystem design analysis
├── crates/
│   ├── mkxp-types/        #   Vec2, Rect, Color, BlendMode, MkxpError
│   ├── mkxp-config/       #   Config loading (RON + INI + env + CLI)
│   └── mkxp-fs/           #   Virtual filesystem (planned)
├── Cargo.toml             # Workspace root
├── README.md
└── LICENSE
```

## Documentation

- [Configuration reference (English)](docs/CONFIG.en.md)
- [Configuration reference (Chinese)](docs/CONFIG.zh.md)
- [Filesystem design notes](docs/FS_DESIGN.md)

## Building

```bash
# Build all crates
cargo build

# Run all tests
cargo test

# Build documentation
cargo doc --no-deps --open
```

## License

GNU General Public License v2.0. See [LICENSE](LICENSE).
