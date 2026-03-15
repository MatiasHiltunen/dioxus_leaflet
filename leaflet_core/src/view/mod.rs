use crate::crs::Crs;
use crate::geo::{Bounds, Point};
use crate::map::{MapState, TileCoord, TileGrid};
use crate::tile::{ResolvedTileRequest, TileEntryState, TileRepository, TileSource};

const SCENE_OVERFETCH_TILES: f64 = 1.0;

/// A tile prepared for renderer consumption.
#[derive(Clone, Debug, PartialEq)]
pub struct TileSprite {
    pub coord: TileCoord,
    pub request: ResolvedTileRequest,
    pub origin: Point,
    pub size: Point,
    pub distance_to_view_center: f64,
    pub state: TileEntryState,
}

/// CSS transform for the tile container that bridges the gap between the
/// integer tile zoom and the (possibly fractional) display zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContainerTransform {
    pub translate: Point,
    pub scale: f64,
}

/// A renderer-neutral tile scene derived from map state.
#[derive(Clone, Debug, PartialEq)]
pub struct TileScene {
    pub viewport_size: Point,
    pub tiles: Vec<TileSprite>,
    pub transform: ContainerTransform,
}

impl TileScene {
    pub fn build<T>(
        state: &MapState,
        grid: &TileGrid,
        source: &T,
        repository: &TileRepository,
        crs: &dyn Crs,
    ) -> Self
    where
        T: TileSource,
    {
        Self::build_for_tile_zoom(state, grid, source, repository, crs, state.tile_zoom())
    }

    pub fn build_for_tile_zoom<T>(
        state: &MapState,
        grid: &TileGrid,
        source: &T,
        repository: &TileRepository,
        crs: &dyn Crs,
        tile_zoom: f64,
    ) -> Self
    where
        T: TileSource,
    {
        let display_zoom = state.zoom();
        let clamped_tile_zoom = tile_zoom.clamp(state.min_zoom(), state.max_zoom());
        let tile_zoom_u8 = clamped_tile_zoom.round();
        let scale = crs.zoom_scale(display_zoom, tile_zoom_u8);
        let safe_scale = scale.max(f64::EPSILON);
        let half_viewport = state.size() / 2.0;
        let half_tile_space = half_viewport / safe_scale;

        let tile_center_px = crs.lat_lng_to_point(state.center(), tile_zoom_u8);
        // Top-left viewport point represented in the explicit tile zoom space.
        let tile_pixel_origin = tile_center_px - half_tile_space;

        // Fetch beyond viewport so transform scaling and kinetic motion don't
        // expose empty bands at screen edges.
        let tile_center = tile_pixel_origin + half_tile_space;
        let padding = Point::new(
            grid.tile_size * SCENE_OVERFETCH_TILES,
            grid.tile_size * SCENE_OVERFETCH_TILES,
        );
        let tile_pixel_bounds = Bounds::new(
            tile_center - half_tile_space - padding,
            tile_center + half_tile_space + padding,
        );

        let translate = tile_pixel_origin * scale - state.pixel_origin();

        let viewport_center = state.size() / 2.0;
        let tile_size = Point::new(grid.tile_size, grid.tile_size);

        let tiles = grid
            .visible_tiles_at(tile_pixel_bounds, tile_zoom_u8, crs)
            .into_iter()
            .map(|coord| {
                let request = source.resolve_request(coord);
                let origin = grid.tile_position_at(coord, tile_pixel_origin);
                let tile_center = (origin + tile_size / 2.0) * scale + translate;
                TileSprite {
                    coord,
                    state: repository.status(&request.cache_key),
                    request,
                    origin,
                    size: tile_size,
                    distance_to_view_center: tile_center.distance_to(viewport_center),
                }
            })
            .collect();

        Self {
            viewport_size: state.size(),
            tiles,
            transform: ContainerTransform { translate, scale },
        }
    }

    pub fn pending_requests(&self) -> Vec<ResolvedTileRequest> {
        let mut pending = self
            .tiles
            .iter()
            .filter(|tile| matches!(tile.state, TileEntryState::Missing))
            .map(|tile| (tile.distance_to_view_center, tile.request.clone()))
            .collect::<Vec<_>>();

        pending.sort_by(|left, right| left.0.total_cmp(&right.0));
        pending
            .into_iter()
            .map(|(_, request)| request)
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crs::Epsg3857;
    use crate::geo::LatLng;
    use crate::tile::{TileImage, XyzTileSource};

    #[test]
    fn test_tile_scene_reports_tile_states() {
        let crs = Epsg3857;
        let state = MapState::new(LatLng::new(0.0, 0.0), 1.0, Point::new(256.0, 256.0));
        let grid = TileGrid::new(256.0);
        let source = XyzTileSource::new("https://tiles.test/{z}/{x}/{y}.png");
        let ready_request = source.resolve_request(TileCoord::new(0, 0, 1));

        let mut repository = TileRepository::new(4);
        repository.mark_ready(
            ready_request.cache_key,
            TileImage::new("image/png", vec![1, 2, 3]),
        );

        let scene = TileScene::build(&state, &grid, &source, &repository, &crs);

        assert!(scene
            .tiles
            .iter()
            .any(|tile| matches!(tile.state, TileEntryState::Ready(_))));
        assert_eq!(scene.viewport_size, Point::new(256.0, 256.0));
    }

    #[test]
    fn test_pending_requests_are_sorted_center_first() {
        let crs = Epsg3857;
        let state = MapState::new(LatLng::new(10.0, 20.0), 2.0, Point::new(400.0, 320.0));
        let grid = TileGrid::new(256.0);
        let source = XyzTileSource::new("https://tiles.test/{z}/{x}/{y}.png");
        let repository = TileRepository::new(16);

        let scene = TileScene::build(&state, &grid, &source, &repository, &crs);
        let pending = scene.pending_requests();
        let distances = pending
            .into_iter()
            .map(|request| {
                scene
                    .tiles
                    .iter()
                    .find(|tile| tile.request.cache_key == request.cache_key)
                    .unwrap()
                    .distance_to_view_center
            })
            .collect::<Vec<_>>();

        assert!(!distances.is_empty());
        assert!(distances.windows(2).all(|window| window[0] <= window[1]));
    }

    #[test]
    fn test_explicit_tile_zoom_scene_covers_viewport() {
        let crs = Epsg3857;
        let mut state = MapState::new(LatLng::new(51.505, -0.09), 11.0, Point::new(800.0, 600.0));
        state.set_zoom_snap(0.0);
        state.set_zoom(11.2, &crs);
        let grid = TileGrid::new(256.0);
        let source = XyzTileSource::new("https://tiles.test/{z}/{x}/{y}.png");
        let repository = TileRepository::new(64);

        let scene = TileScene::build_for_tile_zoom(&state, &grid, &source, &repository, &crs, 12.0);
        assert!(!scene.tiles.is_empty());

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for tile in &scene.tiles {
            min_x = min_x.min(tile.origin.x);
            min_y = min_y.min(tile.origin.y);
            max_x = max_x.max(tile.origin.x + tile.size.x);
            max_y = max_y.max(tile.origin.y + tile.size.y);
        }

        let scale = scene.transform.scale;
        let tx = scene.transform.translate.x;
        let ty = scene.transform.translate.y;
        let covered_min_x = min_x * scale + tx;
        let covered_min_y = min_y * scale + ty;
        let covered_max_x = max_x * scale + tx;
        let covered_max_y = max_y * scale + ty;

        assert!(covered_min_x <= 0.0, "covered_min_x={covered_min_x}");
        assert!(covered_min_y <= 0.0, "covered_min_y={covered_min_y}");
        assert!(
            covered_max_x >= state.size().x,
            "covered_max_x={} viewport_w={}",
            covered_max_x,
            state.size().x
        );
        assert!(
            covered_max_y >= state.size().y,
            "covered_max_y={} viewport_h={}",
            covered_max_y,
            state.size().y
        );
    }
}
