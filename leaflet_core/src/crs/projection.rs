use crate::geo::{Bounds, LatLng, Point};

/// Spherical Mercator projection — the standard for web maps (EPSG:3857).
///
/// Assumes Earth is a perfect sphere with radius 6,378,137 m (WGS 84
/// semi-major axis).
pub struct SphericalMercator;

impl SphericalMercator {
    /// WGS 84 semi-major axis in metres.
    pub const R: f64 = 6_378_137.0;
    /// Maximum encodable latitude before singularity.
    pub const MAX_LATITUDE: f64 = 85.051_128_779_8;

    /// Project `LatLng` (degrees) → `Point` (metres).
    pub fn project(ll: LatLng) -> Point {
        let d = std::f64::consts::PI / 180.0;
        let lat = ll.lat.clamp(-Self::MAX_LATITUDE, Self::MAX_LATITUDE);
        let sin_lat = (lat * d).sin();
        Point::new(
            Self::R * ll.lng * d,
            Self::R * ((1.0 + sin_lat) / (1.0 - sin_lat)).ln() / 2.0,
        )
    }

    /// Unproject `Point` (metres) → `LatLng` (degrees).
    pub fn unproject(p: Point) -> LatLng {
        let d = 180.0 / std::f64::consts::PI;
        LatLng::new(
            (2.0 * (p.y / Self::R).exp().atan() - std::f64::consts::FRAC_PI_2) * d,
            p.x * d / Self::R,
        )
    }

    /// The full extent of the projection in metres.
    pub fn bounds() -> Bounds {
        let d = Self::R * std::f64::consts::PI;
        Bounds::new(Point::new(-d, -d), Point::new(d, d))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let ll = LatLng::new(51.505, -0.09);
        let projected = SphericalMercator::project(ll);
        let back = SphericalMercator::unproject(projected);
        assert!(
            (back.lat - ll.lat).abs() < 1e-6,
            "lat: {} vs {}",
            back.lat,
            ll.lat
        );
        assert!(
            (back.lng - ll.lng).abs() < 1e-6,
            "lng: {} vs {}",
            back.lng,
            ll.lng
        );
    }

    #[test]
    fn test_origin_is_zero() {
        let p = SphericalMercator::project(LatLng::new(0.0, 0.0));
        assert!((p.x).abs() < 1e-10);
        assert!((p.y).abs() < 1e-10);
    }

    #[test]
    fn test_bounds_symmetric() {
        let b = SphericalMercator::bounds();
        assert!((b.min.x + b.max.x).abs() < 1e-6);
        assert!((b.min.y + b.max.y).abs() < 1e-6);
    }
}
