use anyrender::{ImageRenderer, PaintScene};
use anyrender_vello::VelloImageRenderer;
use dioxus::native::{use_wgpu, CustomPaintCtx, CustomPaintSource, DeviceHandle, TextureHandle};
use dioxus::prelude::*;
use image::ImageReader;
use kurbo::{Affine, BezPath, Circle, Point as KurboPoint, Stroke, Vec2};
use peniko::{Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use wgpu::{Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages};

use super::{
    platform_dpr, prepare_layer_state, snap_translation, use_pending_tile_requests, PaintTile,
    PaintTileLayer, PreparedLayerState, FALLBACK_REUSE_READY_RATIO, MARKER_HEAD_CENTER_OFFSET_Y,
    MARKER_HEAD_RADIUS, MARKER_TAIL_HALF_WIDTH, MARKER_TAIL_TOP_OFFSET_Y,
    PRIMARY_REPLACE_READY_RATIO,
};
use crate::components::map::MapContext;

#[derive(Clone, Default)]
struct NativeCanvasModel {
    inner: Arc<Mutex<NativeCanvasSnapshot>>,
}

#[derive(Default)]
struct NativeCanvasSnapshot {
    revision: u64,
    render_scene: Option<super::MapPaintScene>,
}

impl NativeCanvasModel {
    fn set_scene(&self, render_scene: &super::MapPaintScene) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.render_scene.as_ref() != Some(render_scene) {
            inner.revision = inner.revision.wrapping_add(1);
            inner.render_scene = Some(render_scene.clone());
        }
    }

    fn snapshot(&self) -> Option<(u64, super::MapPaintScene)> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some((inner.revision, inner.render_scene.clone()?))
    }
}

struct LeafletNativePaintSource {
    model: NativeCanvasModel,
    device_handle: Option<DeviceHandle>,
    renderer: Option<VelloImageRenderer>,
    decoded_images: HashMap<String, Option<ImageData>>,
    cpu_buffer: Vec<u8>,
    texture: Option<wgpu::Texture>,
    texture_handle: Option<TextureHandle>,
    texture_size: (u32, u32),
    last_revision: u64,
    last_scale_bits: u64,
}

impl LeafletNativePaintSource {
    fn new(model: NativeCanvasModel) -> Self {
        Self {
            model,
            device_handle: None,
            renderer: None,
            decoded_images: HashMap::new(),
            cpu_buffer: Vec::new(),
            texture: None,
            texture_handle: None,
            texture_size: (0, 0),
            last_revision: 0,
            last_scale_bits: 0,
        }
    }

    fn ensure_renderer(&mut self, width: u32, height: u32) -> &mut VelloImageRenderer {
        self.renderer
            .get_or_insert_with(|| VelloImageRenderer::new(width.max(1), height.max(1)))
    }

    fn upload_texture(
        &mut self,
        ctx: &mut CustomPaintCtx<'_>,
        width: u32,
        height: u32,
    ) -> Option<()> {
        let device_handle = self.device_handle.as_ref()?;
        if self.texture.is_none() {
            let texture = device_handle.device.create_texture(&TextureDescriptor {
                label: Some("dioxus-leaflet-canvas"),
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                // Vello copies registered textures into its image atlas each frame, so the
                // texture must be both uploadable and copyable as a source.
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_DST
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let texture_handle = ctx.register_texture(texture.clone());
            self.texture = Some(texture);
            self.texture_handle = Some(texture_handle);
        }

        let texture = self.texture.as_ref()?;
        device_handle.queue.write_texture(
            texture.as_image_copy(),
            &self.cpu_buffer,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Some(())
    }
}

impl CustomPaintSource for LeafletNativePaintSource {
    fn resume(&mut self, device_handle: &DeviceHandle) {
        self.device_handle = Some(device_handle.clone());
        self.texture = None;
        self.texture_handle = None;
        self.texture_size = (0, 0);
        self.last_revision = 0;
        self.last_scale_bits = 0;
    }

    fn suspend(&mut self) {
        self.device_handle = None;
        self.texture = None;
        self.texture_handle = None;
        self.texture_size = (0, 0);
        self.last_revision = 0;
        self.last_scale_bits = 0;
    }

    fn render(
        &mut self,
        mut ctx: CustomPaintCtx<'_>,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Option<TextureHandle> {
        let (revision, render_scene) = self.model.snapshot()?;
        let width = width.max(1);
        let height = height.max(1);
        let scale_bits = scale.to_bits();
        let resized = self.texture_size != (width, height);

        if resized {
            if let Some(texture_handle) = self.texture_handle.take() {
                ctx.unregister_texture(texture_handle);
            }
            self.texture = None;
            self.texture_size = (width, height);
            let renderer = self.ensure_renderer(width, height);
            renderer.resize(width, height);
        } else {
            let _ = self.ensure_renderer(width, height);
        }

        let should_rasterize = resized
            || self.texture.is_none()
            || self.texture_handle.is_none()
            || revision != self.last_revision
            || scale_bits != self.last_scale_bits;

        if should_rasterize {
            retain_decoded_images(&mut self.decoded_images, &render_scene);

            let decoded_images = &mut self.decoded_images;
            let cpu_buffer = &mut self.cpu_buffer;
            let renderer = self.renderer.as_mut()?;
            // `render_to_vec` in anyrender_vello 0.6.1 only reserves capacity and leaves the Vec
            // length at 0, which causes WGPU's buffer copy to panic on native targets.
            let buffer_len = width as usize * height as usize * 4;
            cpu_buffer.resize(buffer_len, 0);
            renderer.render(
                |scene| paint_map_scene(scene, &render_scene, decoded_images, scale),
                cpu_buffer.as_mut_slice(),
            );
            self.upload_texture(&mut ctx, width, height)?;
            self.last_revision = revision;
            self.last_scale_bits = scale_bits;
        }

        self.texture_handle.clone()
    }
}

fn retain_decoded_images(
    decoded_images: &mut HashMap<String, Option<ImageData>>,
    render_scene: &super::MapPaintScene,
) {
    let mut keep = HashSet::new();
    for tile in &render_scene.primary.tiles {
        if tile.image.is_some() {
            keep.insert(tile.cache_key.clone());
        }
    }
    for layer in &render_scene.fallbacks {
        for tile in &layer.tiles {
            if tile.image.is_some() {
                keep.insert(tile.cache_key.clone());
            }
        }
    }
    decoded_images.retain(|cache_key, _| keep.contains(cache_key));
}

fn paint_map_scene(
    scene: &mut impl PaintScene,
    render_scene: &super::MapPaintScene,
    decoded_images: &mut HashMap<String, Option<ImageData>>,
    scale: f64,
) {
    scene.reset();

    let selected_fallback = best_fallback_layer(&render_scene.fallbacks);
    if let Some(layer) = selected_fallback {
        paint_tile_layer(scene, layer, decoded_images, scale);
    }

    let primary_total = render_scene.primary.tiles.len().max(1);
    let primary_ready = count_ready_tiles(&render_scene.primary);
    let primary_ready_ratio = primary_ready as f64 / primary_total as f64;

    let (fallback_ready, fallback_total) = selected_fallback
        .map(|layer| (count_ready_tiles(layer), layer.tiles.len().max(1)))
        .unwrap_or((0, 1));
    let fallback_ready_ratio = fallback_ready as f64 / fallback_total as f64;

    let should_defer_primary = selected_fallback.is_some()
        && fallback_ready_ratio >= FALLBACK_REUSE_READY_RATIO
        && primary_ready_ratio < PRIMARY_REPLACE_READY_RATIO;

    if !should_defer_primary {
        paint_tile_layer(scene, &render_scene.primary, decoded_images, scale);
    }

    for marker in &render_scene.markers {
        paint_marker(scene, marker, scale);
    }
}

fn best_fallback_layer(fallback_layers: &[PaintTileLayer]) -> Option<&PaintTileLayer> {
    fallback_layers
        .iter()
        .max_by_key(|layer| count_ready_tiles(layer))
}

fn count_ready_tiles(layer: &PaintTileLayer) -> usize {
    layer
        .tiles
        .iter()
        .filter(|tile| tile.image.is_some())
        .count()
}

fn paint_tile_layer(
    scene: &mut impl PaintScene,
    layer: &PaintTileLayer,
    decoded_images: &mut HashMap<String, Option<ImageData>>,
    scale: f64,
) {
    let layer_scale = layer.transform.scale;
    let tx = snap_translation(layer.transform.translate.x, scale);
    let ty = snap_translation(layer.transform.translate.y, scale);

    for tile in &layer.tiles {
        let Some(image) = image_for_tile(decoded_images, tile) else {
            continue;
        };

        let draw_x = (tile.origin.x * layer_scale + tx) * scale;
        let draw_y = (tile.origin.y * layer_scale + ty) * scale;
        let draw_w = tile.size.x * layer_scale * scale;
        let draw_h = tile.size.y * layer_scale * scale;
        if draw_w <= 0.0 || draw_h <= 0.0 {
            continue;
        }

        let brush = ImageBrush::new(image.clone()).with_quality(ImageQuality::Medium);
        let transform =
            Affine::scale_non_uniform(draw_w / image.width as f64, draw_h / image.height as f64)
                .then_translate(Vec2::new(draw_x, draw_y));
        scene.draw_image(brush.as_ref(), transform);
    }
}

fn image_for_tile(
    decoded_images: &mut HashMap<String, Option<ImageData>>,
    tile: &PaintTile,
) -> Option<ImageData> {
    if let Some(cached) = decoded_images.get(&tile.cache_key) {
        if cached.is_some() || tile.image.is_none() {
            return cached.clone();
        }
    }

    let decoded = tile.image.as_ref().and_then(decode_tile_image);
    decoded_images.insert(tile.cache_key.clone(), decoded.clone());
    decoded
}

fn decode_tile_image(tile: &leaflet_core::tile::TileImage) -> Option<ImageData> {
    let reader = ImageReader::new(Cursor::new(tile.bytes()))
        .with_guessed_format()
        .ok()?;
    let dynamic_image = reader.decode().ok()?;
    let rgba = dynamic_image.to_rgba8();
    let (width, height) = rgba.dimensions();

    Some(ImageData {
        data: rgba.into_raw().into(),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width,
        height,
    })
}

fn paint_marker(scene: &mut impl PaintScene, marker: &super::MarkerSprite, scale: f64) {
    let x = (marker.point.x * scale).round();
    let y = (marker.point.y * scale).round();
    let head_y = y - MARKER_HEAD_CENTER_OFFSET_Y * scale;
    let stroke = Stroke::new(scale.max(1.0));
    let fill_color = parse_color(&marker.color, Color::from_rgb8(0x21, 0x96, 0xF3));

    let mut tail = BezPath::new();
    tail.move_to(KurboPoint::new(x, y));
    tail.line_to(KurboPoint::new(
        x - MARKER_TAIL_HALF_WIDTH * scale,
        y - MARKER_TAIL_TOP_OFFSET_Y * scale,
    ));
    tail.line_to(KurboPoint::new(
        x + MARKER_TAIL_HALF_WIDTH * scale,
        y - MARKER_TAIL_TOP_OFFSET_Y * scale,
    ));
    tail.close_path();

    scene.fill(Fill::NonZero, Affine::IDENTITY, fill_color, None, &tail);
    scene.stroke(
        &stroke,
        Affine::IDENTITY,
        Color::from_rgb8(0x15, 0x65, 0xC0),
        None,
        &tail,
    );

    let head = Circle::new((x, head_y), MARKER_HEAD_RADIUS * scale);
    scene.fill(Fill::NonZero, Affine::IDENTITY, fill_color, None, &head);
    scene.stroke(
        &stroke,
        Affine::IDENTITY,
        Color::from_rgb8(0x15, 0x65, 0xC0),
        None,
        &head,
    );

    let inner = Circle::new((x, head_y), 4.5 * scale);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(0xFF, 0xFF, 0xFF),
        None,
        &inner,
    );
}

fn parse_color(input: &str, fallback: Color) -> Color {
    let Some(hex) = input.strip_prefix('#') else {
        return fallback;
    };

    let parse = |value: &str| u8::from_str_radix(value, 16).ok();
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let r = chars
                .next()
                .and_then(|value| value.to_digit(16))
                .map(|value| (value as u8) * 17);
            let g = chars
                .next()
                .and_then(|value| value.to_digit(16))
                .map(|value| (value as u8) * 17);
            let b = chars
                .next()
                .and_then(|value| value.to_digit(16))
                .map(|value| (value as u8) * 17);
            match (r, g, b) {
                (Some(r), Some(g), Some(b)) => Color::from_rgb8(r, g, b),
                _ => fallback,
            }
        }
        6 => match (parse(&hex[0..2]), parse(&hex[2..4]), parse(&hex[4..6])) {
            (Some(r), Some(g), Some(b)) => Color::from_rgb8(r, g, b),
            _ => fallback,
        },
        8 => match (
            parse(&hex[0..2]),
            parse(&hex[2..4]),
            parse(&hex[4..6]),
            parse(&hex[6..8]),
        ) {
            (Some(r), Some(g), Some(b), Some(a)) => Color::from_rgba8(r, g, b, a),
            _ => fallback,
        },
        _ => fallback,
    }
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

    let model = use_hook(NativeCanvasModel::default);
    let model_for_source = model.clone();
    let canvas_source_id =
        use_wgpu(move || LeafletNativePaintSource::new(model_for_source.clone()));
    let canvas_source_attr = canvas_source_id.to_string();
    model.set_scene(&render_scene);

    rsx! {
        canvas {
            class: "leaflet-tile-canvas leaflet-tile-canvas-native",
            "src": canvas_source_attr,
        }
    }
}
