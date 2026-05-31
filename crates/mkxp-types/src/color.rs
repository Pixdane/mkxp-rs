// Colour types: Color (RGBA) and Tone (colour adjustment).

/// An RGBA colour with `f64` components in the range `0.0..=255.0`.
///
/// Mirrors the RGSS `Color` built-in class. Components use `f64` (not `u8`)
/// because RGSS allows values outside `0..255`; shaders clamp as needed.
///
/// # Examples
///
/// ```
/// use mkxp_types::Color;
///
/// let red = Color::new(255.0, 0.0, 0.0, 255.0);
/// let half_red = Color::new(255.0, 0.0, 0.0, 128.0);
///
/// // Linear interpolation
/// let black = Color::black();
/// let white = Color::white();
/// assert_eq!(black.lerp(white, 0.5), Color::new(127.5, 127.5, 127.5, 255.0));
/// ```
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Color { pub r: f64, pub g: f64, pub b: f64, pub a: f64 }

impl Color {
    /// Creates a new `Color`.
    pub fn new(r: f64, g: f64, b: f64, a: f64) -> Self { Self { r, g, b, a } }

    /// Opaque black — the RGSS default colour.
    pub fn black() -> Self { Self::new(0.0, 0.0, 0.0, 255.0) }

    /// Opaque white.
    pub fn white() -> Self { Self::new(255.0, 255.0, 255.0, 255.0) }

    /// Fully transparent.
    pub fn transparent() -> Self { Self::new(0.0, 0.0, 0.0, 0.0) }

    /// Linear interpolation towards `other` by `t` where `0.0 <= t <= 1.0`.
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
            self.a + (other.a - self.a) * t,
        )
    }
}

impl Default for Color { fn default() -> Self { Self::black() } }

// -- Tone ---------------------------------------------------------------

/// A colour adjustment applied per-sprite or per-viewport.
///
/// Mirrors the RGSS `Tone` built-in class. Each channel ranges from
/// `-255.0` to `255.0`. `gray` shifts brightness; `r`/`g`/`b` shift
/// individual colour channels.
///
/// # Examples
///
/// ```
/// use mkxp_types::Tone;
///
/// let neutral = Tone::neutral();
/// let sepia = Tone::new(50.0, -30.0, -80.0, 10.0);
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Tone { pub r: f64, pub g: f64, pub b: f64, pub gray: f64 }

impl Tone {
    /// Creates a new `Tone`.
    pub fn new(r: f64, g: f64, b: f64, gray: f64) -> Self { Self { r, g, b, gray } }

    /// Neutral tone — no adjustment applied.
    pub fn neutral() -> Self { Self::new(0.0, 0.0, 0.0, 0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_halfway() {
        let black = Color::black();
        let white = Color::white();
        let grey = black.lerp(white, 0.5);
        assert_eq!(grey, Color::new(127.5, 127.5, 127.5, 255.0));
    }
}
