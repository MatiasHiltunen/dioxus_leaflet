use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// A 2D point in pixel coordinates.
///
/// Used for all pixel-space calculations: tile positions, container offsets,
/// projected map coordinates, etc.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn round(self) -> Self {
        Self {
            x: self.x.round(),
            y: self.y.round(),
        }
    }

    #[inline]
    pub fn floor(self) -> Self {
        Self {
            x: self.x.floor(),
            y: self.y.floor(),
        }
    }

    #[inline]
    pub fn ceil(self) -> Self {
        Self {
            x: self.x.ceil(),
            y: self.y.ceil(),
        }
    }

    #[inline]
    pub fn trunc(self) -> Self {
        Self {
            x: self.x.trunc(),
            y: self.y.trunc(),
        }
    }

    /// Component-wise multiplication: `(self.x * other.x, self.y * other.y)`.
    #[inline]
    pub fn scale_by(self, other: Point) -> Self {
        Self {
            x: self.x * other.x,
            y: self.y * other.y,
        }
    }

    /// Component-wise division: `(self.x / other.x, self.y / other.y)`.
    #[inline]
    pub fn unscale_by(self, other: Point) -> Self {
        Self {
            x: self.x / other.x,
            y: self.y / other.y,
        }
    }

    /// Euclidean distance to another point.
    #[inline]
    pub fn distance_to(self, other: Point) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Returns `true` if both coordinates of `other` are ≤ the absolute values
    /// of this point's coordinates.
    #[inline]
    pub fn contains(self, other: Point) -> bool {
        other.x.abs() <= self.x.abs() && other.y.abs() <= self.y.abs()
    }

    /// Check if both components are exactly zero.
    #[inline]
    pub fn is_zero(self) -> bool {
        self.x == 0.0 && self.y == 0.0
    }

    /// Squared distance (avoids sqrt for comparisons).
    #[inline]
    pub fn distance_sq(self, other: Point) -> f64 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        dx * dx + dy * dy
    }

    /// Vector magnitude (distance from origin).
    #[inline]
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

// ─── Operator Impls ──────────────────────────────────────────────────────────

impl Add for Point {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Sub for Point {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl Mul<f64> for Point {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f64) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl Div<f64> for Point {
    type Output = Self;
    #[inline]
    fn div(self, rhs: f64) -> Self {
        Self {
            x: self.x / rhs,
            y: self.y / rhs,
        }
    }
}

impl Neg for Point {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
        }
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Point({:.6}, {:.6})", self.x, self.y)
    }
}

impl From<(f64, f64)> for Point {
    #[inline]
    fn from((x, y): (f64, f64)) -> Self {
        Self { x, y }
    }
}

impl From<(i32, i32)> for Point {
    #[inline]
    fn from((x, y): (i32, i32)) -> Self {
        Self {
            x: x as f64,
            y: y as f64,
        }
    }
}

impl From<[f64; 2]> for Point {
    #[inline]
    fn from(arr: [f64; 2]) -> Self {
        Self {
            x: arr[0],
            y: arr[1],
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arithmetic() {
        let a = Point::new(10.0, 20.0);
        let b = Point::new(3.0, 5.0);

        assert_eq!(a + b, Point::new(13.0, 25.0));
        assert_eq!(a - b, Point::new(7.0, 15.0));
        assert_eq!(a * 2.0, Point::new(20.0, 40.0));
        assert_eq!(a / 2.0, Point::new(5.0, 10.0));
        assert_eq!(-a, Point::new(-10.0, -20.0));
    }

    #[test]
    fn test_rounding() {
        let p = Point::new(1.3, 2.7);
        assert_eq!(p.round(), Point::new(1.0, 3.0));
        assert_eq!(p.floor(), Point::new(1.0, 2.0));
        assert_eq!(p.ceil(), Point::new(2.0, 3.0));
        assert_eq!(p.trunc(), Point::new(1.0, 2.0));
    }

    #[test]
    fn test_distance() {
        let a = Point::new(0.0, 0.0);
        let b = Point::new(3.0, 4.0);
        assert!((a.distance_to(b) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_scale() {
        let p = Point::new(3.0, 4.0);
        let s = Point::new(2.0, 3.0);
        assert_eq!(p.scale_by(s), Point::new(6.0, 12.0));
        assert_eq!(p.unscale_by(s), Point::new(1.5, 4.0 / 3.0));
    }

    #[test]
    fn test_contains() {
        let p = Point::new(5.0, 5.0);
        assert!(p.contains(Point::new(3.0, 3.0)));
        assert!(!p.contains(Point::new(6.0, 3.0)));
    }

    #[test]
    fn test_from_tuple() {
        let p: Point = (10.0, 20.0).into();
        assert_eq!(p, Point::new(10.0, 20.0));

        let p2: Point = (10i32, 20i32).into();
        assert_eq!(p2, Point::new(10.0, 20.0));
    }
}
