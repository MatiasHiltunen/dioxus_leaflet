use dioxus::prelude::*;
use futures_timer::Delay;
use js_sys::{Function, Reflect};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use super::{
    canvas_tooltip_style, hit_test_marker, platform_dpr, prepare_layer_state, snap_translation,
    use_pending_tile_requests, PaintTile, PaintTileLayer, PreparedLayerState,
    DRAW_POLL_INTERVAL_MS, FALLBACK_REUSE_READY_RATIO, MARKER_HEAD_CENTER_OFFSET_Y,
    MARKER_HEAD_RADIUS, MARKER_TAIL_HALF_WIDTH, MARKER_TAIL_TOP_OFFSET_Y, MAX_CACHED_IMAGES,
    PRIMARY_REPLACE_READY_RATIO,
};
use crate::components::map::MapContext;

fn draw_tile_layer(
    ctx: &CanvasRenderingContext2d,
    dpr: f64,
    layer: &PaintTileLayer,
    image_cache: &HashMap<String, HtmlImageElement>,
    track_pending: bool,
) -> bool {
    let scale = layer.transform.scale;
    let tx = snap_translation(layer.transform.translate.x, dpr);
    let ty = snap_translation(layer.transform.translate.y, dpr);
    let _ = ctx.set_transform(dpr * scale, 0.0, 0.0, dpr * scale, dpr * tx, dpr * ty);

    let mut has_pending = false;
    for tile in &layer.tiles {
        if tile.image.is_none() {
            if track_pending {
                has_pending = true;
            }
            continue;
        }

        let Some(image) = image_cache.get(&tile.cache_key) else {
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

fn count_ready_tiles(
    layer: &PaintTileLayer,
    image_cache: &HashMap<String, HtmlImageElement>,
) -> usize {
    layer
        .tiles
        .iter()
        .filter(|tile| {
            tile.image.is_some()
                && image_cache
                    .get(&tile.cache_key)
                    .map(|image| {
                        image.complete() && image.natural_width() > 0 && image.natural_height() > 0
                    })
                    .unwrap_or(false)
        })
        .count()
}

fn best_fallback_layer<'a>(
    fallback_layers: &'a [PaintTileLayer],
    image_cache: &HashMap<String, HtmlImageElement>,
) -> Option<&'a PaintTileLayer> {
    fallback_layers.iter().max_by(|left, right| {
        count_ready_tiles(left, image_cache).cmp(&count_ready_tiles(right, image_cache))
    })
}

fn draw_scene_on_canvas(
    canvas: &HtmlCanvasElement,
    ctx: &CanvasRenderingContext2d,
    dpr: f64,
    render_scene: &super::MapPaintScene,
    image_cache: &HashMap<String, HtmlImageElement>,
) -> bool {
    let css_w = render_scene.viewport_size.x.max(1.0);
    let css_h = render_scene.viewport_size.y.max(1.0);

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

    let selected_fallback = best_fallback_layer(&render_scene.fallbacks, image_cache);
    if let Some(layer) = selected_fallback {
        let _ = draw_tile_layer(ctx, dpr, layer, image_cache, false);
    }

    let primary_total = render_scene.primary.tiles.len().max(1);
    let primary_ready = count_ready_tiles(&render_scene.primary, image_cache);
    let primary_ready_ratio = primary_ready as f64 / primary_total as f64;

    let (fallback_ready, fallback_total) = selected_fallback
        .map(|layer| {
            (
                count_ready_tiles(layer, image_cache),
                layer.tiles.len().max(1),
            )
        })
        .unwrap_or((0, 1));
    let fallback_ready_ratio = fallback_ready as f64 / fallback_total as f64;

    let should_defer_primary = selected_fallback.is_some()
        && fallback_ready_ratio >= FALLBACK_REUSE_READY_RATIO
        && primary_ready_ratio < PRIMARY_REPLACE_READY_RATIO;
    let should_draw_primary = !should_defer_primary;

    let has_pending = if should_draw_primary {
        draw_tile_layer(ctx, dpr, &render_scene.primary, image_cache, true)
    } else {
        primary_ready < render_scene.primary.tiles.len()
    };

    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    for marker in &render_scene.markers {
        draw_marker_pin(ctx, marker.point, &marker.color);
    }

    has_pending
}

fn draw_marker_pin(ctx: &CanvasRenderingContext2d, point: leaflet_core::geo::Point, color: &str) {
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

fn collect_ready_tiles<'a>(layer: &'a PaintTileLayer, tiles: &mut Vec<&'a PaintTile>) {
    for tile in &layer.tiles {
        if tile.image.is_some() {
            tiles.push(tile);
        }
    }
}

fn collect_keep_keys(render_scene: &super::MapPaintScene) -> HashSet<String> {
    let mut keep = HashSet::new();
    for tile in &render_scene.primary.tiles {
        keep.insert(tile.cache_key.clone());
    }
    for layer in &render_scene.fallbacks {
        for tile in &layer.tiles {
            keep.insert(tile.cache_key.clone());
        }
    }
    keep
}

#[component]
pub fn CanvasTileLayerComponent() -> Element {
    let ctx = use_context::<MapContext>();

    let (initial_primary_zoom, initial_density_zoom) = {
        let state = ctx.state.read();
        let source = ctx.tile_source.read();
        let screen_dpr = platform_dpr();
        let source_pixel_ratio = source.source_pixel_ratio();
        (
            state.tile_zoom_for_density(screen_dpr, source_pixel_ratio),
            state.density_zoom(screen_dpr, source_pixel_ratio),
        )
    };

    let mut previous_primary_zoom_signal = use_signal(|| initial_primary_zoom);
    let mut previous_density_zoom_signal = use_signal(|| initial_density_zoom);

    let prepared: PreparedLayerState = prepare_layer_state(
        &ctx,
        *previous_primary_zoom_signal.read(),
        *previous_density_zoom_signal.read(),
    );
    let render_scene = prepared.render_scene.clone();
    let pending_requests = prepared.pending_requests.clone();

    use_pending_tile_requests(ctx, pending_requests);

    use_effect(use_reactive(
        (&prepared.primary_zoom, &prepared.density_zoom),
        move |(primary_zoom, density_zoom)| {
            if (*previous_primary_zoom_signal.peek() - primary_zoom).abs() > f64::EPSILON {
                previous_primary_zoom_signal.set(primary_zoom);
            }
            if (*previous_density_zoom_signal.peek() - density_zoom).abs() > f64::EPSILON {
                previous_density_zoom_signal.set(density_zoom);
            }
        },
    ));

    let surface = use_signal(|| None::<(HtmlCanvasElement, CanvasRenderingContext2d)>);
    let mut image_cache = use_signal(|| HashMap::<String, HtmlImageElement>::new());
    let redraw_tick = use_signal(|| 0_u64);
    let mut draw_poll_in_flight = use_signal(|| false);
    let mut marker_click_seq = ctx.marker_click_seq;
    let mut marker_clicked_id = ctx.marker_clicked_id;
    let hovered_marker = use_signal(|| None::<super::HoveredMarker>);
    let redraw_tick_value = *redraw_tick.read();
    let marker_sprites_for_hover = render_scene.markers.clone();
    let marker_sprites_for_click = render_scene.markers.clone();
    let mut hovered_marker_for_move = hovered_marker;
    let mut hovered_marker_for_leave = hovered_marker;
    let hovered_marker_value = hovered_marker.read().clone();

    use_effect(use_reactive(
        (&render_scene, &redraw_tick_value),
        move |(render_scene, _)| {
            let keep_keys = collect_keep_keys(&render_scene);
            let mut ready_tiles = Vec::new();
            collect_ready_tiles(&render_scene.primary, &mut ready_tiles);
            for layer in &render_scene.fallbacks {
                collect_ready_tiles(layer, &mut ready_tiles);
            }

            {
                let mut cache = image_cache.write();
                cache.retain(|cache_key, _| keep_keys.contains(cache_key));

                for tile in ready_tiles {
                    if cache.contains_key(&tile.cache_key) {
                        continue;
                    }
                    if cache.len() >= MAX_CACHED_IMAGES {
                        break;
                    }
                    let Some(image_data) = &tile.image else {
                        continue;
                    };
                    if let Ok(image) = HtmlImageElement::new() {
                        image.set_src(image_data.data_url());
                        cache.insert(tile.cache_key.clone(), image);
                    }
                }
            }

            let dpr = platform_dpr();
            let surface = surface.read().clone();
            let has_pending = if let Some((canvas, ctx2d)) = surface {
                let cache = image_cache.read();
                draw_scene_on_canvas(&canvas, &ctx2d, dpr, &render_scene, &cache)
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
                let pointer = leaflet_core::geo::Point::new(offset.x, offset.y);
                marker_clicked_id.set(
                    hit_test_marker(&marker_sprites_for_click, pointer).map(|marker| marker.id),
                );
                *marker_click_seq.write() += 1;
            },
            onpointermove: move |evt: PointerEvent| {
                let offset = evt.data().element_coordinates();
                let pointer = leaflet_core::geo::Point::new(offset.x, offset.y);
                let next_hover = hit_test_marker(&marker_sprites_for_hover, pointer);
                if *hovered_marker_for_move.peek() != next_hover {
                    hovered_marker_for_move.set(next_hover);
                }
            },
            onpointerleave: move |_| {
                hovered_marker_for_leave.set(None);
            },
            onmounted: move |evt| {
                let mounted: Rc<MountedData> = evt.data();
                let mut surface = surface;
                let mut redraw_tick = redraw_tick;

                async move {
                    let canvas = mounted
                        .downcast::<web_sys::Element>()
                        .cloned()
                        .and_then(|element| element.dyn_into::<HtmlCanvasElement>().ok());

                    let Some(canvas) = canvas else {
                        return;
                    };

                    if let Some(ctx2d) = try_get_canvas_2d_context(&canvas) {
                        surface.set(Some((canvas, ctx2d)));
                        *redraw_tick.write() += 1;
                    }
                }
            },
        }
        if let Some(hovered_marker) = hovered_marker_value.filter(|hovered| !hovered.title.is_empty()) {
            div {
                class: "leaflet-canvas-tooltip",
                style: canvas_tooltip_style(&hovered_marker),
                "{hovered_marker.title}"
            }
        }
    }
}
