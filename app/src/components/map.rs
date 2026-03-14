use dioxus::prelude::*;
use futures_timer::Delay;
use leaflet_core::crs::{Crs, Epsg3857};
use leaflet_core::geo::{LatLng, Point};
use leaflet_core::map::MapState;
use leaflet_core::tile::{HttpTileClient, TileRepository, XyzTileSource};
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

use super::tile_layer::TileLayerComponent;

// ─── Inertia constants (matching Leaflet defaults) ────────────────────────────

const INERTIA_DECELERATION: f64 = 3400.0;
const INERTIA_MAX_SPEED: f64 = f64::INFINITY;
const INERTIA_EASE_LINEARITY: f64 = 0.2;
const INERTIA_SAMPLE_WINDOW_MS: u128 = 50;
const INERTIA_MIN_SPEED: f64 = 10.0;

// ─── Wheel zoom constants ─────────────────────────────────────────────────────

/// Maximum time window from first wheel event to zoom application.
const WHEEL_DEBOUNCE_MS: u64 = 40;
/// Divisor for sigmoid input — tuned for raw pixel deltas (~100-120 per tick).
const WHEEL_PX_PER_ZOOM_LEVEL: f64 = 120.0;

// ─── Zoom animation constants ────────────────────────────────────────────────

const ZOOM_ANIM_DURATION_MS: f64 = 250.0;
const ZOOM_ANIM_EASE_POWER: f64 = 3.0;

fn ease_out(t: f64, power: f64) -> f64 {
    1.0 - (1.0 - t).powf(power)
}

#[derive(Clone, Copy)]
struct DragSample {
    pos: Point,
    time: Instant,
}

/// Computes inertia velocity from recent drag samples and spawns a
/// decelerating pan animation. Matching Leaflet's `Draggable` + `Map.Drag`
/// inertia algorithm.
fn launch_inertia(
    mut drag_samples: Signal<Vec<DragSample>>,
    mut inertia_gen: Signal<u32>,
    mut map_state: Signal<MapState>,
) {
    let now = Instant::now();

    drag_samples
        .write()
        .retain(|s| now.duration_since(s.time).as_millis() <= INERTIA_SAMPLE_WINDOW_MS);

    let samples = drag_samples.read();
    if samples.len() < 2 {
        return;
    }

    let first = samples[0];
    let last = samples[samples.len() - 1];
    drop(samples);

    let direction = last.pos - first.pos;
    let duration_secs = last.time.duration_since(first.time).as_secs_f64();
    if duration_secs <= 0.0 {
        return;
    }

    let speed_vector = direction * (INERTIA_EASE_LINEARITY / duration_secs);
    let speed = speed_vector.length();
    if speed < INERTIA_MIN_SPEED {
        return;
    }

    let limited_speed = speed.min(INERTIA_MAX_SPEED);
    let limited_speed_vector = speed_vector * (limited_speed / speed);
    let decel_duration = limited_speed / (INERTIA_DECELERATION * INERTIA_EASE_LINEARITY);
    let offset = limited_speed_vector * (-decel_duration / 2.0);
    if offset.length() < 1.0 {
        return;
    }

    *inertia_gen.write() += 1;
    let gen = *inertia_gen.read();
    let ease_power = 1.0 / INERTIA_EASE_LINEARITY.max(0.2);

    let crs = Epsg3857;
    let state = map_state.read();
    let start_zoom = state.zoom();
    let start_center_px = crs.lat_lng_to_point(state.center(), start_zoom);
    drop(state);

    let start_time = Instant::now();

    spawn(async move {
        loop {
            Delay::new(Duration::from_millis(16)).await;

            if *inertia_gen.peek() != gen {
                break;
            }

            let t = (start_time.elapsed().as_secs_f64() / decel_duration).min(1.0);
            let new_center_px = start_center_px + offset * ease_out(t, ease_power);
            let new_center = crs.point_to_lat_lng(new_center_px, start_zoom);
            map_state.write().set_center(new_center, &crs);

            if t >= 1.0 {
                break;
            }
        }
    });
}

/// Spawns an animated zoom transition from `start_zoom` to `target_zoom`,
/// keeping the geographic location under `anchor_container` visually pinned.
fn launch_zoom_animation(
    mut map_state: Signal<MapState>,
    mut zoom_anim_gen: Signal<u32>,
    start_zoom: f64,
    target_zoom: f64,
    anchor_ll: LatLng,
    anchor_container: Point,
    size: Point,
) {
    *zoom_anim_gen.write() += 1;
    let gen = *zoom_anim_gen.read();

    let crs = Epsg3857;
    let start_time = Instant::now();

    spawn(async move {
        loop {
            if *zoom_anim_gen.peek() != gen {
                break;
            }

            let elapsed = start_time.elapsed().as_secs_f64() * 1000.0;
            let t = (elapsed / ZOOM_ANIM_DURATION_MS).min(1.0);
            let eased = ease_out(t, ZOOM_ANIM_EASE_POWER);

            let z = if t >= 1.0 {
                target_zoom
            } else {
                start_zoom + (target_zoom - start_zoom) * eased
            };

            let anchor_px = crs.lat_lng_to_point(anchor_ll, z);
            let center_px = anchor_px - anchor_container + size / 2.0;
            let center = crs.point_to_lat_lng(center_px, z);

            if t >= 1.0 {
                // Finalize using snapped integer zoom to avoid post-animation drift.
                map_state.write().set_view(center, target_zoom, &crs);
                break;
            }

            map_state.write().set_view_exact(center, z, &crs);
            Delay::new(Duration::from_millis(16)).await;
        }
    });
}

fn cancel_zoom_animation_and_snap(mut zoom_anim_gen: Signal<u32>, mut map_state: Signal<MapState>) {
    *zoom_anim_gen.write() += 1;
    let crs = Epsg3857;
    let mut state = map_state.write();
    let center = state.center();
    let snapped = state.tile_zoom();
    state.set_view(center, snapped, &crs);
}

fn client_to_container_point(client: Point, map_client_origin: Point, map_size: Point) -> Point {
    let x = (client.x - map_client_origin.x).clamp(0.0, map_size.x);
    let y = (client.y - map_client_origin.y).clamp(0.0, map_size.y);
    Point::new(x, y)
}

/// Shared map context provided to all child components.
#[derive(Clone, Copy)]
pub struct MapContext {
    pub state: Signal<MapState>,
    pub tile_source: Signal<XyzTileSource>,
    pub tile_repository: Signal<TileRepository>,
    pub tile_client: Signal<HttpTileClient>,
}

/// The root map component. Manages map state, tile source state, and input.
#[component]
pub fn LeafletMap(
    center: LatLng,
    zoom: f64,
    #[props(default = "100%".to_string())] width: String,
    #[props(default = "400px".to_string())] height: String,
    #[props(default = "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_string())]
    tile_url: String,
    #[props(default = "".to_string())] attribution: String,
    children: Element,
) -> Element {
    let mut map_state = use_signal(|| MapState::new(center, zoom, Point::new(800.0, 600.0)));
    let mut tile_source = use_signal(|| XyzTileSource::new(tile_url.clone()));
    let mut tile_repository = use_signal(|| TileRepository::new(384));
    let tile_client = use_signal(HttpTileClient::default);

    use_context_provider(|| MapContext {
        state: map_state,
        tile_source,
        tile_repository,
        tile_client,
    });

    // ─── Drag + inertia state ─────────────────────────────────────────────

    let mut dragging = use_signal(|| false);
    let mut drag_start = use_signal(Point::default);
    let mut drag_start_center = use_signal(|| center);
    let mut drag_samples = use_signal(|| Vec::<DragSample>::new());
    let mut inertia_gen = use_signal(|| 0_u32);

    // ─── Wheel zoom state ─────────────────────────────────────────────────

    let mut wheel_delta = use_signal(|| 0.0_f64);
    let mut wheel_start_time = use_signal(|| None::<Instant>);
    let mut wheel_timer_gen = use_signal(|| 0_u32);
    let mut pointer_container_pos = use_signal(|| Point::new(400.0, 300.0));
    let map_client_origin = use_signal(Point::default);
    let mut map_element = use_signal(|| None::<Rc<MountedData>>);

    // ─── Zoom animation state ────────────────────────────────────────────

    let zoom_anim_gen = use_signal(|| 0_u32);

    // ─── Prop sync ────────────────────────────────────────────────────────

    use_effect(use_reactive((&center, &zoom), move |(center, zoom)| {
        let needs_update = {
            let state = map_state.peek();
            state.center() != center || state.zoom() != zoom
        };
        if needs_update {
            let crs = Epsg3857;
            map_state.write().set_view(center, zoom, &crs);
        }
    }));

    use_effect(use_reactive((&tile_url,), move |(tile_url,)| {
        let next_source = XyzTileSource::new(tile_url.clone());
        if *tile_source.peek() != next_source {
            tile_source.set(next_source);
            tile_repository.write().clear();
        }
    }));

    // ─── Pointer events (drag + inertia) ──────────────────────────────────

    let on_pointer_down = move |evt: PointerEvent| {
        let coords = evt.data().client_coordinates();
        dragging.set(true);
        drag_start.set(Point::new(coords.x, coords.y));
        drag_start_center.set(map_state.read().center());

        let now = Instant::now();
        let mut samples = drag_samples.write();
        samples.clear();
        samples.push(DragSample {
            pos: Point::new(coords.x, coords.y),
            time: now,
        });
        drop(samples);

        *inertia_gen.write() += 1;
        cancel_zoom_animation_and_snap(zoom_anim_gen, map_state);
    };

    let on_pointer_move = move |evt: PointerEvent| {
        let client = evt.data().client_coordinates();
        let state = map_state.read();
        let container = client_to_container_point(
            Point::new(client.x, client.y),
            *map_client_origin.read(),
            state.size(),
        );
        drop(state);
        pointer_container_pos.set(container);

        if !*dragging.read() {
            return;
        }

        let coords = evt.data().client_coordinates();
        let current = Point::new(coords.x, coords.y);
        let delta = *drag_start.read() - current;

        let crs = Epsg3857;
        let state = map_state.read();
        let start_center_px = crs.lat_lng_to_point(*drag_start_center.read(), state.zoom());
        let new_center_px = start_center_px + delta;
        let new_center = crs.point_to_lat_lng(new_center_px, state.zoom());
        drop(state);

        map_state.write().set_center(new_center, &crs);

        let now = Instant::now();
        let mut samples = drag_samples.write();
        samples.push(DragSample {
            pos: current,
            time: now,
        });
        samples.retain(|s| now.duration_since(s.time).as_millis() <= INERTIA_SAMPLE_WINDOW_MS);
    };

    let on_pointer_up = move |_: PointerEvent| {
        dragging.set(false);
        launch_inertia(drag_samples, inertia_gen, map_state);
    };

    // ─── Wheel zoom (pointer-centric, debounced sigmoid) ──────────────────

    let on_wheel = move |evt: WheelEvent| {
        evt.prevent_default();
        *inertia_gen.write() += 1;

        let client = evt.data().client_coordinates();
        let state = map_state.read();
        let container = client_to_container_point(
            Point::new(client.x, client.y),
            *map_client_origin.read(),
            state.size(),
        );
        drop(state);
        pointer_container_pos.set(container);

        let delta_y = evt.data().delta().strip_units().y;
        *wheel_delta.write() += -delta_y;

        let now = Instant::now();
        if wheel_start_time.peek().is_none() {
            wheel_start_time.set(Some(now));
        }

        let elapsed_ms = match *wheel_start_time.peek() {
            Some(start) => now.duration_since(start).as_millis() as u64,
            None => 0,
        };
        let remaining = WHEEL_DEBOUNCE_MS.saturating_sub(elapsed_ms);

        *wheel_timer_gen.write() += 1;
        let gen = *wheel_timer_gen.read();

        spawn(async move {
            Delay::new(Duration::from_millis(remaining)).await;
            if *wheel_timer_gen.peek() != gen {
                return;
            }

            let accumulated = *wheel_delta.peek();
            wheel_delta.set(0.0);
            wheel_start_time.set(None);

            if accumulated == 0.0 {
                return;
            }

            // Sigmoid mapping: bounds accumulated delta to ~±4 zoom levels.
            let d2 = accumulated / (WHEEL_PX_PER_ZOOM_LEVEL * 4.0);
            let d3 = 4.0 * (2.0_f64 / (1.0 + (-d2.abs()).exp())).ln() / std::f64::consts::LN_2;
            let d4 = d3.ceil().max(1.0);
            let zoom_delta = if accumulated > 0.0 { d4 } else { -d4 };

            let crs = Epsg3857;
            let container_pos = *pointer_container_pos.peek();
            let state = map_state.read();
            let start_zoom = state.zoom();
            let min_z = state.min_zoom();
            let max_z = state.max_zoom();

            // Snap target to next integer level (floor-based for zoom-in,
            // ceil-based for zoom-out) so mid-animation re-scrolls compose
            // correctly.
            let base = if zoom_delta > 0.0 {
                start_zoom.floor()
            } else {
                start_zoom.ceil()
            };
            let target_zoom = (base + zoom_delta).clamp(min_z, max_z);

            let anchor_ll = state.container_point_to_lat_lng(container_pos, &crs);
            let size = state.size();
            drop(state);

            if (target_zoom - start_zoom).abs() > f64::EPSILON {
                launch_zoom_animation(
                    map_state,
                    zoom_anim_gen,
                    start_zoom,
                    target_zoom,
                    anchor_ll,
                    container_pos,
                    size,
                );
            }
        });
    };

    // ─── Other input events ───────────────────────────────────────────────

    let on_dblclick = move |evt: MouseEvent| {
        *inertia_gen.write() += 1;
        let crs = Epsg3857;
        let client = evt.data().client_coordinates();
        let state = map_state.read();
        let click_point = client_to_container_point(
            Point::new(client.x, client.y),
            *map_client_origin.read(),
            state.size(),
        );
        let start_zoom = state.zoom();
        let target_zoom = start_zoom.floor() + 1.0;
        let target_zoom = target_zoom.min(state.max_zoom());
        let anchor_ll = state.container_point_to_lat_lng(click_point, &crs);
        let size = state.size();
        drop(state);

        if (target_zoom - start_zoom).abs() > f64::EPSILON {
            launch_zoom_animation(
                map_state,
                zoom_anim_gen,
                start_zoom,
                target_zoom,
                anchor_ll,
                click_point,
                size,
            );
        }
    };

    let on_resize = move |evt: ResizeEvent| {
        let crs = Epsg3857;
        if let Ok(size) = evt.data().get_border_box_size() {
            map_state
                .write()
                .set_size(Point::new(size.width, size.height), &crs);
        }

        if let Some(element) = map_element.read().clone() {
            let mut map_client_origin = map_client_origin;
            spawn(async move {
                if let Ok(rect) = element.get_client_rect().await {
                    map_client_origin.set(Point::new(rect.origin.x, rect.origin.y));
                }
            });
        }
    };

    let on_keydown = move |evt: KeyboardEvent| {
        let crs = Epsg3857;
        let key = evt.key();

        match key {
            Key::ArrowUp => {
                let state = map_state.read();
                let center_px = crs.lat_lng_to_point(state.center(), state.zoom());
                let new_center =
                    crs.point_to_lat_lng(center_px + Point::new(0.0, -100.0), state.zoom());
                drop(state);
                map_state.write().set_center(new_center, &crs);
            }
            Key::ArrowDown => {
                let state = map_state.read();
                let center_px = crs.lat_lng_to_point(state.center(), state.zoom());
                let new_center =
                    crs.point_to_lat_lng(center_px + Point::new(0.0, 100.0), state.zoom());
                drop(state);
                map_state.write().set_center(new_center, &crs);
            }
            Key::ArrowLeft => {
                let state = map_state.read();
                let center_px = crs.lat_lng_to_point(state.center(), state.zoom());
                let new_center =
                    crs.point_to_lat_lng(center_px + Point::new(-100.0, 0.0), state.zoom());
                drop(state);
                map_state.write().set_center(new_center, &crs);
            }
            Key::ArrowRight => {
                let state = map_state.read();
                let center_px = crs.lat_lng_to_point(state.center(), state.zoom());
                let new_center =
                    crs.point_to_lat_lng(center_px + Point::new(100.0, 0.0), state.zoom());
                drop(state);
                map_state.write().set_center(new_center, &crs);
            }
            Key::Character(ref c) if c == "+" || c == "=" => {
                cancel_zoom_animation_and_snap(zoom_anim_gen, map_state);
                let state = map_state.read();
                let new_zoom = (state.zoom() + 1.0).min(state.max_zoom());
                drop(state);
                map_state.write().set_zoom(new_zoom, &crs);
            }
            Key::Character(ref c) if c == "-" || c == "_" => {
                cancel_zoom_animation_and_snap(zoom_anim_gen, map_state);
                let state = map_state.read();
                let new_zoom = (state.zoom() - 1.0).max(state.min_zoom());
                drop(state);
                map_state.write().set_zoom(new_zoom, &crs);
            }
            _ => {}
        }
    };

    let on_zoom_in = move |_| {
        cancel_zoom_animation_and_snap(zoom_anim_gen, map_state);
        let crs = Epsg3857;
        let state = map_state.read();
        let new_zoom = (state.zoom() + 1.0).min(state.max_zoom());
        drop(state);
        map_state.write().set_zoom(new_zoom, &crs);
    };

    let on_zoom_out = move |_| {
        cancel_zoom_animation_and_snap(zoom_anim_gen, map_state);
        let crs = Epsg3857;
        let state = map_state.read();
        let new_zoom = (state.zoom() - 1.0).max(state.min_zoom());
        drop(state);
        map_state.write().set_zoom(new_zoom, &crs);
    };

    rsx! {
        div {
            class: "leaflet-map",
            style: "width: {width}; height: {height};",
            tabindex: "0",
            onmounted: move |evt| {
                let data = evt.data();
                map_element.set(Some(data.clone()));
                let mut map_client_origin = map_client_origin;
                async move {
                    if let Ok(rect) = data.get_client_rect().await {
                        map_client_origin.set(Point::new(rect.origin.x, rect.origin.y));
                    }
                }
            },

            onresize: on_resize,
            onpointerdown: on_pointer_down,
            onpointermove: on_pointer_move,
            onpointerup: on_pointer_up,
            onpointerleave: move |_| {
                if *dragging.read() {
                    dragging.set(false);
                    launch_inertia(drag_samples, inertia_gen, map_state);
                }
            },
            onwheel: on_wheel,
            ondoubleclick: on_dblclick,
            onkeydown: on_keydown,
            oncontextmenu: move |evt| evt.prevent_default(),

            TileLayerComponent {}

            div {
                class: "leaflet-marker-container",
                {children}
            }

            div { class: "leaflet-zoom-control",
                button {
                    class: "leaflet-zoom-btn",
                    onclick: on_zoom_in,
                    title: "Zoom in",
                    "+"
                }
                button {
                    class: "leaflet-zoom-btn",
                    onclick: on_zoom_out,
                    title: "Zoom out",
                    "−"
                }
            }

            if !attribution.is_empty() {
                div {
                    class: "leaflet-attribution",
                    "{attribution}"
                }
            }
        }
    }
}
