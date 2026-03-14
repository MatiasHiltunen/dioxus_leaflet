use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;

use base64::Engine;

use crate::map::TileCoord;

/// A resolved tile request ready to fetch over HTTP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTileRequest {
    pub cache_key: String,
    pub url: String,
}

/// A portable interface for tile sources that can resolve a tile coordinate
/// into a concrete request.
pub trait TileSource {
    fn resolve_request(&self, coord: TileCoord) -> ResolvedTileRequest;
}

/// A configurable XYZ tile source with a Leaflet-like URL template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XyzTileSource {
    url_template: String,
    subdomains: Vec<String>,
    zoom_offset: i32,
    zoom_reverse: bool,
    max_zoom: Option<u8>,
    tms: bool,
    detect_retina: bool,
    retina_suffix: String,
    wrap_x: bool,
    custom_values: HashMap<String, String>,
}

impl XyzTileSource {
    pub fn new(url_template: impl Into<String>) -> Self {
        Self {
            url_template: url_template.into(),
            subdomains: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            zoom_offset: 0,
            zoom_reverse: false,
            max_zoom: None,
            tms: false,
            detect_retina: false,
            retina_suffix: "@2x".to_string(),
            wrap_x: true,
            custom_values: HashMap::new(),
        }
    }

    #[inline]
    pub fn url_template(&self) -> &str {
        &self.url_template
    }

    pub fn with_subdomains(
        mut self,
        subdomains: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.subdomains = subdomains.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_zoom_offset(mut self, zoom_offset: i32) -> Self {
        self.zoom_offset = zoom_offset;
        self
    }

    pub fn with_zoom_reverse(mut self, zoom_reverse: bool, max_zoom: Option<u8>) -> Self {
        self.zoom_reverse = zoom_reverse;
        self.max_zoom = max_zoom;
        self
    }

    pub fn with_tms(mut self, tms: bool) -> Self {
        self.tms = tms;
        self
    }

    pub fn with_retina(mut self, detect_retina: bool, retina_suffix: impl Into<String>) -> Self {
        self.detect_retina = detect_retina;
        self.retina_suffix = retina_suffix.into();
        self
    }

    pub fn with_wrap_x(mut self, wrap_x: bool) -> Self {
        self.wrap_x = wrap_x;
        self
    }

    pub fn with_custom_value(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_values.insert(key.into(), value.into());
        self
    }

    fn resolved_x(&self, x: i32, zoom: u8) -> i32 {
        if !self.wrap_x {
            return x;
        }
        let num_tiles = 1i32 << zoom as i32;
        ((x % num_tiles) + num_tiles) % num_tiles
    }

    fn resolved_y(&self, y: i32, zoom: u8) -> (i32, i32) {
        let max_y = (1i32 << zoom as i32) - 1;
        let inverted_y = max_y - y;
        let resolved_y = if self.tms { inverted_y } else { y };
        (resolved_y, inverted_y)
    }

    fn resolved_zoom(&self, zoom: u8) -> i32 {
        let base_zoom = if self.zoom_reverse {
            i32::from(self.max_zoom.unwrap_or(zoom)) - i32::from(zoom)
        } else {
            i32::from(zoom)
        };
        base_zoom + self.zoom_offset
    }

    fn resolved_subdomain(&self, x: i32, y: i32) -> &str {
        if self.subdomains.is_empty() {
            return "a";
        }
        let index = (x.unsigned_abs() + y.unsigned_abs()) as usize % self.subdomains.len();
        self.subdomains[index].as_str()
    }
}

impl TileSource for XyzTileSource {
    fn resolve_request(&self, coord: TileCoord) -> ResolvedTileRequest {
        let x = self.resolved_x(coord.x, coord.z);
        let (y, inverted_y) = self.resolved_y(coord.y, coord.z);
        let z = self.resolved_zoom(coord.z);
        let s = self.resolved_subdomain(x, y);
        let r = if self.detect_retina {
            self.retina_suffix.as_str()
        } else {
            ""
        };

        let mut url = self
            .url_template
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string())
            .replace("{-y}", &inverted_y.to_string())
            .replace("{z}", &z.to_string())
            .replace("{s}", s)
            .replace("{r}", r);

        for (key, value) in &self.custom_values {
            url = url.replace(&format!("{{{key}}}"), value);
        }

        ResolvedTileRequest {
            cache_key: url.clone(),
            url,
        }
    }
}

/// Tile image bytes plus metadata that renderers can convert into textures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileImage {
    mime_type: String,
    bytes: Arc<[u8]>,
    data_url: Arc<str>,
}

impl TileImage {
    pub fn new(mime_type: impl Into<String>, bytes: Vec<u8>) -> Self {
        let mime_type = mime_type.into();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let data_url = format!("data:{mime_type};base64,{encoded}");
        Self {
            mime_type,
            bytes: Arc::from(bytes),
            data_url: Arc::from(data_url),
        }
    }

    #[inline]
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    #[inline]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    #[inline]
    pub fn data_url(&self) -> &str {
        self.data_url.as_ref()
    }
}

/// A small in-memory cache for loaded tiles.
#[derive(Clone, Debug)]
pub struct MemoryTileCache {
    max_entries: usize,
    order: VecDeque<String>,
    entries: HashMap<String, TileImage>,
}

impl Default for MemoryTileCache {
    fn default() -> Self {
        Self::new(256)
    }
}

impl MemoryTileCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<&TileImage> {
        self.entries.get(key)
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    pub fn insert(&mut self, key: String, tile: TileImage) {
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key.clone());
        self.entries.insert(key.clone(), tile);

        while self.entries.len() > self.max_entries {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

/// View of a tile in the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TileEntryState {
    Missing,
    Loading,
    Ready(TileImage),
    Failed(String),
}

/// Tracks cached, in-flight, and failed tile fetches.
#[derive(Clone, Debug, Default)]
pub struct TileRepository {
    cache: MemoryTileCache,
    inflight: HashSet<String>,
    failures: HashMap<String, String>,
}

impl TileRepository {
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: MemoryTileCache::new(max_entries),
            inflight: HashSet::new(),
            failures: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.inflight.clear();
        self.failures.clear();
    }

    pub fn status(&self, key: &str) -> TileEntryState {
        if let Some(tile) = self.cache.get(key) {
            TileEntryState::Ready(tile.clone())
        } else if self.inflight.contains(key) {
            TileEntryState::Loading
        } else if let Some(error) = self.failures.get(key) {
            TileEntryState::Failed(error.clone())
        } else {
            TileEntryState::Missing
        }
    }

    pub fn mark_loading(&mut self, key: impl Into<String>) -> bool {
        let key = key.into();
        if self.cache.contains(&key)
            || self.inflight.contains(&key)
            || self.failures.contains_key(&key)
        {
            return false;
        }
        self.inflight.insert(key);
        true
    }

    pub fn mark_ready(&mut self, key: impl Into<String>, tile: TileImage) {
        let key = key.into();
        self.inflight.remove(&key);
        self.failures.remove(&key);
        self.cache.insert(key, tile);
    }

    pub fn mark_failed(&mut self, key: impl Into<String>, error: impl Into<String>) {
        let key = key.into();
        self.inflight.remove(&key);
        self.failures.insert(key, error.into());
    }
}

/// HTTP client for loading tiles in Rust on every supported target.
#[derive(Clone, Debug)]
pub struct HttpTileClient {
    client: reqwest::Client,
}

impl Default for HttpTileClient {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("dioxus-leaflet/0.1.0")
            .build()
            .unwrap_or_default();
        Self::new(client)
    }
}

impl HttpTileClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    pub async fn fetch_tile<T>(
        &self,
        source: &T,
        coord: TileCoord,
    ) -> Result<TileImage, TileFetchError>
    where
        T: TileSource,
    {
        let request = source.resolve_request(coord);
        self.fetch_resolved(&request).await
    }

    pub async fn fetch_resolved(
        &self,
        request: &ResolvedTileRequest,
    ) -> Result<TileImage, TileFetchError> {
        let response = self
            .client
            .get(&request.url)
            .send()
            .await
            .map_err(TileFetchError::Request)?;
        let response = response
            .error_for_status()
            .map_err(TileFetchError::Request)?;

        let mime_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .unwrap_or_else(|| guess_mime_type(&request.url).to_string());

        let bytes = response
            .bytes()
            .await
            .map_err(TileFetchError::Request)?
            .to_vec();

        Ok(TileImage::new(mime_type, bytes))
    }
}

#[derive(Debug)]
pub enum TileFetchError {
    Request(reqwest::Error),
}

impl fmt::Display for TileFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for TileFetchError {}

fn guess_mime_type(url: &str) -> &'static str {
    if url.ends_with(".png") {
        "image/png"
    } else if url.ends_with(".jpg") || url.ends_with(".jpeg") {
        "image/jpeg"
    } else if url.ends_with(".webp") {
        "image/webp"
    } else if url.ends_with(".avif") {
        "image/avif"
    } else if url.ends_with(".gif") {
        "image/gif"
    } else if url.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xyz_source_resolves_standard_tiles() {
        let source = XyzTileSource::new("https://{s}.tiles.test/{z}/{x}/{y}{r}.png")
            .with_subdomains(["a", "b", "c"])
            .with_retina(true, "@2x");

        let request = source.resolve_request(TileCoord::new(1, 2, 3));
        assert_eq!(request.url, "https://a.tiles.test/3/1/2@2x.png");
        assert_eq!(request.cache_key, request.url);
    }

    #[test]
    fn test_xyz_source_wraps_x_and_inverts_y_for_tms() {
        let source = XyzTileSource::new("https://tiles.test/{z}/{x}/{y}/{-y}.png").with_tms(true);

        let request = source.resolve_request(TileCoord::new(-1, 1, 2));
        assert_eq!(request.url, "https://tiles.test/2/3/2/2.png");
    }

    #[test]
    fn test_cache_evicts_oldest_tile() {
        let tile = TileImage::new("image/png", vec![1, 2, 3]);
        let mut cache = MemoryTileCache::new(2);
        cache.insert("a".to_string(), tile.clone());
        cache.insert("b".to_string(), tile.clone());
        cache.insert("c".to_string(), tile);

        assert_eq!(cache.len(), 2);
        assert!(!cache.contains("a"));
        assert!(cache.contains("b"));
        assert!(cache.contains("c"));
    }

    #[test]
    fn test_repository_tracks_loading_ready_and_failed_tiles() {
        let tile = TileImage::new("image/png", vec![1, 2, 3]);
        let mut repository = TileRepository::new(4);

        assert!(matches!(repository.status("tile"), TileEntryState::Missing));
        assert!(repository.mark_loading("tile"));
        assert!(matches!(repository.status("tile"), TileEntryState::Loading));

        repository.mark_ready("tile", tile.clone());
        assert_eq!(repository.status("tile"), TileEntryState::Ready(tile));

        repository.mark_failed("tile-2", "boom");
        assert_eq!(
            repository.status("tile-2"),
            TileEntryState::Failed("boom".to_string())
        );
        assert!(!repository.mark_loading("tile-2"));
    }

    #[test]
    fn test_tile_image_builds_data_url() {
        let tile = TileImage::new("image/png", vec![0x89, 0x50, 0x4e, 0x47]);
        assert!(tile.data_url().starts_with("data:image/png;base64,"));
    }
}
