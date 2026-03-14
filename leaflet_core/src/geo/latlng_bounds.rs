use super::LatLng;
use std::fmt;

/// A rectangular geographical area defined by south-west and north-east corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatLngBounds {
    pub sw: LatLng,
    pub ne: LatLng,
}

impl LatLngBounds {
    /// Create bounds from two corner points. The actual SW/NE is normalised.
    pub fn new(a: LatLng, b: LatLng) -> Self {
        Self {
            sw: LatLng::new(a.lat.min(b.lat), a.lng.min(b.lng)),
            ne: LatLng::new(a.lat.max(b.lat), a.lng.max(b.lng)),
        }
    }

    /// Extend bounds to include a point.
    pub fn extend_latlng(self, ll: LatLng) -> Self {
        Self {
            sw: LatLng::new(self.sw.lat.min(ll.lat), self.sw.lng.min(ll.lng)),
            ne: LatLng::new(self.ne.lat.max(ll.lat), self.ne.lng.max(ll.lng)),
        }
    }

    /// Extend bounds to include another bounds.
    pub fn extend_bounds(self, other: LatLngBounds) -> Self {
        self.extend_latlng(other.sw).extend_latlng(other.ne)
    }

    /// Center of the bounds.
    pub fn center(self) -> LatLng {
        LatLng::new(
            (self.sw.lat + self.ne.lat) / 2.0,
            (self.sw.lng + self.ne.lng) / 2.0,
        )
    }

    /// South-west corner.
    #[inline]
    pub fn south_west(self) -> LatLng {
        self.sw
    }

    /// North-east corner.
    #[inline]
    pub fn north_east(self) -> LatLng {
        self.ne
    }

    /// North-west corner.
    #[inline]
    pub fn north_west(self) -> LatLng {
        LatLng::new(self.ne.lat, self.sw.lng)
    }

    /// South-east corner.
    #[inline]
    pub fn south_east(self) -> LatLng {
        LatLng::new(self.sw.lat, self.ne.lng)
    }

    #[inline]
    pub fn west(self) -> f64 {
        self.sw.lng
    }
    #[inline]
    pub fn south(self) -> f64 {
        self.sw.lat
    }
    #[inline]
    pub fn east(self) -> f64 {
        self.ne.lng
    }
    #[inline]
    pub fn north(self) -> f64 {
        self.ne.lat
    }

    /// Does this bounds contain the given point?
    pub fn contains_latlng(self, ll: LatLng) -> bool {
        ll.lat >= self.sw.lat
            && ll.lat <= self.ne.lat
            && ll.lng >= self.sw.lng
            && ll.lng <= self.ne.lng
    }

    /// Does this bounds fully contain another bounds?
    pub fn contains_bounds(self, other: LatLngBounds) -> bool {
        self.contains_latlng(other.sw) && self.contains_latlng(other.ne)
    }

    /// Do the two bounds share at least one point?
    pub fn intersects(self, other: LatLngBounds) -> bool {
        let lat_ok = other.ne.lat >= self.sw.lat && other.sw.lat <= self.ne.lat;
        let lng_ok = other.ne.lng >= self.sw.lng && other.sw.lng <= self.ne.lng;
        lat_ok && lng_ok
    }

    /// Do the two bounds share a positive-area intersection?
    pub fn overlaps(self, other: LatLngBounds) -> bool {
        let lat_ok = other.ne.lat > self.sw.lat && other.sw.lat < self.ne.lat;
        let lng_ok = other.ne.lng > self.sw.lng && other.sw.lng < self.ne.lng;
        lat_ok && lng_ok
    }

    /// Expand or shrink by a ratio in each direction.
    pub fn pad(self, ratio: f64) -> Self {
        let h = (self.sw.lat - self.ne.lat).abs() * ratio;
        let w = (self.sw.lng - self.ne.lng).abs() * ratio;
        Self {
            sw: LatLng::new(self.sw.lat - h, self.sw.lng - w),
            ne: LatLng::new(self.ne.lat + h, self.ne.lng + w),
        }
    }

    /// `"west,south,east,north"` for WMS/web service requests.
    pub fn to_bbox_string(self) -> String {
        format!(
            "{},{},{},{}",
            self.west(),
            self.south(),
            self.east(),
            self.north()
        )
    }

    pub fn equals(self, other: LatLngBounds, margin: Option<f64>) -> bool {
        self.sw.equals(other.sw, margin) && self.ne.equals(other.ne, margin)
    }
}

impl fmt::Display for LatLngBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LatLngBounds({}, {})", self.sw, self.ne)
    }
}

impl From<(LatLng, LatLng)> for LatLngBounds {
    fn from((a, b): (LatLng, LatLng)) -> Self {
        Self::new(a, b)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_normalises() {
        let b = LatLngBounds::new(LatLng::new(50.0, 30.0), LatLng::new(40.0, 20.0));
        assert_eq!(b.sw, LatLng::new(40.0, 20.0));
        assert_eq!(b.ne, LatLng::new(50.0, 30.0));
    }

    #[test]
    fn test_center() {
        let b = LatLngBounds::new(LatLng::new(40.0, -74.0), LatLng::new(41.0, -73.0));
        let c = b.center();
        assert!((c.lat - 40.5).abs() < 1e-10);
        assert!((c.lng - (-73.5)).abs() < 1e-10);
    }

    #[test]
    fn test_contains() {
        let b = LatLngBounds::new(LatLng::new(40.0, -80.0), LatLng::new(50.0, -70.0));
        assert!(b.contains_latlng(LatLng::new(45.0, -75.0)));
        assert!(!b.contains_latlng(LatLng::new(51.0, -75.0)));
    }

    #[test]
    fn test_intersects() {
        let a = LatLngBounds::new(LatLng::new(0.0, 0.0), LatLng::new(10.0, 10.0));
        let b = LatLngBounds::new(LatLng::new(5.0, 5.0), LatLng::new(15.0, 15.0));
        assert!(a.intersects(b));

        let c = LatLngBounds::new(LatLng::new(11.0, 11.0), LatLng::new(20.0, 20.0));
        assert!(!a.intersects(c));
    }

    #[test]
    fn test_bbox_string() {
        let b = LatLngBounds::new(LatLng::new(40.0, -74.0), LatLng::new(41.0, -73.0));
        assert_eq!(b.to_bbox_string(), "-74,40,-73,41");
    }
}
