pub mod epsg3857;
pub mod projection;

use crate::geo::latlng::wrap_num;
use crate::geo::{Bounds, LatLng, LatLngBounds, Point, Transformation};

pub use epsg3857::Epsg3857;

/// Coordinate Reference System — converts between geographic coordinates and
/// pixel coordinates.
///
/// Implemented as a trait so users can bring their own CRS (EPSG:4326 etc.)
/// while we provide `Epsg3857` as the default.
pub trait Crs {
    /// CRS code string, e.g. `"EPSG:3857"`.
    fn code(&self) -> &str;

    /// Project `LatLng` to pixel coordinates at the given zoom.
    fn lat_lng_to_point(&self, ll: LatLng, zoom: f64) -> Point {
        let projected = self.project(ll);
        self.transformation().transform(projected, self.scale(zoom))
    }

    /// Inverse of `lat_lng_to_point`.
    fn point_to_lat_lng(&self, p: Point, zoom: f64) -> LatLng {
        let untransformed = self.transformation().untransform(p, self.scale(zoom));
        self.unproject(untransformed)
    }

    /// Project LatLng to CRS-native units (e.g. metres for EPSG:3857).
    fn project(&self, ll: LatLng) -> Point;

    /// Inverse of `project`.
    fn unproject(&self, p: Point) -> LatLng;

    /// Scale factor at the given zoom level. Default: `256 * 2^zoom`.
    fn scale(&self, zoom: f64) -> f64 {
        256.0 * 2.0_f64.powf(zoom)
    }

    /// Inverse of `scale`: returns the zoom level for a given scale factor.
    fn zoom(&self, scale: f64) -> f64 {
        (scale / 256.0).ln() / std::f64::consts::LN_2
    }

    /// The affine transformation applied after projection.
    fn transformation(&self) -> Transformation;

    /// Projected bounds at the given zoom, or `None` for infinite CRS.
    fn projected_bounds(&self, zoom: f64) -> Option<Bounds> {
        if self.is_infinite() {
            return None;
        }
        let b = self.projection_bounds();
        let s = self.scale(zoom);
        let t = self.transformation();
        Some(Bounds::new(t.transform(b.min, s), t.transform(b.max, s)))
    }

    /// Raw projection bounds (in projection units, before transformation).
    fn projection_bounds(&self) -> Bounds;

    /// Whether the CRS is unbounded.
    fn is_infinite(&self) -> bool {
        false
    }

    /// Longitude wrapping range, e.g. `Some((-180, 180))`.
    fn wrap_lng(&self) -> Option<(f64, f64)> {
        None
    }

    /// Latitude wrapping range.
    fn wrap_lat(&self) -> Option<(f64, f64)> {
        None
    }

    /// Wrap a `LatLng` according to the CRS's wrapping rules.
    fn wrap_lat_lng(&self, ll: LatLng) -> LatLng {
        let lng = match self.wrap_lng() {
            Some((min, max)) => wrap_num(ll.lng, min, max),
            None => ll.lng,
        };
        let lat = match self.wrap_lat() {
            Some((min, max)) => wrap_num(ll.lat, min, max),
            None => ll.lat,
        };
        LatLng {
            lat,
            lng,
            alt: ll.alt,
        }
    }

    /// Wrap a `LatLngBounds` so its center is within the CRS's wrapping rules.
    fn wrap_lat_lng_bounds(&self, bounds: LatLngBounds) -> LatLngBounds {
        let center = bounds.center();
        let new_center = self.wrap_lat_lng(center);
        let lat_shift = center.lat - new_center.lat;
        let lng_shift = center.lng - new_center.lng;
        if lat_shift == 0.0 && lng_shift == 0.0 {
            return bounds;
        }
        LatLngBounds::new(
            LatLng::new(bounds.sw.lat - lat_shift, bounds.sw.lng - lng_shift),
            LatLng::new(bounds.ne.lat - lat_shift, bounds.ne.lng - lng_shift),
        )
    }

    /// Zoom scale ratio between two zoom levels.
    fn zoom_scale(&self, to_zoom: f64, from_zoom: f64) -> f64 {
        self.scale(to_zoom) / self.scale(from_zoom)
    }

    /// Inverse of `zoom_scale`.
    fn scale_zoom(&self, scale: f64, from_zoom: f64) -> f64 {
        self.zoom(self.scale(from_zoom) * scale)
    }
}
