//! Maps source image pixels through a normalized crop into a node's local box.

use crate::{
    Error, Result,
    geometry::{Affine, Rect},
};
use serde_json::Value;

pub struct ImagePlacement {
    pub transform: Affine,
    pub viewport: Rect,
}

pub fn placement(
    fill: &Value,
    image_width: f32,
    image_height: f32,
    box_rect: Rect,
) -> Result<ImagePlacement> {
    let transform = if let Some(value) = fill.get("transform") {
        let values = value
            .as_array()
            .filter(|values| values.len() == 6)
            .ok_or_else(|| invalid("Image transform requires six finite numbers"))?;
        let values = values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .map(|number| number as f32)
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| invalid("Image transform requires six finite numbers"))
            })
            .collect::<Result<Vec<_>>>()?;
        Affine(values.try_into().expect("six checked values"))
    } else {
        Affine::default()
    };
    let crop = transform
        .inverse()
        .ok_or_else(|| invalid("Image crop transform must be invertible"))?;
    let [a, b, c, d, _, _] = crop.0;
    let crop_width = (a * image_width).hypot(b * image_height);
    let crop_height = (c * image_width).hypot(d * image_height);
    if !crop_width.is_finite()
        || !crop_height.is_finite()
        || crop_width <= 0.0
        || crop_height <= 0.0
    {
        return Err(invalid("Image crop has invalid dimensions"));
    }
    let mode = image_mode(fill)?;
    let (width, height) = match mode {
        "stretch" => (box_rect.width, box_rect.height),
        "contain" | "cover" => {
            let x = box_rect.width / crop_width;
            let y = box_rect.height / crop_height;
            let ratio = if mode == "cover" { x.max(y) } else { x.min(y) };
            (crop_width * ratio, crop_height * ratio)
        }
        _ => unreachable!("validated image mode"),
    };
    let viewport = Rect::new(
        box_rect.x + (box_rect.width - width) / 2.0,
        box_rect.y + (box_rect.height - height) / 2.0,
        width,
        height,
    );
    let mut mapping = Affine::translation(viewport.x, viewport.y)
        .then(Affine::scale(width, height))
        .then(transform)
        .then(Affine::scale(1.0 / image_width, 1.0 / image_height));
    if mode == "cover" && box_rect.width > 0.0 && box_rect.height > 0.0 {
        let inverse = mapping
            .inverse()
            .ok_or_else(|| invalid("Image placement is not invertible"))?;
        let visible = box_rect.transformed(inverse);
        let span_x = visible.width / image_width;
        let span_y = visible.height / image_height;
        let zoom = span_x.max(span_y).max(1.0);
        if !zoom.is_finite() {
            return Err(invalid("Image crop placement exceeds numeric limits"));
        }
        let center = (
            box_rect.x + box_rect.width / 2.0,
            box_rect.y + box_rect.height / 2.0,
        );
        let desired = inverse.point(center.0, center.1);
        let half_x = (span_x / zoom / 2.0).min(0.5);
        let half_y = (span_y / zoom / 2.0).min(0.5);
        let x = (desired.0 / image_width).clamp(half_x, 1.0 - half_x) * image_width;
        let y = (desired.1 / image_height).clamp(half_y, 1.0 - half_y) * image_height;
        let [a, b, c, d, _, _] = mapping.0;
        let (a, b, c, d) = (a * zoom, b * zoom, c * zoom, d * zoom);
        mapping = Affine([
            a,
            b,
            c,
            d,
            center.0 - a * x - c * y,
            center.1 - b * x - d * y,
        ]);
    }
    if mapping.0.iter().any(|value| !value.is_finite()) {
        return Err(invalid("Image crop placement exceeds numeric limits"));
    }
    Ok(ImagePlacement {
        transform: mapping,
        viewport,
    })
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_DOCUMENT", message)
}

pub fn image_mode(fill: &Value) -> Result<&str> {
    match fill.get("mode") {
        None => Ok("cover"),
        Some(Value::String(mode)) if matches!(mode.as_str(), "cover" | "contain" | "stretch") => Ok(mode),
        value => Err(invalid("Image mode must be cover, contain, or stretch; use contain to show the whole image")
            .details(serde_json::json!({"property":"mode","value":value,"allowed_modes":["cover","contain","stretch"]}))),
    }
}
