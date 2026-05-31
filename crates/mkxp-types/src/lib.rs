//! # mkxp-types
//!
//! Foundational data types shared across the mkxp-rs workspace.
//! Zero dependencies. Pure value types for 2D math, colours, and error handling.

mod vec;
mod color;
mod rect;
mod blend_mode;
mod error;

pub use vec::{Vec2, Vec2i};
pub use color::{Color, Tone};
pub use rect::{Rect, FloatRect};
pub use blend_mode::BlendMode;
pub use error::MkxpError;
