//! Mathematical contours and scanline area sampling for the authored shape cases.
//!
//! This reference does not use Skia, application geometry, or exported PNGs.

use super::Raster;
use serde_json::Value;
use std::f64::consts::{FRAC_PI_2, TAU};

type Point = (f64, f64);
const Y_SAMPLES: u32 = 16;
const CHORD_ERROR: f64 = 1.0 / 256.0;

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}
fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}
fn mul(a: Point, b: f64) -> Point {
    (a.0 * b, a.1 * b)
}
fn dot(a: Point, b: Point) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn cross(a: Point, b: Point) -> f64 {
    a.0 * b.1 - a.1 * b.0
}
fn unit(a: Point) -> Point {
    mul(a, 1.0 / a.0.hypot(a.1))
}
fn number(value: &Value, key: &str, default: f64) -> f64 {
    value[key].as_f64().unwrap_or(default)
}

fn arc(center: Point, radius: f64, start: f64, sweep: f64) -> Vec<Point> {
    let step = if radius <= CHORD_ERROR {
        TAU
    } else {
        2.0 * (1.0 - CHORD_ERROR / radius).acos()
    };
    let count = (sweep.abs() / step).ceil().max(1.0) as usize;
    assert!(count <= 4096);
    (0..=count)
        .map(|i| {
            let angle = start + sweep * i as f64 / count as f64;
            add(center, (radius * angle.cos(), radius * angle.sin()))
        })
        .collect()
}

fn polygon(width: f64, height: f64, count: usize) -> Vec<Point> {
    assert!((3..=16).contains(&count));
    (0..count)
        .map(|i| {
            let angle = i as f64 * TAU / count as f64 - FRAC_PI_2;
            (
                width * (1.0 + angle.cos()) / 2.0,
                height * (1.0 + angle.sin()) / 2.0,
            )
        })
        .collect()
}

fn rounded(vertices: &[Point], requested: f64) -> Vec<Point> {
    assert!(requested.is_finite() && requested >= 0.0);
    if requested == 0.0 {
        return vertices.to_vec();
    }
    let count = vertices.len();
    let shortest = (0..count)
        .map(|i| {
            let edge = sub(vertices[(i + 1) % count], vertices[i]);
            edge.0.hypot(edge.1)
        })
        .fold(f64::INFINITY, f64::min);
    let radius = requested.min(shortest / 2.0);
    let mut contour = Vec::new();
    for (i, &vertex) in vertices.iter().enumerate() {
        let before = unit(sub(vertices[(i + count - 1) % count], vertex));
        let after = unit(sub(vertices[(i + 1) % count], vertex));
        let half = dot(before, after).clamp(-1.0, 1.0).acos() / 2.0;
        let center = add(vertex, mul(unit(add(before, after)), radius / half.sin()));
        let distance = radius / half.tan();
        let first = sub(add(vertex, mul(before, distance)), center);
        let last = sub(add(vertex, mul(after, distance)), center);
        let start = first.1.atan2(first.0);
        let sweep = (last.1.atan2(last.0) - start).rem_euclid(TAU);
        contour.extend(arc(center, radius, start, sweep));
    }
    contour
}

fn offset(vertices: &[Point], distance: f64) -> Vec<Point> {
    let count = vertices.len();
    (0..count)
        .map(|i| {
            let vertex = vertices[i];
            let previous = unit(sub(vertex, vertices[(i + count - 1) % count]));
            let next = unit(sub(vertices[(i + 1) % count], vertex));
            let first = add(vertex, mul((previous.1, -previous.0), distance));
            let second = add(vertex, mul((next.1, -next.0), distance));
            add(
                first,
                mul(
                    previous,
                    cross(sub(second, first), next) / cross(previous, next),
                ),
            )
        })
        .collect()
}

fn color(text: &str) -> [u8; 4] {
    assert!(text.len() == 7 && text.starts_with('#'));
    [
        u8::from_str_radix(&text[1..3], 16).unwrap(),
        u8::from_str_radix(&text[3..5], 16).unwrap(),
        u8::from_str_radix(&text[5..7], 16).unwrap(),
        255,
    ]
}

fn fill(image: &mut Raster, contours: &[Vec<Point>], color: [u8; 4]) {
    let min = contours
        .iter()
        .flatten()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as u32;
    let max = contours
        .iter()
        .flatten()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, image.height as f64) as u32;
    let mut crossings = Vec::<(f64, i32)>::new();
    let mut coverage = vec![0.0; image.width as usize];
    for y in min..max {
        coverage.fill(0.0);
        for sample in 0..Y_SAMPLES {
            let scan = y as f64 + (sample as f64 + 0.5) / Y_SAMPLES as f64;
            crossings.clear();
            for contour in contours {
                for (i, &a) in contour.iter().enumerate() {
                    let b = contour[(i + 1) % contour.len()];
                    let direction = if a.1 <= scan && scan < b.1 {
                        1
                    } else if b.1 <= scan && scan < a.1 {
                        -1
                    } else {
                        continue;
                    };
                    crossings.push((a.0 + (scan - a.1) * (b.0 - a.0) / (b.1 - a.1), direction));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            let mut previous = 0.0f64;
            for &(position, direction) in &crossings {
                if winding != 0 {
                    let left = previous.clamp(0.0, image.width as f64);
                    let right = position.clamp(0.0, image.width as f64);
                    for (x, covered) in coverage
                        .iter_mut()
                        .enumerate()
                        .take(right.ceil() as usize)
                        .skip(left.floor() as usize)
                    {
                        *covered += (right.min(x as f64 + 1.0) - left.max(x as f64)).max(0.0)
                            / Y_SAMPLES as f64;
                    }
                }
                winding += direction;
                previous = position;
            }
            assert_eq!(winding, 0, "Unclosed reference contour");
        }
        for (x, &alpha) in coverage.iter().enumerate() {
            let alpha = alpha.clamp(0.0, 1.0);
            if alpha == 0.0 {
                continue;
            }
            let old = image.pixel(x as u32, y);
            let blended = std::array::from_fn(|channel| {
                (old[channel] as f64 * (1.0 - alpha) + color[channel] as f64 * alpha).round() as u8
            });
            image.set(x as u32, y, blended);
        }
    }
}

fn transform(points: Vec<Point>, x: f64, y: f64, scale: f64) -> Vec<Point> {
    points
        .into_iter()
        .map(|p| ((p.0 + x) * scale, (p.1 + y) * scale))
        .collect()
}

/// Render the bounded, flat-color original shape inputs. Unsupported fixture
/// edits fail explicitly so that they cannot silently weaken the oracle.
pub fn raster(document: &Value, scale: u32) -> Raster {
    assert!((1..=2).contains(&scale));
    let root = &document["children"][0];
    assert_eq!(root["type"], "frame");
    let scale = scale as f64;
    let mut image = Raster::solid(
        (number(root, "width", 0.0) * scale) as u32,
        (number(root, "height", 0.0) * scale) as u32,
        color(root["fill"].as_str().unwrap()),
    );
    for node in root["children"].as_array().unwrap() {
        assert_eq!(number(node, "rotation", 0.0), 0.0);
        assert_eq!(number(node, "opacity", 1.0), 1.0);
        assert!(node.get("effect").is_none());
        let (x, y) = (number(node, "x", 0.0), number(node, "y", 0.0));
        let (w, h) = (number(node, "width", 0.0), number(node, "height", 0.0));
        assert!(w > 0.0 && h > 0.0);
        let ellipse = node["type"] == "ellipse";
        let points = match node["type"].as_str().unwrap() {
            "ellipse" => arc((w / 2.0, h / 2.0), w / 2.0, 0.0, TAU),
            "polygon" => rounded(
                &polygon(w, h, node["polygonCount"].as_u64().unwrap_or(3) as usize),
                number(node, "cornerRadius", 0.0),
            ),
            "path" => {
                assert_eq!(node["geometry"], "M0 50 L25 0 L50 50 Z");
                vec![(0.0, 50.0), (25.0, 0.0), (50.0, 50.0)]
            }
            _ => panic!("Unsupported original shape fixture"),
        };
        if ellipse {
            assert_eq!(w, h, "Authored ellipse cases are circles");
        }
        fill(
            &mut image,
            &[transform(points.clone(), x, y, scale)],
            color(node["fill"].as_str().unwrap()),
        );
        let width = node["strokeWidth"]
            .as_f64()
            .unwrap_or_else(|| number(&node["strokeWidth"], "top", 0.0));
        assert!(width.is_finite() && width >= 0.0);
        if width > 0.0 {
            assert_eq!(node["strokeAlignment"], "center");
            let (outer, mut inner) = if ellipse {
                (
                    arc((w / 2.0, h / 2.0), (w + width) / 2.0, 0.0, TAU),
                    arc((w / 2.0, h / 2.0), (w - width) / 2.0, 0.0, TAU),
                )
            } else {
                assert_eq!(number(node, "cornerRadius", 0.0), 0.0);
                (offset(&points, width / 2.0), offset(&points, -width / 2.0))
            };
            inner.reverse();
            fill(
                &mut image,
                &[transform(outer, x, y, scale), transform(inner, x, y, scale)],
                color(node["stroke"].as_str().unwrap()),
            );
        }
    }
    image
}
