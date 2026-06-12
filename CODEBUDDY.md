# CODEBUDDY.md — mkxp-rs project guide for agents

## What this project is

A pure-Rust reimplementation of [mkxp-z](https://github.com/mkxp-z/mkxp-z), a
cross-platform runtime for RPG Maker XP / VX / VX Ace games.  Licensed GPL-2.0.

## Current state

| Crate | File | Tests | Key facts |
|-------|------|-------|-----------|
| `mkxp-types` | `crates/mkxp-types/` | 15 unit + 8 doc | Vec2, Color, Rect, BlendMode, MkxpError. Zero non-optional deps. Serde behind feature flag. |
| `mkxp-config` | `crates/mkxp-config/` | 12 unit + 3 doc | 5-layer config loading (CLI > env > user > game RON > Game.ini). Uses `config` crate. |
| `mkxp-fs` | `crates/mkxp-fs/` | 85 unit + 9 doc | VPath, Mountable trait, FileSystem, RgssArchive, PathCache. Pure Rust, no C deps. |
| `mkxp-audio` | `crates/mkxp-audio/` | 40 unit + 12 doc | BGM/BGS/ME/SE + MIDI. kira (mixing) + rustysynth (SoundFont MIDI). Zero C deps. |
| `mkxp-log` | `crates/mkxp-log/` | 23 unit + 13 doc | tracing-based logger. `MkxpLayer` (ISO 8601 + span lifecycle). EnvFilter, Composite targets, From<&Config>. |
| `mkxp-graphics` | `crates/mkxp-graphics/` | 10 unit | wgpu renderer, fixed game coordinate system, viewport scale modes, temporary demo-state reset API. Does not depend on winit. |
| `mkxp-window` | `crates/mkxp-window/` | 48 unit | Binary crate. `main.rs` loads config/logging and runs `App::<DemoScriptEngine>`; `app.rs` owns winit/wgpu bootstrap, runtime config consumption, restart/shutdown, and thread lifecycle; `WindowController` owns winit window, muda menu, shortcuts, resize policy, and emits `WindowOutput`; `render_host.rs` consumes `RenderCommand` on a dedicated render thread and owns frame timing / `GraphicsState::update()`; `script_host.rs` provides the internal `ScriptEngine`/`ScriptContext` boundary and blocks at `FrameSync` without per-frame winit wakeups. |

Next major crate: `mkxp-binding` (magnus Ruby MRI).

## Audio crate — things to know before working on it

### Tech stack
```
kira 0.12.1   — mixing, sub-tracks, tween (fade), static sound playback
cpal 0.17.1  — raw audio output for real-time MIDI streaming
rustysynth 1.3.6 — SoundFont (.sf2) loading + MIDI file parsing + synthesis
ringbuf 0.5.0 — lock-free SPSC ring buffer (direct dep of mkxp-audio)
symphonia 0.5.5 — OGG/MP3/FLAC/WAV decoding (transitive dep of kira)
```

All deps are cached in `~/.cargo/registry`.

### kira 0.12 API gotchas

kira 0.12 uses track-based routing — sounds are played **directly on tracks**
rather than routed via `output_destination`.  Key facts:

- `TrackHandle::play(sound_data)` plays a sound on that track — replaces the
  old `manager.play() + StaticSoundSettings::output_destination(track_id)` pattern.
- `AudioManager::play(sound_data)` still exists; it delegates to
  `main_track().play()` and is used for SE sounds (auto-concurrent).
- `AudioManager` is now generic: `AudioManager<DefaultBackend>`.
- `Tween`, `PlaybackRate` are re-exported from `kira` root (the `manager`
  and `tween` modules are private).
- `TrackHandle::id()` no longer exists — use tracks directly.
- `set_volume()` takes `impl Into<Value<Decibels>>` — use `amplitude_to_db()`
  to convert linear amplitude (0.0–1.0) to dB.  See `manager.rs`.
- `PlaybackRate` is a tuple struct `PlaybackRate(f64)`, not an enum.
  Old `PlaybackRate::Factor(rate)` → just `PlaybackRate(rate)`.
- `set_volume()` and `set_playback_rate()` take a `Tween` parameter for
  smooth transitions.  Works the same as 0.9's `Tween::default()`.
- `StaticSoundData::from_cursor()` still exists and is the primary way to
  load audio from in-memory bytes.

Reference: `~/.cargo/registry/src/mirrors.ustc.edu.cn-*/kira-0.12.1/src/`

### rustysynth API gotchas

- `SoundFont::new(&mut reader)` — takes `&mut`, returns `Result`.
- `Synthesizer::new(&Arc<SoundFont>, &SynthesizerSettings)` — needs `Arc`, not `&mut`.
- `SynthesizerSettings` is `#[non_exhaustive]` — use `SynthesizerSettings::new(44100)`,
  not struct literal syntax.
- `MidiFileSequencer::new(synthesizer)` — takes **ownership** of Synthesizer.
- `seq.play(&Arc<MidiFile>, bool)` — MIDI file must be wrapped in `Arc`.
- `seq.render(&mut [f32], &mut [f32])` — **separate** left/right buffers, not interleaved.
- `seq.get_position()` returns `f64` seconds.  No `get_length()` on sequencer —
  use `midi_file.get_length()`.
- `seq.end_of_sequence()` — check for completion.  Always `false` when looping.
- `MidiFileLoopType::RpgMaker` — handles CC 111 loop markers.
- `MidiFile::new_with_loop_type(reader, loop_type)` for RPG Maker loop support.

### Pitch conversion — CRITICAL

**mkxp-z uses linear multiplier:** `pitch / 100.0` → `alSourcef(AL_PITCH)`.
We initially used semitones (`(pitch-100)/100*12`) which is **wrong** — 6% error
at extreme values.  The correct conversion:

```rust
// mkxp-z: clamp(pitch, 50, 150) / 100.0
// Our code: Pitch::new(value).as_multiplier() → PlaybackRate::Factor(rate)
// 100 → 1.0x, 150 → 1.5x, 50 → 0.5x
```

Do **not** use `PlaybackRate::Semitones`.  Use `PlaybackRate::Factor`.

### Volume model — three layers

mkxp-z has 5 volume layers (`Base * BaseRatio * FadeOut * FadeIn * External`).
We implement 3 (`base * ratio * external`), with FadeOut handled by kira tween.

- `bgm_base_volumes[i]` — set by `bgm_set_volume(v, track>=0)` (Base)
- `bgm_ratio` — set by `bgm_set_volume(v, track=-127)` (BaseRatio)
- `bgm_external` — set by ME/BGM interaction (External)
- Effective: `base * ratio * external`, applied via `apply_bgm_volumes()`.

### Embedded SoundFont fallback

`crates/mkxp-audio/src/default.sf2` — 556-byte minimal valid SoundFont 2.04
(1 preset "Piano", 48 samples of silence).  Uses `include_bytes!` in `midi.rs`.
Generated by Python script.  When `midi_soundfont` config is empty, this is used
instead, matching mkxp-z's "FluidSynth runs with no SF loaded" behavior.

The `.gitignore` has `*.sf2` to prevent large real soundfonts from being
committed.  The embedded `default.sf2` is force-added via `git add -f`.

### cpal Stream is !Send on macOS

`cpal::Stream` contains `!Send` types on macOS (CoreAudio `PropertyListener`).
Do not try to assert `Send` on types containing `cpal::Stream`.  The `MidiStream`
struct explicitly uses thread-safe components (ringbuf Producer) for the render
thread.

### Audio device not available in sandbox

Tests that create `AudioManager::new()` will fail in the sandbox with
`"A backend-specific error has occurred"`.  Mark these tests with
`#[ignore = "requires audio device"]`.  Doctests that call `AudioManager::new()`
should use ` ```no_run` or ` ```ignore`.

Unit tests that don't touch the audio device (volume math, ring buffer logic,
LRU cache, resolve_track, WAV encoding) run fine in the sandbox.

## Project conventions

### Error handling
Three-layer model: `MkxpError` (shared vocabulary) → crate enum with `#[from]`
→ `anyhow::Result` at binary layer.  **Read `docs/ERROR_HANDLING.md` before
adding any new error type.**  All error enums use `thiserror`.  `MkxpError::Io`
wraps `std::io::Error` (not String) — preserves `kind()` and source chain.

`AudioError` follows the same pattern: crate-specific variants (`FileNotFound`,
`UnsupportedFormat`, `Midi`, `SoundFont`, `Device`) + `Mkxp(#[from] MkxpError)`.

### Path types
All virtual path passing uses `VPath` (`crates/mkxp-fs/src/vpath.rs`), a
validated newtype over `String`.  Never use `std::path::Path` for virtual paths.

### Testing
- Write tests alongside code in `#[cfg(test)] mod tests`
- Doc-tests for all public items with `# Examples` sections
- For tests that require an audio device: `#[ignore = "requires audio device"]`
- For doctests that need an audio device: ` ```no_run`
- Temporary files go in `std::env::temp_dir()` — no `tempfile` crate dependency

### RGSS archives
- Three formats: RGSS1 (.rgssad) and RGSS2 (.rgss2a) are byte-identical
  (version 1, interleaved entries+data); RGSS3 (.rgss3a) uses version 3,
  separate index+data
- All three share base magic `RGSSAD\0`, differ only in version byte
- XOR uses LCG `next = prev * 7 + 3`, seeded from `0xDEAD_CAFE`
- Source reference: `/tmp/mkxp-z/src/crypto/rgssad.cpp`

### Git workflow
- Branch prefix: `codex/` or `feat/`
- Base branch: `master`
- All crates are workspace members in root `Cargo.toml` (resolver = "2")
- Edition 2024

### Edition 2024 pattern matching
- `if let Some(ref mut x) = &mut y` — the `ref mut` is **redundant** in edition 2024.
  Just use `if let Some(x) = y` when matching on `&mut Option<T>`.
  The compiler will suggest removing `ref mut`.

## Logging crate — things to know before working on it

### Architecture
```
tracing (facade macros)      ←  used by all product crates
    ↑
mkxp-log crate               ←  subscriber impl, only linked in binary
    ↑
tracing-subscriber           ←  Layer + EnvFilter + registry
```

Every product crate (`mkxp-config`, `mkxp-fs`, `mkxp-audio`) depends on
`tracing = "0.1"` and uses `info!()`, `warn!()`, `debug!()`, `trace!()`,
`error!()` macros directly — no `mkxp-log` dependency needed.  Only the
binary entry point links `mkxp-log` and calls `mkxp_log::init()` once.

### Logging conventions — FOLLOW THESE

These rules apply to every new line of logging you add:

1. **`?` never logs.**  Error propagation via `?` is silent.  The
   `thiserror` `Display` impl on the error value already carries the full
   diagnostic context.

2. **`warn!` only at degradation points.**  When the system catches an
   error and chooses a fallback (e.g. missing SoundFont → embedded
   default, case-mismatch path → corrected), log `warn!` because the
   error was swallowed and callers won't see it.

3. **`error!` only at unrecoverable points.**  Subsystem init failure,
   device loss, stream errors — things that mean the process is about to
   exit or a feature is permanently unavailable.

4. **`info!` for lifecycle milestones.**  Manager init, BGM play/stop,
   config source loaded, renderer ready, audio reset.  Default level
   (`info`) should produce no more than 1–2 lines per game frame.

5. **`debug!` for developer diagnostics.**  Fade parameters, volume
   changes, cache hits/misses, state-machine transitions.  Hidden in
   production, visible with `RUST_LOG=debug` or `debug.mode=true`.

6. **`trace!` for extreme detail.**  Per-track volume computation,
   per-tick MIDI rendering.  Only enabled when chasing a specific bug.
   Do NOT put `trace!` on hot paths that run hundreds of times per
   frame — even filtered-out macros have a small cost.

7. **`#[instrument]` on public API boundaries.**  Annotate functions
   that are called from Ruby bindings or cross-crate boundaries.  The
   attribute auto-creates a named span so all downstream logs appear
   nested.  Use `skip(self)` to avoid dumping large structs, and
   `fields(...)` for the 1–2 most diagnostic parameters.

8. **Never `println!` / `eprintln!`.**  These cannot be filtered,
   redirected, or silenced.  Use the tracing macros even in test
   binaries.

### Filter precedence
```
RUST_LOG env var  (highest)
    ↓
LogConfig::target_filters
    ↓
LogConfig::default_level
    ↓
debug.mode shortcut (false → Info, true → Debug)
    ↓
debug.log_level string (overrides everything above)
```

### Config mapping
```rust
// From Cargo.toml
mkxp-config = { path = "../mkxp-config" }

// In binary main():
let log_cfg = LogConfig::from(&config);
mkxp_log::init(log_cfg)?;
```

### Time crate
`time = "0.3"` with features `["formatting", "local-offset"]` is a
direct dependency of `mkxp-log`.  Timestamps use local timezone offset:
`[2026-06-02T22:48:26.584+08:00:00]`.

### Key files

| File | Why |
|------|-----|
| `crates/mkxp-log/src/lib.rs` | LogConfig, LogLevel, init(), From<&Config>, parse_log_level |
| `crates/mkxp-log/src/layer.rs` | MkxpLayer — on_event, on_new_span, on_close, Composite flattening |
| `crates/mkxp-log/src/error.rs` | LogError — AlreadySet, CreateDir, OpenFile, Mkxp |
| `docs/LOGGING.md` | Full design doc + Phase 1/2 records |

## Key files to read

| File | Why |
|------|-----|
| `docs/ERROR_HANDLING.md` | Error model — read before adding any new error type |
| `docs/FS_DESIGN.md` | File system design + mkxp-z comparison |
| `docs/AUDIO_DESIGN.md` | Audio system design, mkxp-z vs mkxp-rs audit, all 17 API checks |
| `docs/TYPES.md` | Foundation type reference |
| `docs/CONFIG.en.md` | Configuration reference — env var mapping uses `__` separator |
| `docs/WINDOW_CONTROLLER_DESIGN.md` | WindowController ownership/output boundary and current implementation status |
| `crates/mkxp-window/src/window_control.rs` | WindowController — owns window/menu/resize/fullscreen/shortcut state, no wgpu |
| `crates/mkxp-window/src/main.rs` | Binary entry — loads mkxp-config, initializes mkxp-log, creates winit event loop, selects `App::<DemoScriptEngine>` |
| `crates/mkxp-window/src/app.rs` | winit ApplicationHandler, wgpu bootstrap, RuntimeConfig consumption, WindowOutput → RenderCommand routing, restart/shutdown/thread joins |
| `crates/mkxp-window/src/render_host.rs` | Dedicated render thread, RenderCommand receiver, FPS gate, GraphicsState update/error propagation |
| `crates/mkxp-window/src/frame_sync.rs` | Script/render frame barrier; script blocks at Graphics.update, render thread waits on Condvar |
| `crates/mkxp-window/src/runtime.rs` | RuntimeConfig, SharedRuntime, RuntimeControl, script/render outcome slots, RuntimeEvent |
| `crates/mkxp-window/src/error.rs` | WindowError, ScriptError, ScriptExit, panic payload conversion |
| `crates/mkxp-audio/src/manager.rs` | AudioManager — BGM/BGS/ME/SE + volume layers + ME/BGM interaction |
| `crates/mkxp-audio/src/midi_stream.rs` | Real-time MIDI streaming via ringbuf + cpal |
| `crates/mkxp-audio/src/se_cache.rs` | 10MB LRU SE cache (matching mkxp-z SE_CACHE_MEM) |
| `crates/mkxp-audio/src/midi.rs` | MidiEngine — SoundFont loading + synthesizer factory |

## Network note

Cargo is configured to use USTC mirror.  If DNS fails, use `--offline` for builds
and tests.  `cargo search` and `curl` to external domains (crates.io, docs.rs,
github.com) all fail when DNS is down.  The in-app browser sometimes works for
docs.rs when curl fails — but the Node REPL `js` tool is unreliable for follow-up
calls (kernel resets lose `tab` binding, `js` tool sporadically becomes
unavailable).

GitHub token may expire — run `gh auth login` if push fails.

## mkxp-z source

A shallow clone is at `/tmp/mkxp-z/`.  Key files for audio:

| File | Purpose |
|------|---------|
| `src/audio/audio.h` | Audio class public API — 17 methods to verify against |
| `src/audio/audio.cpp` | BGM/BGS/ME/SE + meWatchFun state machine |
| `src/audio/audiostream.h` | VolumeType enum (Base/BaseRatio/FadeOut/FadeIn/External) |
| `src/audio/audiostream.cpp` | play/stop/fade/volume/pitch logic |
| `src/audio/soundemitter.cpp` | SE pool with 10MB LRU buffer cache |
| `src/audio/midisource.cpp` | SMF parser + FluidSynth synthesis |
| `src/audio/sharedmidistate.h` | FluidSynth lifecycle, dynamic library loading |
| `src/crypto/rgssad.cpp` | RGSS encryption |
| `src/config.cpp` | Config defaults (midiSoundFont, SESourceCount, BGMTrackCount) |

## Gotchas

### merge crate semantics
merge 0.2 removes blanket impls for standard library types.  Every `Option<T>`
field must use `#[merge(strategy = merge::option::overwrite_none)]`.
`a.merge(b)` keeps **a's** values and fills gaps from b.  Always merge from
highest-priority source down to lowest.

### config crate requires `#[serde(default)]`
When using the `config` crate, all missing top-level sections cause a "missing
field" error even if they're `Option<T>`.  Add `#[serde(default)]` to every
section field.

### Environment variable naming
The `config` crate uses `__` (double underscore) as the hierarchy separator.
`MKXP_WINDOW__TITLE` maps to `window.title`, **not** `MKXP_WINDOW_TITLE`.
The CONFIG docs (en/zh) document the full variable list with correct naming.

### Game.ini correction
mkxp-z only reads `Title` and `Scripts` from Game.ini.  It detects RGSS version
from the Scripts file extension (`.rxdata`→1, `.rvdata`→2, `.rvdata2`→3), not
from the `Library` DLL name.  The `RTP` field is never read.

### File editing
Use `apply_patch` for source, test, configuration, and documentation edits.
It is confirmed working in this workspace. Avoid shell redirection, heredocs,
`tee`, `sed -i`, or ad-hoc scripts for manual edits unless a large approved
mechanical transformation makes `apply_patch` impractical.

### Config changes from mkxp-z
- `midi_synth` config option was **removed** (rustysynth is the only MIDI backend).
- `soundfont` was **renamed** to `midi_soundfont` to match mkxp-z's `midiSoundFont`.
- The `Audio` config struct has `midi_soundfont`, `midi_chorus`, `midi_reverb`,
  `se_source_count`, `bgm_track_count`.

### WMA audio is NOT supported
Neither mkxp-z (SDL_sound) nor mkxp-rs (symphonia) supports WMA.  RPG Maker
games use OGG, MP3, WAV, or MIDI.  Do not add WMA support.
