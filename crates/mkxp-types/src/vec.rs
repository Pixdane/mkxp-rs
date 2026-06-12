// Two-dimensional vector types.

use std::ops::{Add, Div, Mul, Sub};

/// A 2D vector with `f32` components.
///
/// Used for texture coordinates, sub-pixel sprite positions, shader uniforms,
/// and scaling factors.
///
/// # Examples
///
/// ```
/// use mkxp_types::Vec2;
///
/// let a = Vec2::new(1.0, 2.0);
/// let b = Vec2::new(3.0, 4.0);
/// assert_eq!(a + b, Vec2::new(4.0, 6.0));
/// assert_eq!(a * 2.0, Vec2::new(2.0, 4.0));
/// assert_eq!(Vec2::new(3.0, 4.0).length(), 5.0);
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    /// Creates a new `Vec2`.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Returns the zero vector.
    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    /// Returns the unit vector `(1, 1)`.
    pub const fn one() -> Self {
        Self { x: 1.0, y: 1.0 }
    }

    /// Dot product.
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    /// Euclidean length. For squared length (cheaper), use `dot(self, self)`.
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl Div<f32> for Vec2 {
    type Output = Self;
    fn div(self, rhs: f32) -> Self {
        Self::new(self.x / rhs, self.y / rhs)
    }
}

impl From<Vec2i> for Vec2 {
    fn from(v: Vec2i) -> Self {
        Self::new(v.x as f32, v.y as f32)
    }
}

// -- Vec2i --------------------------------------------------------------

/// A 2D vector with `i32` components.
///
/// Used for pixel coordinates, window dimensions, mouse position, and
/// drawable sizes.
///
/// # Examples
///
/// ```
/// use mkxp_types::{Vec2, Vec2i};
///
/// let p = Vec2i::new(10, 20);
/// let f: Vec2 = p.into();
/// assert_eq!(f, Vec2::new(10.0, 20.0));
///
/// // Conversion from Vec2 truncates towards zero
/// let v: Vec2i = Vec2::new(3.7, -1.2).into();
/// assert_eq!(v, Vec2i::new(3, -1));
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Vec2i {
    pub x: i32,
    pub y: i32,
}

impl Vec2i {
    /// Creates a new `Vec2i`.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Returns the zero vector.
    pub const fn zero() -> Self {
        Self { x: 0, y: 0 }
    }
}

impl From<Vec2> for Vec2i {
    /// Truncates fractional parts towards zero.
    fn from(v: Vec2) -> Self {
        Self::new(v.x as i32, v.y as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = Vec2::new(1.0, 2.0);
        let b = Vec2::new(3.0, 4.0);
        assert_eq!(a + b, Vec2::new(4.0, 6.0));
        assert_eq!(a - b, Vec2::new(-2.0, -2.0));
        assert_eq!(a * 2.0, Vec2::new(2.0, 4.0));
        assert_eq!(b / 2.0, Vec2::new(1.5, 2.0));
    }

    #[test]
    fn dot_and_length() {
        assert_eq!(Vec2::new(3.0, 4.0).length(), 5.0);
        assert_eq!(Vec2::new(1.0, 2.0).dot(Vec2::new(3.0, 4.0)), 11.0);
    }

    #[test]
    fn from_vec2i_truncates() {
        let v: Vec2i = Vec2::new(3.7, -1.2).into();
        assert_eq!(v, Vec2i::new(3, -1));
    }
}
