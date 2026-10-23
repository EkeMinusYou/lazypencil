//! Native Coons meshes. Adjacent patches share subdivision counts and boundary curves.
use super::*;
use skia_safe::{SamplingOptions, Vertices, vertices::VertexMode};

const MAX_GRID_SIDE: usize = 64;
const MAX_SUBDIVISIONS: usize = 2048;
const MAX_VERTICES: usize = 2 * 1024 * 1024;
const GEOMETRY_TOLERANCE: f64 = 0.25;

type Vec2 = [f64; 2];

fn add(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: Vec2, b: f64) -> Vec2 {
    [a[0] * b, a[1] * b]
}
fn lerp(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    add(mul(a, 1.0 - t), mul(b, t))
}
fn length(a: Vec2) -> f64 {
    a[0].hypot(a[1])
}
fn pair(value: &Value) -> Result<Vec2> {
    let values = value
        .as_array()
        .filter(|v| v.len() == 2)
        .ok_or_else(|| invalid("A mesh position or handle must contain exactly two numbers"))?;
    let mut result = [0.0; 2];
    for (output, value) in result.iter_mut().zip(values) {
        *output = value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("Mesh coordinates must be finite numbers"))?;
    }
    Ok(result)
}
fn side(fill: &Value, name: &str) -> Result<usize> {
    match fill.get(name) {
        None => Ok(2),
        Some(value) => value
            .as_f64()
            .filter(|n| n.fract() == 0.0 && (2.0..=MAX_GRID_SIDE as f64).contains(n))
            .map(|n| n as usize)
            .ok_or_else(|| invalid("Mesh columns and rows must be integers within 2..64")),
    }
}

struct Vertex {
    position: Vec2,
    handles: [Option<Vec2>; 4], // left, right, top, bottom; normalized relative offsets
    color: [f64; 4],            // premultiplied sRGB and alpha
}

struct Mesh {
    columns: usize,
    rows: usize,
    vertices: Vec<Vertex>,
}
impl Mesh {
    fn parse(fill: &Value) -> Result<Self> {
        let columns = side(fill, "columns")?;
        let rows = side(fill, "rows")?;
        let count = columns * rows;
        let colors = fill["colors"]
            .as_array()
            .filter(|colors| colors.len() == count)
            .ok_or_else(|| invalid("A mesh requires columns * rows colors in row-major order"))?;
        let points = fill
            .get("points")
            .map(|points| {
                points
                    .as_array()
                    .filter(|points| points.len() == count)
                    .ok_or_else(|| invalid("Mesh points must contain columns * rows entries"))
            })
            .transpose()?;
        let mut vertices = Vec::with_capacity(count);
        for (i, color) in colors.iter().enumerate() {
            let color = color
                .as_str()
                .ok_or_else(|| invalid("Each mesh vertex requires a resolved color string"))?;
            let color = color::parse(color).map_err(|_| {
                invalid("Invalid mesh vertex color; expected #RGB, #RRGGBB, or #RRGGBBAA")
                    .details(json!({"vertex_index":i}))
            })?;
            let alpha = color.a() as f64 / 255.0;
            let default = [
                (i % columns) as f64 / (columns - 1) as f64,
                (i / columns) as f64 / (rows - 1) as f64,
            ];
            let mut vertex = Vertex {
                position: default,
                handles: [None; 4],
                color: [
                    color.r() as f64 / 255.0 * alpha,
                    color.g() as f64 / 255.0 * alpha,
                    color.b() as f64 / 255.0 * alpha,
                    alpha,
                ],
            };
            if let Some(points) = points {
                let point = &points[i];
                if let Some(object) = point.as_object() {
                    vertex.position = pair(
                        object
                            .get("position")
                            .ok_or_else(|| invalid("A mesh point object requires position"))?,
                    )?;
                    for (index, name) in ["leftHandle", "rightHandle", "topHandle", "bottomHandle"]
                        .iter()
                        .enumerate()
                    {
                        vertex.handles[index] = object.get(*name).map(pair).transpose()?;
                    }
                } else {
                    vertex.position = pair(point)?;
                }
                if vertex.position.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    return Err(invalid("Mesh vertex positions must be within 0..1")
                        .details(json!({"vertex_index":i})));
                }
            }
            vertices.push(vertex);
        }
        Ok(Self {
            columns,
            rows,
            vertices,
        })
    }

    fn vertex(&self, x: usize, y: usize) -> &Vertex {
        &self.vertices[y * self.columns + x]
    }

    fn control(&self, x: usize, y: usize, direction: usize) -> Vec2 {
        let vertex = self.vertex(x, y);
        let offset = vertex.handles[direction].unwrap_or_else(|| {
            let (before, after, spacing) = if direction < 2 {
                (
                    self.vertex(x.saturating_sub(1), y).position,
                    self.vertex((x + 1).min(self.columns - 1), y).position,
                    if x == 0 || x == self.columns - 1 {
                        3.0
                    } else {
                        6.0
                    },
                )
            } else {
                (
                    self.vertex(x, y.saturating_sub(1)).position,
                    self.vertex(x, (y + 1).min(self.rows - 1)).position,
                    if y == 0 || y == self.rows - 1 {
                        3.0
                    } else {
                        6.0
                    },
                )
            };
            mul(
                sub(after, before),
                if direction.is_multiple_of(2) {
                    -1.0
                } else {
                    1.0
                } / spacing,
            )
        });
        add(vertex.position, offset)
    }

    fn patch(&self, x: usize, y: usize, width: f64, height: f64) -> Patch {
        let scale = |point: Vec2| [point[0] * width, point[1] * height];
        let horizontal = |y| {
            [
                self.vertex(x, y).position,
                self.control(x, y, 1),
                self.control(x + 1, y, 0),
                self.vertex(x + 1, y).position,
            ]
            .map(scale)
        };
        let vertical = |x| {
            [
                self.vertex(x, y).position,
                self.control(x, y, 3),
                self.control(x, y + 1, 2),
                self.vertex(x, y + 1).position,
            ]
            .map(scale)
        };
        Patch {
            top: horizontal(y),
            bottom: horizontal(y + 1),
            left: vertical(x),
            right: vertical(x + 1),
            colors: [
                self.vertex(x, y).color,
                self.vertex(x + 1, y).color,
                self.vertex(x, y + 1).color,
                self.vertex(x + 1, y + 1).color,
            ],
        }
    }
}

struct Patch {
    top: [Vec2; 4],
    bottom: [Vec2; 4],
    left: [Vec2; 4],
    right: [Vec2; 4],
    colors: [[f64; 4]; 4],
}
fn cubic(curve: &[Vec2; 4], t: f64) -> Vec2 {
    let a = lerp(curve[0], curve[1], t);
    let b = lerp(curve[1], curve[2], t);
    let c = lerp(curve[2], curve[3], t);
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}
fn second_derivative(curve: &[Vec2; 4]) -> f64 {
    (0..2)
        .map(|i| length(add(sub(curve[i], mul(curve[i + 1], 2.0)), curve[i + 2])) * 6.0)
        .fold(0.0, f64::max)
}
fn derivative_difference(a: &[Vec2; 4], b: &[Vec2; 4]) -> f64 {
    (0..3)
        .map(|i| length(sub(sub(a[i + 1], a[i]), sub(b[i + 1], b[i]))) * 3.0)
        .fold(0.0, f64::max)
}
fn strip_rows(nx: usize, ny: usize) -> usize {
    (65536 / (nx + 1) - 1).min(ny)
}
impl Patch {
    fn level(&self) -> Result<usize> {
        // JSON can hold finite offsets whose scaled control points overflow. Reject
        // these before derivative maxima could discard NaN or coordinates reach Skia.
        if self
            .top
            .iter()
            .chain(&self.bottom)
            .chain(&self.left)
            .chain(&self.right)
            .flatten()
            .any(|value| !value.is_finite() || value.abs() > f32::MAX as f64)
        {
            return Err(complexity());
        }
        let uu = second_derivative(&self.top).max(second_derivative(&self.bottom));
        let vv = second_derivative(&self.left).max(second_derivative(&self.right));
        let corner_cross = add(
            sub(self.top[0], self.top[3]),
            sub(self.bottom[3], self.bottom[0]),
        );
        let uv = derivative_difference(&self.bottom, &self.top)
            + derivative_difference(&self.right, &self.left)
            + length(corner_cross);
        // Taylor's remainder bounds each triangle's geometric interpolation error.
        let geometry = ((uu + vv + 2.0 * uv) / (2.0 * GEOMETRY_TOLERANCE))
            .sqrt()
            .ceil();
        let color_cross = (0..4)
            .map(|c| {
                (self.colors[0][c] - self.colors[1][c] - self.colors[2][c] + self.colors[3][c])
                    .abs()
            })
            .fold(0.0, f64::max);
        // Bilinear color versus the two affine triangles: at most 1/4 of an 8-bit step.
        let color = (255.0 * color_cross).sqrt().ceil();
        let level = geometry.max(color).max(1.0);
        if !level.is_finite() || level > MAX_SUBDIVISIONS as f64 {
            return Err(complexity());
        }
        Ok(level as usize)
    }

    fn position(&self, u: f64, v: f64) -> Point {
        // Evaluate shared boundaries identically on both sides, without cancellation.
        let point = if u == 0.0 {
            cubic(&self.left, v)
        } else if u == 1.0 {
            cubic(&self.right, v)
        } else if v == 0.0 {
            cubic(&self.top, u)
        } else if v == 1.0 {
            cubic(&self.bottom, u)
        } else {
            let horizontal = lerp(cubic(&self.top, u), cubic(&self.bottom, u), v);
            let vertical = lerp(cubic(&self.left, v), cubic(&self.right, v), u);
            let corners = lerp(
                lerp(self.top[0], self.top[3], u),
                lerp(self.bottom[0], self.bottom[3], u),
                v,
            );
            sub(add(horizontal, vertical), corners)
        };
        Point::new(point[0] as f32, point[1] as f32)
    }

    fn color(&self, u: f64, v: f64) -> Color {
        let value: [f64; 4] = std::array::from_fn(|c| {
            (self.colors[0][c] * (1.0 - u) + self.colors[1][c] * u) * (1.0 - v)
                + (self.colors[2][c] * (1.0 - u) + self.colors[3][c] * u) * v
        });
        if value[3] == 0.0 {
            return Color::TRANSPARENT;
        }
        Color::from_argb(
            (value[3] * 255.0).round() as u8,
            (value[0] / value[3] * 255.0).round() as u8,
            (value[1] / value[3] * 255.0).round() as u8,
            (value[2] / value[3] * 255.0).round() as u8,
        )
    }

    fn draw(
        &self,
        canvas: &Canvas,
        nx: usize,
        ny: usize,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let stride = nx + 1;
        let strip_rows = strip_rows(nx, ny);
        let mut paint = Paint::default();
        paint.set_color(Color::WHITE).set_blend_mode(BlendMode::Src);
        for start in (0..ny).step_by(strip_rows) {
            check()?;
            let end = (start + strip_rows).min(ny);
            let mut positions = Vec::with_capacity((end - start + 1) * stride);
            let mut colors = Vec::with_capacity(positions.capacity());
            for row in start..=end {
                let v = row as f64 / ny as f64;
                for column in 0..=nx {
                    let u = column as f64 / nx as f64;
                    positions.push(self.position(u, v));
                    colors.push(self.color(u, v));
                }
            }
            let mut indices = Vec::with_capacity((end - start) * nx * 6);
            for row in 0..end - start {
                for column in 0..nx {
                    let a = (row * stride + column) as u16;
                    let b = a + 1;
                    let c = ((row + 1) * stride + column) as u16;
                    indices.extend_from_slice(&[a, b, c, b, c + 1, c]);
                }
            }
            // The public constructor requires texture coordinates even without a texture.
            let vertices = Vertices::new_copy(
                VertexMode::Triangles,
                &positions,
                &positions,
                &colors,
                Some(&indices),
            );
            canvas.draw_vertices(&vertices, BlendMode::Dst, &paint);
        }
        Ok(())
    }
}

fn complexity() -> Error {
    Error::new("RESOURCE_LIMIT", "Mesh tessellation exceeds the native render budget; reduce scale, grid density, or handle curvature")
        .details(json!({"max_subdivisions":MAX_SUBDIVISIONS,"max_vertices":MAX_VERTICES}))
}

pub(super) fn draw(
    renderer: &mut Renderer<'_>,
    node: &ResolvedNode,
    fill: &Value,
    shape: &Path,
    rect: SkRect,
    property: &str,
) -> Result<()> {
    renderer.project.check_cancelled()?;
    let mesh = Mesh::parse(fill)?;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_blender(blending::paint(fill, node, property)?);
    paint.set_alpha_f(unit(fill.get("opacity"), 1.0)?);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return Ok(());
    }
    let (width, height) = raster_dimensions(
        (rect.width() as f64 * renderer.raster_scale as f64)
            .ceil()
            .max(1.0) as u64,
        (rect.height() as f64 * renderer.raster_scale as f64)
            .ceil()
            .max(1.0) as u64,
    )?;
    let mut columns = vec![1; mesh.columns - 1];
    let mut rows = vec![1; mesh.rows - 1];
    let mut patches = Vec::with_capacity(columns.len() * rows.len());
    for (y, row) in rows.iter_mut().enumerate() {
        for (x, column) in columns.iter_mut().enumerate() {
            let patch = mesh.patch(x, y, width as f64, height as f64);
            let level = patch.level()?;
            *column = (*column).max(level);
            *row = (*row).max(level);
            patches.push(patch);
        }
    }
    let count = rows.iter().try_fold(0_usize, |total, &ny| {
        columns.iter().try_fold(total, |total, &nx| {
            // Include repeated boundary vertices when patches span several strips.
            let generated = (nx + 1) * (ny + ny.div_ceil(strip_rows(nx, ny)));
            total
                .checked_add(generated)
                .filter(|count| *count <= MAX_VERTICES)
                .ok_or_else(complexity)
        })
    })?;
    // Reserve the raster and both Rust/native vertex/index buffers before allocation.
    let _lease = renderer
        .rasters
        .reserve(width as u64 * height as u64 * 4 + count as u64 * 80)?;
    let mut surface = skia_safe::surfaces::raster_n32_premul((width, height))
        .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot allocate a mesh raster"))?;
    surface.canvas().clear(Color::TRANSPARENT);
    for (y, ny) in rows.iter().enumerate() {
        for (x, nx) in columns.iter().enumerate() {
            patches[y * columns.len() + x].draw(surface.canvas(), *nx, *ny, || {
                renderer.project.check_cancelled()?;
                if let Some(runtime) = renderer.runtime.as_deref() {
                    runtime.check()?;
                }
                Ok(())
            })?;
        }
    }
    let local = matrix(
        Affine::translation(rect.left(), rect.top()).then(Affine::scale(
            rect.width() / width as f32,
            rect.height() / height as f32,
        )),
    );
    paint.set_shader(surface.image_snapshot().to_shader(
        (TileMode::Clamp, TileMode::Clamp),
        SamplingOptions::from(skia_safe::FilterMode::Linear),
        Some(&local),
    ));
    renderer.canvas.draw_path(shape, &paint);
    Ok(())
}
