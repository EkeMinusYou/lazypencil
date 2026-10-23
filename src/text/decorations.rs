//! Recover solid decoration rectangles from the same native Paragraph paint.
//!
//! SkParagraph's public glyph visitor excludes decoration drawing. Its SVG
//! canvas keeps glyphs as text and records the actual decoration rectangles,
//! including fallback-run metrics, justification, and line translations. This
//! adapter consumes only that private, pinned Skia output, never user SVG.

use crate::{Error, Result, geometry::Rect};
use skia_safe::{Matrix, Path, PathBuilder, Rect as SkRect, textlayout::Paragraph};

const MAX_BYTES: usize = 16 * 1024 * 1024;

fn invalid() -> Error {
    Error::new(
        "RENDER_ERROR",
        "Cannot recover native text decoration geometry",
    )
}

pub(super) fn geometry(paragraph: &Paragraph, bounds: Rect) -> Result<Path> {
    let offset = ((-bounds.x).max(0.0).ceil(), (-bounds.y).max(0.0).ceil());
    let width = (bounds.right() + offset.0).ceil().max(1.0);
    let height = (bounds.bottom() + offset.1).ceil().max(1.0);
    if !width.is_finite() || !height.is_finite() || width > 16_000_000.0 || height > 16_000_000.0 {
        return Err(invalid());
    }
    let canvas = skia_safe::svg::Canvas::new(
        SkRect::from_wh(width, height),
        skia_safe::svg::canvas::Flags::NO_PRETTY_XML,
    );
    paragraph.paint(&canvas, offset);
    let data = canvas.end();
    if data.len() > MAX_BYTES {
        return Err(Error::new(
            "RENDER_ERROR",
            "Text decoration recording exceeds 16 MiB",
        ));
    }
    let source = std::str::from_utf8(data.as_bytes()).map_err(|_| invalid())?;
    rectangles(source, offset)
}

fn number(node: roxmltree::Node<'_, '_>, name: &str, default: f32) -> Result<f32> {
    node.attribute(name)
        .map(|value| value.parse::<f32>().map_err(|_| invalid()))
        .transpose()?
        .map_or(Ok(default), |value| {
            value.is_finite().then_some(value).ok_or_else(invalid)
        })
}

fn transform(value: &str) -> Result<Matrix> {
    let (name, arguments) = value.split_once('(').ok_or_else(invalid)?;
    let arguments = arguments.strip_suffix(')').ok_or_else(invalid)?;
    let numbers = arguments
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    match (name, numbers.as_slice()) {
        ("translate", [x]) => Ok(Matrix::translate((*x, 0.0))),
        ("translate", [x, y]) => Ok(Matrix::translate((*x, *y))),
        ("scale", [x]) => Ok(Matrix::scale((*x, *x))),
        ("scale", [x, y]) => Ok(Matrix::scale((*x, *y))),
        ("matrix", [a, b, c, d, x, y]) => {
            Ok(Matrix::new_all(*a, *c, *x, *b, *d, *y, 0.0, 0.0, 1.0))
        }
        _ => Err(invalid()),
    }
}

fn rectangles(source: &str, offset: (f32, f32)) -> Result<Path> {
    let document = roxmltree::Document::parse(source).map_err(|_| invalid())?;
    let mut builder = PathBuilder::new();
    for node in document.descendants().filter(|node| node.is_element()) {
        if node
            .ancestors()
            .any(|parent| matches!(parent.tag_name().name(), "text" | "defs"))
        {
            continue;
        }
        match node.tag_name().name() {
            "svg" | "g" => continue,
            "rect" => {}
            // All currently exposed decorations are solid, through lines.
            // Reject a changed native format instead of silently omitting ink.
            _ => return Err(invalid()),
        }
        let width = number(node, "width", 0.0)?;
        let height = number(node, "height", 0.0)?;
        if width < 0.0
            || height < 0.0
            || node.attribute("stroke").is_some()
            || number(node, "rx", 0.0)? != 0.0
            || number(node, "ry", 0.0)? != 0.0
        {
            return Err(invalid());
        }
        let mut matrix = Matrix::default();
        for parent in node.ancestors().filter(|node| node.is_element()) {
            if parent.attribute("clip-path").is_some() {
                return Err(invalid());
            }
            if let Some(value) = parent.attribute("transform") {
                matrix = transform(value)? * matrix;
            }
        }
        let rect = SkRect::from_xywh(
            number(node, "x", 0.0)?,
            number(node, "y", 0.0)?,
            width,
            height,
        );
        let path = Path::rect(rect, None);
        builder.add_path_with_transform(
            &path,
            &(Matrix::translate((-offset.0, -offset.1)) * matrix),
            None,
        );
    }
    Ok(builder.detach())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Fonts, TextSpec};
    use skia_safe::{Color, Paint, textlayout::TextAlign};

    #[test]
    fn recovered_geometry_matches_native_paint_across_lines_and_fallback_runs() {
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fonts/regular.ttf");
        let mut fonts = Fonts::new(&[font]).unwrap();
        let mut total_added = 0;
        for content in ["A A 日本語", "A 日\n本 A", "A A A A A A", "A   ", "   ", ""] {
            for align in [
                TextAlign::Left,
                TextAlign::Center,
                TextAlign::Right,
                TextAlign::Justify,
            ] {
                for (underline, strikethrough) in [(true, false), (false, true), (true, true)] {
                    let mut spec = TextSpec {
                        content: content.into(),
                        family: "LP Test Sans".into(),
                        size: 31.5,
                        line_height: Some(1.7),
                        letter_spacing: 1.25,
                        underline,
                        strikethrough,
                        align,
                        ..Default::default()
                    };
                    let decorated = fonts.layout(&spec, Some(140.0)).unwrap();
                    spec.underline = false;
                    spec.strikethrough = false;
                    let plain = fonts.layout(&spec, Some(140.0)).unwrap();
                    let bounds = decorated.ink_bounds.union(Rect::new(
                        0.0,
                        0.0,
                        140.0,
                        decorated.metrics.height,
                    ));
                    let path = geometry(&decorated.paragraph, bounds).unwrap();
                    let surface = |scale: i32, paint: &dyn Fn(&skia_safe::Canvas)| {
                        let mut surface =
                            skia_safe::surfaces::raster_n32_premul((240 * scale, 320 * scale))
                                .unwrap();
                        surface.canvas().clear(Color::TRANSPARENT);
                        surface.canvas().scale((scale as f32, scale as f32));
                        surface.canvas().translate((40.0, 16.0));
                        paint(surface.canvas());
                        surface
                            .image_snapshot()
                            .make_raster_image(None, None)
                            .unwrap()
                    };
                    // Supersampling reduces native alpha seams between run
                    // rectangles, which the unioned stroke deliberately removes.
                    let native =
                        surface(4, &|canvas| decorated.paragraph.paint(canvas, (0.0, 0.0)));
                    let glyphs = surface(4, &|canvas| plain.paragraph.paint(canvas, (0.0, 0.0)));
                    let recovered = surface(1, &|canvas| {
                        let mut paint = Paint::default();
                        paint.set_anti_alias(true);
                        canvas.draw_path(&path, &paint);
                    });
                    let native = native.peek_pixels().unwrap();
                    let glyphs = glyphs.peek_pixels().unwrap();
                    let recovered = recovered.peek_pixels().unwrap();
                    for y in 0..320 {
                        for x in 0..240 {
                            let average = |pixels: &skia_safe::Pixmap<'_>| -> u8 {
                                let mut sum = 0u32;
                                for dy in 0..4 {
                                    for dx in 0..4 {
                                        sum +=
                                            pixels.get_color((x * 4 + dx, y * 4 + dy)).a() as u32;
                                    }
                                }
                                ((sum + 8) / 16) as u8
                            };
                            let expected = average(&native);
                            let actual = recovered.get_color((x, y)).a();
                            if average(&glyphs) == 0 {
                                total_added += usize::from(expected > 0);
                                assert!(
                                    actual.abs_diff(expected)
                                        <= if expected == 255 { 2 } else { 32 },
                                    "Missing decoration: {content:?}/{align:?}/{underline}/{strikethrough} at {x},{y}: {actual}, native={expected}"
                                );
                            }
                            if actual > 0 {
                                assert!(
                                    actual <= expected.saturating_add(32),
                                    "Unexpected decoration: {content:?}/{align:?} at {x},{y}: {expected}"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(
            total_added > 1000,
            "The oracle must include decoration ink outside glyphs"
        );
    }
}
