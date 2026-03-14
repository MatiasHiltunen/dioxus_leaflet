use crate::crs::Crs;
use crate::geo::Point;
use crate::map::{MapState, TileCoord, TileGrid};
use crate::tile::{ResolvedTileRequest, TileEntryState, TileRepository, TileSource};

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

/// A renderer-neutral tile scene derived from map state.
#[derive(Clone, Debug, PartialEq)]
pub struct TileScene {
    pub viewport_size: Point,
    pub tiles: Vec<TileSprite>,
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
        let viewport_center = state.size() / 2.0;
        let tile_size = Point::new(grid.tile_size, grid.tile_size);
        let tiles = grid
            .visible_tiles(state, crs)
            .into_iter()
            .map(|coord| {
                let request = source.resolve_request(coord);
                let origin = grid.tile_position(coord, state);
                let tile_center = origin + tile_size / 2.0;
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
}
