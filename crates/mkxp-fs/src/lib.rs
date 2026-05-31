//! mkxp-fs — virtual file system for mkxp-rs.
//!
//! A PhysFS-inspired layered file system that mounts real directories and
//! RGSS encrypted archives into a unified virtual directory tree.
//! Supports case-insensitive path resolution via an optional path cache.

mod error;
mod filesystem;
mod vpath;
pub mod mountable;
pub mod path_cache;
pub mod rgss;

pub use error::FsError;
pub use filesystem::FileSystem;
pub use vpath::VPath;
