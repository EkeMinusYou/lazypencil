use crate::{
    Error, Result, color,
    geometry::{Affine, Rect},
    layout::{LayoutSnapshot, stroke_widths},
    project::{Project, ResolvedNode},
};
use serde::Serialize;
use serde_json::{Value, json};
use skia_safe::{
    BlendMode, Canvas, ClipOp, Color, Data, Image, ImageFilter, Matrix, Paint, Path, PathBuilder,
    PathFillType, Point, RRect, Rect as SkRect, TileMode, Vector, canvas::SaveLayerRec, gradient,
    image_filters, paint,
};
use std::{collections::BTreeMap, path::PathBuf, rc::Rc};

const MAX_PIXELS: u64 = 32 * 1024 * 1024;

mod backdrop;
mod blending;
mod mesh;
mod polygon;
use backdrop::{background_nodes, composite_layer};
pub use blending::{MODES as BLEND_MODES, blend};

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub scale: f32,
    pub region: Option<Rect>,
    pub node: Option<String>,
    pub background: Color,
    pub time: f64,
    pub refresh: bool,
    /// Capture current browser DOM without refreshing layout or static shaders.
    pub refresh_pages: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            region: None,
            node: None,
            background: Color::TRANSPARENT,
            time: 0.0,
            refresh: false,
            refresh_pages: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageDependency {
    pub path: PathBuf,
    pub hash: String,
}

pub struct RenderedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub region: Rect,
    pub snapshot_id: String,
    pub assets: Vec<ImageDependency>,
    pub dynamic_artifacts: Vec<Value>,
}

pub fn png(
    project: &Project,
    snapshot: &LayoutSnapshot,
    options: &RenderOptions,
) -> Result<RenderedImage> {
    png_impl(project, snapshot, options, None, None)
}

pub fn png_with_runtime(
    project: &Project,
    snapshot: &LayoutSnapshot,
    options: &RenderOptions,
    runtime: &mut crate::dynamic::DynamicRuntime,
    cancel: Option<tokio_util::sync::CancellationToken>,
) -> Result<RenderedImage> {
    png_with_runtime_observed(project, snapshot, options, runtime, cancel, None)
}

pub(crate) fn png_with_runtime_observed(
    project: &Project,
    snapshot: &LayoutSnapshot,
    options: &RenderOptions,
    runtime: &mut crate::dynamic::DynamicRuntime,
    cancel: Option<tokio_util::sync::CancellationToken>,
    observer: Option<&mut dyn FnMut(&'static str)>,
) -> Result<RenderedImage> {
    runtime.configure(project.runtime_settings())?;
    runtime.begin_render(cancel, options.refresh);
    if options.refresh_pages {
        runtime.refresh_pages();
    }
    let result = png_impl(project, snapshot, options, Some(runtime), observer);
    runtime.render_result(result)
}

fn png_impl(
    project: &Project,
    snapshot: &LayoutSnapshot,
    options: &RenderOptions,
    mut runtime: Option<&mut crate::dynamic::DynamicRuntime>,
    mut observer: Option<&mut dyn FnMut(&'static str)>,
) -> Result<RenderedImage> {
    if let Some(runtime) = runtime.as_deref() {
        runtime.check()?;
    }
    if !options.time.is_finite() || options.time < 0.0 {
        return Err(invalid("Shader time must be a finite nonnegative number"));
    }
    if !options.scale.is_finite() || !(0.01..=16.0).contains(&options.scale) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Render scale must be within 0.01..16",
        ));
    }
    if snapshot.revision != project.root_document().revision()
        || snapshot.dependency_revision != project.dependency_revision()
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "The layout snapshot does not belong to this project revision",
        ));
    }
    let selected = options
        .node
        .as_ref()
        .map(|id| {
            snapshot
                .items
                .iter()
                .position(|i| &i.record.id == id)
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", format!("No resolved node {id:?}")))
        })
        .transpose()?;
    let region = options.region.unwrap_or_else(|| {
        selected
            .map(|i| snapshot.items[i].record.visual_bounds)
            .unwrap_or(snapshot.bounds)
    });
    if [region.x, region.y, region.width, region.height]
        .iter()
        .any(|n| !n.is_finite())
        || region.width < 0.0
        || region.height < 0.0
    {
        return Err(Error::new("INVALID_ARGUMENT", "Invalid render region"));
    }
    let width = (region.width * options.scale).ceil().max(1.0) as u64;
    let height = (region.height * options.scale).ceil().max(1.0) as u64;
    if width > 16384 || height > 16384 || width * height > MAX_PIXELS {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Render exceeds 16384 pixels per side or 32 megapixels",
        ));
    }
    if let Some(runtime) = runtime.as_deref_mut() {
        runtime.prepare_pages(project, snapshot)?;
    }
    let padding = backdrop_padding(snapshot, options.scale)?;
    let background_nodes = background_nodes(snapshot);
    let (working_width, working_height) =
        raster_dimensions(width + 2 * padding as u64, height + 2 * padding as u64)?;
    let rasters = Rc::new(backdrop::RasterBudget::default());
    if let Some(runtime) = runtime.as_deref() {
        runtime.check()?;
    }
    let _working = rasters.reserve(working_width as u64 * working_height as u64 * 12)?;
    let mut surface = skia_safe::surfaces::raster_n32_premul((working_width, working_height))
        .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot allocate a raster surface"))?;
    let canvas = surface.canvas();
    canvas.clear(options.background);
    let viewport = Affine::translation(padding as f32, padding as f32)
        .then(Affine::scale(options.scale, options.scale))
        .then(Affine::translation(-region.x, -region.y));
    let mut renderer = Renderer {
        project,
        snapshot,
        canvas,
        viewport,
        images: BTreeMap::new(),
        runtime: runtime.as_deref_mut(),
        raster_scale: options.scale,
        time: options.time,
        artifacts: Vec::new(),
        skip_effect: None,
        external_backdrop: None,
        backdrop_padding: padding,
        background_nodes: &background_nodes,
        coverage: None,
        own_pass: None,
        tight_text_layer_hints: true,
        rasters,
    };
    // Node selection determines the viewport; compositing keeps scene context.
    for root in &snapshot.roots {
        renderer.node(*root)?;
    }
    if let Some(runtime) = renderer.runtime.as_deref() {
        runtime.check()?;
    }
    project.verify_unchanged()?;
    let mut assets = Vec::new();
    for (path, (expected, _)) in renderer.images {
        assets.push(ImageDependency {
            path,
            hash: expected,
        });
    }
    let dynamic_artifacts = renderer.artifacts;
    let image = crop_image(
        &surface.image_snapshot(),
        padding,
        width as u32,
        height as u32,
    )?;
    // This interval includes setup, asset checks and cropping, and separates
    // final PNG encoding from the rest of the render request.
    if let Some(observer) = observer.as_mut() {
        observer("render_raster_ms");
    }
    let encoded = skia_safe::png_encoder::encode_image(None, &image, &Default::default())
        .ok_or_else(|| Error::new("RENDER_ERROR", "PNG encoding failed"))?;
    if let Some(observer) = observer.as_mut() {
        observer("render_png_encode_ms");
    }
    if let Some(runtime) = runtime.as_deref() {
        runtime.check()?;
    }
    Ok(RenderedImage {
        png: encoded.as_bytes().into(),
        width: width as u32,
        height: height as u32,
        region,
        snapshot_id: snapshot.id.clone(),
        assets,
        dynamic_artifacts,
    })
}

struct Renderer<'a> {
    project: &'a Project,
    snapshot: &'a LayoutSnapshot,
    canvas: &'a Canvas,
    viewport: Affine,
    images: BTreeMap<PathBuf, (String, Image)>,
    runtime: Option<&'a mut crate::dynamic::DynamicRuntime>,
    raster_scale: f32,
    time: f64,
    artifacts: Vec<Value>,
    skip_effect: Option<usize>,
    external_backdrop: Option<Backdrop>,
    backdrop_padding: u32,
    background_nodes: &'a [bool],
    coverage: Option<Image>,
    own_pass: Option<(usize, OwnPass)>,
    tight_text_layer_hints: bool,
    rasters: Rc<backdrop::RasterBudget>,
}

#[derive(Clone, Copy, PartialEq)]
enum OwnPass {
    Fill,
    Stroke,
}

struct NodeLayer {
    color: Image,
    coverage: Image,
    bounds: Rect,
    opacity: f32,
    _reservation: Option<backdrop::RasterLease>,
}

struct Backdrop {
    image: Image,
    offset: f32,
}

struct NodeRaster {
    bounds: Rect,
    viewport: Affine,
    width: i32,
    height: i32,
}

impl Renderer<'_> {
    fn node_raster(&self, bounds: Rect) -> Result<NodeRaster> {
        let device = bounds.transformed(self.viewport);
        if [device.x, device.y, device.right(), device.bottom()]
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(invalid("Cannot rasterize nonfinite layer bounds"));
        }
        let (left, top) = (device.x.floor(), device.y.floor());
        let (width, height) = raster_dimensions(
            (device.right().ceil() - left).max(1.0) as u64,
            (device.bottom().ceil() - top).max(1.0) as u64,
        )?;
        let inverse = self
            .viewport
            .inverse()
            .ok_or_else(|| invalid("Cannot invert the layer viewport"))?;
        let (x, y) = inverse.point(left, top);
        // Align intermediate surfaces to the destination pixel grid. Rounding only
        // their dimensions and then fitting back into fractional bounds stretches ink.
        Ok(NodeRaster {
            bounds: Rect::new(
                x,
                y,
                width as f32 / self.raster_scale,
                height as f32 / self.raster_scale,
            ),
            viewport: Affine::translation(-left, -top).then(self.viewport),
            width,
            height,
        })
    }

    fn visible(&self, index: usize) -> bool {
        let bounds = self.snapshot.items[index]
            .record
            .visual_bounds
            .transformed(self.viewport);
        let info = self.canvas.image_info();
        // Include one device pixel for antialiasing at a geometric boundary.
        bounds.right() >= -1.0
            && bounds.bottom() >= -1.0
            && bounds.x <= info.width() as f32 + 1.0
            && bounds.y <= info.height() as f32 + 1.0
    }

    fn node(&mut self, index: usize) -> Result<()> {
        if let Some(runtime) = self.runtime.as_deref() {
            runtime.check()?;
        }
        let item = &self.snapshot.items[index];
        if !item.record.enabled || !self.visible(index) {
            return Ok(());
        }
        let node = &item.node;
        let kind = node.kind();
        let skip_effect = self.skip_effect == Some(index);
        if !skip_effect && self.background_nodes[index] {
            let layer = self.backdrop_layer(index)?;
            composite_layer(self.canvas, self.viewport, &layer, false);
            return Ok(());
        }
        let own_pass = self
            .own_pass
            .filter(|(root, _)| *root == index)
            .map(|(_, pass)| pass);
        let separated_effect = node.properties.get("effect").is_some_and(|effects| {
            values(effects).iter().any(|effect| {
                matches!(effect["type"].as_str(), Some("shader" | "background_blur"))
                    && effect["enabled"] != false
            })
        });
        let needs_backdrop_layer = (node.properties.contains_key("effect")
            || number(node.properties.get("opacity"), 1.0)? < 1.0)
            && self.subtree_uses_background(index);
        if !skip_effect && (separated_effect || needs_backdrop_layer) {
            return self.process_effects(index);
        }
        if !matches!(
            kind,
            "frame"
                | "group"
                | "rectangle"
                | "ellipse"
                | "polygon"
                | "path"
                | "text"
                | "note"
                | "prompt"
                | "context"
                | "script"
                | "browser"
                | "icon"
        ) {
            return Err(unsupported(
                node,
                format!("Rendering {kind:?} is not implemented yet"),
            ));
        }
        if kind == "script"
            && !self
                .snapshot
                .script_executions
                .iter()
                .any(|execution| execution["node_id"] == node.id())
        {
            return Err(unsupported(
                node,
                "Script has not been evaluated; calculate through the script-aware document service",
            ));
        }
        self.canvas.save();
        self.canvas.reset_matrix();
        self.canvas
            .concat(&matrix(self.viewport.then(item.record.transform)));
        let opacity = if skip_effect {
            1.0
        } else {
            number(node.properties.get("opacity"), 1.0)?
        };
        if !(0.0..=1.0).contains(&opacity) {
            return Err(invalid("Opacity must be within 0..1"));
        }
        let filter = if skip_effect {
            None
        } else {
            effects_filter(node)?
        };
        let has_layer = !skip_effect && (opacity < 1.0 || node.properties.contains_key("effect"));
        let bounds = if has_layer {
            self.allocation_hint(
                item.record
                    .visual_bounds
                    .transformed(self.viewport)
                    .outset(1.0, 1.0),
                self.viewport.then(item.record.transform),
                true,
                filter.as_ref(),
            )
        } else {
            None
        };
        let mut layer = Paint::default();
        layer.set_alpha_f(opacity);
        layer.set_image_filter(filter);
        let tight_text_layer_hints = self.tight_text_layer_hints;
        self.tight_text_layer_hints &= !has_layer;
        if has_layer {
            let mut record = SaveLayerRec::default().paint(&layer);
            if let Some(bounds) = &bounds {
                record = record.bounds(bounds);
            }
            self.canvas.save_layer(&record);
        }
        let rect = SkRect::from_xywh(0.0, 0.0, item.record.rect.width, item.record.rect.height);
        let shape = shape(node, rect)?;
        if matches!(kind, "text" | "note" | "prompt" | "context") {
            self.text(
                index,
                rect,
                own_pass != Some(OwnPass::Stroke),
                own_pass != Some(OwnPass::Fill),
            )?;
        } else if kind == "browser" && own_pass != Some(OwnPass::Stroke) {
            if rect.width() > 0.0 && rect.height() > 0.0 {
                let raster = self
                    .runtime
                    .as_deref_mut()
                    .ok_or_else(|| {
                        unsupported(
                            node,
                            "Browser rendering requires a configured dynamic runtime",
                        )
                    })?
                    .page_in_project(
                        self.project,
                        node,
                        rect.width(),
                        rect.height(),
                        self.raster_scale,
                    )?;
                let image = Image::from_encoded(Data::new_copy(&raster.png))
                    .ok_or_else(|| invalid("Cannot decode browser PNG"))?;
                self.artifacts
                    .push(json!({"node_id":node.id(),"acquisition":raster.metadata}));
                self.canvas.save();
                self.canvas.clip_path(&shape, ClipOp::Intersect, true);
                self.canvas
                    .draw_image_rect(&image, None, rect, &Paint::default());
                self.canvas.restore();
            }
        } else if !matches!(kind, "group" | "script" | "browser")
            && own_pass != Some(OwnPass::Stroke)
        {
            self.fills(node, node.properties.get("fill"), &shape, rect, "fill")?;
        }
        self.canvas.save();
        if node.properties.get("clip") == Some(&Value::Bool(true)) {
            self.canvas.clip_path(&shape, ClipOp::Intersect, true);
        }
        if own_pass.is_none() {
            for child in &item.children {
                self.node(*child)?;
            }
        }
        self.canvas.restore();
        self.canvas.reset_matrix();
        self.canvas
            .concat(&matrix(self.viewport.then(item.record.transform)));
        if !matches!(kind, "text" | "note" | "prompt" | "context")
            && own_pass != Some(OwnPass::Fill)
        {
            self.stroke(node, &shape, rect)?;
        }
        if has_layer {
            self.canvas.restore();
        }
        self.tight_text_layer_hints = tight_text_layer_hints;
        self.canvas.restore();
        Ok(())
    }

    fn stroke(&mut self, node: &ResolvedNode, shape: &Path, rect: SkRect) -> Result<()> {
        if let Some(stroke) = node.properties.get("stroke") {
            let geometry = stroke_geometry(node, shape, self.canvas.local_to_device_as_3x3())?;
            if geometry.is_empty() {
                return Ok(());
            }
            self.fills(node, Some(stroke), &geometry, rect, "stroke")?;
        }
        Ok(())
    }

    fn text(
        &mut self,
        index: usize,
        rect: SkRect,
        fill_enabled: bool,
        stroke_enabled: bool,
    ) -> Result<()> {
        let item = &self.snapshot.items[index];
        let node = &item.node;
        let block = self
            .snapshot
            .text_blocks
            .get(&index)
            .ok_or_else(|| Error::new("RENDER_ERROR", "Missing measured paragraph"))?;
        let offset = match node
            .properties
            .get("textAlignVertical")
            .and_then(Value::as_str)
            .unwrap_or("top")
        {
            "top" => 0.0,
            "middle" => (rect.height() - block.metrics.height) / 2.0,
            "bottom" => rect.height() - block.metrics.height,
            _ => return Err(invalid("Unknown text vertical alignment")),
        };
        let default = Value::String("#000000".into());
        let fill = node.properties.get("fill").unwrap_or(&default);
        let bounds = SkRect::from_xywh(
            0.0,
            offset,
            rect.width().max(block.metrics.longest_line),
            block.metrics.height,
        );
        let ink = block.ink_bounds;
        let bounds = SkRect::new(
            bounds.left().min(ink.x),
            bounds.top().min(ink.y + offset),
            bounds.right().max(ink.right()),
            bounds.bottom().max(ink.bottom() + offset),
        );
        if fill_enabled && block.native_color {
            let project = self.project;
            for fill in values(fill) {
                project.check_cancelled()?;
                if let Some(runtime) = self.runtime.as_deref() {
                    runtime.check()?;
                }
                if fill["enabled"] == false {
                    continue;
                }
                let draw = |canvas: &Canvas, paint: &Paint| {
                    block.paint_native(canvas, offset, paint, &|| project.check_cancelled())
                };
                if fill["type"] == "image" {
                    self.image_paint(node, fill, rect, "fill", draw)?;
                } else if fill["type"] == "mesh_gradient" {
                    mesh::paint(self, node, fill, rect, "fill", draw)
                        .map_err(|error| blending::at_node(error, node, "fill"))?;
                } else if let Some(paint) = self.fill_paint(node, fill, rect, "fill")? {
                    draw(self.canvas, &paint)?;
                }
            }
        } else if fill_enabled {
            // Both passes only contribute inside the text's box/ink union.
            // Bound their allocation hints instead of copying a full viewport
            // for every label. Keep one device pixel for antialias coverage.
            let transform = self.viewport.then(item.record.transform);
            let layer_bounds = self.allocation_hint(
                Rect::new(bounds.left(), bounds.top(), bounds.width(), bounds.height())
                    .transformed(transform)
                    .outset(1.0, 1.0),
                transform,
                !self.tight_text_layer_hints,
                None,
            );
            let mut fill_layer = SaveLayerRec::default();
            if let Some(bounds) = &layer_bounds {
                fill_layer = fill_layer.bounds(bounds);
            }
            self.canvas.save_layer(&fill_layer);
            self.fills(node, Some(fill), &Path::rect(bounds, None), rect, "fill")?;
            let mut mask = Paint::default();
            mask.set_blend_mode(BlendMode::DstIn);
            let mut mask_layer = SaveLayerRec::default().paint(&mask);
            if let Some(bounds) = &layer_bounds {
                mask_layer = mask_layer.bounds(bounds);
            }
            self.canvas.save_layer(&mask_layer);
            block.paragraph.paint(self.canvas, (0.0, offset));
            self.canvas.restore();
            self.canvas.restore();
        }
        if stroke_enabled && let Some(outline) = &block.outline {
            self.stroke(node, &outline.with_offset((0.0, offset)), rect)?;
        }
        Ok(())
    }

    fn allocation_hint(
        &self,
        mut device: Rect,
        transform: Affine,
        preserve_origin: bool,
        filter: Option<&ImageFilter>,
    ) -> Option<SkRect> {
        let inverse = transform.inverse()?;
        if preserve_origin {
            // Keep the original device origin in filtered, transparent and
            // offscreen passes. Shrink only the opposite edges; the extra
            // pixel lets Skia clamp the origin to the unchanged device clip.
            let mut clip = self.canvas.device_clip_bounds()?;
            if let Some(filter) = filter {
                // A filtered layer can require input outside the visible clip.
                // Preserve that expanded input origin before shrinking its
                // opposite edges, matching the unbounded filter allocation.
                clip = filter.filter_bounds(
                    clip,
                    &matrix(transform),
                    skia_safe::image_filter::MapDirection::Reverse,
                    None,
                );
            }
            device = device.union(Rect::new(
                clip.left as f32 - 1.0,
                clip.top as f32 - 1.0,
                2.0,
                2.0,
            ));
        }
        let local = device.transformed(inverse);
        let hint = SkRect::from_xywh(local.x, local.y, local.width, local.height);
        [hint.left(), hint.top(), hint.right(), hint.bottom()]
            .into_iter()
            .all(f32::is_finite)
            .then_some(hint)
    }

    fn fills(
        &mut self,
        node: &ResolvedNode,
        fills: Option<&Value>,
        shape: &Path,
        rect: SkRect,
        property: &str,
    ) -> Result<()> {
        if let Some(fills) = fills {
            for fill in values(fills) {
                if let Some(runtime) = self.runtime.as_deref() {
                    runtime.check()?;
                }
                if fill["enabled"] == false {
                    continue;
                }
                if fill["type"] == "image" {
                    self.image_fill(node, fill, shape, rect, property)?;
                } else if fill["type"] == "mesh_gradient" {
                    mesh::draw(self, node, fill, shape, rect, property)
                        .map_err(|error| blending::at_node(error, node, property))?;
                } else if let Some(paint) = self.fill_paint(node, fill, rect, property)? {
                    self.canvas.draw_path(shape, &paint);
                }
            }
        }
        Ok(())
    }

    fn fill_paint(
        &mut self,
        node: &ResolvedNode,
        fill: &Value,
        rect: SkRect,
        property: &str,
    ) -> Result<Option<Paint>> {
        if fill["enabled"] == false {
            return Ok(None);
        }
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_blender(blending::paint(fill, node, property)?);
        match fill {
            Value::Object(_) if !fill.get("type").is_some_and(Value::is_string) => {
                return Err(invalid("A paint object requires a type. Updating fill or stroke replaces the entire paint: keep type, url, and mode for images")
                    .details(json!({"node_id":node.id(),"property":property,"example":{"type":"image","url":"<asset URI returned by put_asset>","mode":"contain"}})));
            }
            Value::String(s) => {
                paint.set_color(color::parse(s)?);
            }
            Value::Object(_) if fill["type"] == "color" => {
                paint.set_color(color::parse(
                    fill["color"]
                        .as_str()
                        .ok_or_else(|| invalid("Missing fill color"))?,
                )?);
            }
            Value::Object(_) if fill["type"] == "gradient" => {
                let colors = fill["colors"]
                    .as_array()
                    .ok_or_else(|| invalid("A gradient requires color stops"))?;
                if colors.len() < 2 || colors.len() > 1024 {
                    return Err(invalid("A gradient needs 2..1024 color stops"));
                }
                let stops = colors
                    .iter()
                    .map(|v| {
                        color::parse(
                            v["color"]
                                .as_str()
                                .ok_or_else(|| invalid("Missing gradient color"))?,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                let positions = colors
                    .iter()
                    .map(|v| number(v.get("position"), 0.0))
                    .collect::<Result<Vec<_>>>()?;
                if positions.iter().any(|v| !(0.0..=1.0).contains(v))
                    || positions.windows(2).any(|p| p[0] > p[1])
                {
                    return Err(invalid("Gradient stops must be ordered within 0..1"));
                }
                let cx = number(fill.get("center").and_then(|v| v.get("x")), 0.5)? * rect.width();
                let cy = number(fill.get("center").and_then(|v| v.get("y")), 0.5)? * rect.height();
                let width =
                    number(fill.get("size").and_then(|v| v.get("width")), 1.0)? * rect.width();
                let height =
                    number(fill.get("size").and_then(|v| v.get("height")), 1.0)? * rect.height();
                let rotation = number(fill.get("rotation"), 0.0)?;
                let stops = stops
                    .into_iter()
                    .map(skia_safe::Color4f::from)
                    .collect::<Vec<_>>();
                let gradient = gradient::Gradient::new(
                    gradient::Colors::new(&stops, Some(&positions), TileMode::Clamp, None),
                    gradient::Interpolation::default(),
                );
                let local = matrix(Affine::translation(cx, cy).then(Affine::rotation(rotation)));
                let shader = match fill
                    .get("gradientType")
                    .and_then(Value::as_str)
                    .unwrap_or("linear")
                {
                    "linear" => gradient::shaders::linear_gradient(
                        ((0.0, height / 2.0), (0.0, -height / 2.0)),
                        &gradient,
                        Some(&local),
                    ),
                    "radial" if width > 0.0 && height > 0.0 => {
                        let local = matrix(
                            Affine::translation(cx, cy)
                                .then(Affine::rotation(rotation))
                                .then(Affine::scale(width / 2.0, height / 2.0)),
                        );
                        gradient::shaders::radial_gradient(
                            ((0.0, 0.0), 1.0),
                            &gradient,
                            Some(&local),
                        )
                    }
                    "angular" if width > 0.0 && height > 0.0 => {
                        let local = matrix(
                            Affine::translation(cx, cy)
                                .then(Affine::rotation(rotation))
                                .then(Affine::scale(width / 2.0, height / 2.0)),
                        );
                        gradient::shaders::sweep_gradient(
                            (0.0, 0.0),
                            (0.0, 360.0),
                            &gradient,
                            Some(&local),
                        )
                    }
                    _ => {
                        return Err(invalid(
                            "Unknown gradient type or nonpositive gradient size",
                        ));
                    }
                }
                .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot construct gradient"))?;
                paint.set_shader(shader);
                paint.set_alpha_f(unit(fill.get("opacity"), 1.0)?);
            }
            Value::Object(_) if fill["type"] == "shader" => {
                let (width, height) = crate::dynamic::dimensions(
                    rect.width() as f64 * self.raster_scale as f64,
                    rect.height() as f64 * self.raster_scale as f64,
                )?;
                let background = if uses_background(fill) {
                    let transform = self
                        .snapshot
                        .items
                        .iter()
                        .find(|item| item.node.id() == node.id())
                        .expect("rendered node")
                        .record
                        .transform;
                    let inverse = matrix(self.viewport.then(transform))
                        .invert()
                        .ok_or_else(|| invalid("Cannot invert shader transform"))?;
                    let mut mapping =
                        Matrix::scale((width as f32 / rect.width(), height as f32 / rect.height()));
                    mapping.pre_concat(&inverse);
                    Some(encode_image(
                        &self.capture_background(width, height, &mapping)?,
                    )?)
                } else {
                    None
                };
                let raster = self
                    .runtime
                    .as_deref_mut()
                    .ok_or_else(|| {
                        unsupported(
                            node,
                            "Shader rendering requires a configured dynamic runtime",
                        )
                    })?
                    .shader_for_node(
                        self.project,
                        node,
                        fill,
                        property,
                        width,
                        height,
                        Some([rect.width() as f64, rect.height() as f64]),
                        self.time,
                        None,
                        background.as_deref(),
                    )?;
                let image = Image::from_encoded(Data::new_copy(&raster.png))
                    .ok_or_else(|| invalid("Cannot decode shader PNG"))?;
                let matrix = Matrix::scale((
                    rect.width() / image.width() as f32,
                    rect.height() / image.height() as f32,
                ));
                paint.set_shader(image.to_shader(
                    (TileMode::Clamp, TileMode::Clamp),
                    skia_safe::SamplingOptions::default(),
                    Some(&matrix),
                ));
                paint.set_alpha_f(unit(fill.get("opacity"), 1.0)?);
                self.artifacts
                    .push(json!({"node_id":node.id(),"acquisition":raster.metadata}));
            }
            _ => {
                return Err(unsupported(
                    node,
                    format!("Fill {:?} is not implemented yet", fill.get("type")),
                ));
            }
        }
        Ok(Some(paint))
    }

    fn subtree_uses_background(&self, index: usize) -> bool {
        let item = &self.snapshot.items[index];
        item.record.enabled
            && (["fill", "stroke", "effect"].iter().any(|property| {
                item.node
                    .properties
                    .get(*property)
                    .is_some_and(uses_background)
            }) || item
                .children
                .iter()
                .any(|child| self.subtree_uses_background(*child)))
    }

    fn capture_background(&self, width: u32, height: u32, mapping: &Matrix) -> Result<Image> {
        let mut bitmap = skia_safe::Bitmap::new();
        if !bitmap.try_alloc_pixels_info(&self.canvas.image_info(), None)
            || !self.canvas.read_pixels_to_bitmap(&mut bitmap, (0, 0))
        {
            return Err(invalid("Cannot capture the rendered background"));
        }
        let image = skia_safe::images::raster_from_bitmap(&bitmap)
            .ok_or_else(|| invalid("Cannot capture background pixels"))?;
        let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
            .ok_or_else(|| invalid("Cannot allocate shader background"))?;
        let canvas = surface.canvas();
        canvas.clear(Color::TRANSPARENT);
        canvas.concat(mapping);
        if let Some(backdrop) = &self.external_backdrop {
            canvas.draw_image(&backdrop.image, (backdrop.offset, backdrop.offset), None);
        }
        if let Some(coverage) = &self.coverage {
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::DstOut);
            canvas.draw_image(coverage, (0.0, 0.0), Some(&paint));
            paint.set_blend_mode(BlendMode::Plus);
            canvas.draw_image(&image, (0.0, 0.0), Some(&paint));
        } else {
            canvas.draw_image(&image, (0.0, 0.0), None);
        }
        Ok(surface.image_snapshot())
    }

    fn process_effects(&mut self, index: usize) -> Result<()> {
        let node = self.snapshot.items[index].node.clone();
        let record = self.snapshot.items[index].record.clone();
        if record.visual_bounds.width <= 0.0 || record.visual_bounds.height <= 0.0 {
            return Ok(());
        }
        let raster = self.node_raster(record.visual_bounds)?;
        let bounds = raster.bounds;
        let (width, height) = (raster.width as u32, raster.height as u32);
        let padding = self.backdrop_padding;
        let (back_width, back_height) = raster_dimensions(
            width as u64 + 2 * padding as u64,
            height as u64 + 2 * padding as u64,
        )?;
        let _working = self.rasters.reserve(
            width as u64 * height as u64 * 24 + back_width as u64 * back_height as u64 * 8,
        )?;
        let viewport = raster.viewport;
        let mut mapping = matrix(viewport);
        mapping.pre_concat(
            &matrix(self.viewport)
                .invert()
                .ok_or_else(|| invalid("Cannot invert viewport"))?,
        );
        let background = if self.subtree_uses_background(index) {
            let padded = Matrix::translate((padding as f32, padding as f32)) * mapping;
            Some(self.capture_background(back_width as u32, back_height as u32, &padded)?)
        } else {
            None
        };
        let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
            .ok_or_else(|| invalid("Cannot allocate effect raster"))?;
        let mut nested = Renderer {
            project: self.project,
            snapshot: self.snapshot,
            canvas: surface.canvas(),
            viewport,
            images: std::mem::take(&mut self.images),
            runtime: self.runtime.as_deref_mut(),
            raster_scale: self.raster_scale,
            time: self.time,
            artifacts: Vec::new(),
            skip_effect: Some(index),
            external_backdrop: background.as_ref().map(|image| Backdrop {
                image: image.clone(),
                offset: -(padding as f32),
            }),
            backdrop_padding: padding,
            background_nodes: self.background_nodes,
            coverage: None,
            own_pass: None,
            tight_text_layer_hints: false,
            rasters: self.rasters.clone(),
        };
        nested.node(index)?;
        self.artifacts.extend(nested.artifacts);
        self.images.append(&mut nested.images);
        let mut image = surface.image_snapshot();
        if let Some(effects) = node.properties.get("effect") {
            for effect in values(effects) {
                if let Some(runtime) = self.runtime.as_deref() {
                    runtime.check()?;
                }
                if effect["enabled"] == false {
                    continue;
                }
                if effect["type"] == "shader" {
                    let object = encode_image(&image)?;
                    let background_png = if uses_background(effect) {
                        background
                            .as_ref()
                            .map(|image| {
                                crop_image(image, padding, width, height)
                                    .and_then(|image| encode_image(&image))
                            })
                            .transpose()?
                    } else {
                        None
                    };
                    let raster = self
                        .runtime
                        .as_deref_mut()
                        .ok_or_else(|| {
                            unsupported(
                                &node,
                                "Shader effects require a configured dynamic runtime",
                            )
                        })?
                        .shader_for_node(
                            self.project,
                            &node,
                            effect,
                            "effect",
                            width,
                            height,
                            Some([bounds.width as f64, bounds.height as f64]),
                            self.time,
                            Some(&object),
                            background_png.as_deref(),
                        )?;
                    image = Image::from_encoded(Data::new_copy(&raster.png))
                        .ok_or_else(|| invalid("Cannot decode shader effect PNG"))?;
                    self.artifacts
                        .push(json!({"node_id":node.id(),"acquisition":raster.metadata}));
                } else {
                    let mut effect = effect.clone();
                    for key in ["radius", "blur"] {
                        if let Some(value) = effect.get_mut(key) {
                            *value = json!(number(Some(value), 0.0)? * self.raster_scale);
                        }
                    }
                    if let Some(offset) = effect.get("offset") {
                        let x = number(offset.get("x"), 0.0)?;
                        let y = number(offset.get("y"), 0.0)?;
                        let [a, b, c, d, _, _] = record.transform.0;
                        effect["offset"] = json!({"x":(a*x+c*y)*self.raster_scale,"y":(b*x+d*y)*self.raster_scale});
                    }
                    let mut temporary = node.clone();
                    temporary.properties.insert("effect".into(), effect);
                    let mut paint = Paint::default();
                    paint.set_image_filter(effects_filter(&temporary)?);
                    let mut filtered =
                        skia_safe::surfaces::raster_n32_premul((width as i32, height as i32))
                            .ok_or_else(|| invalid("Cannot allocate native effect raster"))?;
                    filtered.canvas().clear(Color::TRANSPARENT);
                    filtered
                        .canvas()
                        .draw_image(&image, (0.0, 0.0), Some(&paint));
                    image = filtered.image_snapshot();
                }
            }
        }
        let mut paint = Paint::default();
        paint.set_alpha_f(unit(node.properties.get("opacity"), 1.0)?);
        self.canvas.save();
        self.canvas.reset_matrix();
        self.canvas.concat(&matrix(self.viewport));
        self.canvas.draw_image_rect(
            &image,
            None,
            SkRect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height),
            &paint,
        );
        self.canvas.restore();
        Ok(())
    }

    fn image_fill(
        &mut self,
        node: &ResolvedNode,
        fill: &Value,
        shape: &Path,
        rect: SkRect,
        property: &str,
    ) -> Result<()> {
        self.image_paint(node, fill, rect, property, |canvas, paint| {
            canvas.draw_path(shape, paint);
            Ok(())
        })
    }

    fn image_paint(
        &mut self,
        node: &ResolvedNode,
        fill: &Value,
        rect: SkRect,
        property: &str,
        draw: impl FnOnce(&Canvas, &Paint) -> Result<()>,
    ) -> Result<()> {
        let url = fill["url"]
            .as_str()
            .ok_or_else(|| invalid("Image fill requires a local URL"))?;
        let path = self
            .project
            .resolve_file(node.property_source(property), url)?;
        if !self.images.contains_key(&path) {
            let asset = self.project.asset(&path)?;
            let image = Image::from_encoded(Data::new_copy(&asset.bytes))
                .ok_or_else(|| invalid("Cannot decode the image"))?;
            if image.width() <= 0
                || image.height() <= 0
                || image.width() as u64 * image.height() as u64 > MAX_PIXELS
            {
                return Err(invalid("Decoded image exceeds 32 megapixels"));
            }
            self.images
                .insert(path.clone(), (asset.revision.clone(), image));
        }
        let image = &self.images[&path].1;
        let placement = crate::image_geometry::placement(
            fill,
            image.width() as f32,
            image.height() as f32,
            Rect::new(rect.left(), rect.top(), rect.width(), rect.height()),
        )?;
        let mut paint = Paint::default();
        paint.set_alpha_f(unit(fill.get("opacity"), 1.0)?);
        paint.set_anti_alias(true);
        paint.set_blender(blending::paint(fill, node, property)?);
        self.canvas.save();
        let mode = crate::image_geometry::image_mode(fill)?;
        let text = matches!(node.kind(), "text" | "note" | "prompt" | "context");
        if property != "stroke" && !text || mode != "cover" {
            self.canvas.clip_rect(
                SkRect::from_xywh(
                    placement.viewport.x,
                    placement.viewport.y,
                    placement.viewport.width,
                    placement.viewport.height,
                ),
                ClipOp::Intersect,
                true,
            );
        }
        if mode != "cover" {
            let image_box = Path::rect(
                SkRect::from_wh(image.width() as f32, image.height() as f32),
                None,
            )
            .with_transform(&matrix(placement.transform));
            self.canvas.clip_path(&image_box, ClipOp::Intersect, true);
        }
        paint.set_shader(image.to_shader(
            (TileMode::Clamp, TileMode::Clamp),
            skia_safe::SamplingOptions::new(
                skia_safe::FilterMode::Linear,
                skia_safe::MipmapMode::None,
            ),
            Some(&matrix(placement.transform)),
        ));
        let result = draw(self.canvas, &paint);
        self.canvas.restore();
        result
    }
}

pub fn shape(node: &ResolvedNode, rect: SkRect) -> Result<Path> {
    Ok(match node.kind() {
        "icon" => crate::icons::shape(
            node.icon
                .as_ref()
                .ok_or_else(|| invalid("Icon was not resolved in the project snapshot"))?
                .as_ref()
                .map_err(Clone::clone)?,
            rect,
        )?,
        "ellipse" => {
            let inner = unit(node.properties.get("innerRadius"), 0.0)?;
            let start = number(node.properties.get("startAngle"), 0.0)?;
            let sweep = number(node.properties.get("sweepAngle"), 360.0)?;
            if sweep.abs() > 360.0 {
                return Err(invalid("Ellipse sweep must be within -360..360"));
            }
            let mut builder = PathBuilder::new();
            builder.set_fill_type(PathFillType::EvenOdd);
            if sweep.abs() >= 360.0 {
                builder.add_oval(rect, None, None);
                if inner > 0.0 {
                    builder.add_oval(
                        SkRect::from_xywh(
                            rect.width() * (1.0 - inner) / 2.0,
                            rect.height() * (1.0 - inner) / 2.0,
                            rect.width() * inner,
                            rect.height() * inner,
                        ),
                        None,
                        None,
                    );
                }
            } else {
                builder.arc_to(rect, -start, -sweep, true);
                if inner > 0.0 {
                    let inner_rect = SkRect::from_xywh(
                        rect.width() * (1.0 - inner) / 2.0,
                        rect.height() * (1.0 - inner) / 2.0,
                        rect.width() * inner,
                        rect.height() * inner,
                    );
                    builder.arc_to(inner_rect, -start - sweep, sweep, false);
                } else {
                    builder.line_to((rect.width() / 2.0, rect.height() / 2.0));
                }
                builder.close();
            }
            builder.detach()
        }
        "polygon" => {
            let count = number(node.properties.get("polygonCount"), 3.0)?;
            if !(3.0..=1024.0).contains(&count) || count.fract() != 0.0 {
                return Err(invalid("Polygon count must be an integer in 3..1024"));
            }
            let radius = number(node.properties.get("cornerRadius"), 0.0)?;
            if radius < 0.0 {
                return Err(invalid("Polygon corner radius must not be negative"));
            }
            let points = (0..count as usize)
                .map(|i| {
                    let angle =
                        -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / count;
                    Point::new(
                        rect.width() / 2.0 * (1.0 + angle.cos()),
                        rect.height() / 2.0 * (1.0 + angle.sin()),
                    )
                })
                .collect::<Vec<_>>();
            polygon::rounded_path(&points, radius)
        }
        "path" => {
            let geometry = node
                .properties
                .get("geometry")
                .and_then(Value::as_str)
                .unwrap_or("");
            let path =
                Path::from_svg(geometry).ok_or_else(|| invalid("Invalid SVG path geometry"))?;
            let bounds = if let Some(viewbox) = node.properties.get("viewBox") {
                let values = viewbox
                    .as_array()
                    .filter(|v| v.len() == 4)
                    .ok_or_else(|| invalid("A path viewBox requires four numbers"))?;
                SkRect::from_xywh(
                    number(values.first(), 0.0)?,
                    number(values.get(1), 0.0)?,
                    number(values.get(2), 0.0)?,
                    number(values.get(3), 0.0)?,
                )
            } else {
                path.compute_tight_bounds()
            };
            if bounds.width() < 0.0 || bounds.height() < 0.0 {
                return Err(invalid("Negative path viewBox dimensions"));
            }
            let x = if bounds.width() > 0.0 {
                rect.width() / bounds.width()
            } else {
                1.0
            };
            let y = if bounds.height() > 0.0 {
                rect.height() / bounds.height()
            } else {
                1.0
            };
            let transform =
                matrix(Affine::scale(x, y).then(Affine::translation(-bounds.left, -bounds.top)));
            let mut builder = PathBuilder::new_path(&path);
            builder.transform(&transform);
            builder.set_fill_type(
                match node
                    .properties
                    .get("fillRule")
                    .and_then(Value::as_str)
                    .unwrap_or("nonzero")
                {
                    "nonzero" => PathFillType::Winding,
                    "evenodd" => PathFillType::EvenOdd,
                    _ => return Err(invalid("Unknown path fill rule")),
                },
            );
            builder.detach()
        }
        _ => {
            let radii = match node.properties.get("cornerRadius") {
                None => [0.0; 4],
                Some(Value::Array(v)) if v.len() == 4 => [
                    number(v.first(), 0.0)?,
                    number(v.get(1), 0.0)?,
                    number(v.get(2), 0.0)?,
                    number(v.get(3), 0.0)?,
                ],
                Some(v) => [number(Some(v), 0.0)?; 4],
            };
            if radii.iter().any(|v| *v < 0.0) {
                return Err(invalid("Corner radii cannot be negative"));
            }
            let radii = radii.map(|n| Vector::new(n, n));
            let mut rrect = RRect::default();
            rrect.set_rect_radii(rect, &radii);
            Path::rrect(rrect, None)
        }
    })
}

pub(crate) fn stroke_geometry(
    node: &ResolvedNode,
    shape: &Path,
    transform: Matrix,
) -> Result<Path> {
    let widths = stroke_widths(node)?;
    if widths.iter().any(|width| *width != widths[0]) {
        return variable_border(node, shape, widths);
    }
    let alignment = node
        .properties
        .get("strokeAlignment")
        .and_then(Value::as_str)
        .unwrap_or("center");
    if !matches!(alignment, "inner" | "center" | "outer") {
        return Err(invalid("Unknown stroke alignment"));
    }
    if widths[0] == 0.0 {
        return Ok(Path::default());
    }
    let mut stroke_paint = Paint::default();
    stroke_paint.set_style(paint::Style::Stroke);
    stroke_paint.set_stroke_width(widths[0] * if alignment == "center" { 1.0 } else { 2.0 });
    stroke_paint.set_stroke_cap(
        match node
            .properties
            .get("strokeLinecap")
            .and_then(Value::as_str)
            .unwrap_or("butt")
        {
            "butt" => paint::Cap::Butt,
            "round" => paint::Cap::Round,
            "square" => paint::Cap::Square,
            _ => return Err(invalid("Unknown stroke cap")),
        },
    );
    stroke_paint.set_stroke_join(
        match node
            .properties
            .get("strokeLinejoin")
            .and_then(Value::as_str)
            .unwrap_or("miter")
        {
            "miter" => paint::Join::Miter,
            "round" => paint::Join::Round,
            "bevel" => paint::Join::Bevel,
            _ => return Err(invalid("Unknown stroke join")),
        },
    );

    let mut geometry = PathBuilder::new();
    if !skia_safe::path_utils::fill_path_with_paint(
        shape,
        &stroke_paint,
        &mut geometry,
        None,
        Some(transform),
    ) {
        return Err(invalid("Cannot construct stroke geometry"));
    }
    let path = geometry.detach();
    match alignment {
        "inner" => path.op(shape, skia_safe::PathOp::Intersect),
        "outer" => path.op(shape, skia_safe::PathOp::Difference),
        _ => Some(path),
    }
    .ok_or_else(|| invalid("Cannot construct aligned stroke geometry"))
}

fn variable_border(node: &ResolvedNode, shape: &Path, widths: [f32; 4]) -> Result<Path> {
    if !matches!(node.kind(), "rectangle" | "frame" | "browser") {
        return Err(unsupported(
            node,
            "Per-side strokes currently require a rectangular node",
        ));
    }
    let base = shape
        .is_rrect()
        .or_else(|| shape.is_rect().map(|(rect, _, _)| RRect::new_rect(rect)))
        .ok_or_else(|| invalid("A rectangular border requires a rounded rectangle"))?;
    let alignment = node
        .properties
        .get("strokeAlignment")
        .and_then(Value::as_str)
        .unwrap_or("center");
    let outside = match alignment {
        "inner" => 0.0,
        "center" => 0.5,
        "outer" => 1.0,
        _ => return Err(invalid("Unknown stroke alignment")),
    };
    let join = node
        .properties
        .get("strokeLinejoin")
        .and_then(Value::as_str)
        .unwrap_or("miter");
    if !matches!(join, "miter" | "round" | "bevel") {
        return Err(invalid("Unknown stroke join"));
    }
    let out = widths.map(|width| width * outside);
    let inset = widths.map(|width| width * (1.0 - outside));
    let rect = base.rect();
    let outer_rect = SkRect::new(
        rect.left() - out[3],
        rect.top() - out[0],
        rect.right() + out[1],
        rect.bottom() + out[2],
    );
    let inner_rect = SkRect::new(
        rect.left() + inset[3],
        rect.top() + inset[0],
        rect.right() - inset[1],
        rect.bottom() - inset[2],
    );
    let adjacent = [(3, 0), (1, 0), (1, 2), (3, 2)];
    let radii = base.radii_ref();
    let outer_radii = std::array::from_fn(|corner| {
        let (x, y) = adjacent[corner];
        let radius = radii[corner];
        if radius.x == 0.0 && radius.y == 0.0 && join != "round" {
            Vector::new(0.0, 0.0)
        } else {
            Vector::new(radius.x + out[x], radius.y + out[y])
        }
    });
    let inner_radii = std::array::from_fn(|corner| {
        let (x, y) = adjacent[corner];
        Vector::new(
            (radii[corner].x - inset[x]).max(0.0),
            (radii[corner].y - inset[y]).max(0.0),
        )
    });
    let mut outer = RRect::default();
    outer.set_rect_radii(outer_rect, &outer_radii);
    let mut path = Path::rrect(outer, None);
    if join == "bevel" {
        let corners = [
            [
                (rect.left(), outer_rect.top()),
                (outer_rect.left(), rect.top()),
            ],
            [
                (outer_rect.right(), rect.top()),
                (rect.right(), outer_rect.top()),
            ],
            [
                (rect.right(), outer_rect.bottom()),
                (outer_rect.right(), rect.bottom()),
            ],
            [
                (outer_rect.left(), rect.bottom()),
                (rect.left(), outer_rect.bottom()),
            ],
        ];
        let vertices = [
            (outer_rect.left(), outer_rect.top()),
            (outer_rect.right(), outer_rect.top()),
            (outer_rect.right(), outer_rect.bottom()),
            (outer_rect.left(), outer_rect.bottom()),
        ];
        let mut clip = PathBuilder::new();
        // Walk clockwise, with one or two points at each corner.
        for corner in 0..4 {
            let points = if radii[corner].x == 0.0 && radii[corner].y == 0.0 {
                [corners[corner][1], corners[corner][0]]
            } else {
                [vertices[corner]; 2]
            };
            for (position, point) in points.into_iter().enumerate() {
                if corner == 0 && position == 0 {
                    clip.move_to(point);
                } else {
                    clip.line_to(point);
                }
            }
        }
        clip.close();
        path = path
            .op(&clip.detach(), skia_safe::PathOp::Intersect)
            .ok_or_else(|| invalid("Cannot construct bevel border"))?;
    }
    if !inner_rect.is_empty() {
        let mut inner = RRect::default();
        inner.set_rect_radii(inner_rect, &inner_radii);
        path = path
            .op(&Path::rrect(inner, None), skia_safe::PathOp::Difference)
            .ok_or_else(|| invalid("Cannot construct border interior"))?;
    }
    Ok(path)
}

fn effects_filter(node: &ResolvedNode) -> Result<Option<ImageFilter>> {
    let mut filter = None;
    if let Some(effects) = node.properties.get("effect") {
        for effect in values(effects) {
            if effect["enabled"] == false {
                continue;
            }
            if effect["type"] == "shadow" {
                blending::shadow_type(effect)
                    .map_err(|error| blending::at_node(error, node, "effect"))?;
            }
            filter = match effect["type"].as_str() {
                Some("blur") => {
                    let radius = number(effect.get("radius"), 0.0)?;
                    if radius < 0.0 {
                        return Err(invalid("Blur radius cannot be negative"));
                    }
                    image_filters::blur((radius / 2.0, radius / 2.0), TileMode::Decal, filter, None)
                }
                Some("shadow")
                    if effect
                        .get("shadowType")
                        .and_then(Value::as_str)
                        .unwrap_or("outer")
                        == "outer" =>
                {
                    let blur = number(effect.get("blur"), 0.0)?;
                    if blur < 0.0 {
                        return Err(invalid("Shadow blur cannot be negative"));
                    }
                    let dx = number(effect.get("offset").and_then(|v| v.get("x")), 0.0)?;
                    let dy = number(effect.get("offset").and_then(|v| v.get("y")), 0.0)?;
                    let color = color::parse(
                        effect
                            .get("color")
                            .and_then(Value::as_str)
                            .unwrap_or("#00000040"),
                    )?;
                    if blending::name(effect)? != "normal" {
                        return Err(Error::new(
                            "RENDER_ERROR",
                            "A blended outer shadow requires scene-aware compositing",
                        ));
                    }
                    image_filters::drop_shadow(
                        (dx, dy),
                        (blur / 2.0, blur / 2.0),
                        color,
                        None,
                        filter,
                        None,
                    )
                }
                Some("shadow") if effect["shadowType"] == "inner" => {
                    let blur = number(effect.get("blur"), 0.0)?;
                    if blur < 0.0 {
                        return Err(invalid("Shadow blur cannot be negative"));
                    }
                    let blender = blending::name(effect)
                        .and_then(blending::inner_shadow)
                        .map_err(|error| blending::at_node(error, node, "effect"))?;
                    let offset = (
                        number(effect.get("offset").and_then(|value| value.get("x")), 0.0)?,
                        number(effect.get("offset").and_then(|value| value.get("y")), 0.0)?,
                    );
                    let color = color::parse(
                        effect
                            .get("color")
                            .and_then(Value::as_str)
                            .unwrap_or("#00000040"),
                    )?;
                    let shifted = image_filters::offset(offset, filter.clone(), None);
                    let blurred = image_filters::blur(
                        (blur / 2.0, blur / 2.0),
                        TileMode::Decal,
                        shifted,
                        None,
                    );
                    let inverse_alpha = skia_safe::color_filters::matrix_row_major(
                        &[
                            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                            0.0, 0.0, 0.0, 0.0, -1.0, 1.0,
                        ],
                        None,
                    );
                    let inverted = image_filters::color_filter(inverse_alpha, blurred, None);
                    let tint = skia_safe::color_filters::blend(color, BlendMode::SrcIn)
                        .ok_or_else(|| invalid("Cannot construct inner shadow color"))?;
                    let shadow = image_filters::color_filter(tint, inverted, None);
                    image_filters::blend(blender, filter, shadow, None)
                }
                _ => return Err(unsupported(node, "This effect is not implemented yet")),
            };
        }
    }
    Ok(filter)
}

fn uses_background(value: &Value) -> bool {
    if let Some(values) = value.as_array() {
        return values.iter().any(uses_background);
    }
    if value["enabled"] == false {
        return false;
    }
    blending::outer_needs_background(value)
        || value["type"] == "background_blur"
        || value["type"] == "shader"
            && value
                .get("uniforms")
                .and_then(Value::as_object)
                .is_some_and(|values| {
                    values.values().any(|value| {
                        value == "pencil:background"
                            || value.as_array().is_some_and(|values| {
                                values.iter().any(|value| value == "pencil:background")
                            })
                    })
                })
}

fn raster_dimensions(width: u64, height: u64) -> Result<(i32, i32)> {
    if width == 0 || height == 0 || width > 16384 || height > 16384 || width * height > MAX_PIXELS {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Render working raster exceeds 16384 pixels per side or 32 megapixels",
        ));
    }
    Ok((width as i32, height as i32))
}

fn crop_image(image: &Image, padding: u32, width: u32, height: u32) -> Result<Image> {
    if padding == 0 && image.width() == width as i32 && image.height() == height as i32 {
        return Ok(image.clone());
    }
    image
        .make_subset(
            None,
            skia_safe::IRect::from_xywh(
                padding as i32,
                padding as i32,
                width as i32,
                height as i32,
            ),
            skia_safe::image::RequiredProperties::default(),
        )
        .ok_or_else(|| invalid("Cannot crop the render working raster"))
}

fn backdrop_padding(snapshot: &LayoutSnapshot, scale: f32) -> Result<u32> {
    let mut needs_backdrop = false;
    let mut margin = 0.0_f64;
    for item in snapshot.items.iter().filter(|item| item.record.enabled) {
        if let Some(effects) = item.node.properties.get("effect") {
            for effect in values(effects)
                .into_iter()
                .filter(|effect| effect["enabled"] != false)
            {
                needs_backdrop |=
                    effect["type"] == "background_blur" || blending::outer_needs_background(effect);
                let radius = match effect["type"].as_str() {
                    Some("blur" | "background_blur") => number(effect.get("radius"), 0.0)?,
                    Some("shadow") => number(effect.get("blur"), 0.0)?,
                    _ => 0.0,
                };
                if radius < 0.0 {
                    return Err(invalid("Effect blur radius cannot be negative"));
                }
                let [a, b, c, d, _, _] = item.record.transform.0;
                let extent = (a.abs() + c.abs()).max(b.abs() + d.abs()) as f64;
                margin += 1.5 * radius as f64 * extent;
                if effect["type"] == "shadow" {
                    let x = number(effect.get("offset").and_then(|value| value.get("x")), 0.0)?;
                    let y = number(effect.get("offset").and_then(|value| value.get("y")), 0.0)?;
                    margin += ((a * x + c * y).abs().max((b * x + d * y).abs())) as f64;
                }
            }
        }
    }
    if !needs_backdrop || margin == 0.0 {
        return Ok(0);
    }
    let pixels = (margin * scale as f64).ceil() + 2.0;
    if !pixels.is_finite() || pixels > 8192.0 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Background filter support exceeds the working raster limit",
        ));
    }
    Ok(pixels as u32)
}

fn encode_image(image: &Image) -> Result<Vec<u8>> {
    skia_safe::png_encoder::encode_image(None, image, &Default::default())
        .map(|data| data.as_bytes().to_vec())
        .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot encode shader texture input"))
}

fn values(value: &Value) -> Vec<&Value> {
    value
        .as_array()
        .map(|v| v.iter().collect())
        .unwrap_or_else(|| vec![value])
}
fn matrix(affine: Affine) -> Matrix {
    Matrix::from_affine(&affine.0)
}
fn number(value: Option<&Value>, default: f32) -> Result<f32> {
    value
        .map(|v| {
            v.as_f64()
                .map(|n| n as f32)
                .filter(|n| n.is_finite())
                .ok_or_else(|| invalid("Expected a finite numeric property"))
        })
        .unwrap_or(Ok(default))
}
fn unit(value: Option<&Value>, default: f32) -> Result<f32> {
    let n = number(value, default)?;
    if !(0.0..=1.0).contains(&n) {
        return Err(invalid("Value must be within 0..1"));
    }
    Ok(n)
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_DOCUMENT", message)
}
fn unsupported(node: &ResolvedNode, message: impl Into<String>) -> Error {
    Error::new("UNSUPPORTED_FEATURE", message).details(json!({"node_id":node.id()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_phase_observation_preserves_png_bytes_and_rejects_invalid_input_before_reporting() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("scene.pen");
        std::fs::write(
            &path,
            json!({"version":"2.21","children":[{
                "id":"frame","type":"frame","layout":"none","width":80,"height":60,
                "clip":true,"fill":"#E2E8F0","children":[{
                    "id":"shape","type":"ellipse","x":12.5,"y":8.25,"width":42,"height":30,
                    "fill":"#2563EB80","opacity":0.7,
                    "effect":{"type":"shadow","offset":{"x":2,"y":3},"blur":4,"color":"#00000040"}
                }]
            }]})
            .to_string(),
        )
        .unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let project = Project::load(&store).unwrap();
        let snapshot = crate::layout::compute(
            project.resolve().unwrap(),
            &mut crate::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let options = RenderOptions::default();
        let original = png(&project, &snapshot, &options).unwrap();
        let mut phases = Vec::new();
        let observed = png_impl(
            &project,
            &snapshot,
            &options,
            None,
            Some(&mut |phase| phases.push(phase)),
        )
        .unwrap();
        assert_eq!(phases, ["render_raster_ms", "render_png_encode_ms"]);
        assert_eq!(original.png, observed.png);
        assert_eq!(
            (original.width, original.height),
            (observed.width, observed.height)
        );
        phases.clear();
        let invalid = RenderOptions {
            scale: f32::NAN,
            ..options
        };
        assert!(
            png_impl(
                &project,
                &snapshot,
                &invalid,
                None,
                Some(&mut |phase| phases.push(phase))
            )
            .is_err()
        );
        assert!(phases.is_empty());
    }
}
