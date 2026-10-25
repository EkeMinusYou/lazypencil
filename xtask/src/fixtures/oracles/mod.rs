//! Independent mathematical expectations for the original rendering corpus.
//!
//! These helpers do not execute the application, parse its shader metadata, or
//! read images/observations produced by a proprietary application.

pub mod scripts;
pub mod shaders;
pub mod shapes;

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}

impl Raster {
    pub fn solid(width: u32, height: u32, color: [u8; 4]) -> Self {
        assert!(width > 0 && height > 0 && width <= 2048 && height <= 2048);
        Self {
            width,
            height,
            pixels: vec![color; width as usize * height as usize],
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height);
        self.pixels[(y * self.width + x) as usize]
    }

    pub(crate) fn set(&mut self, x: u32, y: u32, color: [u8; 4]) {
        assert!(x < self.width && y < self.height);
        self.pixels[(y * self.width + x) as usize] = color;
    }

    pub(crate) fn rectangle(&mut self, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
        assert!(x + width <= self.width && y + height <= self.height);
        for row in y..y + height {
            for column in x..x + width {
                self.set(column, row, color);
            }
        }
    }
}

pub(crate) fn unorm(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
