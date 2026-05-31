/// Result type for audio operations.
pub type AudioResult<T> = Result<T, crate::AudioError>;

/// Volume level in mkxp-z convention: 0–100, maps to 0.0–1.0 internally.
///
/// Automatically clamped to `[0, 100]` on construction.
///
/// # Examples
///
/// ```
/// use mkxp_audio::Volume;
///
/// let v = Volume::new(50);
/// assert_eq!(v.as_i32(), 50);
/// assert!((v.as_f64() - 0.5).abs() < f64::EPSILON);
///
/// // Clamping
/// assert_eq!(Volume::new(150).as_i32(), 100);
/// assert_eq!(Volume::new(-10).as_i32(), 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume(i32);

impl Volume {
    /// The maximum volume value.
    pub const MAX: i32 = 100;

    /// The minimum volume value.
    pub const MIN: i32 = 0;

    /// Create a new volume, clamped to `[MIN, MAX]`.
    pub fn new(value: i32) -> Self {
        Self(value.clamp(Self::MIN, Self::MAX))
    }

    /// Return the volume as a linear multiplier (`0.0` – `1.0`).
    pub fn as_f64(&self) -> f64 {
        self.0 as f64 / Self::MAX as f64
    }

    /// Return the raw integer value (0–100).
    pub fn as_i32(&self) -> i32 {
        self.0
    }
}

impl Default for Volume {
    fn default() -> Self {
        Self(100)
    }
}

/// Pitch control in mkxp-z convention: 50–150, where 100 is normal pitch.
///
/// Maps to a **linear playback rate** (`pitch / 100`), matching mkxp-z's
/// `alSourcef(src, AL_PITCH, value)`.  150 = 1.5× speed, 50 = 0.5× speed.
///
/// Automatically clamped to `[50, 150]` on construction.
///
/// # Examples
///
/// ```
/// use mkxp_audio::Pitch;
///
/// let p = Pitch::new(100);
/// assert!((p.as_multiplier() - 1.0).abs() < f64::EPSILON);
///
/// // 150 → 1.5× playback rate (one octave + a fifth up)
/// assert!((Pitch::new(150).as_multiplier() - 1.5).abs() < f64::EPSILON);
///
/// // 50 → 0.5× playback rate (one octave down)
/// assert!((Pitch::new(50).as_multiplier() - 0.5).abs() < f64::EPSILON);
///
/// // Clamping
/// assert_eq!(Pitch::new(500).as_multiplier(), Pitch::new(150).as_multiplier());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pitch(i32);

impl Pitch {
    /// Normal pitch (no shift).
    pub const CENTER: i32 = 100;

    /// Minimum pitch (0.5× speed).
    pub const MIN: i32 = 50;

    /// Maximum pitch (1.5× speed).
    pub const MAX: i32 = 150;

    /// Create a new pitch, clamped to `[MIN, MAX]`.
    pub fn new(value: i32) -> Self {
        Self(value.clamp(Self::MIN, Self::MAX))
    }

    /// Return the linear playback rate multiplier.
    ///
    /// `100` → `1.0`, `150` → `1.5`, `50` → `0.5`.
    /// Equivalent to mkxp-z's `alSourcef(src, AL_PITCH, pitch / 100.0)`.
    pub fn as_multiplier(&self) -> f64 {
        self.0 as f64 / Self::CENTER as f64
    }

    /// Return the raw integer value (50–150).
    pub fn as_i32(&self) -> i32 {
        self.0
    }
}

impl Default for Pitch {
    fn default() -> Self {
        Self(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_zero_is_silent() {
        assert!((Volume::new(0).as_f64() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn volume_hundred_is_full() {
        assert!((Volume::new(100).as_f64() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn pitch_center_is_unity() {
        assert!((Pitch::new(100).as_multiplier() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn pitch_150_is_one_and_half() {
        assert!((Pitch::new(150).as_multiplier() - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn pitch_50_is_half() {
        assert!((Pitch::new(50).as_multiplier() - 0.5).abs() < f64::EPSILON);
    }
}
