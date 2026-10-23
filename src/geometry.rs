use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self::new(
            x,
            y,
            (self.right().min(other.right()) - x).max(0.0),
            (self.bottom().min(other.bottom()) - y).max(0.0),
        )
    }

    pub fn outset(self, x: f32, y: f32) -> Self {
        Self::new(
            self.x - x,
            self.y - y,
            self.width + 2.0 * x,
            self.height + 2.0 * y,
        )
    }

    pub fn transformed(self, matrix: Affine) -> Self {
        let points = [
            matrix.point(self.x, self.y),
            matrix.point(self.right(), self.y),
            matrix.point(self.x, self.bottom()),
            matrix.point(self.right(), self.bottom()),
        ];
        let x = points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
        let y = points.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        let right = points.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
        let bottom = points.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        Self::new(x, y, right - x, bottom - y)
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Affine(pub [f32; 6]);

impl Default for Affine {
    fn default() -> Self {
        Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }
}

impl Affine {
    pub fn translation(x: f32, y: f32) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, x, y])
    }
    pub fn rotation(degrees: f32) -> Self {
        let (s, c) = (-degrees).to_radians().sin_cos();
        Self([c, s, -s, c, 0.0, 0.0])
    }
    pub fn scale(x: f32, y: f32) -> Self {
        Self([x, 0.0, 0.0, y, 0.0, 0.0])
    }
    pub fn then(self, other: Self) -> Self {
        let [a, b, c, d, x, y] = self.0;
        let [e, f, g, h, u, v] = other.0;
        Self([
            a * e + c * f,
            b * e + d * f,
            a * g + c * h,
            b * g + d * h,
            a * u + c * v + x,
            b * u + d * v + y,
        ])
    }
    pub fn point(self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, u, v] = self.0;
        (a * x + c * y + u, b * x + d * y + v)
    }

    pub fn inverse(self) -> Option<Self> {
        let [a, b, c, d, x, y] = self.0.map(f64::from);
        let determinant = a * d - b * c;
        if !determinant.is_finite() || determinant == 0.0 {
            return None;
        }
        let values = [
            d / determinant,
            -b / determinant,
            -c / determinant,
            a / determinant,
            (c * y - d * x) / determinant,
            (b * x - a * y) / determinant,
        ]
        .map(|value| value as f32);
        values
            .iter()
            .all(|value| value.is_finite())
            .then_some(Self(values))
    }
}
