use super::Point;
use std::fmt;

/// A rectangular area in pixel coordinates, defined by `min` (top-left) and
/// `max` (bottom-right) corners.
///
/// Constructed incrementally via [`Bounds::extend`], or directly from two
/// corner points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}

impl Bounds {
    /// Create bounds from two corner points.
    #[inline]
    pub fn new(a: Point, b: Point) -> Self {
        Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    /// Start building bounds from a single point.
    #[inline]
    pub fn from_point(p: Point) -> Self {
        Self { min: p, max: p }
    }

    /// Extend the bounds to include the given point.
    #[inline]
    pub fn extend_point(self, p: Point) -> Self {
        Self {
            min: Point::new(self.min.x.min(p.x), self.min.y.min(p.y)),
            max: Point::new(self.max.x.max(p.x), self.max.y.max(p.y)),
        }
    }

    /// Extend the bounds to include another bounds.
    #[inline]
    pub fn extend_bounds(self, other: Bounds) -> Self {
        self.extend_point(other.min).extend_point(other.max)
    }

    /// Center point of the bounds.
    #[inline]
    pub fn center(self) -> Point {
        (self.min + self.max) / 2.0
    }

    /// Size as a Point (width, height).
    #[inline]
    pub fn size(self) -> Point {
        self.max - self.min
    }

    /// Returns `true` if the point is inside the bounds (inclusive).
    #[inline]
    pub fn contains_point(self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    /// Returns `true` if this bounds fully contains `other`.
    #[inline]
    pub fn contains_bounds(self, other: Bounds) -> bool {
        self.contains_point(other.min) && self.contains_point(other.max)
    }

    /// Returns `true` if the two bounds share at least one point.
    #[inline]
    pub fn intersects(self, other: Bounds) -> bool {
        other.max.x >= self.min.x
            && other.min.x <= self.max.x
            && other.max.y >= self.min.y
            && other.min.y <= self.max.y
    }

    /// Returns `true` if the intersection is a positive-area rectangle.
    #[inline]
    pub fn overlaps(self, other: Bounds) -> bool {
        other.max.x > self.min.x
            && other.min.x < self.max.x
            && other.max.y > self.min.y
            && other.min.y < self.max.y
    }

    /// Expand or shrink the bounds by a ratio in each direction.
    /// A ratio of `0.5` extends by 50%; negative values shrink.
    pub fn pad(self, ratio: f64) -> Self {
        let s = self.size();
        let dx = s.x.abs() * ratio;
        let dy = s.y.abs() * ratio;
        Self {
            min: Point::new(self.min.x - dx, self.min.y - dy),
            max: Point::new(self.max.x + dx, self.max.y + dy),
        }
    }

    /// Bottom-left corner.
    #[inline]
    pub fn bottom_left(self) -> Point {
        Point::new(self.min.x, self.max.y)
    }

    /// Top-right corner.
    #[inline]
    pub fn top_right(self) -> Point {
        Point::new(self.max.x, self.min.y)
    }
}

impl fmt::Display for Bounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bounds({}, {})", self.min, self.max)
    }
}

impl From<(Point, Point)> for Bounds {
    fn from((a, b): (Point, Point)) -> Self {
        Self::new(a, b)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_normalises() {
        let b = Bounds::new(Point::new(40.0, 60.0), Point::new(10.0, 10.0));
        assert_eq!(b.min, Point::new(10.0, 10.0));
        assert_eq!(b.max, Point::new(40.0, 60.0));
    }

    #[test]
    fn test_extend() {
        let b = Bounds::from_point(Point::new(5.0, 5.0)).extend_point(Point::new(10.0, 20.0));
        assert_eq!(b.min, Point::new(5.0, 5.0));
        assert_eq!(b.max, Point::new(10.0, 20.0));
    }

    #[test]
    fn test_center_and_size() {
        let b = Bounds::new(Point::new(0.0, 0.0), Point::new(10.0, 20.0));
        assert_eq!(b.center(), Point::new(5.0, 10.0));
        assert_eq!(b.size(), Point::new(10.0, 20.0));
    }

    #[test]
    fn test_contains() {
        let b = Bounds::new(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        assert!(b.contains_point(Point::new(5.0, 5.0)));
        assert!(!b.contains_point(Point::new(11.0, 5.0)));

        let inner = Bounds::new(Point::new(2.0, 2.0), Point::new(8.0, 8.0));
        assert!(b.contains_bounds(inner));
    }

    #[test]
    fn test_intersects_overlaps() {
        let a = Bounds::new(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        let b = Bounds::new(Point::new(10.0, 10.0), Point::new(20.0, 20.0));
        assert!(a.intersects(b)); // share corner
        assert!(!a.overlaps(b)); // no area overlap

        let c = Bounds::new(Point::new(5.0, 5.0), Point::new(15.0, 15.0));
        assert!(a.intersects(c));
        assert!(a.overlaps(c));
    }

    #[test]
    fn test_pad() {
        let b = Bounds::new(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        let p = b.pad(0.5);
        assert_eq!(p.min, Point::new(-5.0, -5.0));
        assert_eq!(p.max, Point::new(15.0, 15.0));
    }
}
