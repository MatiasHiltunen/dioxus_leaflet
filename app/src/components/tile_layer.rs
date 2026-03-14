use dioxus::prelude::*;
use leaflet_core::crs::Epsg3857;
use leaflet_core::map::TileGrid;
use leaflet_core::tile::TileEntryState;
use leaflet_core::view::TileScene;

use super::map::MapContext;

/// Renders the visible tiles and loads them through Rust-side HTTP fetches.
#[component]
pub fn TileLayerComponent() -> Element {
    let ctx = use_context::<MapContext>();
    let mut tile_repository = ctx.tile_repository;
    let tile_client = ctx.tile_client;
    let state = ctx.state.read();
    let source = ctx.tile_source.read().clone();

    let crs = Epsg3857;
    let grid = TileGrid::new(256.0);
    let scene = {
        let repository = tile_repository.read();
        TileScene::build(&state, &grid, &source, &repository, &crs)
    };
    let pending_requests = scene.pending_requests();

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

    rsx! {
        div { class: "leaflet-tile-container",
            for tile in scene.tiles {
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
                    },
                }
            }
        }
    }
}
