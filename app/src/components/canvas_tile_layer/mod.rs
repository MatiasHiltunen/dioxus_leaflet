use dioxus::prelude::*;
use leaflet_core::crs::{Crs, Epsg3857};
use leaflet_core::geo::{Bounds, Point};
use leaflet_core::map::{MapState, TileGrid};
use leaflet_core::tile::{
    ResolvedTileRequest, TileEntryState, TileImage, TileRepository, TileSource, XyzTileSource,
};
use leaflet_core::view::{ContainerTransform, TileScene};
use std::collections::HashSet;

use super::map::{CanvasMarker, MapContext};

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
mod native;

#[cfg(all(not(target_arch = "wasm32"), feature = "native"))]
pub use native::CanvasTileLayerComponent;

#[cfg(target_arch = "wasm32")]
pub use web::CanvasTileLayerComponent;

pub(crate) const PREFETCH_MAX_REQUESTS: usize = 32;
pub(crate) const PREFETCH_PADDING_TILES: f64 = 1.0;
pub(crate) const PREFETCH_NEAR_TRANSITION_DISTANCE: f64 = 0.2;
pub(crate) const PRIMARY_REPLACE_READY_RATIO: f64 = 0.35;
pub(crate) const FALLBACK_REUSE_READY_RATIO: f64 = 0.98;
#[cfg(target_arch = "wasm32")]
pub(crate) const DRAW_POLL_INTERVAL_MS: u64 = 16;
#[cfg(target_arch = "wasm32")]
pub(crate) const MAX_CACHED_IMAGES: usize = 768;

pub(crate) const MARKER_HEAD_RADIUS: f64 = 10.0;
pub(crate) const MARKER_HEAD_CENTER_OFFSET_Y: f64 = 26.0;
pub(crate) const MARKER_TAIL_HALF_WIDTH: f64 = 7.0;
pub(crate) const MARKER_TAIL_TOP_OFFSET_Y: f64 = 18.0;
pub(crate) const MARKER_HIT_PADDING: f64 = 2.0;
pub(crate) const TOOLTIP_OFFSET_Y: f64 = 8.0;

#[derive(Clone, PartialEq)]
pub(crate) struct MarkerSprite {
    pub(crate) id: u64,
    pub(crate) point: Point,
    pub(crate) color: String,
    pub(crate) title: String,
}

#[derive(Clone, PartialEq)]
pub(crate) struct HoveredMarker {
    pub(crate) id: u64,
    pub(crate) point: Point,
    pub(crate) title: String,
}

#[derive(Clone, PartialEq)]
pub(crate) struct PaintTile {
    pub(crate) cache_key: String,
    pub(crate) origin: Point,
    pub(crate) size: Point,
    pub(crate) image: Option<TileImage>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct PaintTileLayer {
    pub(crate) transform: ContainerTransform,
    pub(crate) tiles: Vec<PaintTile>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct MapPaintScene {
    pub(crate) viewport_size: Point,
    pub(crate) primary: PaintTileLayer,
    pub(crate) fallbacks: Vec<PaintTileLayer>,
    pub(crate) markers: Vec<MarkerSprite>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct PreparedLayerState {
    pub(crate) primary_zoom: f64,
    pub(crate) density_zoom: f64,
    pub(crate) pending_requests: Vec<ResolvedTileRequest>,
    pub(crate) render_scene: MapPaintScene,
}

#[inline]
pub(crate) fn platform_dpr() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        return web_sys::window()
            .map(|window| window.device_pixel_ratio())
            .unwrap_or(1.0)
            .max(1.0);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        1.0
    }
}

pub(crate) fn prepare_layer_state(
    ctx: &MapContext,
    previous_primary_zoom: f64,
    previous_density_zoom: f64,
) -> PreparedLayerState {
    let state = ctx.state.read();
    let source = ctx.tile_source.read().clone();
    let tile_size = (*ctx.tile_size.read()).max(1.0);
    let repository = ctx.tile_repository.read();
    let crs = Epsg3857;
    let grid = TileGrid::new(tile_size);

    let screen_dpr = platform_dpr();
    let source_pixel_ratio = source.source_pixel_ratio();
    let density_zoom = state.density_zoom(screen_dpr, source_pixel_ratio);
    let primary_zoom = state.tile_zoom_for_density(screen_dpr, source_pixel_ratio);
    let zoom_direction = (density_zoom - previous_density_zoom).signum();

    let mut fallback_zooms = Vec::<f64>::new();
    if (previous_primary_zoom - primary_zoom).abs() > f64::EPSILON {
        fallback_zooms.push(previous_primary_zoom.clamp(state.min_zoom(), state.max_zoom()));
    }
    fallback_zooms.sort_by(|left, right| left.total_cmp(right));
    fallback_zooms.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON);

    let primary_scene =
        TileScene::build_for_tile_zoom(&state, &grid, &source, &repository, &crs, primary_zoom);
    let fallback_scenes = fallback_zooms
        .iter()
        .map(|zoom| {
            TileScene::build_for_tile_zoom(&state, &grid, &source, &repository, &crs, *zoom)
        })
        .collect::<Vec<_>>();

    let mut prefetch_levels = Vec::<f64>::new();
    if zoom_direction > 0.0 && primary_zoom < state.max_zoom() {
        let distance_to_transition = primary_zoom - density_zoom;
        if (0.0..=PREFETCH_NEAR_TRANSITION_DISTANCE).contains(&distance_to_transition) {
            prefetch_levels.push((primary_zoom + 1.0).clamp(state.min_zoom(), state.max_zoom()));
        }
    } else if zoom_direction < 0.0 && primary_zoom > state.min_zoom() {
        let distance_to_transition = density_zoom - (primary_zoom - 1.0);
        if (0.0..=PREFETCH_NEAR_TRANSITION_DISTANCE).contains(&distance_to_transition) {
            prefetch_levels.push((primary_zoom - 1.0).clamp(state.min_zoom(), state.max_zoom()));
        }
    }
    prefetch_levels.sort_by(|left, right| left.total_cmp(right));
    prefetch_levels.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON);

    let prefetch_requests =
        prefetch_neighbor_requests(&state, &grid, &source, &repository, &prefetch_levels, &crs);
    let pending_requests =
        build_pending_requests(&primary_scene, &fallback_scenes, prefetch_requests);
    let markers = build_marker_sprites(ctx, &state, &crs);
    let render_scene = build_render_scene(&primary_scene, &fallback_scenes, markers);

    PreparedLayerState {
        primary_zoom,
        density_zoom,
        pending_requests,
        render_scene,
    }
}

pub(crate) fn use_pending_tile_requests(
    ctx: MapContext,
    pending_requests: Vec<ResolvedTileRequest>,
) {
    let mut tile_repository = ctx.tile_repository;
    let tile_client = ctx.tile_client;

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
}

pub(crate) fn canvas_tooltip_style(hovered_marker: &HoveredMarker) -> String {
    let left = hovered_marker.point.x;
    let top = hovered_marker.point.y - TOOLTIP_OFFSET_Y;
    format!("left: {left}px; top: {top}px;")
}

pub(crate) fn hit_test_marker(markers: &[MarkerSprite], point: Point) -> Option<HoveredMarker> {
    markers.iter().rev().find_map(|marker| {
        if marker_contains_point(marker, point) {
            Some(HoveredMarker {
                id: marker.id,
                point: marker.point,
                title: marker.title.clone(),
            })
        } else {
            None
        }
    })
}

#[inline]
pub(crate) fn snap_translation(value: f64, dpr: f64) -> f64 {
    (value * dpr).round() / dpr
}

fn pixel_bounds_for_zoom(
    state: &MapState,
    grid: &TileGrid,
    zoom: f64,
    padding_tiles: f64,
    crs: &dyn Crs,
) -> Bounds {
    let half = state.size() / 2.0;
    let center_px = crs.lat_lng_to_point(state.center(), zoom);
    let pixel_origin = (center_px - half).round();
    let center = pixel_origin + half;
    let pad = (grid.tile_size * padding_tiles.max(0.0)).round();
    Bounds::new(
        Point::new(center.x - half.x - pad, center.y - half.y - pad),
        Point::new(center.x + half.x + pad, center.y + half.y + pad),
    )
}

fn prefetch_neighbor_requests(
    state: &MapState,
    grid: &TileGrid,
    source: &XyzTileSource,
    repository: &TileRepository,
    zoom_levels: &[f64],
    crs: &dyn Crs,
) -> Vec<ResolvedTileRequest> {
    let mut requests = Vec::<ResolvedTileRequest>::new();
    let mut seen = HashSet::<String>::new();
    let min_zoom = state.min_zoom();
    let max_zoom = state.max_zoom();

    for requested_zoom in zoom_levels {
        let neighbor_zoom = requested_zoom.clamp(min_zoom, max_zoom).round();
        let pixel_bounds =
            pixel_bounds_for_zoom(state, grid, neighbor_zoom, PREFETCH_PADDING_TILES, crs);

        for coord in grid.visible_tiles_at(pixel_bounds, neighbor_zoom, crs) {
            let request = source.resolve_request(coord);
            if !matches!(
                repository.status(&request.cache_key),
                TileEntryState::Missing
            ) {
                continue;
            }
            if seen.insert(request.cache_key.clone()) {
                requests.push(request);
            }
            if requests.len() >= PREFETCH_MAX_REQUESTS {
                return requests;
            }
        }
    }

    requests
}

fn build_pending_requests(
    primary_scene: &TileScene,
    fallback_scenes: &[TileScene],
    prefetched: Vec<ResolvedTileRequest>,
) -> Vec<ResolvedTileRequest> {
    let mut seen = HashSet::<String>::new();
    let mut requests = Vec::new();

    push_scene_requests(&mut requests, &mut seen, primary_scene);
    for scene in fallback_scenes {
        push_scene_requests(&mut requests, &mut seen, scene);
    }

    for request in prefetched {
        if seen.insert(request.cache_key.clone()) {
            requests.push(request);
        }
    }

    requests
}

fn push_scene_requests(
    requests: &mut Vec<ResolvedTileRequest>,
    seen: &mut HashSet<String>,
    scene: &TileScene,
) {
    for request in scene.pending_requests() {
        if seen.insert(request.cache_key.clone()) {
            requests.push(request);
        }
    }
}

fn build_marker_sprites(ctx: &MapContext, state: &MapState, crs: &dyn Crs) -> Vec<MarkerSprite> {
    let mut markers = ctx
        .marker_registry
        .read()
        .iter()
        .map(|(id, marker): (&u64, &CanvasMarker)| MarkerSprite {
            id: *id,
            point: state.lat_lng_to_container_point(marker.position, crs),
            color: marker.color.clone(),
            title: marker.title.clone(),
        })
        .collect::<Vec<_>>();

    markers.sort_by(|left, right| {
        left.point
            .y
            .total_cmp(&right.point.y)
            .then_with(|| left.id.cmp(&right.id))
    });
    markers
}

fn build_render_scene(
    primary_scene: &TileScene,
    fallback_scenes: &[TileScene],
    markers: Vec<MarkerSprite>,
) -> MapPaintScene {
    MapPaintScene {
        viewport_size: primary_scene.viewport_size,
        primary: build_render_layer(primary_scene),
        fallbacks: fallback_scenes
            .iter()
            .map(build_render_layer)
            .collect::<Vec<_>>(),
        markers,
    }
}

fn build_render_layer(scene: &TileScene) -> PaintTileLayer {
    let tiles = scene
        .tiles
        .iter()
        .map(|tile| PaintTile {
            cache_key: tile.request.cache_key.clone(),
            origin: tile.origin,
            size: tile.size,
            image: match &tile.state {
                TileEntryState::Ready(image) => Some(image.clone()),
                _ => None,
            },
        })
        .collect::<Vec<_>>();

    PaintTileLayer {
        transform: scene.transform,
        tiles,
    }
}

#[inline]
fn point_in_circle(point: Point, center: Point, radius: f64) -> bool {
    let dx = point.x - center.x;
    let dy = point.y - center.y;
    dx * dx + dy * dy <= radius * radius
}

#[inline]
fn signed_area(point: Point, a: Point, b: Point) -> f64 {
    (point.x - b.x) * (a.y - b.y) - (a.x - b.x) * (point.y - b.y)
}

#[inline]
fn point_in_triangle(point: Point, a: Point, b: Point, c: Point) -> bool {
    let d1 = signed_area(point, a, b);
    let d2 = signed_area(point, b, c);
    let d3 = signed_area(point, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn marker_contains_point(marker: &MarkerSprite, point: Point) -> bool {
    let tip = marker.point;
    let head_center = Point::new(tip.x, tip.y - MARKER_HEAD_CENTER_OFFSET_Y);
    if point_in_circle(point, head_center, MARKER_HEAD_RADIUS + MARKER_HIT_PADDING) {
        return true;
    }

    let left = Point::new(
        tip.x - MARKER_TAIL_HALF_WIDTH - MARKER_HIT_PADDING,
        tip.y - MARKER_TAIL_TOP_OFFSET_Y,
    );
    let right = Point::new(
        tip.x + MARKER_TAIL_HALF_WIDTH + MARKER_HIT_PADDING,
        tip.y - MARKER_TAIL_TOP_OFFSET_Y,
    );
    let tip_padded = Point::new(tip.x, tip.y + MARKER_HIT_PADDING);
    point_in_triangle(point, tip_padded, left, right)
}
