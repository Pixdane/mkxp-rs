# CODEBUDDY.md — mkxp-rs project guide for agents

## What this project is

A pure-Rust reimplementation of [mkxp-z](https://github.com/mkxp-z/mkxp-z), a
cross-platform runtime for RPG Maker XP / VX / VX Ace games.  Licensed GPL-2.0.

## Current state

Three foundation crates are **done**:

| Crate | File | Tests | Key facts |
|-------|------|-------|-----------|
| `mkxp-types` | `crates/mkxp-types/` | 15 unit + 8 doc | Vec2, Color, Rect, BlendMode, MkxpError. Zero non-optional deps. Serde behind feature flag. |
| `mkxp-config` | `crates/mkxp-config/` | 12 unit + 3 doc | 5-layer config loading (CLI > env > user > game RON > Game.ini). Uses `config` crate. |
| `mkxp-fs` | `crates/mkxp-fs/` | 85 unit + 9 doc | VPath, Mountable trait, FileSystem, RgssArchive, PathCache. Pure Rust, no C deps. |

Next crates to build (in priority order): `mkxp-graphics` (wgpu renderer), `mkxp-audio` (rodio), `mkxp-binding` (magnus Ruby MRI integration).

## Project conventions

### Error handling
Three-layer model: `MkxpError` (shared vocabulary) → crate enum with `#[from]` → `anyhow::Result` at binary layer.  **Read `docs/ERROR_HANDLING.md` before adding any new error type.**  All error enums use `thiserror`.  `MkxpError::Io` wraps `std::io::Error` (not String) — preserves `kind()` and source chain.

### Path types
All virtual path passing uses `VPath` (`crates/mkxp-fs/src/vpath.rs`), a validated newtype over `String`.  Public APIs that receive raw user input take `&str` and convert internally.  Never use `std::path::Path` for virtual paths — they are always forward-slash, relative, no `..`.

### Testing
- Write tests alongside code in `#[cfg(test)] mod tests`
- Doc-tests for all public items with `# Examples` sections
- For real-file integration tests: use `#[ignore]` with a descriptive reason string
- Temporary files go in `std::env::temp_dir()` — no `tempfile` crate dependency

### RGSS archives
- Three formats: RGSS1 (.rgssad) and RGSS2 (.rgss2a) are byte-identical (version 1, interleaved entries+data); RGSS3 (.rgss3a) uses version 3, separate index+data
- All three share base magic `RGSSAD\0`, differ only in version byte
- XOR uses LCG `next = prev * 7 + 3`, seeded from `0xDEAD_CAFE`
- Zip support is NOT needed (mkxp-z doesn't use it)
- Source reference: `/tmp/mkxp-z/src/crypto/rgssad.cpp`

### Git workflow
- Branch prefix: `codex/` or `feat/`
- Base branch: `master`
- All crates are workspace members in root `Cargo.toml` (resolver = "2")
- Edition 2024

## Key files to read

| File | Why |
|------|-----|
| `docs/ERROR_HANDLING.md` | Error model — read before adding any new error type |
| `docs/FS_DESIGN.md` | File system design + mkxp-z comparison |
| `docs/TYPES.md` | Foundation type reference |
| `crates/mkxp-fs/src/mountable.rs` | Trait you'll implement for new data sources |
| `crates/mkxp-fs/src/filesystem.rs` | FileSystem API — how mount/read/exists work |
| `crates/mkxp-fs/src/vpath.rs` | VPath — always use this, never raw &str for paths |

## Network note

Cargo is configured to use USTC mirror.  If DNS fails, use `--offline` for builds
and tests.  GitHub token may expire — run `gh auth login` if push fails.

## mkxp-z source

A shallow clone is at `/tmp/mkxp-z/`.  Key files:
- `src/crypto/rgssad.cpp` — RGSS encryption (reference for mkxp-fs)
- `src/filesystem/filesystem.cpp` — PhysFS-based file system (reference for mkxp-fs)
- `src/config.cpp` — config loading (reference for mkxp-config)
