//! # mkxp-types
//!
//! Foundational data types shared across the mkxp-rs workspace.
//! Zero dependencies. Pure value types for 2D math, colours, and error handling.

mod blend_mode;
mod color;
mod error;
mod rect;
mod vec;

pub use blend_mode::BlendMode;
pub use color::{Color, Tone};
pub use error::MkxpError;
pub use rect::{FloatRect, Rect};
pub use vec::{Vec2, Vec2i};
