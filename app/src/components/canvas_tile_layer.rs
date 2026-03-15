#![cfg(target_arch = "wasm32")]

use dioxus::prelude::*;
use futures_timer::Delay;
use js_sys::{Array, Function, Object, Reflect};
use leaflet_core::crs::{Crs, Epsg3857};
use leaflet_core::geo::{Bounds, Point};
use leaflet_core::map::{MapState, TileGrid};
use leaflet_core::tile::TileSource;
use leaflet_core::view::TileScene;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, MessageEvent, Worker,
};

use super::map::MapContext;

const PREFETCH_MAX_URLS: usize = 32;
const PREFETCH_PADDING_TILES: f64 = 1.0;
const PREFETCH_NEAR_TRANSITION_DISTANCE: f64 = 0.2;
const PRIMARY_REPLACE_READY_RATIO: f64 = 0.35;
const FALLBACK_REUSE_READY_RATIO: f64 = 0.98;
const DRAW_POLL_INTERVAL_MS: u64 = 16;
const MAX_CACHED_IMAGES: usize = 768;
static TILE_WORKER_ASSET: Asset = asset!("/assets/tile_worker.js");

const MARKER_HEAD_RADIUS: f64 = 10.0;
const MARKER_HEAD_CENTER_OFFSET_Y: f64 = 26.0;
const MARKER_TAIL_HALF_WIDTH: f64 = 7.0;
const MARKER_TAIL_TOP_OFFSET_Y: f64 = 18.0;
const MARKER_HIT_PADDING: f64 = 2.0;

#[derive(Serialize)]
struct WorkerTile {
    url: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Clone, PartialEq, Serialize)]
struct WorkerMarker {
    x: f64,
    y: f64,
    color: String,
}

#[derive(Serialize)]
struct WorkerTileLayer {
    scale: f64,
    translate_x: f64,
    translate_y: f64,
    tiles: Vec<WorkerTile>,
}

#[derive(Serialize)]
struct WorkerSceneMessage {
    r#type: &'static str,
    width: f64,
    height: f64,
    dpr: f64,
    tile_size: f64,
    primary: WorkerTileLayer,
    fallback_layers: Vec<WorkerTileLayer>,
    markers: Vec<WorkerMarker>,
    desired_urls: Vec<String>,
}

#[derive(Clone, PartialEq)]
struct MarkerSprite {
    id: u64,
    point: Point,
    color: String,
    title: String,
}

#[derive(Clone, PartialEq)]
struct HoveredMarker {
    id: u64,
    point: Point,
    title: String,
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

fn prefetch_neighbor_urls(
    state: &MapState,
    grid: &TileGrid,
    source: &leaflet_core::tile::XyzTileSource,
    zoom_levels: &[f64],
    crs: &dyn Crs,
) -> Vec<String> {
    let mut urls = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let min_zoom = state.min_zoom();
    let max_zoom = state.max_zoom();

    for requested_zoom in zoom_levels {
        let neighbor_zoom = requested_zoom.clamp(min_zoom, max_zoom).round();
        let pixel_bounds =
            pixel_bounds_for_zoom(state, grid, neighbor_zoom, PREFETCH_PADDING_TILES, crs);
        for coord in grid.visible_tiles_at(pixel_bounds, neighbor_zoom, crs) {
            let url = source.resolve_request(coord).url;
            if seen.insert(url.clone()) {
                urls.push(url);
            }
            if urls.len() >= PREFETCH_MAX_URLS {
                return urls;
            }
        }
    }

    urls
}

fn push_scene_urls(urls: &mut Vec<String>, seen: &mut HashSet<String>, scene: &TileScene) {
    for tile in &scene.tiles {
        let url = tile.request.url.clone();
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }
}

fn build_desired_urls(
    primary_scene: &TileScene,
    fallback_scenes: &[TileScene],
    prefetched: Vec<String>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut urls = Vec::with_capacity(
        primary_scene.tiles.len()
            + fallback_scenes
                .iter()
                .map(|scene| scene.tiles.len())
                .sum::<usize>()
            + prefetched.len(),
    );

    push_scene_urls(&mut urls, &mut seen, primary_scene);
    for scene in fallback_scenes {
        push_scene_urls(&mut urls, &mut seen, scene);
    }

    for url in prefetched {
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }

    urls
}

fn draw_tile_layer(
    ctx: &CanvasRenderingContext2d,
    dpr: f64,
    scene: &TileScene,
    image_cache: &HashMap<String, HtmlImageElement>,
    track_pending: bool,
) -> bool {
    let s = scene.transform.scale;
    let tx = snap_translation(scene.transform.translate.x, dpr);
    let ty = snap_translation(scene.transform.translate.y, dpr);
    let _ = ctx.set_transform(dpr * s, 0.0, 0.0, dpr * s, dpr * tx, dpr * ty);

    let mut has_pending = false;
    for tile in &scene.tiles {
        let Some(image) = image_cache.get(&tile.request.url) else {
            if track_pending {
                has_pending = true;
            }
            continue;
        };

        if !image.complete() {
            if track_pending {
                has_pending = true;
            }
            continue;
        }

        if image.natural_width() == 0 || image.natural_height() == 0 {
            continue;
        }

        let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(
            image,
            tile.origin.x,
            tile.origin.y,
            tile.size.x,
            tile.size.y,
        );
    }

    has_pending
}

fn count_ready_tiles(scene: &TileScene, image_cache: &HashMap<String, HtmlImageElement>) -> usize {
    scene
        .tiles
        .iter()
        .filter(|tile| {
            image_cache
                .get(&tile.request.url)
                .map(|image| {
                    image.complete() && image.natural_width() > 0 && image.natural_height() > 0
                })
                .unwrap_or(false)
        })
        .count()
}

fn best_fallback_scene<'a>(
    fallback_scenes: &'a [TileScene],
    image_cache: &HashMap<String, HtmlImageElement>,
) -> Option<&'a TileScene> {
    fallback_scenes.iter().max_by(|left, right| {
        count_ready_tiles(left, image_cache).cmp(&count_ready_tiles(right, image_cache))
    })
}

fn draw_scene_on_canvas(
    canvas: &HtmlCanvasElement,
    ctx: &CanvasRenderingContext2d,
    dpr: f64,
    primary_scene: &TileScene,
    fallback_scenes: &[TileScene],
    markers: &[MarkerSprite],
    image_cache: &HashMap<String, HtmlImageElement>,
) -> bool {
    let css_w = primary_scene.viewport_size.x.max(1.0);
    let css_h = primary_scene.viewport_size.y.max(1.0);

    let pixel_w = (css_w * dpr).round().max(1.0) as u32;
    let pixel_h = (css_h * dpr).round().max(1.0) as u32;

    if canvas.width() != pixel_w {
        canvas.set_width(pixel_w);
    }
    if canvas.height() != pixel_h {
        canvas.set_height(pixel_h);
    }

    let _ = ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
    ctx.clear_rect(0.0, 0.0, pixel_w as f64, pixel_h as f64);

    let selected_fallback = best_fallback_scene(fallback_scenes, image_cache);
    if let Some(scene) = selected_fallback {
        let _ = draw_tile_layer(ctx, dpr, scene, image_cache, false);
    }
    let primary_total = primary_scene.tiles.len().max(1);
    let primary_ready = count_ready_tiles(primary_scene, image_cache);
    let primary_ready_ratio = primary_ready as f64 / primary_total as f64;
    let (fallback_ready, fallback_total) = selected_fallback
        .map(|scene| {
            (
                count_ready_tiles(scene, image_cache),
                scene.tiles.len().max(1),
            )
        })
        .unwrap_or((0, 1));
    let fallback_ready_ratio = fallback_ready as f64 / fallback_total as f64;
    let should_defer_primary = selected_fallback.is_some()
        && fallback_ready_ratio >= FALLBACK_REUSE_READY_RATIO
        && primary_ready_ratio < PRIMARY_REPLACE_READY_RATIO;
    let should_draw_primary = !should_defer_primary;
    let has_pending = if should_draw_primary {
        draw_tile_layer(ctx, dpr, primary_scene, image_cache, true)
    } else {
        primary_ready < primary_scene.tiles.len()
    };

    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    for marker in markers {
        draw_marker_pin(ctx, marker.point, &marker.color);
    }

    has_pending
}

fn draw_marker_pin(ctx: &CanvasRenderingContext2d, point: Point, color: &str) {
    let x = point.x;
    let y = point.y;
    let head_y = y - MARKER_HEAD_CENTER_OFFSET_Y;

    ctx.set_fill_style_str(color);
    ctx.set_stroke_style_str("#1565C0");
    ctx.set_line_width(1.0);
    ctx.set_line_join("round");
    ctx.begin_path();
    ctx.move_to(x, y);
    ctx.line_to(x - MARKER_TAIL_HALF_WIDTH, y - MARKER_TAIL_TOP_OFFSET_Y);
    ctx.line_to(x + MARKER_TAIL_HALF_WIDTH, y - MARKER_TAIL_TOP_OFFSET_Y);
    ctx.close_path();
    ctx.fill();
    ctx.stroke();

    ctx.begin_path();
    let _ = ctx.arc(
        x,
        head_y,
        MARKER_HEAD_RADIUS,
        0.0,
        std::f64::consts::PI * 2.0,
    );
    ctx.fill();
    ctx.stroke();

    ctx.set_fill_style_str("#ffffff");
    ctx.begin_path();
    let _ = ctx.arc(x, head_y, 4.5, 0.0, std::f64::consts::PI * 2.0);
    ctx.fill();
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

fn hit_test_marker(markers: &[MarkerSprite], point: Point) -> Option<HoveredMarker> {
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
fn snap_translation(value: f64, dpr: f64) -> f64 {
    (value * dpr).round() / dpr
}

fn configure_canvas_quality(ctx: &CanvasRenderingContext2d) {
    ctx.set_image_smoothing_enabled(true);
    let _ = Reflect::set(
        ctx.as_ref(),
        &JsValue::from_str("imageSmoothingQuality"),
        &JsValue::from_str("high"),
    );
}

fn try_get_canvas_2d_context(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    let raw_ctx = canvas.get_context("2d").ok().flatten()?;
    let ctx = raw_ctx.dyn_into::<CanvasRenderingContext2d>().ok()?;
    configure_canvas_quality(&ctx);
    Some(ctx)
}

#[inline]
fn window_dpr() -> f64 {
    web_sys::window()
        .map(|window| window.device_pixel_ratio())
        .unwrap_or(1.0)
        .max(1.0)
}

fn build_worker_tiles(scene: &TileScene) -> WorkerTileLayer {
    let tiles = scene
        .tiles
        .iter()
        .map(|tile| WorkerTile {
            url: tile.request.url.clone(),
            x: tile.origin.x,
            y: tile.origin.y,
            w: tile.size.x,
            h: tile.size.y,
        })
        .collect::<Vec<_>>();

    WorkerTileLayer {
        scale: scene.transform.scale,
        translate_x: scene.transform.translate.x,
        translate_y: scene.transform.translate.y,
        tiles,
    }
}

fn build_worker_scene(
    primary_scene: &TileScene,
    fallback_scenes: &[TileScene],
    desired_urls: Vec<String>,
    tile_size: f64,
    markers: &[WorkerMarker],
) -> WorkerSceneMessage {
    let primary = build_worker_tiles(primary_scene);
    let fallback_layers = fallback_scenes
        .iter()
        .map(build_worker_tiles)
        .collect::<Vec<_>>();

    let markers = markers
        .iter()
        .map(|marker| WorkerMarker {
            x: marker.x,
            y: marker.y,
            color: marker.color.clone(),
        })
        .collect::<Vec<_>>();

    WorkerSceneMessage {
        r#type: "scene",
        width: primary_scene.viewport_size.x,
        height: primary_scene.viewport_size.y,
        dpr: window_dpr(),
        tile_size,
        primary,
        fallback_layers,
        markers,
        desired_urls,
    }
}

fn schedule_redraw_next_frame(
    mut redraw_tick: Signal<u64>,
    mut draw_poll_in_flight: Signal<bool>,
) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };

    let callback = Closure::once_into_js(move |_ts: f64| {
        *redraw_tick.write() += 1;
        draw_poll_in_flight.set(false);
    });

    window
        .request_animation_frame(callback.unchecked_ref::<Function>())
        .is_ok()
}

fn try_init_worker(canvas: &HtmlCanvasElement) -> Result<Worker, JsValue> {
    let worker = Worker::new(&TILE_WORKER_ASSET.to_string())?;
    let offscreen = canvas.transfer_control_to_offscreen()?;

    let init = Object::new();
    Reflect::set(
        &init,
        &JsValue::from_str("type"),
        &JsValue::from_str("init"),
    )?;
    Reflect::set(&init, &JsValue::from_str("canvas"), &offscreen)?;

    let transfer = Array::new();
    transfer.push(&offscreen);
    worker.post_message_with_transfer(&init, &transfer)?;

    Ok(worker)
}

fn post_worker_control_message(worker: &Worker, kind: &str) -> Result<(), JsValue> {
    let message = Object::new();
    Reflect::set(
        &message,
        &JsValue::from_str("type"),
        &JsValue::from_str(kind),
    )?;
    worker.post_message(&message)
}

fn shutdown_worker(worker: &Worker) {
    let _ = post_worker_control_message(worker, "dispose");
    worker.set_onmessage(None);
    worker.terminate();
}

#[component]
pub fn CanvasTileLayerComponent() -> Element {
    let ctx = use_context::<MapContext>();
    let state = ctx.state.read();
    let source = ctx.tile_source.read().clone();
    let tile_size = (*ctx.tile_size.read()).max(1.0);
    let crs = Epsg3857;
    let grid = TileGrid::new(tile_size);

    let display_zoom = state.zoom();
    let primary_zoom = state.tile_zoom();
    let mut previous_primary_zoom_signal = use_signal(|| primary_zoom);
    let previous_primary_zoom = *previous_primary_zoom_signal.read();
    let mut previous_display_zoom_signal = use_signal(|| display_zoom);
    let previous_display_zoom = *previous_display_zoom_signal.read();
    let zoom_direction = (display_zoom - previous_display_zoom).signum();

    let mut fallback_zooms = Vec::<f64>::new();
    if (previous_primary_zoom - primary_zoom).abs() > f64::EPSILON {
        fallback_zooms.push(previous_primary_zoom.clamp(state.min_zoom(), state.max_zoom()));
    }
    fallback_zooms.sort_by(|a, b| a.total_cmp(b));
    fallback_zooms.dedup_by(|a, b| (*a - *b).abs() <= f64::EPSILON);

    let (primary_scene, fallback_scenes) = {
        let repository = ctx.tile_repository.read();
        let primary =
            TileScene::build_for_tile_zoom(&state, &grid, &source, &repository, &crs, primary_zoom);
        let fallback = fallback_zooms
            .iter()
            .map(|zoom| {
                TileScene::build_for_tile_zoom(&state, &grid, &source, &repository, &crs, *zoom)
            })
            .collect::<Vec<_>>();
        (primary, fallback)
    };

    let mut prefetch_levels = Vec::<f64>::new();
    if zoom_direction > 0.0 && primary_zoom < state.max_zoom() {
        // Prefetch only when approaching the rounding transition to next level.
        let transition_zoom = primary_zoom + 0.5;
        let distance_to_transition = transition_zoom - display_zoom;
        if (0.0..=PREFETCH_NEAR_TRANSITION_DISTANCE).contains(&distance_to_transition) {
            prefetch_levels.push((primary_zoom + 1.0).clamp(state.min_zoom(), state.max_zoom()));
        }
    } else if zoom_direction < 0.0 && primary_zoom > state.min_zoom() {
        let transition_zoom = primary_zoom - 0.5;
        let distance_to_transition = display_zoom - transition_zoom;
        if (0.0..=PREFETCH_NEAR_TRANSITION_DISTANCE).contains(&distance_to_transition) {
            prefetch_levels.push((primary_zoom - 1.0).clamp(state.min_zoom(), state.max_zoom()));
        }
    }
    prefetch_levels.sort_by(|a, b| a.total_cmp(b));
    prefetch_levels.dedup_by(|a, b| (*a - *b).abs() <= f64::EPSILON);

    let prefetched = prefetch_neighbor_urls(&state, &grid, &source, &prefetch_levels, &crs);
    let should_fetch_fallback_layers = (display_zoom - primary_zoom).abs() > f64::EPSILON
        || (previous_primary_zoom - primary_zoom).abs() > f64::EPSILON;
    let desired_urls = if should_fetch_fallback_layers {
        build_desired_urls(&primary_scene, &fallback_scenes, prefetched)
    } else {
        build_desired_urls(&primary_scene, &[], prefetched)
    };
    let mut marker_sprites = ctx
        .marker_registry
        .read()
        .iter()
        .map(|(id, marker)| MarkerSprite {
            id: *id,
            point: state.lat_lng_to_container_point(marker.position, &crs),
            color: marker.color.clone(),
            title: marker.title.clone(),
        })
        .collect::<Vec<_>>();
    marker_sprites.sort_by(|left, right| {
        left.point
            .y
            .total_cmp(&right.point.y)
            .then_with(|| left.id.cmp(&right.id))
    });
    let worker_markers = marker_sprites
        .iter()
        .map(|marker| WorkerMarker {
            x: marker.point.x,
            y: marker.point.y,
            color: marker.color.clone(),
        })
        .collect::<Vec<_>>();

    let mut canvas_mounted = use_signal(|| None::<Rc<MountedData>>);
    let mut worker = use_signal(|| None::<Worker>);
    let mut worker_ready = use_signal(|| false);
    let mut worker_onmessage = use_signal(|| None::<Closure<dyn FnMut(MessageEvent)>>);
    let mut use_main_thread_fallback = use_signal(|| false);
    let mut fallback_surface = use_signal(|| None::<(HtmlCanvasElement, CanvasRenderingContext2d)>);
    let mut fallback_dpr = use_signal(window_dpr);
    let mut fallback_viewport_key = use_signal(|| (0_u32, 0_u32));
    let mut image_cache = use_signal(|| HashMap::<String, HtmlImageElement>::new());
    let mut redraw_tick = use_signal(|| 0_u64);
    let mut draw_poll_in_flight = use_signal(|| false);
    let mut marker_click_seq = ctx.marker_click_seq;
    let mut marker_clicked_id = ctx.marker_clicked_id;
    let hovered_marker = use_signal(|| None::<HoveredMarker>);
    let redraw_tick_value = *redraw_tick.read();
    let worker_ready_value = *worker_ready.read();
    let hovered_marker_value = hovered_marker.read().clone();
    let marker_sprites_for_hover = marker_sprites.clone();
    let marker_sprites_for_click = marker_sprites.clone();
    let mut hovered_marker_for_move = hovered_marker;
    let mut hovered_marker_for_leave = hovered_marker;

    use_effect(use_reactive(
        (&primary_zoom, &display_zoom),
        move |(primary_zoom, display_zoom)| {
            if (*previous_primary_zoom_signal.peek() - primary_zoom).abs() > f64::EPSILON {
                previous_primary_zoom_signal.set(primary_zoom);
            }
            if (*previous_display_zoom_signal.peek() - display_zoom).abs() > f64::EPSILON {
                previous_display_zoom_signal.set(display_zoom);
            }
        },
    ));

    use_drop(move || {
        if let Some(active_worker) = worker.read().clone() {
            shutdown_worker(&active_worker);
        }
    });

    use_effect(use_reactive(
        (
            &primary_scene,
            &fallback_scenes,
            &desired_urls,
            &worker_markers,
            &marker_sprites,
            &redraw_tick_value,
            &worker_ready_value,
        ),
        move |(
            primary_scene,
            fallback_scenes,
            desired_urls,
            worker_markers,
            marker_sprites,
            _,
            worker_ready_value,
        )| {
            let active_worker = { worker.read().clone() };
            if let Some(active_worker) = active_worker {
                if !worker_ready_value {
                    return;
                }

                let payload = build_worker_scene(
                    &primary_scene,
                    &fallback_scenes,
                    desired_urls.clone(),
                    tile_size,
                    &worker_markers,
                );
                let posted = serde_wasm_bindgen::to_value(&payload)
                    .ok()
                    .and_then(|value| active_worker.post_message(&value).ok())
                    .is_some();

                if posted {
                    return;
                }

                shutdown_worker(&active_worker);
                worker.set(None);
                worker_ready.set(false);
                worker_onmessage.set(None);
                if fallback_surface.read().is_none() {
                    if let Some(mounted) = canvas_mounted.read().clone() {
                        if let Some(canvas) = mounted
                            .downcast::<web_sys::Element>()
                            .cloned()
                            .and_then(|element| element.dyn_into::<HtmlCanvasElement>().ok())
                        {
                            if let Some(ctx2d) = try_get_canvas_2d_context(&canvas) {
                                fallback_surface.set(Some((canvas, ctx2d)));
                                fallback_dpr.set(window_dpr());
                            }
                        }
                    }
                }
                use_main_thread_fallback.set(true);
                *redraw_tick.write() += 1;
            }

            if !*use_main_thread_fallback.read() {
                return;
            }

            if fallback_surface.read().is_none() {
                if let Some(mounted) = canvas_mounted.read().clone() {
                    if let Some(canvas) = mounted
                        .downcast::<web_sys::Element>()
                        .cloned()
                        .and_then(|element| element.dyn_into::<HtmlCanvasElement>().ok())
                    {
                        if let Some(ctx2d) = try_get_canvas_2d_context(&canvas) {
                            fallback_surface.set(Some((canvas, ctx2d)));
                            fallback_dpr.set(window_dpr());
                        }
                    }
                }
            }

            let viewport_key = (
                primary_scene.viewport_size.x.round().max(0.0) as u32,
                primary_scene.viewport_size.y.round().max(0.0) as u32,
            );
            if *fallback_viewport_key.peek() != viewport_key {
                fallback_viewport_key.set(viewport_key);
                fallback_dpr.set(window_dpr());
            }

            let keep_urls: HashSet<&String> = desired_urls.iter().collect();

            {
                let mut cache = image_cache.write();
                cache.retain(|url, _| keep_urls.contains(url));

                for url in desired_urls {
                    if cache.contains_key(&url) {
                        continue;
                    }

                    if cache.len() >= MAX_CACHED_IMAGES {
                        break;
                    }

                    if let Ok(image) = HtmlImageElement::new() {
                        image.set_src(&url);
                        cache.insert(url.clone(), image);
                    }
                }
            }

            let dpr = *fallback_dpr.read();
            let surface = fallback_surface.read().clone();
            let has_pending = if let Some((canvas, ctx2d)) = surface {
                let cache = image_cache.read();
                draw_scene_on_canvas(
                    &canvas,
                    &ctx2d,
                    dpr,
                    &primary_scene,
                    &fallback_scenes,
                    &marker_sprites,
                    &cache,
                )
            } else {
                false
            };

            if has_pending && !*draw_poll_in_flight.peek() {
                draw_poll_in_flight.set(true);
                let mut redraw_tick = redraw_tick;
                let mut draw_poll_in_flight = draw_poll_in_flight;
                if !schedule_redraw_next_frame(redraw_tick, draw_poll_in_flight) {
                    spawn(async move {
                        Delay::new(Duration::from_millis(DRAW_POLL_INTERVAL_MS)).await;
                        *redraw_tick.write() += 1;
                        draw_poll_in_flight.set(false);
                    });
                }
            }
        },
    ));

    rsx! {
        canvas {
            class: "leaflet-tile-canvas",
            onclick: move |evt: MouseEvent| {
                let offset = evt.data().element_coordinates();
                let pointer = Point::new(offset.x, offset.y);
                marker_clicked_id.set(
                    hit_test_marker(&marker_sprites_for_click, pointer).map(|marker| marker.id),
                );
                *marker_click_seq.write() += 1;
            },
            onpointermove: move |evt: PointerEvent| {
                let offset = evt.data().element_coordinates();
                let pointer = Point::new(offset.x, offset.y);
                let next_hover = hit_test_marker(&marker_sprites_for_hover, pointer);
                if *hovered_marker_for_move.peek() != next_hover {
                    hovered_marker_for_move.set(next_hover);
                }
            },
            onpointerleave: move |_| {
                hovered_marker_for_leave.set(None);
            },
            onmounted: move |evt| {
                let mounted = evt.data();
                canvas_mounted.set(Some(mounted.clone()));
                let mut redraw_tick = redraw_tick;
                let mut worker = worker;
                let mut worker_ready = worker_ready;
                let mut worker_onmessage = worker_onmessage;
                let mut use_main_thread_fallback = use_main_thread_fallback;
                let mut fallback_surface = fallback_surface;
                let mut fallback_dpr = fallback_dpr;

                async move {
                    let canvas = mounted
                        .downcast::<web_sys::Element>()
                        .cloned()
                        .and_then(|element| element.dyn_into::<HtmlCanvasElement>().ok());

                    let Some(canvas) = canvas else {
                        use_main_thread_fallback.set(true);
                        *redraw_tick.write() += 1;
                        return;
                    };

                    match try_init_worker(&canvas) {
                        Ok(created_worker) => {
                            worker_ready.set(false);
                            let mut worker_signal = worker;
                            let mut worker_ready_signal = worker_ready;
                            let mut fallback_signal = use_main_thread_fallback;
                            let mut redraw_signal = redraw_tick;
                            let mut worker_onmessage_signal = worker_onmessage;

                            let on_message = Closure::wrap(Box::new(move |evt: MessageEvent| {
                                let message_type = Reflect::get(
                                    &evt.data(),
                                    &JsValue::from_str("type"),
                                )
                                .ok()
                                .and_then(|value| value.as_string());

                                match message_type.as_deref() {
                                    Some("ready") => {
                                        worker_ready_signal.set(true);
                                        *redraw_signal.write() += 1;
                                    }
                                    Some("init_failed") => {
                                        if let Some(active_worker) = worker_signal.read().clone() {
                                            shutdown_worker(&active_worker);
                                        }
                                        worker_signal.set(None);
                                        worker_ready_signal.set(false);
                                        worker_onmessage_signal.set(None);
                                        fallback_signal.set(true);
                                        *redraw_signal.write() += 1;
                                    }
                                    _ => {}
                                }
                            }) as Box<dyn FnMut(MessageEvent)>);

                            created_worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
                            worker_onmessage.set(Some(on_message));
                            worker.set(Some(created_worker));
                        }
                        Err(_) => {
                            if let Some(ctx2d) = try_get_canvas_2d_context(&canvas) {
                                fallback_surface.set(Some((canvas.clone(), ctx2d)));
                                fallback_dpr.set(window_dpr());
                            }
                            use_main_thread_fallback.set(true);
                            *redraw_tick.write() += 1;
                        }
                    }
                }
            },
        }
        if let Some(hovered) = hovered_marker_value {
            if !hovered.title.is_empty() {
                div {
                    class: "leaflet-marker-tooltip-canvas",
                    style: "left: {hovered.point.x}px; top: {hovered.point.y}px;",
                    "{hovered.title}"
                }
            }
        }
    }
}
