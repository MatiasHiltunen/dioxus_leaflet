use dioxus::prelude::*;
use leaflet_core::crs::Epsg3857;
use leaflet_core::geo::LatLng;

use super::map::MapContext;

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

#[inline]
fn snap_to_device_px(value: f64, dpr: f64) -> f64 {
    (value * dpr).round() / dpr
}

/// A marker at a geographic position, rendered as a positioned SVG pin.
#[component]
pub fn Marker(
    position: LatLng,
    #[props(default = "".to_string())] title: String,
    #[props(default = "#2196F3".to_string())] color: String,
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
