//! Backdrop-aware layers carry replacement coverage independently of color alpha.
use super::*;
use std::cell::Cell;

const SCRATCH_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Default)]
pub(super) struct RasterBudget {
    live: Cell<u64>,
}
pub(super) struct RasterLease {
    budget: Rc<RasterBudget>,
    bytes: u64,
}
impl RasterBudget {
    pub(super) fn reserve(self: &Rc<Self>, bytes: u64) -> Result<RasterLease> {
        let next = self.live.get().saturating_add(bytes);
        if next > SCRATCH_BYTES {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Render working raster reservations exceed 512 MiB; reduce output scale, layer dimensions, or filter support",
            ));
        }
        self.live.set(next);
        Ok(RasterLease {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Drop for RasterLease {
    fn drop(&mut self) {
        self.budget.live.set(self.budget.live.get() - self.bytes);
    }
}

pub(super) fn background_nodes(snapshot: &LayoutSnapshot) -> Vec<bool> {
    let mut flags = vec![false; snapshot.items.len()];
    for index in (0..snapshot.items.len()).rev() {
        let item = &snapshot.items[index];
        flags[index] = item.record.enabled
            && (item.node.properties.get("effect").is_some_and(|effects| {
                values(effects).iter().any(|effect| {
                    (effect["type"] == "background_blur" && effect["enabled"] != false)
                        || blending::outer_needs_background(effect)
                })
            }) || item.children.iter().any(|child| flags[*child]));
    }
    flags
}

pub(super) fn composite_layer(
    canvas: &Canvas,
    viewport: Affine,
    layer: &NodeLayer,
    coverage_only: bool,
) {
    canvas.save();
    canvas.reset_matrix();
    canvas.concat(&matrix(viewport));
    let rect = SkRect::from_xywh(
        layer.bounds.x,
        layer.bounds.y,
        layer.bounds.width,
        layer.bounds.height,
    );
    let mut paint = Paint::default();
    paint.set_alpha_f(layer.opacity);
    if coverage_only {
        canvas.draw_image_rect(&layer.coverage, None, rect, &paint);
    } else {
        paint.set_blend_mode(BlendMode::DstOut);
        canvas.draw_image_rect(&layer.coverage, None, rect, &paint);
        paint.set_blend_mode(BlendMode::Plus);
        canvas.draw_image_rect(&layer.color, None, rect, &paint);
    }
    canvas.restore();
}

fn surface(width: i32, height: i32) -> Result<skia_safe::Surface> {
    raster_dimensions(width as u64, height as u64)?;
    let mut surface = skia_safe::surfaces::raster_n32_premul((width, height))
        .ok_or_else(|| invalid("Cannot allocate backdrop working raster"))?;
    surface.canvas().clear(Color::TRANSPARENT);
    Ok(surface)
}

fn filtered(image: &Image, filter: Option<ImageFilter>) -> Result<Image> {
    let mut surface = surface(image.width(), image.height())?;
    let mut paint = Paint::default();
    paint.set_image_filter(filter);
    surface.canvas().draw_image(image, (0.0, 0.0), Some(&paint));
    Ok(surface.image_snapshot())
}

fn max_coverage(coverage: &Image, color: &Image) -> Result<Image> {
    let a = image_filters::image(
        coverage.clone(),
        None,
        None,
        Some(skia_safe::SamplingOptions::default()),
    );
    let b = image_filters::image(
        color.clone(),
        None,
        None,
        Some(skia_safe::SamplingOptions::default()),
    );
    let difference = image_filters::arithmetic(0.0, 1.0, -1.0, 0.0, true, a.clone(), b, None);
    let maximum = image_filters::arithmetic(0.0, 1.0, 1.0, 0.0, true, a, difference, None);
    filtered(coverage, maximum)
}

impl Renderer<'_> {
    fn raster_pass(
        &mut self,
        index: usize,
        raster: &NodeRaster,
        background: Option<&Image>,
        pass: Option<OwnPass>,
    ) -> Result<Image> {
        let mut surface = surface(raster.width, raster.height)?;
        let viewport = raster.viewport;
        let mut painter = Renderer {
            project: self.project,
            snapshot: self.snapshot,
            canvas: surface.canvas(),
            viewport,
            images: std::mem::take(&mut self.images),
            runtime: self.runtime.as_deref_mut(),
            raster_scale: self.raster_scale,
            time: self.time,
            artifacts: Vec::new(),
            skip_effect: pass.map(|_| index),
            external_backdrop: background.map(|background| Backdrop {
                image: background.clone(),
                offset: -(self.backdrop_padding as f32),
            }),
            backdrop_padding: self.backdrop_padding,
            background_nodes: self.background_nodes,
            coverage: None,
            own_pass: pass.map(|pass| (index, pass)),
            text_layer_hints: false,
            rasters: self.rasters.clone(),
        };
        painter.node(index)?;
        self.images.append(&mut painter.images);
        self.artifacts.append(&mut painter.artifacts);
        Ok(surface.image_snapshot())
    }

    pub(super) fn backdrop_layer(&mut self, index: usize) -> Result<NodeLayer> {
        if let Some(runtime) = self.runtime.as_deref() {
            runtime.check()?;
        }
        let item = &self.snapshot.items[index];
        let record = item.record.clone();
        let node = item.node.clone();
        if record.visual_bounds.width <= 0.0 || record.visual_bounds.height <= 0.0 {
            let image = surface(1, 1)?.image_snapshot();
            return Ok(NodeLayer {
                color: image.clone(),
                coverage: image,
                bounds: record.visual_bounds,
                opacity: 1.0,
                _reservation: None,
            });
        }
        let raster = self.node_raster(record.visual_bounds)?;
        let bounds = raster.bounds;
        let (width, height) = (raster.width, raster.height);
        if !self.background_nodes[index] && !self.subtree_uses_background(index) {
            let reservation = self.rasters.reserve(width as u64 * height as u64 * 12)?;
            let color = self.raster_pass(index, &raster, None, None)?;
            return Ok(NodeLayer {
                coverage: color.clone(),
                color,
                bounds,
                opacity: 1.0,
                _reservation: Some(reservation),
            });
        }
        let padding = self.backdrop_padding;
        let (back_width, back_height) = raster_dimensions(
            width as u64 + 2 * padding as u64,
            height as u64 + 2 * padding as u64,
        )?;
        let reservation = self.rasters.reserve(
            width as u64 * height as u64 * 40 + back_width as u64 * back_height as u64 * 12,
        )?;
        let viewport = raster.viewport;
        let mut mapping =
            matrix(Affine::translation(padding as f32, padding as f32).then(viewport));
        mapping.pre_concat(
            &matrix(self.viewport)
                .invert()
                .ok_or_else(|| invalid("Cannot invert backdrop viewport"))?,
        );
        let background =
            self.capture_background(back_width as u32, back_height as u32, &mapping)?;
        if !self.background_nodes[index] {
            let color = self.raster_pass(index, &raster, Some(&background), None)?;
            return Ok(NodeLayer {
                coverage: color.clone(),
                color,
                bounds,
                opacity: 1.0,
                _reservation: Some(reservation),
            });
        }
        let own = self.raster_pass(index, &raster, Some(&background), Some(OwnPass::Fill))?;
        let mut color = surface(width, height)?;
        let mut coverage = surface(width, height)?;
        color.canvas().draw_image(&own, (0.0, 0.0), None);
        coverage.canvas().draw_image(&own, (0.0, 0.0), None);
        let children = self.snapshot.items[index].children.clone();
        for child in children {
            if !self.snapshot.items[child].record.enabled {
                continue;
            }
            let mut source = Renderer {
                project: self.project,
                snapshot: self.snapshot,
                canvas: color.canvas(),
                viewport,
                images: std::mem::take(&mut self.images),
                runtime: self.runtime.as_deref_mut(),
                raster_scale: self.raster_scale,
                time: self.time,
                artifacts: Vec::new(),
                skip_effect: None,
                external_backdrop: Some(Backdrop {
                    image: background.clone(),
                    offset: -(padding as f32),
                }),
                backdrop_padding: padding,
                background_nodes: self.background_nodes,
                coverage: Some(coverage.image_snapshot()),
                own_pass: None,
                text_layer_hints: false,
                rasters: self.rasters.clone(),
            };
            let layer = source.backdrop_layer(child)?;
            self.images.append(&mut source.images);
            self.artifacts.append(&mut source.artifacts);
            // Release the parent's coverage snapshot before painting into it.
            // Keeping it in the reader forces a full surface copy per sibling.
            drop(source);
            color.canvas().save();
            coverage.canvas().save();
            if node.properties.get("clip") == Some(&Value::Bool(true)) {
                let clip = shape(
                    &node,
                    SkRect::from_wh(record.rect.width, record.rect.height),
                )?
                .with_transform(&matrix(viewport.then(record.transform)));
                color.canvas().clip_path(&clip, ClipOp::Intersect, true);
                coverage.canvas().clip_path(&clip, ClipOp::Intersect, true);
            }
            composite_layer(color.canvas(), viewport, &layer, false);
            composite_layer(coverage.canvas(), viewport, &layer, true);
            color.canvas().restore();
            coverage.canvas().restore();
        }
        if node.properties.contains_key("stroke") {
            let mut source = Renderer {
                project: self.project,
                snapshot: self.snapshot,
                canvas: color.canvas(),
                viewport,
                images: std::mem::take(&mut self.images),
                runtime: self.runtime.as_deref_mut(),
                raster_scale: self.raster_scale,
                time: self.time,
                artifacts: Vec::new(),
                skip_effect: None,
                external_backdrop: Some(Backdrop {
                    image: background.clone(),
                    offset: -(padding as f32),
                }),
                backdrop_padding: padding,
                background_nodes: self.background_nodes,
                coverage: Some(coverage.image_snapshot()),
                own_pass: None,
                text_layer_hints: false,
                rasters: self.rasters.clone(),
            };
            let mapping = Matrix::translate((padding as f32, padding as f32));
            let stroke_background =
                source.capture_background(back_width as u32, back_height as u32, &mapping)?;
            let stroke = source.raster_pass(
                index,
                &raster,
                Some(&stroke_background),
                Some(OwnPass::Stroke),
            )?;
            self.images.append(&mut source.images);
            self.artifacts.append(&mut source.artifacts);
            drop(source);
            let layer = NodeLayer {
                coverage: stroke.clone(),
                color: stroke,
                bounds,
                opacity: 1.0,
                _reservation: None,
            };
            composite_layer(color.canvas(), viewport, &layer, false);
            composite_layer(coverage.canvas(), viewport, &layer, true);
        }
        let mut color = color.image_snapshot();
        let mut coverage = coverage.image_snapshot();
        if let Some(effects) = node.properties.get("effect") {
            for effect in values(effects)
                .into_iter()
                .filter(|effect| effect["enabled"] != false)
            {
                if let Some(runtime) = self.runtime.as_deref() {
                    runtime.check()?;
                }
                if effect["type"] == "background_blur" {
                    let radius = number(effect.get("radius"), 0.0)?;
                    if radius < 0.0 {
                        return Err(invalid("Background blur radius cannot be negative"));
                    }
                    let blurred = filtered(
                        &background,
                        image_filters::blur(
                            (
                                radius * self.raster_scale / 2.0,
                                radius * self.raster_scale / 2.0,
                            ),
                            TileMode::Decal,
                            None,
                            None,
                        ),
                    )?;
                    let mask = self.backdrop_mask(index, viewport, &coverage)?;
                    let mut back = surface(width, height)?;
                    back.canvas().draw_image(
                        &blurred,
                        (-(padding as f32), -(padding as f32)),
                        None,
                    );
                    let mut paint = Paint::default();
                    paint.set_blend_mode(BlendMode::DstIn);
                    back.canvas().draw_image(&mask, (0.0, 0.0), Some(&paint));
                    paint.set_blend_mode(BlendMode::DstOut);
                    back.canvas()
                        .draw_image(&coverage, (0.0, 0.0), Some(&paint));
                    paint.set_blend_mode(BlendMode::Plus);
                    back.canvas().draw_image(&color, (0.0, 0.0), Some(&paint));
                    color = back.image_snapshot();
                    let mut merged = surface(width, height)?;
                    merged.canvas().draw_image(&mask, (0.0, 0.0), None);
                    merged.canvas().draw_image(&coverage, (0.0, 0.0), None);
                    coverage = merged.image_snapshot();
                } else if effect["type"] == "shader" {
                    let object = encode_image(&color)?;
                    let background_png = uses_background(effect)
                        .then(|| {
                            crop_image(&background, padding, width as u32, height as u32)
                                .and_then(|image| encode_image(&image))
                        })
                        .transpose()?;
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
                            width as u32,
                            height as u32,
                            None,
                            self.time,
                            Some(&object),
                            background_png.as_deref(),
                        )?;
                    color = Image::from_encoded(Data::new_copy(&raster.png))
                        .ok_or_else(|| invalid("Cannot decode backdrop shader result"))?;
                    coverage = max_coverage(&coverage, &color)?;
                    self.artifacts
                        .push(json!({"node_id":node.id(),"acquisition":raster.metadata}));
                } else {
                    let mut scaled = effect.clone();
                    for key in ["radius", "blur"] {
                        if let Some(value) = scaled.get_mut(key) {
                            *value = json!(number(Some(value), 0.0)? * self.raster_scale);
                        }
                    }
                    if let Some(offset) = scaled.get("offset") {
                        let x = number(offset.get("x"), 0.0)?;
                        let y = number(offset.get("y"), 0.0)?;
                        let [a, b, c, d, _, _] = record.transform.0;
                        scaled["offset"] = json!({"x":(a*x+c*y)*self.raster_scale,"y":(b*x+d*y)*self.raster_scale});
                    }
                    if effect["type"] == "shadow" {
                        blending::shadow_type(&scaled)
                            .map_err(|error| blending::at_node(error, &node, "effect"))?;
                    }
                    if effect["type"] == "shadow" && effect["shadowType"] != "inner" {
                        let mode = blending::name(&scaled)
                            .map_err(|error| blending::at_node(error, &node, "effect"))?;
                        let blender = blending::shadow_source(mode)
                            .map_err(|error| blending::at_node(error, &node, "effect"))?;
                        let offset = scaled.get("offset");
                        let color_value = color::parse(
                            scaled
                                .get("color")
                                .and_then(Value::as_str)
                                .unwrap_or("#00000040"),
                        )?;
                        let blur = number(scaled.get("blur"), 0.0)?;
                        if blur < 0.0 {
                            return Err(invalid("Shadow blur cannot be negative"));
                        }
                        let shadow = image_filters::drop_shadow_only(
                            (
                                number(offset.and_then(|value| value.get("x")), 0.0)?,
                                number(offset.and_then(|value| value.get("y")), 0.0)?,
                            ),
                            (blur / 2.0, blur / 2.0),
                            color_value,
                            None,
                            None,
                            None,
                        );
                        let shadow = filtered(&color, shadow)?;
                        let shadow_color = if mode == "normal" {
                            shadow.clone()
                        } else {
                            let backdrop =
                                crop_image(&background, padding, width as u32, height as u32)?;
                            let backdrop = image_filters::image(backdrop, None, None, None);
                            let source = image_filters::image(shadow.clone(), None, None, None);
                            filtered(
                                &shadow,
                                image_filters::blend(blender, backdrop, source, None),
                            )?
                        };
                        let mut output = surface(width, height)?;
                        output.canvas().draw_image(&shadow_color, (0.0, 0.0), None);
                        output.canvas().draw_image(&color, (0.0, 0.0), None);
                        color = output.image_snapshot();
                        let mut merged = surface(width, height)?;
                        merged.canvas().draw_image(&shadow, (0.0, 0.0), None);
                        merged.canvas().draw_image(&coverage, (0.0, 0.0), None);
                        coverage = merged.image_snapshot();
                    } else {
                        let mut temporary = node.clone();
                        temporary.properties.insert("effect".into(), scaled);
                        let filter = effects_filter(&temporary)?;
                        if effect["type"] != "shadow" {
                            coverage = filtered(&coverage, filter.clone())?;
                        }
                        color = filtered(&color, filter)?;
                    }
                }
            }
        }
        Ok(NodeLayer {
            color,
            coverage,
            bounds,
            opacity: unit(node.properties.get("opacity"), 1.0)?,
            _reservation: Some(reservation),
        })
    }

    fn backdrop_mask(&self, index: usize, viewport: Affine, coverage: &Image) -> Result<Image> {
        let item = &self.snapshot.items[index];
        let node = &item.node;
        if matches!(node.kind(), "group" | "script") {
            return Ok(coverage.clone());
        }
        let mut mask = surface(coverage.width(), coverage.height())?;
        let canvas = mask.canvas();
        canvas.concat(&matrix(viewport.then(item.record.transform)));
        if matches!(node.kind(), "text" | "note" | "prompt" | "context") {
            let block = self
                .snapshot
                .text_blocks
                .get(&index)
                .ok_or_else(|| invalid("Missing backdrop text mask"))?;
            let offset = match node
                .properties
                .get("textAlignVertical")
                .and_then(Value::as_str)
                .unwrap_or("top")
            {
                "top" => 0.0,
                "middle" => (item.record.rect.height - block.metrics.height) / 2.0,
                "bottom" => item.record.rect.height - block.metrics.height,
                _ => return Err(invalid("Unknown text vertical alignment")),
            };
            block.paragraph.paint(canvas, (0.0, offset));
        } else {
            let path = shape(
                node,
                SkRect::from_wh(item.record.rect.width, item.record.rect.height),
            )?;
            let mut white = Paint::default();
            white.set_color(Color::WHITE);
            white.set_anti_alias(true);
            canvas.draw_path(&path, &white);
        }
        Ok(mask.image_snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_reservations_are_released_when_a_nested_job_fails() {
        let budget = Rc::new(RasterBudget::default());
        let run = || -> Result<()> {
            let _parent = budget.reserve(SCRATCH_BYTES - 1024)?;
            let _child = budget.reserve(2048)?;
            Ok(())
        };
        assert_eq!(run().unwrap_err().code, "INVALID_ARGUMENT");
        assert_eq!(budget.live.get(), 0);
        let lease = budget.reserve(SCRATCH_BYTES).unwrap();
        assert!(budget.reserve(1).is_err());
        drop(lease);
        assert!(budget.reserve(1).is_ok());
    }
}
