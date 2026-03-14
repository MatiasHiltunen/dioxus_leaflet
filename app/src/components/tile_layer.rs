use dioxus::prelude::*;
use leaflet_core::crs::{Crs, Epsg3857};
use leaflet_core::geo::Bounds;
use leaflet_core::map::MapState;
use leaflet_core::map::TileGrid;
use leaflet_core::tile::{ResolvedTileRequest, TileEntryState, TileRepository, TileSource};
use leaflet_core::view::TileScene;
use std::collections::HashSet;

use super::map::MapContext;

/// Renders the visible tiles and loads them through Rust-side HTTP fetches.
const PREFETCH_MAX_REQUESTS: usize = 24;

fn prefetch_neighbor_zoom_requests(
    state: &MapState,
    grid: &TileGrid,
    source: &leaflet_core::tile::XyzTileSource,
    repository: &TileRepository,
    crs: &dyn Crs,
) -> Vec<ResolvedTileRequest> {
    let mut requests = Vec::new();
    let tile_zoom = state.tile_zoom();
    let min_zoom = state.min_zoom().round();
    let max_zoom = state.max_zoom().round();
    let half = state.size() / 2.0;

    for neighbor_zoom in [tile_zoom - 1.0, tile_zoom + 1.0] {
        if neighbor_zoom < min_zoom || neighbor_zoom > max_zoom {
            continue;
        }

        let center_px = crs.lat_lng_to_point(state.center(), neighbor_zoom);
        let pixel_origin = (center_px - half).round();
        let center = pixel_origin + half;
        let pixel_bounds = Bounds::new(center - half, center + half);

        for coord in grid.visible_tiles_at(pixel_bounds, neighbor_zoom, crs) {
            let request = source.resolve_request(coord);
            if matches!(repository.status(&request.cache_key), TileEntryState::Missing) {
                requests.push(request);
                if requests.len() >= PREFETCH_MAX_REQUESTS {
                    return requests;
                }
            }
        }
    }

    requests
}

#[component]
pub fn TileLayerComponent() -> Element {
    let ctx = use_context::<MapContext>();
    let mut tile_repository = ctx.tile_repository;
    let tile_client = ctx.tile_client;
    let state = ctx.state.read();
    let source = ctx.tile_source.read().clone();
    let use_browser_tile_urls = cfg!(target_arch = "wasm32");

    let crs = Epsg3857;
    let grid = TileGrid::new(256.0);
    let (scene, pending_requests) = {
        let repository = tile_repository.read();
        let scene = TileScene::build(&state, &grid, &source, &repository, &crs);
        let pending_requests = if use_browser_tile_urls {
            Vec::new()
        } else {
            let prefetch_requests =
                prefetch_neighbor_zoom_requests(&state, &grid, &source, &repository, &crs);
            let mut seen = HashSet::new();
            scene.pending_requests()
                .into_iter()
                .chain(prefetch_requests)
                .filter(|request| seen.insert(request.cache_key.clone()))
                .collect::<Vec<_>>()
        };
        (scene, pending_requests)
    };

    use_effect(use_reactive(
        (&pending_requests,),
        move |(pending_requests,)| {
            let client = tile_client.read().clone();
            for request in pending_requests {
                let should_fetch = {
                    let mut repository = tile_repository.write();
                    repository.mark_loading(request.cache_key.clone())
                };

                if !should_fetch {
                    continue;
                }

                let mut tile_repository = tile_repository;
                let client = client.clone();
                spawn(async move {
                    let result = client.fetch_resolved(&request).await;

                    match result {
                        Ok(tile) => tile_repository.write().mark_ready(request.cache_key, tile),
                        Err(error) => tile_repository
                            .write()
                            .mark_failed(request.cache_key, error.to_string()),
                    }
                });
            }
        },
    ));

    let tx = scene.transform.translate.x;
    let ty = scene.transform.translate.y;
    let s = scene.transform.scale;

    rsx! {
        div {
            class: "leaflet-tile-container",
            style: "transform: translate3d({tx}px, {ty}px, 0) scale({s});",
            for tile in scene.tiles {
                if use_browser_tile_urls {
                    div {
                        key: "{tile.coord.key()}",
                        class: "leaflet-tile leaflet-tile-ready",
                        style: "left: {tile.origin.x}px; top: {tile.origin.y}px; width: {tile.size.x}px; height: {tile.size.y}px; background-image: url('{tile.request.url}');",
                    }
                } else {
                    match tile.state {
                        TileEntryState::Ready(image) => rsx! {
                            div {
                                key: "{tile.coord.key()}",
                                class: "leaflet-tile leaflet-tile-ready",
                                style: "left: {tile.origin.x}px; top: {tile.origin.y}px; width: {tile.size.x}px; height: {tile.size.y}px; background-image: url('{image.data_url()}');",
                            }
                        },
                        TileEntryState::Failed(_) => rsx! {
                            div {
                                key: "{tile.coord.key()}",
                                class: "leaflet-tile leaflet-tile-error",
                                style: "left: {tile.origin.x}px; top: {tile.origin.y}px; width: {tile.size.x}px; height: {tile.size.y}px;",
                            }
                        },
                        TileEntryState::Loading | TileEntryState::Missing => rsx! {
                            div {
                                key: "{tile.coord.key()}",
                                class: "leaflet-tile leaflet-tile-loading",
                                style: "left: {tile.origin.x}px; top: {tile.origin.y}px; width: {tile.size.x}px; height: {tile.size.y}px;",
                            }
                        }
                    }
                }
            }
        }
    }
}
