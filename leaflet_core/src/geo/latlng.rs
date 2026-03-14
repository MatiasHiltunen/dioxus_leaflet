use std::fmt;

/// A geographical point with latitude, longitude, and optional altitude.
///
/// All angle values are in **degrees**. This is a lightweight, `Copy` type
/// designed for high-performance map calculations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
    pub alt: Option<f64>,
}

/// Mean Earth radius in metres (IUGG recommended value).
const EARTH_RADIUS: f64 = 6_371_000.0;

impl LatLng {
    #[inline]
    pub const fn new(lat: f64, lng: f64) -> Self {
        Self {
            lat,
            lng,
            alt: None,
        }
    }

    #[inline]
    pub const fn with_alt(lat: f64, lng: f64, alt: f64) -> Self {
        Self {
            lat,
            lng,
            alt: Some(alt),
        }
    }

    /// Haversine distance in metres.
    pub fn distance_to(self, other: LatLng) -> f64 {
        let rad = std::f64::consts::PI / 180.0;
        let lat1 = self.lat * rad;
        let lat2 = other.lat * rad;
        let sin_d_lat = ((other.lat - self.lat) * rad / 2.0).sin();
        let sin_d_lon = ((other.lng - self.lng) * rad / 2.0).sin();
        let a = sin_d_lat * sin_d_lat + lat1.cos() * lat2.cos() * sin_d_lon * sin_d_lon;
        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
        EARTH_RADIUS * c
    }

    /// Wrap longitude to `[-180, 180]`.
    pub fn wrap(self) -> Self {
        Self {
            lat: self.lat,
            lng: wrap_num(self.lng, -180.0, 180.0),
            alt: self.alt,
        }
    }

    /// Returns a `LatLngBounds` where each boundary is `size_m / 2` metres
    /// from this point.
    pub fn to_bounds(self, size_m: f64) -> super::LatLngBounds {
        let lat_acc = 180.0 * size_m / 40_075_017.0;
        let lng_acc = lat_acc / (std::f64::consts::PI / 180.0 * self.lat).cos();
        super::LatLngBounds::new(
            LatLng::new(self.lat - lat_acc, self.lng - lng_acc),
            LatLng::new(self.lat + lat_acc, self.lng + lng_acc),
        )
    }

    /// Approximate equality within a margin (default 1e-9 degrees ≈ 0.1 mm).
    pub fn equals(self, other: LatLng, max_margin: Option<f64>) -> bool {
        let margin = max_margin.unwrap_or(1e-9);
        (self.lat - other.lat)
            .abs()
            .max((self.lng - other.lng).abs())
            <= margin
    }
}

/// Wrap `x` into `[min, max)`.
#[inline]
pub(crate) fn wrap_num(x: f64, min: f64, max: f64) -> f64 {
    let d = max - min;
    ((x - min) % d + d) % d + min
}

impl fmt::Display for LatLng {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LatLng({:.6}, {:.6})", self.lat, self.lng)
    }
}

impl From<(f64, f64)> for LatLng {
    fn from((lat, lng): (f64, f64)) -> Self {
        Self::new(lat, lng)
    }
}

impl From<[f64; 2]> for LatLng {
    fn from(arr: [f64; 2]) -> Self {
        Self::new(arr[0], arr[1])
    }
}

impl From<[f64; 3]> for LatLng {
    fn from(arr: [f64; 3]) -> Self {
        Self::with_alt(arr[0], arr[1], arr[2])
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haversine() {
        // London to Paris ≈ 343 km
        let london = LatLng::new(51.5074, -0.1278);
        let paris = LatLng::new(48.8566, 2.3522);
        let dist = london.distance_to(paris);
        assert!(
            (dist - 343_556.0).abs() < 1000.0,
            "Expected ~343 km, got {dist:.0}"
        );
    }

    #[test]
    fn test_wrap() {
        let p = LatLng::new(0.0, 190.0);
        assert!((p.wrap().lng - (-170.0)).abs() < 1e-10);

        let p2 = LatLng::new(0.0, -200.0);
        assert!((p2.wrap().lng - 160.0).abs() < 1e-10);
    }

    #[test]
    fn test_equals() {
        let a = LatLng::new(50.0, 30.0);
        let b = LatLng::new(50.0 + 1e-10, 30.0 - 1e-10);
        assert!(a.equals(b, None));
        assert!(!a.equals(LatLng::new(50.1, 30.0), None));
    }

    #[test]
    fn test_from_tuple() {
        let ll: LatLng = (51.5, -0.1).into();
        assert_eq!(ll.lat, 51.5);
        assert_eq!(ll.lng, -0.1);
    }
}
