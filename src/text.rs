use crate::{Error, Result};
use serde::Serialize;
use skia_safe::{
    Color, FontArguments, FontStyle, Path, PathBuilder,
    font_arguments::{VariationPosition, variation_position::Coordinate},
    font_style::{Slant, Weight, Width},
    textlayout::{
        Paragraph, ParagraphBuilder, ParagraphStyle, TextAlign, TextDecoration, TextStyle,
    },
};

pub const DEFAULT_FONT_FAMILY: &str = "Noto Sans JP";
pub const DEFAULT_FONT_BYTES: &[u8] = include_bytes!("../resources/fonts/NotoSansJP.ttf");

mod container;
mod decorations;
mod fonts;
pub use fonts::{FontAxis, FontInfo, Fonts};

impl Fonts {
    pub fn layout(&mut self, spec: &TextSpec, width: Option<f32>) -> Result<TextBlock> {
        self.layout_checked(spec, width, &|| Ok(()))
    }

    /// Cancellation is cooperative around native shaping and glyph batches.
    pub fn layout_with_cancellation(
        &mut self,
        spec: &TextSpec,
        width: Option<f32>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<TextBlock> {
        self.layout_checked(spec, width, &|| crate::cancel::check(cancel))
    }

    pub(crate) fn layout_checked(
        &mut self,
        spec: &TextSpec,
        width: Option<f32>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<TextBlock> {
        check()?;
        if !spec.size.is_finite() || !(0.0..=4096.0).contains(&spec.size) || spec.size == 0.0 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Font size must be positive and at most 4096",
            ));
        }
        if !(1..=1000).contains(&spec.weight)
            || !spec.letter_spacing.is_finite()
            || spec.content.len() > 1024 * 1024
            || spec.family.is_empty()
            || spec.family.len() > 256
            || spec.locale.len() > 128
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid font weight, spacing, text length, family name, or locale",
            ));
        }
        if spec.line_height.is_some_and(|h| !h.is_finite() || h <= 0.0) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Line height must be positive",
            ));
        }
        if width.is_some_and(|w| !w.is_finite() || !(0.0..=1_000_000.0).contains(&w)) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Text width must be finite and within 0..1000000",
            ));
        }
        let mut style = TextStyle::new();
        // Glyph paths are unhinted. Keep paragraph ink and design-space
        // outlines consistent instead of grid-fitting only the raster glyphs.
        style.set_font_hinting(skia_safe::FontHinting::None);
        style.set_font_size(spec.size);
        style.set_font_style(FontStyle::new(
            Weight::from(spec.weight),
            Width::NORMAL,
            if spec.italic {
                Slant::Italic
            } else {
                Slant::Upright
            },
        ));
        style.set_letter_spacing(spec.letter_spacing);
        style.set_color(spec.color);
        let mut decorations = TextDecoration::NO_DECORATION;
        if spec.underline {
            decorations |= TextDecoration::UNDERLINE;
        }
        if spec.strikethrough {
            decorations |= TextDecoration::LINE_THROUGH;
        }
        style.set_decoration_type(decorations);
        style.set_decoration_color(spec.color);
        style.set_locale(&spec.locale);
        if let Some(height) = spec.line_height {
            style.set_height(height);
            style.set_height_override(true);
        }
        let coordinates = [Coordinate {
            axis: Coordinate::wght,
            value: spec.weight as f32,
        }];
        let arguments = FontArguments::new().set_variation_design_position(VariationPosition {
            coordinates: &coordinates,
        });
        style.set_font_arguments(&arguments);
        let resolved = self.resolve(
            &spec.family,
            style.font_style(),
            style.font_arguments().expect("font arguments were set"),
        )?;
        check()?;
        style.set_font_families(&resolved.families);
        let mut paragraph_style = ParagraphStyle::new();
        paragraph_style.set_text_style(&style);
        paragraph_style.set_text_align(spec.align);
        let mut builder = ParagraphBuilder::new(&paragraph_style, self.collection.clone());
        builder.add_text(&spec.content);
        let mut paragraph = builder.build();
        check()?;
        paragraph.layout(width.unwrap_or(1_000_000.0));
        check()?;
        if width.is_none() {
            let natural = paragraph.max_intrinsic_width().ceil();
            paragraph.layout(natural.max(0.0));
            check()?;
        }
        let mut missing = paragraph.unresolved_codepoints();
        missing.sort_unstable();
        missing.dedup();
        if paragraph.unresolved_glyphs().is_some_and(|count| count > 0) {
            return Err(Error::new("ASSET_NOT_FOUND", "The selected fonts do not cover all shaped text")
                .details(serde_json::json!({"font_family":spec.family,"fallback_families":self.fallback_families(),"missing_characters":missing.iter().take(64).filter_map(|value| char::from_u32(*value as u32).map(|character|character.to_string())).collect::<Vec<_>>(),"missing_character_count":missing.len(),"missing_characters_truncated":missing.len()>64})));
        }
        let mut used = Vec::new();
        let mut used_ids = std::collections::BTreeSet::new();
        let mut native_color = false;
        let mut unknown_font = false;
        let horizontal = paragraph
            .get_line_metrics()
            .iter()
            .map(|line| (line.left as f32, line.width as f32))
            .collect::<Vec<_>>();
        let mut ink_bounds: Option<crate::geometry::Rect> = None;
        let mut outline = spec.outline_required.then(PathBuilder::new);
        let mut unsupported = false;
        let mut interrupted = None;
        // Visit the exact positioned glyph runs used by Paragraph::paint.
        paragraph.visit(|line, info| {
            if interrupted.is_some() {
                return;
            }
            if let Err(error) = check() {
                interrupted = Some(error);
                return;
            }
            if let Some(info) = info {
                let font = info.font();
                if info.count() > 0 {
                    let id = font.typeface().unique_id();
                    if used_ids.insert(id) {
                        native_color |=
                            [*b"COLR", *b"sbix", *b"CBDT", *b"SVG "].iter().any(|tag| {
                                font.typeface()
                                    .get_table_size(u32::from_be_bytes(*tag))
                                    .is_some_and(|size| size > 0)
                            });
                        if let Some(information) = resolved.information.get(&id) {
                            let mut information = information.clone();
                            information.synthetic_bold |= font.is_embolden();
                            information.synthetic_italic |= font.skew_x() != 0.0;
                            used.push(information);
                        } else {
                            unknown_font = true;
                        }
                    }
                }
                let origin = info.origin();
                let mut bounds = vec![skia_safe::Rect::default(); info.count()];
                font.get_bounds(info.glyphs(), &mut bounds, None);
                let mut include = |rect: crate::geometry::Rect| {
                    ink_bounds = Some(ink_bounds.map_or(rect, |current| current.union(rect)));
                };
                for (index, ((glyph, position), bounds)) in info
                    .glyphs()
                    .iter()
                    .zip(info.positions())
                    .zip(bounds)
                    .enumerate()
                {
                    // Token checks can lock; keep the cost bounded for dense
                    // text while checking every run and at most 64 glyphs.
                    if index.is_multiple_of(64)
                        && let Err(error) = check()
                    {
                        interrupted = Some(error);
                        return;
                    }
                    let position = (origin.x + position.x, origin.y + position.y);
                    if !bounds.is_empty() {
                        include(crate::geometry::Rect::new(
                            bounds.left() + position.0,
                            bounds.top() + position.1,
                            bounds.width(),
                            bounds.height(),
                        ));
                    }
                    if let Some(builder) = &mut outline {
                        if let Some(path) = font.get_path(*glyph) {
                            builder.add_path_with_offset(&path, position, None);
                        } else if !bounds.is_empty() {
                            unsupported = true;
                        }
                    }
                }
                if let Some(position) = info.positions().first() {
                    let (left, width) = horizontal[line];
                    let (_, metrics) = font.metrics();
                    let baseline = origin.y + position.y;
                    let underline = metrics
                        .underline_thickness()
                        .filter(|value| *value > 0.0)
                        .unwrap_or(spec.size / 14.0);
                    if spec.underline && width > 0.0 {
                        let y = baseline
                            + metrics
                                .underline_position()
                                .filter(|value| *value > 0.0)
                                .unwrap_or(underline);
                        include(crate::geometry::Rect::new(
                            left,
                            y - underline / 2.0,
                            width,
                            underline,
                        ));
                    }
                    if spec.strikethrough && width > 0.0 {
                        let thickness = if spec.underline {
                            underline
                        } else {
                            metrics
                                .strikeout_thickness()
                                .filter(|value| *value > 0.0)
                                .unwrap_or(underline)
                        };
                        let y = baseline
                            + metrics
                                .strikeout_position()
                                .unwrap_or(-metrics.x_height / 2.0);
                        include(crate::geometry::Rect::new(
                            left,
                            y - thickness / 2.0,
                            width,
                            thickness,
                        ));
                    }
                }
            }
        });
        if let Some(error) = interrupted {
            return Err(error);
        }
        check()?;
        if unknown_font {
            return Err(Error::new(
                "RENDER_ERROR",
                "A shaped font is outside the resolved font snapshot",
            ));
        }
        let native_runs = if native_color {
            NativeGlyphRun::capture(&mut paragraph, check)?
        } else {
            Vec::new()
        };
        let metrics = TextMetrics {
            hinting: "none",
            requested_family: spec.family.clone(),
            locale: spec.locale.clone(),
            width: width.unwrap_or_else(|| paragraph.max_intrinsic_width()),
            height: paragraph.height(),
            longest_line: paragraph.longest_line(),
            intrinsic_width: paragraph.max_intrinsic_width(),
            baseline: paragraph.alphabetic_baseline(),
            line_count: paragraph.line_number(),
            fonts: used,
        };
        if unsupported {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "The selected font has visible glyphs without vector outlines; text strokes cannot be drawn faithfully",
            ));
        }
        let outline = outline.map(|mut builder| builder.detach());
        let ink_bounds = ink_bounds
            .map(|bounds| bounds.outset(1.0, 1.0))
            .unwrap_or_default();
        let decoration =
            if (outline.is_some() || native_color) && (spec.underline || spec.strikethrough) {
                let bounds = ink_bounds.union(crate::geometry::Rect::new(
                    0.0,
                    0.0,
                    paragraph.max_width(),
                    paragraph.height(),
                ));
                let decoration = decorations::geometry(&paragraph, bounds)?;
                check()?;
                Some(decoration)
            } else {
                None
            };
        let outline =
            if let (Some(outline), Some(decoration)) = (outline.as_ref(), decoration.as_ref()) {
                Some(
                    outline
                        .op(decoration, skia_safe::PathOp::Union)
                        .ok_or_else(|| {
                            Error::new(
                                "RENDER_ERROR",
                                "Cannot combine glyph and decoration geometry",
                            )
                        })?,
                )
            } else {
                outline
            };
        check()?;
        Ok(TextBlock {
            paragraph,
            metrics,
            outline,
            ink_bounds,
            native_color,
            native_decoration: native_color.then_some(decoration).flatten(),
            native_runs,
        })
    }
}

#[derive(Clone)]
pub struct TextSpec {
    pub content: String,
    pub family: String,
    pub size: f32,
    pub weight: i32,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub outline_required: bool,
    pub letter_spacing: f32,
    pub line_height: Option<f32>,
    pub color: Color,
    pub align: TextAlign,
    pub locale: String,
}

impl Default for TextSpec {
    fn default() -> Self {
        Self {
            content: String::new(),
            family: DEFAULT_FONT_FAMILY.into(),
            size: 16.0,
            weight: 400,
            italic: false,
            underline: false,
            strikethrough: false,
            outline_required: false,
            letter_spacing: 0.0,
            line_height: None,
            color: Color::BLACK,
            align: TextAlign::Left,
            locale: "ja".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextMetrics {
    pub hinting: &'static str,
    pub requested_family: String,
    pub locale: String,
    pub width: f32,
    pub height: f32,
    pub longest_line: f32,
    pub intrinsic_width: f32,
    pub baseline: f32,
    pub line_count: usize,
    pub fonts: Vec<FontInfo>,
}

pub struct TextBlock {
    pub paragraph: Paragraph,
    pub metrics: TextMetrics,
    pub outline: Option<Path>,
    pub ink_bounds: crate::geometry::Rect,
    pub(crate) native_color: bool,
    native_decoration: Option<Path>,
    native_runs: Vec<NativeGlyphRun>,
}

struct NativeGlyphRun {
    glyphs: Vec<skia_safe::GlyphId>,
    positions: Vec<skia_safe::Point>,
    origin: skia_safe::Point,
    font: skia_safe::Font,
}

impl NativeGlyphRun {
    fn capture(paragraph: &mut Paragraph, check: &dyn Fn() -> Result<()>) -> Result<Vec<Self>> {
        let mut runs = Vec::new();
        let mut count = 0usize;
        let mut error = None;
        paragraph.visit(|_, run| {
            if error.is_some() {
                return;
            }
            if let Err(failure) = check() {
                error = Some(failure);
                return;
            }
            if let Some(run) = run {
                count = count.saturating_add(run.count());
                if count > 1024 * 1024 {
                    error = Some(Error::new(
                        "RESOURCE_LIMIT",
                        "Color text exceeds 1048576 positioned glyphs",
                    ));
                    return;
                }
                if run.count() > 0 {
                    runs.push(Self {
                        glyphs: run.glyphs().to_vec(),
                        positions: run.positions().to_vec(),
                        origin: run.origin(),
                        font: run.font().clone(),
                    });
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        Ok(runs)
    }
}

impl TextBlock {
    /// Reuse the shaped glyphs; drawing native glyphs preserves their color palettes.
    pub(crate) fn paint_native(
        &self,
        canvas: &skia_safe::Canvas,
        offset: f32,
        paint: &skia_safe::Paint,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        for run in &self.native_runs {
            for (glyphs, positions) in run.glyphs.chunks(64).zip(run.positions.chunks(64)) {
                check()?;
                canvas.draw_glyphs_at(
                    glyphs,
                    positions,
                    (run.origin.x, run.origin.y + offset),
                    &run.font,
                    paint,
                );
            }
        }
        if let Some(decoration) = &self.native_decoration {
            check()?;
            canvas.draw_path(&decoration.with_offset((0.0, offset)), paint);
        }
        Ok(())
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::cell::Cell;

    fn outline_png(block: &TextBlock) -> Vec<u8> {
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((304, block.metrics.height.ceil() as i32 + 8))
                .unwrap();
        surface.canvas().clear(Color::WHITE);
        let mut paint = skia_safe::Paint::default();
        paint.set_color(Color::BLACK);
        paint.set_anti_alias(true);
        surface
            .canvas()
            .draw_path(block.outline.as_ref().unwrap(), &paint);
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn native_cancellation_interrupts_glyph_outlines_without_poisoning_font_reuse() {
        let mut fonts = Fonts::new(&[]).unwrap();
        let spec = TextSpec {
            content: "日本語 Native geometry ".repeat(32),
            size: 18.,
            outline_required: true,
            underline: true,
            ..Default::default()
        };
        let before = fonts.layout(&spec, Some(300.)).unwrap();
        let metrics = serde_json::to_value(&before.metrics).unwrap();
        let outline = outline_png(&before);
        let checks = Cell::new(0);
        let cancel = tokio_util::sync::CancellationToken::new();
        let result = fonts.layout_checked(&spec, Some(300.), &|| {
            checks.set(checks.get() + 1);
            // Initial checks cover font selection and shaping. This interruption
            // occurs while walking positioned glyph runs and their outlines.
            if checks.get() == 16 {
                cancel.cancel();
            }
            crate::cancel::check(Some(&cancel))
        });
        assert_eq!(
            result
                .err()
                .expect("An interrupted glyph walk must not return a TextBlock")
                .code,
            "CANCELLED"
        );
        assert_eq!(checks.get(), 16);
        let after = fonts
            .layout_with_cancellation(&spec, Some(300.), None)
            .unwrap();
        assert_eq!(serde_json::to_value(&after.metrics).unwrap(), metrics);
        assert_eq!(outline_png(&after), outline);
        assert_eq!(after.ink_bounds, before.ink_bounds);
    }

    #[test]
    fn native_cancellation_preserves_deadline_errors_from_glyph_walks() {
        let mut fonts = Fonts::new(&[]).unwrap();
        let spec = TextSpec {
            content: "Native deadline geometry ".repeat(32),
            outline_required: true,
            ..Default::default()
        };
        let checks = Cell::new(0);
        let result = fonts.layout_checked(&spec, Some(240.), &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 16 {
                Err(Error::new("RENDER_TIMEOUT", "Calculation deadline elapsed"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.err().unwrap().code, "RENDER_TIMEOUT");
        assert_eq!(checks.get(), 16);
        fonts.layout(&spec, Some(240.)).unwrap();
    }
}
