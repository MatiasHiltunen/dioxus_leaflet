use crate::crs::projection::SphericalMercator;
use crate::crs::Crs;
use crate::geo::{Bounds, LatLng, Point, Transformation};

/// EPSG:3857 — the standard CRS for web maps (Google Maps, OSM, etc.).
///
/// Uses Spherical Mercator projection with the standard tile-grid
/// transformation that maps the world into a 256×256 tile at zoom 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct Epsg3857;

impl Epsg3857 {
    const TRANSFORMATION: Transformation = {
        let scale = 0.5 / (std::f64::consts::PI * SphericalMercator::R);
        Transformation::new(scale, 0.5, -scale, 0.5)
    };
}

impl Crs for Epsg3857 {
    fn code(&self) -> &str {
        "EPSG:3857"
    }

    fn project(&self, ll: LatLng) -> Point {
        SphericalMercator::project(ll)
    }

    fn unproject(&self, p: Point) -> LatLng {
        SphericalMercator::unproject(p)
    }

    fn transformation(&self) -> Transformation {
        Self::TRANSFORMATION
    }

    fn projection_bounds(&self) -> Bounds {
        SphericalMercator::bounds()
    }

    fn wrap_lng(&self) -> Option<(f64, f64)> {
        Some((-180.0, 180.0))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lat_lng_to_point_roundtrip() {
        let crs = Epsg3857;
        let ll = LatLng::new(51.505, -0.09);
        let zoom = 13.0;
        let p = crs.lat_lng_to_point(ll, zoom);
        let back = crs.point_to_lat_lng(p, zoom);
        assert!((back.lat - ll.lat).abs() < 1e-6);
        assert!((back.lng - ll.lng).abs() < 1e-6);
    }

    #[test]
    fn test_scale() {
        let crs = Epsg3857;
        assert_eq!(crs.scale(0.0), 256.0);
        assert_eq!(crs.scale(1.0), 512.0);
        assert_eq!(crs.scale(2.0), 1024.0);
    }

    #[test]
    fn test_zoom() {
        let crs = Epsg3857;
        assert!((crs.zoom(256.0) - 0.0).abs() < 1e-10);
        assert!((crs.zoom(512.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_origin_maps_to_center() {
        let crs = Epsg3857;
        let p = crs.lat_lng_to_point(LatLng::new(0.0, 0.0), 0.0);
        // At zoom 0, world is 256 px. Origin should map to center (128, 128).
        assert!((p.x - 128.0).abs() < 0.1, "x = {}", p.x);
        assert!((p.y - 128.0).abs() < 0.1, "y = {}", p.y);
    }

    #[test]
    fn test_projected_bounds() {
        let crs = Epsg3857;
        let bounds = crs.projected_bounds(0.0).unwrap();
        // At zoom 0, bounds should be ~(0,0)-(256,256)
        assert!((bounds.min.x - 0.0).abs() < 1.0);
        assert!((bounds.min.y - 0.0).abs() < 1.0);
        assert!((bounds.max.x - 256.0).abs() < 1.0);
        assert!((bounds.max.y - 256.0).abs() < 1.0);
    }
}
