use dioxus::prelude::*;
use leaflet_core::crs::{Crs, Epsg3857};
use leaflet_core::geo::{LatLng, Point};
use leaflet_core::map::MapState;
use leaflet_core::tile::{HttpTileClient, TileRepository, XyzTileSource};

use super::tile_layer::TileLayerComponent;

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

    let mut dragging = use_signal(|| false);
    let mut drag_start = use_signal(Point::default);
    let mut drag_start_center = use_signal(|| center);

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

    let on_pointer_down = move |evt: PointerEvent| {
        let coords = evt.data().client_coordinates();
        dragging.set(true);
        drag_start.set(Point::new(coords.x, coords.y));
        drag_start_center.set(map_state.read().center());
    };

    let on_pointer_move = move |evt: PointerEvent| {
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
    };

    let on_pointer_up = move |_evt: PointerEvent| {
        dragging.set(false);
    };

    let on_wheel = move |evt: WheelEvent| {
        let crs = Epsg3857;
        let delta_y = evt.data().delta().strip_units().y;
        let zoom_delta = if delta_y < 0.0 { 1.0 } else { -1.0 };

        let state = map_state.read();
        let new_zoom = (state.zoom() + zoom_delta).clamp(state.min_zoom(), state.max_zoom());
        drop(state);

        map_state.write().set_zoom(new_zoom, &crs);
    };

    let on_dblclick = move |evt: MouseEvent| {
        let crs = Epsg3857;
        let coords = evt.data().element_coordinates();

        let state = map_state.read();
        let click_point = Point::new(coords.x, coords.y);
        let click_ll = state.container_point_to_lat_lng(click_point, &crs);
        let new_zoom = (state.zoom() + 1.0).min(state.max_zoom());
        drop(state);

        map_state.write().set_view(click_ll, new_zoom, &crs);
    };

    let on_resize = move |evt: ResizeEvent| {
        let crs = Epsg3857;
        if let Ok(size) = evt.data().get_border_box_size() {
            map_state
                .write()
                .set_size(Point::new(size.width, size.height), &crs);
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
                let state = map_state.read();
                let new_zoom = (state.zoom() + 1.0).min(state.max_zoom());
                drop(state);
                map_state.write().set_zoom(new_zoom, &crs);
            }
            Key::Character(ref c) if c == "-" || c == "_" => {
                let state = map_state.read();
                let new_zoom = (state.zoom() - 1.0).max(state.min_zoom());
                drop(state);
                map_state.write().set_zoom(new_zoom, &crs);
            }
            _ => {}
        }
    };

    let on_zoom_in = move |_| {
        let crs = Epsg3857;
        let state = map_state.read();
        let new_zoom = (state.zoom() + 1.0).min(state.max_zoom());
        drop(state);
        map_state.write().set_zoom(new_zoom, &crs);
    };

    let on_zoom_out = move |_| {
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

            onresize: on_resize,
            onpointerdown: on_pointer_down,
            onpointermove: on_pointer_move,
            onpointerup: on_pointer_up,
            onpointerleave: move |_| dragging.set(false),
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
