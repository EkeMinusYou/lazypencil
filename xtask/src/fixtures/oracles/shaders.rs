//! Evaluate the authored fixture equations directly, without a GLSL parser.

use super::{Raster, unorm};

#[derive(Clone, Copy)]
pub enum Scene {
    Defaults,
    AnnotationBinding,
    LogicalCoordinates,
}

pub fn raster(scene: Scene, scale: u32) -> Raster {
    assert!((1..=2).contains(&scale));
    let (width, height) = match scene {
        Scene::Defaults => (280, 20),
        Scene::AnnotationBinding => (160, 120),
        Scene::LogicalCoordinates => (400, 20),
    };
    let mut image = Raster::solid(width * scale, height * scale, [255; 4]);
    for y in 0..image.height {
        for x in 0..image.width {
            let color = match scene {
                Scene::Defaults => defaults(x / scale / 20),
                Scene::AnnotationBinding => {
                    let (lx, ly) = (x / scale, y / scale);
                    if lx % 40 >= 32 || ly % 24 >= 16 {
                        continue;
                    }
                    annotation(ly / 24 * 4 + lx / 40)
                }
                Scene::LogicalCoordinates => {
                    let column = x / (40 * scale);
                    let local_x = x % (40 * scale);
                    if local_x >= 32 * scale || y >= 16 * scale {
                        continue;
                    }
                    match column {
                        // A builtin name is not a preprocessor definition.
                        5 => [0, 255, 0, 255],
                        // Object replacement and a macro parameter both use
                        // the literal vec4(16,8,0,1), hence 0.5 in each axis.
                        6 | 7 => [unorm(0.5), unorm(0.5), 0, 255],
                        _ => [
                            unorm((local_x as f64 + 0.5) / (scale as f64 * 32.0)),
                            unorm((16.0 * scale as f64 - y as f64 - 0.5) / (scale as f64 * 16.0)),
                            if column == 9 { unorm(0.25) } else { 0 },
                            255,
                        ],
                    }
                }
            };
            image.set(x, y, color);
        }
    }
    image
}

fn defaults(column: u32) -> [u8; 4] {
    match column {
        0 | 6 => [unorm(0.75), 0, 0, 255],
        1 | 7 | 8 | 12 => [unorm(0.25), 0, 0, 255],
        2 | 3 | 13 => [0, 0, 0, 255],
        4 | 9 => [0x3a, 0x7f, 0xc1, 255],
        5 => [0x20, 0x60, 0xa0, 255],
        10 => [unorm(0.25), unorm(0.75), 0, 255],
        11 => [unorm(0.5), unorm(0.5), 0, 255],
        _ => panic!("Unknown authored default panel"),
    }
}

fn annotation(index: u32) -> [u8; 4] {
    match index {
        0 | 7..=10 => [unorm(32.0 / 128.0), unorm(16.0 / 64.0), 0, 255],
        // The grouped declaration shares @resolution across size and other.
        13 => [unorm(64.0 / 128.0), unorm(32.0 / 64.0), 0, 255],
        14 | 18 | 19 => [unorm(0.75), 0, 0, 255],
        1..=19 => [0, 0, 0, 255],
        _ => panic!("Unknown authored annotation panel"),
    }
}
