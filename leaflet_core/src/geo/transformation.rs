use super::Point;

/// Affine transformation: transforms `(x, y)` → `(a*x + b, c*y + d)`.
///
/// Used by CRS projections to convert between projected coordinates and
/// pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transformation {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
}

impl Transformation {
    #[inline]
    pub const fn new(a: f64, b: f64, c: f64, d: f64) -> Self {
        Self { a, b, c, d }
    }

    /// Forward transform, optionally multiplied by `scale`.
    #[inline]
    pub fn transform(&self, p: Point, scale: f64) -> Point {
        Point::new(
            scale * (self.a * p.x + self.b),
            scale * (self.c * p.y + self.d),
        )
    }

    /// Inverse transform, optionally divided by `scale`.
    #[inline]
    pub fn untransform(&self, p: Point, scale: f64) -> Point {
        Point::new(
            (p.x / scale - self.b) / self.a,
            (p.y / scale - self.d) / self.c,
        )
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let t = Transformation::new(2.0, 5.0, -1.0, 10.0);
        let p = Point::new(1.0, 2.0);
        let transformed = t.transform(p, 1.0);
        assert_eq!(transformed, Point::new(7.0, 8.0));
        let back = t.untransform(transformed, 1.0);
        assert!((back.x - p.x).abs() < 1e-10);
        assert!((back.y - p.y).abs() < 1e-10);
    }

    #[test]
    fn test_with_scale() {
        let t = Transformation::new(1.0, 0.0, 1.0, 0.0);
        let p = Point::new(10.0, 20.0);
        let scaled = t.transform(p, 256.0);
        assert_eq!(scaled, Point::new(2560.0, 5120.0));
        let back = t.untransform(scaled, 256.0);
        assert!((back.x - p.x).abs() < 1e-10);
        assert!((back.y - p.y).abs() < 1e-10);
    }
}
