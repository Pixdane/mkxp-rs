//! mkxp-fs — virtual file system for mkxp-rs.
//!
//! Provides a PhysFS-inspired layered file system that mounts real
//! directories and RGSS encrypted archives into a unified virtual
//! directory tree.  Supports case-insensitive path resolution via
//! an optional path cache.

mod error;
pub mod mountable;
pub mod rgss;
mod path_cache;

pub use error::FsError;
