use crate::crs::Crs;
use crate::geo::{Bounds, Point};
use crate::map::MapState;

/// A tile address in the standard `{x, y, z}` web-map scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
    pub z: u8,
}

impl TileCoord {
    #[inline]
    pub fn new(x: i32, y: i32, z: u8) -> Self {
        Self { x, y, z }
    }

    /// Unique string key for use in tile caches.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.z, self.x, self.y)
    }
}

/// Tile grid calculator — determines which tiles are visible for a given
/// map state and calculates their pixel positions.
pub struct TileGrid {
    pub tile_size: f64,
}

impl TileGrid {
    pub fn new(tile_size: f64) -> Self {
        Self { tile_size }
    }

    /// Get the range of tile coordinates visible in the current viewport.
    ///
    /// Returns `(min_tile, max_tile)` as tile-grid-space points.
    pub fn visible_tile_range(&self, state: &MapState, crs: &dyn Crs) -> (TileCoord, TileCoord) {
        self.visible_tile_range_at(state.pixel_bounds(), state.zoom(), crs)
    }

    /// Get the range of tile coordinates visible for explicit pixel bounds and
    /// zoom level. This allows computing tiles at a different zoom than what
    /// `state.zoom()` reports (e.g. integer tile zoom during fractional display
    /// zoom animation).
    pub fn visible_tile_range_at(
        &self,
        pixel_bounds: Bounds,
        zoom: f64,
        crs: &dyn Crs,
    ) -> (TileCoord, TileCoord) {
        let zoom_u8 = zoom.round() as u8;
        let tile_bounds = self.pixel_bounds_to_tile_range(pixel_bounds);

        let (min, max) = if let Some(proj_bounds) = crs.projected_bounds(zoom) {
            let global = self.pixel_bounds_to_tile_range(proj_bounds);
            let min_x = if crs.wrap_lng().is_some() {
                tile_bounds.min.x
            } else {
                tile_bounds.min.x.max(global.min.x)
            };
            let max_x = if crs.wrap_lng().is_some() {
                tile_bounds.max.x
            } else {
                tile_bounds.max.x.min(global.max.x)
            };

            (
                TileCoord::new(min_x, tile_bounds.min.y.max(global.min.y), zoom_u8),
                TileCoord::new(max_x, tile_bounds.max.y.min(global.max.y), zoom_u8),
            )
        } else {
            (
                TileCoord::new(tile_bounds.min.x, tile_bounds.min.y, zoom_u8),
                TileCoord::new(tile_bounds.max.x, tile_bounds.max.y, zoom_u8),
            )
        };

        (min, max)
    }

    /// Iterate all visible tile coordinates.
    pub fn visible_tiles(&self, state: &MapState, crs: &dyn Crs) -> Vec<TileCoord> {
        self.visible_tiles_at(state.pixel_bounds(), state.zoom(), crs)
    }

    /// Iterate all visible tile coordinates for explicit pixel bounds and zoom.
    pub fn visible_tiles_at(
        &self,
        pixel_bounds: Bounds,
        zoom: f64,
        crs: &dyn Crs,
    ) -> Vec<TileCoord> {
        let (min, max) = self.visible_tile_range_at(pixel_bounds, zoom, crs);
        let mut tiles = Vec::new();
        for y in min.y..=max.y {
            for x in min.x..=max.x {
                tiles.push(TileCoord::new(x, y, min.z));
            }
        }
        tiles
    }

    /// Get the pixel position of a tile relative to the map container.
    pub fn tile_position(&self, coord: TileCoord, state: &MapState) -> Point {
        self.tile_position_at(coord, state.pixel_origin())
    }

    /// Get the pixel position of a tile relative to an explicit pixel origin.
    pub fn tile_position_at(&self, coord: TileCoord, pixel_origin: Point) -> Point {
        let tile_pixel = Point::new(
            coord.x as f64 * self.tile_size,
            coord.y as f64 * self.tile_size,
        );
        tile_pixel - pixel_origin
    }

    /// Format a tile URL from a template string.
    ///
    /// Supported placeholders: `{x}`, `{y}`, `{z}`, `{s}` (subdomain).
    pub fn format_tile_url(template: &str, coord: TileCoord, subdomains: &[char]) -> String {
        let subdomain_idx =
            ((coord.x.unsigned_abs() + coord.y.unsigned_abs()) as usize) % subdomains.len().max(1);
        let subdomain = subdomains.get(subdomain_idx).copied().unwrap_or('a');

        template
            .replace("{x}", &coord.x.to_string())
            .replace("{y}", &coord.y.to_string())
            .replace("{z}", &coord.z.to_string())
            .replace("{s}", &subdomain.to_string())
    }

    /// Wraps tile x coordinate for longitude-wrapping CRS.
    pub fn wrap_x(&self, x: i32, zoom: u8) -> i32 {
        let num_tiles = 1i32 << (zoom as i32);
        ((x % num_tiles) + num_tiles) % num_tiles
    }

    // ─── Internal ────────────────────────────────────────────────────────────

    fn pixel_bounds_to_tile_range(&self, bounds: Bounds) -> TileRange {
        TileRange {
            min: TileXY {
                x: (bounds.min.x / self.tile_size).floor() as i32,
                y: (bounds.min.y / self.tile_size).floor() as i32,
            },
            max: TileXY {
                x: ((bounds.max.x / self.tile_size).ceil() as i32) - 1,
                y: ((bounds.max.y / self.tile_size).ceil() as i32) - 1,
            },
        }
    }
}

#[derive(Debug)]
struct TileXY {
    x: i32,
    y: i32,
}

#[derive(Debug)]
struct TileRange {
    min: TileXY,
    max: TileXY,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crs::Epsg3857;
    use crate::geo::LatLng;

    #[test]
    fn test_visible_tiles_zoom_0() {
        let crs = Epsg3857;
        let state = MapState::new(LatLng::new(0.0, 0.0), 0.0, Point::new(256.0, 256.0));
        let grid = TileGrid::new(256.0);
        let tiles = grid.visible_tiles(&state, &crs);
        // At zoom 0 with a 256x256 viewport, we should see exactly 1 tile
        assert_eq!(tiles.len(), 1, "Expected 1 tile, got {:?}", tiles);
        assert_eq!(tiles[0], TileCoord::new(0, 0, 0));
    }

    #[test]
    fn test_visible_tiles_zoom_1() {
        let crs = Epsg3857;
        // At zoom 1, world is 512x512. With 512x512 viewport centered at origin,
        // should see 4 tiles.
        let state = MapState::new(LatLng::new(0.0, 0.0), 1.0, Point::new(512.0, 512.0));
        let grid = TileGrid::new(256.0);
        let tiles = grid.visible_tiles(&state, &crs);
        assert_eq!(tiles.len(), 4, "Expected 4 tiles, got {}", tiles.len());
    }

    #[test]
    fn test_tile_url_formatting() {
        let url = TileGrid::format_tile_url(
            "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
            TileCoord::new(10, 20, 5),
            &['a', 'b', 'c'],
        );
        assert_eq!(url, "https://tile.openstreetmap.org/5/10/20.png");
    }

    #[test]
    fn test_tile_url_subdomain() {
        let url = TileGrid::format_tile_url(
            "https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png",
            TileCoord::new(1, 2, 3),
            &['a', 'b', 'c'],
        );
        // subdomain index = (1 + 2) % 3 = 0 → 'a'
        assert!(url.starts_with("https://a."));
    }

    #[test]
    fn test_tile_position() {
        let state = MapState::new(LatLng::new(0.0, 0.0), 0.0, Point::new(256.0, 256.0));
        let grid = TileGrid::new(256.0);
        let pos = grid.tile_position(TileCoord::new(0, 0, 0), &state);
        // Tile (0,0) should start at the container's top-left (approximately)
        assert!(pos.x.abs() < 2.0, "x = {}", pos.x);
        assert!(pos.y.abs() < 2.0, "y = {}", pos.y);
    }

    #[test]
    fn test_wrap_x() {
        let grid = TileGrid::new(256.0);
        assert_eq!(grid.wrap_x(-1, 2), 3); // -1 wraps to 3 (4 tiles at zoom 2)
        assert_eq!(grid.wrap_x(4, 2), 0); // 4 wraps to 0
        assert_eq!(grid.wrap_x(2, 2), 2); // within range, no change
    }

    #[test]
    fn test_visible_tiles_keep_wrapped_columns() {
        let crs = Epsg3857;
        let state = MapState::new(LatLng::new(0.0, 179.0), 2.0, Point::new(512.0, 256.0));
        let grid = TileGrid::new(256.0);
        let tiles = grid.visible_tiles(&state, &crs);

        assert!(
            tiles.iter().any(|tile| tile.x > 3),
            "expected wrapped x-columns past the world edge, got {tiles:?}"
        );
    }
}
