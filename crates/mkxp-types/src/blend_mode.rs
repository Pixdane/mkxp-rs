// Blend mode for combining pixel colours during rendering.

use crate::MkxpError;

/// How source and destination pixels are combined.
///
/// Mirrors the RGSS `BlendType` used by `Sprite#blend_type` and `Bitmap#blt`.
///
/// # Examples
///
/// ```
/// use mkxp_types::BlendMode;
///
/// assert_eq!(BlendMode::Normal as u8, 0);
/// assert_eq!(BlendMode::try_from(1u8), Ok(BlendMode::Addition));
/// assert!(BlendMode::try_from(99u8).is_err());
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BlendMode {
    /// Source overwrites destination (default).
    #[default]
    Normal = 0,
    /// Source and destination are added together.
    Addition = 1,
    /// Destination is subtracted from source.
    Subtraction = 2,
    /// Source and destination are multiplied.
    Multiply = 3,
}

impl From<BlendMode> for u8 {
    fn from(m: BlendMode) -> Self { m as u8 }
}

impl TryFrom<u8> for BlendMode {
    type Error = MkxpError;

    /// Converts a `u8` to a `BlendMode`. Returns an error for unknown values.
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Normal), 1 => Ok(Self::Addition),
            2 => Ok(Self::Subtraction), 3 => Ok(Self::Multiply),
            _ => Err(MkxpError::Unsupported(format!("unknown blend mode: {v}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(BlendMode::try_from(BlendMode::Addition as u8), Ok(BlendMode::Addition));
        assert_eq!(BlendMode::try_from(BlendMode::Multiply as u8), Ok(BlendMode::Multiply));
    }

    #[test]
    fn invalid() {
        assert!(BlendMode::try_from(99).is_err());
    }
}
