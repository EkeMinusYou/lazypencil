use super::{Primitive, ResolvedIcon, integrity};
use crate::Result;
use skia_safe::{Matrix, Paint, Path, PathBuilder, RRect, Rect, paint};
use std::collections::BTreeMap;

pub fn shape(icon: &ResolvedIcon, rect: Rect) -> Result<Path> {
    let data = &icon.pack.data;
    let mut combined = Path::default();
    let mut stroke = Paint::default();
    stroke
        .set_style(paint::Style::Stroke)
        .set_stroke_width(data.stroke_width)
        .set_stroke_cap(paint::Cap::Round)
        .set_stroke_join(paint::Join::Round);
    if data.font.is_some() {
        combined = font_path(icon)?;
    }
    for primitive in &data.icons[&icon.canonical] {
        let path = primitive_path(primitive)?;
        if primitive
            .1
            .get("fill")
            .is_some_and(|fill| fill == "currentColor")
        {
            combined = combined
                .op(&path, skia_safe::PathOp::Union)
                .ok_or_else(|| integrity("Cannot combine icon fill geometry"))?;
        }
        if data.stroke_width == 0.0 {
            continue;
        }
        let mut outline = PathBuilder::new();
        if !skia_safe::path_utils::fill_path_with_paint(
            &path,
            &stroke,
            &mut outline,
            None,
            Some(Matrix::default()),
        ) {
            return Err(integrity("Cannot construct icon stroke geometry"));
        }
        combined = combined
            .op(&outline.detach(), skia_safe::PathOp::Union)
            .ok_or_else(|| integrity("Cannot combine icon stroke geometry"))?;
    }
    let [x, y, width, height] = data.view_box;
    let scale = (rect.width() / width).min(rect.height() / height).max(0.0);
    let transform = Matrix::translate((
        rect.left() + (rect.width() - width * scale) / 2.0,
        rect.top() + (rect.height() - height * scale) / 2.0,
    )) * Matrix::scale((scale, scale))
        * Matrix::translate((-x, -y));
    Ok(combined.with_transform(&transform))
}

fn font_path(icon: &ResolvedIcon) -> Result<Path> {
    use skia_safe::{
        Data, Font, FontArguments, FontMgr, FourByteTag,
        font_arguments::{VariationPosition, variation_position::Coordinate},
    };
    let pack = &icon.pack;
    let base = pack
        .font_face
        .get_or_init(|| {
            let bytes = pack
                .font_bytes
                .as_ref()
                .ok_or_else(|| integrity("Missing captured icon font"))?;
            let face = FontMgr::new()
                .new_from_data(Data::new_copy(bytes), Some(0))
                .ok_or_else(|| integrity("Cannot load the pinned icon font"))?;
            if face.units_per_em() != Some(960) {
                return Err(integrity("Unexpected icon font design units"));
            }
            let axes = face
                .variation_design_parameters()
                .ok_or_else(|| integrity("Icon font has no variable axes"))?;
            for (tag, min, max) in [
                (Coordinate::wght, 100.0, 700.0),
                (Coordinate::opsz, 20.0, 48.0),
                (FourByteTag::from_chars('F', 'I', 'L', 'L'), 0.0, 1.0),
                (FourByteTag::from_chars('G', 'R', 'A', 'D'), -50.0, 200.0),
            ] {
                if !axes
                    .iter()
                    .any(|axis| axis.tag == tag && axis.min == min && axis.max == max)
                {
                    return Err(integrity(
                        "Pinned icon font axes differ from their contract",
                    ));
                }
            }
            Ok(face)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let weight = icon
        .weight
        .ok_or_else(|| integrity("A font icon requires a resolved weight"))?;
    let key = weight.to_bits();
    let face = {
        let mut variants = pack
            .font_variants
            .lock()
            .map_err(|_| integrity("Icon font cache is unavailable"))?;
        if let Some(index) = variants.iter().position(|(value, _)| *value == key) {
            let entry = variants.remove(index).expect("Located font instance");
            let face = entry.1.clone();
            variants.push_front(entry);
            face
        } else {
            let coordinates = [
                Coordinate {
                    axis: Coordinate::wght,
                    value: weight,
                },
                Coordinate {
                    axis: Coordinate::opsz,
                    value: 24.0,
                },
                Coordinate {
                    axis: FourByteTag::from_chars('F', 'I', 'L', 'L'),
                    value: 0.0,
                },
                Coordinate {
                    axis: FourByteTag::from_chars('G', 'R', 'A', 'D'),
                    value: 0.0,
                },
            ];
            let args = FontArguments::new().set_variation_design_position(VariationPosition {
                coordinates: &coordinates,
            });
            let face = base
                .clone_with_arguments(&args)
                .ok_or_else(|| integrity("Cannot instance the pinned icon font"))?;
            let position = face
                .variation_design_position()
                .ok_or_else(|| integrity("Icon font did not report its variation position"))?;
            if !coordinates.iter().all(|expected| {
                position.iter().any(|actual| {
                    actual.axis == expected.axis && (actual.value - expected.value).abs() < 0.001
                })
            }) {
                return Err(integrity("Icon font did not apply the requested variation"));
            }
            variants.push_front((key, face.clone()));
            variants.truncate(8);
            face
        }
    };
    let codepoint = pack.data.font.as_ref().expect("Font data")[&icon.canonical];
    let glyph = face.unichar_to_glyph(codepoint as i32);
    if glyph == 0 {
        return Err(integrity("A pinned icon name has no font glyph"));
    }
    let mut font = Font::from_typeface(face, 960.0);
    font.set_hinting(skia_safe::FontHinting::None);
    font.get_path(glyph)
        .ok_or_else(|| integrity("A pinned icon glyph has no vector outline"))
}

fn number(attributes: &BTreeMap<String, String>, key: &str, default: f32) -> Result<f32> {
    attributes
        .get(key)
        .map(|value| {
            value
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| integrity(format!("Invalid icon geometry number {key}")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}
fn primitive_path(Primitive(kind, attrs): &Primitive) -> Result<Path> {
    let mut builder = PathBuilder::new();
    match kind.as_str() {
        "path" => {
            return Path::from_svg(attrs.get("d").map(String::as_str).unwrap_or_default())
                .ok_or_else(|| integrity("Invalid icon SVG path"));
        }
        "line" => {
            builder.move_to((number(attrs, "x1", 0.0)?, number(attrs, "y1", 0.0)?));
            builder.line_to((number(attrs, "x2", 0.0)?, number(attrs, "y2", 0.0)?));
        }
        "circle" => {
            builder.add_circle(
                (number(attrs, "cx", 0.0)?, number(attrs, "cy", 0.0)?),
                number(attrs, "r", 0.0)?,
                None,
            );
        }
        "ellipse" => {
            let (cx, cy, rx, ry) = (
                number(attrs, "cx", 0.0)?,
                number(attrs, "cy", 0.0)?,
                number(attrs, "rx", 0.0)?,
                number(attrs, "ry", 0.0)?,
            );
            builder.add_oval(
                Rect::from_xywh(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry),
                None,
                None,
            );
        }
        "rect" => {
            let rect = Rect::from_xywh(
                number(attrs, "x", 0.0)?,
                number(attrs, "y", 0.0)?,
                number(attrs, "width", 0.0)?,
                number(attrs, "height", 0.0)?,
            );
            let rx = number(attrs, "rx", number(attrs, "ry", 0.0)?)?;
            let ry = number(attrs, "ry", rx)?;
            builder.add_rrect(RRect::new_rect_xy(rect, rx, ry), None, None);
        }
        "polyline" | "polygon" => {
            let numbers = attrs
                .get("points")
                .map(String::as_str)
                .unwrap_or_default()
                .split(|character: char| character == ',' || character.is_ascii_whitespace())
                .filter(|part| !part.is_empty())
                .map(|part| {
                    part.parse::<f32>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| integrity("Invalid icon points"))
                })
                .collect::<Result<Vec<_>>>()?;
            if numbers.len() < 2 || numbers.len() % 2 != 0 {
                return Err(integrity("Icon points need coordinate pairs"));
            }
            for (index, point) in numbers.as_chunks::<2>().0.iter().enumerate() {
                if index == 0 {
                    builder.move_to((point[0], point[1]));
                } else {
                    builder.line_to((point[0], point[1]));
                }
            }
            if kind == "polygon" {
                builder.close();
            }
        }
        _ => return Err(integrity("Unsupported primitive in pinned icon pack")),
    }
    Ok(builder.detach())
}
