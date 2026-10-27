//! Recover the visible silhouette when a native glyph has no usable outline.
//!
//! Color-table base paths need not describe the painted layers. Sample their
//! native alpha instead, and retain holes as oppositely oriented contours.
use crate::{Error, Result};
use skia_safe::{Font, GlyphId, Path, PathBuilder, Rect};
use std::collections::BTreeMap;

const MAX_SIDE: f32 = 2048.0;
const MAX_PIXELS: usize = 64 * 1024 * 1024;
const MAX_POINTS: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct Budget {
    pixels: usize,
    points: usize,
}

fn limit() -> Error {
    Error::new("RESOURCE_LIMIT", "Bitmap text outline budget exceeded")
}

fn invalid() -> Error {
    Error::new("RENDER_ERROR", "Cannot recover native glyph silhouette")
}

pub(super) fn glyph(
    font: &Font,
    glyph: GlyphId,
    bounds: Rect,
    budget: &mut Budget,
    check: &dyn Fn() -> Result<()>,
) -> Result<Path> {
    check()?;
    if ![bounds.left(), bounds.top(), bounds.right(), bounds.bottom()]
        .into_iter()
        .all(f32::is_finite)
    {
        return Err(invalid());
    }
    let scale = 4.0_f32
        .min(MAX_SIDE / bounds.width().max(1.0))
        .min(MAX_SIDE / bounds.height().max(1.0));
    let left = (bounds.left() * scale).floor() - 2.0;
    let top = (bounds.top() * scale).floor() - 2.0;
    let width = ((bounds.right() * scale).ceil() + 2.0 - left) as usize;
    let height = ((bounds.bottom() * scale).ceil() + 2.0 - top) as usize;
    let pixels = width.checked_mul(height).ok_or_else(limit)?;
    budget.pixels = budget.pixels.checked_add(pixels).ok_or_else(limit)?;
    if width < 3 || height < 3 || budget.pixels > MAX_PIXELS {
        return Err(limit());
    }
    let mut surface =
        skia_safe::surfaces::raster_n32_premul((width as i32, height as i32)).ok_or_else(limit)?;
    let canvas = surface.canvas();
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.translate((-left, -top));
    canvas.scale((scale, scale));
    let mut paint = skia_safe::Paint::default();
    paint
        .set_anti_alias(true)
        .set_color(skia_safe::Color::WHITE);
    canvas.draw_glyphs_at(
        &[glyph],
        &[skia_safe::Point::default()],
        (0.0, 0.0),
        font,
        &paint,
    );
    check()?;
    let mut alpha = vec![0; pixels];
    let info = skia_safe::ImageInfo::new(
        (width as i32, height as i32),
        skia_safe::ColorType::Alpha8,
        skia_safe::AlphaType::Premul,
        None,
    );
    if !surface.image_snapshot().read_pixels(
        &info,
        &mut alpha,
        width,
        (0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return Err(invalid());
    }
    // Never accept a contour clipped by the temporary glyph surface.
    if (0..width).any(|x| alpha[x] != 0 || alpha[(height - 1) * width + x] != 0)
        || (0..height).any(|y| alpha[y * width] != 0 || alpha[y * width + width - 1] != 0)
    {
        return Err(invalid());
    }
    contours(&alpha, width, height, (left, top), scale, budget, check)
}

fn contours(
    alpha: &[u8],
    width: usize,
    height: usize,
    origin: (f32, f32),
    scale: f32,
    budget: &mut Budget,
    check: &dyn Fn() -> Result<()>,
) -> Result<Path> {
    // A half-byte isosurface includes even low-opacity ink. Using half the
    // maximum alpha would silently drop translucent parts of a color glyph.
    const LEVEL: f32 = 0.5;
    let mut edges = BTreeMap::new();
    for y in 0..height - 1 {
        if y.is_multiple_of(32) {
            check()?;
        }
        for x in 0..width - 1 {
            let i = y * width + x;
            let values = [
                alpha[i],
                alpha[i + 1],
                alpha[i + width + 1],
                alpha[i + width],
            ];
            let bits = values
                .iter()
                .enumerate()
                .fold(0, |bits, (i, a)| bits | (u8::from(*a > 0) << i));
            let [t, r, b, l] = [i * 2, (i + 1) * 2 + 1, (i + width) * 2, i * 2 + 1];
            let saddle = || {
                let [a, b, c, d] = values.map(|a| f32::from(a) - LEVEL);
                a * c - b * d
            };
            let pairs: &[(usize, usize)] = match bits {
                0 | 15 => &[],
                1 => &[(t, l)],
                2 => &[(r, t)],
                3 => &[(r, l)],
                4 => &[(b, r)],
                5 if saddle() > 0.0 => &[(t, r), (b, l)],
                5 => &[(t, l), (b, r)],
                6 => &[(b, t)],
                7 => &[(b, l)],
                8 => &[(l, b)],
                9 => &[(t, b)],
                10 if saddle() < 0.0 => &[(l, t), (r, b)],
                10 => &[(r, t), (l, b)],
                11 => &[(r, b)],
                12 => &[(l, r)],
                13 => &[(t, r)],
                14 => &[(l, t)],
                _ => unreachable!(),
            };
            for &(from, to) in pairs {
                budget.points = budget.points.checked_add(1).ok_or_else(limit)?;
                if budget.points > MAX_POINTS {
                    return Err(limit());
                }
                if edges.insert(from, to).is_some() {
                    return Err(invalid());
                }
            }
        }
    }
    let point = |edge: usize| {
        let i = edge / 2;
        let horizontal = edge.is_multiple_of(2);
        let next = i + if horizontal { 1 } else { width };
        let a = f32::from(alpha[i]);
        let b = f32::from(alpha[next]);
        let amount = (LEVEL - a) / (b - a);
        (
            (origin.0 + (i % width) as f32 + 0.5 + if horizontal { amount } else { 0.0 }) / scale,
            (origin.1 + (i / width) as f32 + 0.5 + if horizontal { 0.0 } else { amount }) / scale,
        )
    };
    let mut path = PathBuilder::new();
    while let Some((&start, _)) = edges.first_key_value() {
        check()?;
        path.move_to(point(start));
        let mut current = start;
        let mut count = 0usize;
        loop {
            let next = edges.remove(&current).ok_or_else(invalid)?;
            count += 1;
            if count.is_multiple_of(1024) {
                check()?;
            }
            if next == start {
                path.close();
                break;
            }
            path.line_to(point(next));
            current = next;
        }
    }
    Ok(path.detach())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn assert_mask(alpha: &[u8], width: usize, height: usize) {
        let path = contours(
            alpha,
            width,
            height,
            (0.0, 0.0),
            1.0,
            &mut Budget::default(),
            &|| Ok(()),
        )
        .unwrap();
        let mut surface =
            skia_safe::surfaces::raster_n32_premul((width as i32, height as i32)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let mut paint = skia_safe::Paint::default();
        paint
            .set_color(skia_safe::Color::WHITE)
            .set_anti_alias(false);
        surface.canvas().draw_path(&path, &paint);
        let image = surface.image_snapshot();
        let pixels = image.peek_pixels().unwrap();
        for y in 0..height {
            for x in 0..width {
                assert_eq!(
                    pixels.get_color((x as i32, y as i32)).a() > 0,
                    alpha[y * width + x] > 0,
                    "{x},{y}"
                );
            }
        }
    }

    #[test]
    fn silhouettes_keep_holes_low_alpha_islands_and_all_saddle_configurations() {
        let mut alpha = vec![0; 13 * 13];
        for y in 1..12 {
            for x in 1..12 {
                if !(3..10).contains(&x) || !(3..10).contains(&y) {
                    alpha[y * 13 + x] = 255;
                }
            }
        }
        alpha[6 * 13 + 6] = 1;
        assert_mask(&alpha, 13, 13);
        for bits in 0u8..16 {
            let mut alpha = vec![0; 16];
            for (bit, index) in [5, 6, 10, 9].into_iter().enumerate() {
                if bits & (1 << bit) != 0 {
                    alpha[index] = 255;
                }
            }
            assert_mask(&alpha, 4, 4);
        }
    }

    #[test]
    fn silhouette_recovery_bounds_work_and_checks_cancellation_during_tracing() {
        let mut alpha = vec![0; 80 * 80];
        for y in 1..79 {
            for x in 1..79 {
                alpha[y * 80 + x] = 255;
            }
        }
        let mut budget = Budget {
            points: MAX_POINTS,
            ..Default::default()
        };
        assert_eq!(
            contours(&alpha, 80, 80, (0.0, 0.0), 1.0, &mut budget, &|| Ok(()))
                .unwrap_err()
                .code,
            "RESOURCE_LIMIT"
        );
        let calls = Cell::new(0);
        let check = || {
            calls.set(calls.get() + 1);
            if calls.get() >= 2 {
                Err(Error::new("CANCELLED", "Expected cancellation"))
            } else {
                Ok(())
            }
        };
        assert_eq!(
            contours(
                &alpha,
                80,
                80,
                (0.0, 0.0),
                1.0,
                &mut Budget::default(),
                &check
            )
            .unwrap_err()
            .code,
            "CANCELLED"
        );
        assert_mask(&alpha, 80, 80);
    }
}
