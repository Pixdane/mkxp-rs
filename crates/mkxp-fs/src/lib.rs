//! mkxp-fs — virtual file system for mkxp-rs.
//!
//! A PhysFS-inspired layered file system that mounts real directories and
//! RGSS encrypted archives into a unified virtual directory tree.
//! Supports case-insensitive path resolution via an optional path cache.
//!
//! # Logging
//!
//! When the `tracing` subscriber is active:
//!
//! | Event | Level | Content |
//! |-------|-------|---------|
//! | Source mounted | `info` | `mountpoint` of each mounted source |
//! | Path case mismatch | `warn` | `requested` → `actual` when the path cache resolves a different name |
//! | Path cache built | `info` | `entries` count after `build_path_cache()` |
//! | RGSS archive parsed | `info` | `files` count for each encrypted archive |
//!
//! All `?` propagation paths are silent — only degradation points (`warn`)
//! and lifecycle milestones (`info`) produce output.

mod error;
mod filesystem;
mod vpath;
pub mod mountable;
pub mod path_cache;
pub mod rgss;

pub use error::FsError;
pub use filesystem::FileSystem;
pub use vpath::VPath;
