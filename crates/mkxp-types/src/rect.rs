// Rectangle types: Rect (i32 pixels) and FloatRect (f32 normalised).

use crate::Vec2i;

/// An axis-aligned integer rectangle.
///
/// Mirrors the RGSS `Rect` built-in class. Used for viewport clip regions,
/// bitmap blit source/destination rectangles, and sprite `src_rect`.
///
/// # Examples
///
/// ```
/// use mkxp_types::{Rect, Vec2i};
///
/// let r = Rect::new(10, 20, 100, 50);
/// assert!(r.contains_point(Vec2i::new(50, 40)));
/// assert!(!r.contains_point(Vec2i::new(0, 0)));
///
/// // Intersection
/// let a = Rect::new(0, 0, 100, 100);
/// let b = Rect::new(50, 50, 100, 100);
/// assert_eq!(a.intersection(b), Rect::new(50, 50, 50, 50));
///
/// // Move in place
/// let mut r = Rect::empty();
/// r.set(1, 2, 3, 4);
/// assert_eq!(r, Rect::new(1, 2, 3, 4));
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }

impl Rect {
    /// Creates a new `Rect`.
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self { Self { x, y, width, height } }

    /// An empty rectangle at the origin — `(0, 0, 0, 0)`.
    pub fn empty() -> Self { Self::new(0, 0, 0, 0) }

    /// Returns `true` when width or height is zero or negative.
    pub fn is_empty(self) -> bool { self.width <= 0 || self.height <= 0 }

    /// Returns `true` when `point` lies inside the rectangle.
    ///
    /// The left and top edges are inclusive; right and bottom are exclusive.
    pub fn contains_point(self, point: Vec2i) -> bool {
        point.x >= self.x && point.y >= self.y
            && point.x < self.x + self.width
            && point.y < self.y + self.height
    }

    /// Returns the intersection of two rectangles, or `Rect::empty()` if they
    /// do not overlap.
    pub fn intersection(self, other: Self) -> Self {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);
        if x2 <= x1 || y2 <= y1 { Rect::empty() }
        else { Rect::new(x1, y1, x2 - x1, y2 - y1) }
    }

    /// Translates by `(d.x, d.y)` without changing size.
    pub fn translate(self, d: Vec2i) -> Self {
        Self::new(self.x + d.x, self.y + d.y, self.width, self.height)
    }

    /// Sets all four fields at once — matches the RGSS `Rect#set` idiom.
    pub fn set(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.x = x; self.y = y; self.width = width; self.height = height;
    }
}

// -- FloatRect ----------------------------------------------------------

/// An axis-aligned floating-point rectangle.
///
/// Internal type used to map RGSS pixel coordinates into normalised texture
/// coordinates (`0.0..1.0`) for the GPU.
///
/// # Examples
///
/// ```
/// use mkxp_types::{Rect, FloatRect};
///
/// let r = Rect::new(10, 20, 640, 480);
/// let fr: FloatRect = r.into();
/// assert_eq!(fr, FloatRect::new(10.0, 20.0, 640.0, 480.0));
/// ```
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FloatRect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }

impl FloatRect {
    /// Creates a new `FloatRect`.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self { Self { x, y, width, height } }

    /// Returns `true` when width or height is zero or negative.
    pub fn is_empty(self) -> bool { self.width <= 0.0 || self.height <= 0.0 }
}

impl From<Rect> for FloatRect {
    fn from(r: Rect) -> Self { Self::new(r.x as f32, r.y as f32, r.width as f32, r.height as f32) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_point() {
        let r = Rect::new(10, 20, 100, 50);
        assert!(r.contains_point(Vec2i::new(10, 20)));
        assert!(r.contains_point(Vec2i::new(109, 69)));
        assert!(!r.contains_point(Vec2i::new(110, 20)));
        assert!(!r.contains_point(Vec2i::new(10, 70)));
    }

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(50, 50, 100, 100);
        assert_eq!(a.intersection(b), Rect::new(50, 50, 50, 50));
    }

    #[test]
    fn rect_intersection_no_overlap() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(100, 100, 10, 10);
        assert!(a.intersection(b).is_empty());
    }

    #[test]
    fn rect_set() {
        let mut r = Rect::empty();
        r.set(1, 2, 3, 4);
        assert_eq!(r, Rect::new(1, 2, 3, 4));
    }

    #[test]
    fn floatrect_from_rect() {
        let r = Rect::new(10, 20, 640, 480);
        let fr: FloatRect = r.into();
        assert_eq!(fr, FloatRect::new(10.0, 20.0, 640.0, 480.0));
    }
}
