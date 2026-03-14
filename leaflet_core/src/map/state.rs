use crate::crs::Crs;
use crate::geo::{Bounds, LatLng, LatLngBounds, Point};

/// The core map state: center, zoom, container size, and CRS.
///
/// This is framework-agnostic — it contains no rendering or event logic.
/// Dioxus components wrap this in a `Signal` for reactivity.
pub struct MapState {
    center: LatLng,
    zoom: f64,
    /// Container size in CSS pixels.
    size: Point,
    min_zoom: f64,
    max_zoom: f64,
    max_bounds: Option<LatLngBounds>,
    zoom_snap: f64,
    /// Pixel origin: the pixel coordinate of the top-left corner of the map
    /// container, relative to the CRS origin at the current zoom.
    pixel_origin: Point,
}

impl MapState {
    pub fn new(center: LatLng, zoom: f64, size: Point) -> Self {
        let mut state = Self {
            center,
            zoom: 0.0,
            size,
            min_zoom: 0.0,
            max_zoom: 25.0,
            max_bounds: None,
            zoom_snap: 1.0,
            pixel_origin: Point::default(),
        };
        state.zoom = state.limit_zoom(zoom);
        state.update_pixel_origin(&crate::crs::Epsg3857);
        state
    }

    // ─── Getters ─────────────────────────────────────────────────────────────

    #[inline]
    pub fn center(&self) -> LatLng {
        self.center
    }

    #[inline]
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    #[inline]
    pub fn size(&self) -> Point {
        self.size
    }

    #[inline]
    pub fn pixel_origin(&self) -> Point {
        self.pixel_origin
    }

    #[inline]
    pub fn min_zoom(&self) -> f64 {
        self.min_zoom
    }

    #[inline]
    pub fn max_zoom(&self) -> f64 {
        self.max_zoom
    }

    // ─── Setters & mutations ─────────────────────────────────────────────────

    pub fn set_view(&mut self, center: LatLng, zoom: f64, crs: &dyn Crs) {
        self.center = center;
        self.zoom = self.limit_zoom(zoom);
        self.update_pixel_origin(crs);
    }

    pub fn set_zoom(&mut self, zoom: f64, crs: &dyn Crs) {
        self.zoom = self.limit_zoom(zoom);
        self.update_pixel_origin(crs);
    }

    pub fn set_center(&mut self, center: LatLng, crs: &dyn Crs) {
        self.center = center;
        self.update_pixel_origin(crs);
    }

    pub fn set_size(&mut self, size: Point, crs: &dyn Crs) {
        self.size = size;
        self.update_pixel_origin(crs);
    }

    /// Zoom to `new_zoom` while keeping `container_point` fixed on screen.
    ///
    /// This replicates Leaflet's `setZoomAround`: the geographic location
    /// under `container_point` stays visually pinned while the zoom changes.
    pub fn set_zoom_around(&mut self, container_point: Point, new_zoom: f64, crs: &dyn Crs) {
        let new_zoom = self.limit_zoom(new_zoom);
        let scale = crs.zoom_scale(new_zoom, self.zoom);
        let view_half = self.size / 2.0;
        let center_offset = (container_point - view_half) * (1.0 - 1.0 / scale);
        let new_center = self.container_point_to_lat_lng(view_half + center_offset, crs);
        self.set_view(new_center, new_zoom, crs);
    }

    pub fn set_min_zoom(&mut self, z: f64) {
        self.min_zoom = z;
    }

    pub fn set_max_zoom(&mut self, z: f64) {
        self.max_zoom = z;
    }

    pub fn set_max_bounds(&mut self, bounds: Option<LatLngBounds>) {
        self.max_bounds = bounds;
    }

    pub fn set_zoom_snap(&mut self, snap: f64) {
        self.zoom_snap = snap;
    }

    /// Set view with fractional zoom, bypassing `zoom_snap`.
    ///
    /// Used during zoom animations where the zoom interpolates smoothly
    /// between integer levels. The zoom is clamped but never snapped.
    pub fn set_view_exact(&mut self, center: LatLng, zoom: f64, crs: &dyn Crs) {
        self.center = center;
        self.zoom = zoom.clamp(self.min_zoom, self.max_zoom);
        self.update_pixel_origin(crs);
    }

    /// The integer zoom level at which tiles should be fetched.
    ///
    /// During smooth zoom animations, `zoom()` may be fractional (e.g. 10.3).
    /// Tiles are always loaded at `tile_zoom = round(zoom)` and the fractional
    /// gap is bridged by a CSS scale transform on the tile container.
    #[inline]
    pub fn tile_zoom(&self) -> f64 {
        self.zoom.round().clamp(self.min_zoom, self.max_zoom)
    }

    // ─── Coordinate conversions ──────────────────────────────────────────────

    /// Project LatLng to pixel coordinates at the current zoom (or specified zoom).
    pub fn project(&self, ll: LatLng, zoom: Option<f64>, crs: &dyn Crs) -> Point {
        crs.lat_lng_to_point(ll, zoom.unwrap_or(self.zoom))
    }

    /// Unproject pixel coordinates to LatLng at the current zoom (or specified zoom).
    pub fn unproject(&self, p: Point, zoom: Option<f64>, crs: &dyn Crs) -> LatLng {
        crs.point_to_lat_lng(p, zoom.unwrap_or(self.zoom))
    }

    /// Convert LatLng to pixel position relative to the map container.
    pub fn lat_lng_to_container_point(&self, ll: LatLng, crs: &dyn Crs) -> Point {
        self.project(ll, None, crs) - self.pixel_origin
    }

    /// Convert container pixel position to LatLng.
    pub fn container_point_to_lat_lng(&self, p: Point, crs: &dyn Crs) -> LatLng {
        self.unproject(p + self.pixel_origin, None, crs)
    }

    /// Get the pixel bounds of the current view.
    pub fn pixel_bounds(&self) -> Bounds {
        let half = self.size / 2.0;
        let center_pixel = self.pixel_origin + half;
        Bounds::new(center_pixel - half, center_pixel + half)
    }

    /// Get the geographic bounds of the current view.
    pub fn lat_lng_bounds(&self, crs: &dyn Crs) -> LatLngBounds {
        let pb = self.pixel_bounds();
        let sw = crs.point_to_lat_lng(Point::new(pb.min.x, pb.max.y), self.zoom);
        let ne = crs.point_to_lat_lng(Point::new(pb.max.x, pb.min.y), self.zoom);
        LatLngBounds::new(sw, ne)
    }

    /// Calculate the highest zoom level at which `bounds` fits in the viewport.
    pub fn bounds_zoom(
        &self,
        bounds: LatLngBounds,
        inside: bool,
        padding: Point,
        crs: &dyn Crs,
    ) -> f64 {
        let mut zoom = self.max_zoom;
        let min_zoom = self.min_zoom;
        let size = self.size - padding * 2.0;

        loop {
            let ne_point = crs.lat_lng_to_point(bounds.ne, zoom);
            let sw_point = crs.lat_lng_to_point(bounds.sw, zoom);
            let bounds_size = Bounds::new(ne_point, sw_point).size();

            let fits = if inside {
                bounds_size.x <= size.x || bounds_size.y <= size.y
            } else {
                bounds_size.x <= size.x && bounds_size.y <= size.y
            };

            if fits || zoom <= min_zoom {
                break;
            }
            zoom -= 1.0;
        }

        if self.zoom_snap > 0.0 {
            zoom = (zoom / self.zoom_snap).floor() * self.zoom_snap;
        }
        zoom.clamp(min_zoom, self.max_zoom)
    }

    /// Zoom scale ratio between two zoom levels.
    pub fn zoom_scale(&self, to_zoom: f64, from_zoom: f64, crs: &dyn Crs) -> f64 {
        crs.zoom_scale(to_zoom, from_zoom)
    }

    /// Inverse of `zoom_scale`.
    pub fn scale_zoom(&self, scale: f64, from_zoom: f64, crs: &dyn Crs) -> f64 {
        crs.scale_zoom(scale, from_zoom)
    }

    // ─── Internal ────────────────────────────────────────────────────────────

    fn update_pixel_origin(&mut self, crs: &dyn Crs) {
        let center_pixel = crs.lat_lng_to_point(self.center, self.zoom);
        self.pixel_origin = (center_pixel - self.size / 2.0).round();
    }

    fn limit_zoom(&self, zoom: f64) -> f64 {
        let z = zoom.clamp(self.min_zoom, self.max_zoom);
        if self.zoom_snap > 0.0 {
            (z / self.zoom_snap).round() * self.zoom_snap
        } else {
            z
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crs::Epsg3857;

    #[test]
    fn test_container_point_roundtrip() {
        let crs = Epsg3857;
        let state = MapState::new(LatLng::new(51.505, -0.09), 13.0, Point::new(800.0, 600.0));

        let ll = LatLng::new(51.51, -0.08);
        let cp = state.lat_lng_to_container_point(ll, &crs);
        let back = state.container_point_to_lat_lng(cp, &crs);
        assert!((back.lat - ll.lat).abs() < 1e-5);
        assert!((back.lng - ll.lng).abs() < 1e-5);
    }

    #[test]
    fn test_center_is_at_container_center() {
        let crs = Epsg3857;
        let state = MapState::new(LatLng::new(51.505, -0.09), 13.0, Point::new(800.0, 600.0));
        let cp = state.lat_lng_to_container_point(state.center(), &crs);
        // Center should map to roughly (400, 300)
        assert!((cp.x - 400.0).abs() < 2.0, "x = {}", cp.x);
        assert!((cp.y - 300.0).abs() < 2.0, "y = {}", cp.y);
    }

    #[test]
    fn test_zoom_clamping() {
        let state = MapState::new(LatLng::new(0.0, 0.0), 15.0, Point::new(400.0, 400.0));
        assert_eq!(state.zoom(), 15.0);

        let mut state2 = state;
        let crs = Epsg3857;
        state2.set_zoom(30.0, &crs);
        assert_eq!(state2.zoom(), 25.0);

        state2.set_zoom(-5.0, &crs);
        assert_eq!(state2.zoom(), 0.0);
    }

    #[test]
    fn test_initial_zoom_respects_zoom_snap() {
        let state = MapState::new(LatLng::new(0.0, 0.0), 13.5, Point::new(400.0, 400.0));
        assert_eq!(state.zoom(), 14.0);
    }
}
