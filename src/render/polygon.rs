use skia_safe::{Path, PathBuilder, PathDirection, Point, path_builder::ArcSize};

/// Circular fillets use the two original edges at each vertex. Construct the
/// endpoints independently: an oversized radius can pass the next vertex, so
/// deriving the next tangent from the previous arc endpoint changes the shape.
pub(super) fn rounded_path(points: &[Point], radius: f32) -> Path {
    // The native reference caps the radius at half the shortest edge, even
    // when the resulting tangent points overlap on an acute polygon.
    let radius = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let next = points[(index + 1) % points.len()];
            let x = f64::from(point.x) - f64::from(next.x);
            let y = f64::from(point.y) - f64::from(next.y);
            (x.hypot(y) / 2.0) as f32
        })
        .fold(radius, f32::min);
    if radius == 0.0 {
        return Path::polygon(points, true, None, None);
    }
    let corners = points
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            let previous = points[(index + points.len() - 1) % points.len()];
            let next = points[(index + 1) % points.len()];
            let incoming = unit(*vertex, previous);
            let outgoing = unit(*vertex, next);
            let Some((incoming, outgoing)) = incoming.zip(outgoing) else {
                return (*vertex, *vertex);
            };
            let cosine = (incoming.0 * outgoing.0 + incoming.1 * outgoing.1).clamp(-1.0, 1.0);
            if 1.0 - cosine <= f64::EPSILON {
                return (*vertex, *vertex);
            }
            let distance = f64::from(radius) * ((1.0 + cosine) / (1.0 - cosine)).sqrt();
            let endpoint = |direction: (f64, f64)| {
                Point::new(
                    (f64::from(vertex.x) + direction.0 * distance) as f32,
                    (f64::from(vertex.y) + direction.1 * distance) as f32,
                )
            };
            (endpoint(incoming), endpoint(outgoing))
        })
        .collect::<Vec<_>>();
    let mut builder = PathBuilder::new();
    builder.move_to(corners[0].0);
    for (start, end) in corners {
        builder.line_to(start);
        builder.arc_to_radius(
            (radius, radius),
            0.0,
            ArcSize::Small,
            PathDirection::CW,
            end,
        );
    }
    builder.close();
    builder.detach()
}

fn unit(from: Point, to: Point) -> Option<(f64, f64)> {
    let x = f64::from(to.x) - f64::from(from.x);
    let y = f64::from(to.y) - f64::from(from.y);
    let length = x.hypot(y);
    if length > 0.0 {
        Some((x / length, y / length))
    } else {
        None
    }
}
