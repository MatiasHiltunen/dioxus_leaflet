use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use leaflet_core::crs::Epsg3857;
use leaflet_core::geo::LatLng;
#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "wasm32")]
use super::map::CanvasMarker;
use super::map::MapContext;

#[cfg(not(target_arch = "wasm32"))]
#[inline]
fn window_dpr() -> f64 {
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

#[cfg(not(target_arch = "wasm32"))]
#[inline]
fn snap_to_device_px(value: f64, dpr: f64) -> f64 {
    (value * dpr).round() / dpr
}

#[cfg(target_arch = "wasm32")]
static NEXT_MARKER_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(target_arch = "wasm32")]
#[inline]
fn next_marker_id() -> u64 {
    NEXT_MARKER_ID.fetch_add(1, Ordering::Relaxed)
}

/// A marker at a geographic position.
///
/// On web/wasm we register markers into the canvas marker layer to avoid
/// per-marker DOM nodes. On non-wasm we keep the DOM/SVG fallback renderer.
#[component]
#[cfg(target_arch = "wasm32")]
pub fn Marker(
    position: LatLng,
    #[props(default = "".to_string())] title: String,
    #[props(default = "#2196F3".to_string())] color: String,
    #[props(default)] on_click: Option<EventHandler<LatLng>>,
) -> Element {
    let ctx = use_context::<MapContext>();
    let marker_id = use_signal(next_marker_id);
    let marker_id_value = *marker_id.read();
    let mut marker_registry = ctx.marker_registry;
    let clicked_id = ctx.marker_clicked_id;
    let click_seq = *ctx.marker_click_seq.read();

    use_effect(use_reactive(
        (&position, &title, &color),
        move |(position, title, color)| {
            let next = CanvasMarker {
                position,
                title: title.clone(),
                color: color.clone(),
            };

            let mut registry = marker_registry.write();
            if registry.get(&marker_id_value) != Some(&next) {
                registry.insert(marker_id_value, next);
            }
        },
    ));

    use_drop(move || {
        marker_registry.write().remove(&marker_id_value);
    });

    use_effect(use_reactive((&click_seq,), move |(click_seq,)| {
        if click_seq == 0 {
            return;
        }
        if *clicked_id.peek() == Some(marker_id_value) {
            if let Some(on_click) = on_click.clone() {
                on_click.call(position);
            }
        }
    }));

    rsx! {}
}

#[component]
#[cfg(not(target_arch = "wasm32"))]
pub fn Marker(
    position: LatLng,
    #[props(default = "".to_string())] title: String,
    #[props(default = "#2196F3".to_string())] color: String,
    #[props(default)] on_click: Option<EventHandler<LatLng>>,
) -> Element {
    let ctx = use_context::<MapContext>();
    let state = ctx.state.read();

    let crs = Epsg3857;
    let point = state.lat_lng_to_container_point(position, &crs);
    let dpr = window_dpr();
    let marker_x = snap_to_device_px(point.x, dpr);
    let marker_y = snap_to_device_px(point.y, dpr);

    rsx! {
        div {
            class: "leaflet-marker",
            style: "left: {marker_x}px; top: {marker_y}px;",
            onclick: move |_| {
                if let Some(on_click) = on_click.clone() {
                    on_click.call(position);
                }
            },

            svg {
                class: "leaflet-marker-icon",
                width: "25",
                height: "41",
                view_box: "0 0 25 41",
                path {
                    d: "M12.5 0C5.6 0 0 5.6 0 12.5 0 21.9 12.5 41 12.5 41S25 21.9 25 12.5C25 5.6 19.4 0 12.5 0z",
                    fill: "{color}",
                    stroke: "#1565C0",
                    stroke_width: "1",
                }
                circle {
                    cx: "12.5",
                    cy: "12.5",
                    r: "5",
                    fill: "#fff",
                }
            }

            if !title.is_empty() {
                div {
                    class: "leaflet-marker-tooltip",
                    "{title}"
                }
            }
        }
    }
}
