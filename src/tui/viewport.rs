//! Keep camera coordinates in display pixels while bounding raster allocations.
use super::*;

pub(super) const MAX_SIDE: u32 = 8192;
pub(super) const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub(super) struct Viewport {
    pub display: [u32; 2],
    pub raster: [u32; 2],
    pub scale: f32,
}

impl Viewport {
    pub fn new(font: ratatui_image::FontSize, cells: Size) -> Result<Self> {
        let display = [
            u32::from(cells.width) * u32::from(font.width),
            u32::from(cells.height) * u32::from(font.height),
        ];
        if display.contains(&0) {
            return Err(Error::new(
                "RENDER_ERROR",
                "Preview viewport must have nonzero dimensions",
            ));
        }
        let pixels = u64::from(display[0]) * u64::from(display[1]);
        let scale = if display.iter().all(|side| *side <= MAX_SIDE) && pixels <= MAX_PIXELS {
            1.0
        } else {
            // Leave room for f32 region/scale rounding and the renderer's ceil.
            let side = f64::from(MAX_SIDE - 2);
            let budget = (MAX_PIXELS - u64::from(4 * MAX_SIDE + 4)) as f64;
            (side / f64::from(display[0]))
                .min(side / f64::from(display[1]))
                .min((budget / pixels as f64).sqrt()) as f32
        };
        let raster =
            display.map(|side| (f64::from(side) * f64::from(scale)).ceil().max(1.0) as u32);
        Ok(Self {
            display,
            raster,
            scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_image::FontSize;

    #[test]
    fn large_viewports_preserve_display_coordinates_and_bound_raster_dimensions() {
        for (font, cells) in [
            (FontSize::new(10, 20), Size::new(80, 24)),
            (FontSize::new(10, 20), Size::new(900, 1)),
            (FontSize::new(10, 20), Size::new(1, 900)),
            (FontSize::new(32, 64), Size::new(256, 128)),
            (FontSize::new(256, 256), Size::new(u16::MAX, u16::MAX)),
        ] {
            let viewport = Viewport::new(font, cells).unwrap();
            assert_eq!(
                viewport.display,
                [
                    u32::from(font.width) * u32::from(cells.width),
                    u32::from(font.height) * u32::from(cells.height)
                ]
            );
            assert!(
                viewport
                    .raster
                    .into_iter()
                    .all(|side| (1..=MAX_SIDE).contains(&side))
            );
            assert!(u64::from(viewport.raster[0]) * u64::from(viewport.raster[1]) <= MAX_PIXELS);
            for axis in 0..2 {
                assert!(
                    (f64::from(viewport.raster[axis])
                        - f64::from(viewport.display[axis]) * f64::from(viewport.scale))
                    .abs()
                        <= 1.0
                );
            }
            if cells == Size::new(80, 24) {
                assert_eq!(viewport.scale, 1.0);
                assert_eq!(viewport.raster, viewport.display);
            } else {
                assert!(viewport.scale < 1.0);
            }
        }
        assert!(Viewport::new(FontSize::new(10, 20), Size::new(0, 1)).is_err());
    }
}
