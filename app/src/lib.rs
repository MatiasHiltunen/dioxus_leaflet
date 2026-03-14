pub mod components;

use components::LeafletMap;
use dioxus::prelude::*;
use leaflet_core::geo::LatLng;

const OSM_TILES: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

#[component]
pub fn App() -> Element {
    rsx! {
        document::Style { {include_str!("../assets/leaflet.css")} }
        LeafletMap {
            center: LatLng::new(51.505, -0.09),
            zoom: 13.0,
            width: "100vw".to_string(),
            height: "100vh".to_string(),
            tile_url: OSM_TILES.to_string(),
            attribution: "© OpenStreetMap contributors".to_string(),

            components::Marker {
                position: LatLng::new(51.505, -0.09),
                title: "London".to_string(),
            }
            components::Marker {
                position: LatLng::new(51.51, -0.08),
                title: "East London".to_string(),
            }
        }
    }
}

#[cfg(any(
    feature = "web",
    feature = "desktop",
    feature = "mobile",
    feature = "native"
))]
pub fn launch() {
    dioxus::launch(App);
}
