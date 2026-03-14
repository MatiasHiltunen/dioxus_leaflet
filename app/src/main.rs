#[cfg(any(
    feature = "web",
    feature = "desktop",
    feature = "mobile",
    feature = "native"
))]
use dioxus_leaflet::launch;

#[cfg(any(
    feature = "web",
    feature = "desktop",
    feature = "mobile",
    feature = "native"
))]
fn main() {
    launch();
}

#[cfg(all(
    feature = "custom_renderer",
    not(any(
        feature = "web",
        feature = "desktop",
        feature = "mobile",
        feature = "native"
    ))
))]
fn main() {
    panic!("Launch the exported `dioxus_leaflet::App` component from your custom renderer crate.");
}

#[cfg(not(any(
    feature = "web",
    feature = "desktop",
    feature = "mobile",
    feature = "native",
    feature = "custom_renderer"
)))]
fn main() {
    panic!("Enable a renderer feature or use `custom_renderer` with your own launcher.");
}
