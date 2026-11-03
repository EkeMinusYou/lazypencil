//! Bounded full-resolution static-scene rasters; cropping never interpolates pixels.
use super::*;
use std::io::{Read, Write};

const CAPACITY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 8;
const MAX_ATLAS_PIXELS: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    picture: u32,
    phase_x: u32,
    phase_y: u32,
    background: u32,
}

struct Entry {
    image: StoredImage,
    left: i32,
    top: i32,
    bytes: u64,
    used: u64,
}

// Device pixels are compressed without color conversion. Vector atlases keep
// their native image so their frequent integer crops need no decompression.
enum StoredImage {
    Native(Image),
    Deflated {
        width: i32,
        height: i32,
        bytes: Vec<u8>,
    },
}

impl StoredImage {
    fn compact(image: Image) -> Self {
        let (width, height) = (image.width(), image.height());
        let info = skia_safe::ImageInfo::new_n32_premul((width, height), None);
        let mut pixels = vec![0; width as usize * height as usize * 4];
        if image.read_pixels(
            &info,
            &mut pixels,
            width as usize * 4,
            (0, 0),
            skia_safe::image::CachingHint::Disallow,
        ) {
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            if encoder.write_all(&pixels).is_ok()
                && let Ok(bytes) = encoder.finish()
                && bytes.len() < pixels.len()
            {
                return Self::Deflated {
                    width,
                    height,
                    bytes,
                };
            }
        }
        Self::Native(image)
    }

    fn image(&self) -> Option<Image> {
        match self {
            Self::Native(image) => Some(image.clone()),
            Self::Deflated {
                width,
                height,
                bytes,
            } => {
                let mut pixels = vec![0; *width as usize * *height as usize * 4];
                flate2::read::ZlibDecoder::new(bytes.as_slice())
                    .read_exact(&mut pixels)
                    .ok()?;
                skia_safe::images::raster_from_data(
                    &skia_safe::ImageInfo::new_n32_premul((*width, *height), None),
                    Data::new_copy(&pixels),
                    *width as usize * 4,
                )
            }
        }
    }

    fn bytes(&self) -> u64 {
        match self {
            Self::Native(image) => image.width() as u64 * image.height() as u64 * 4,
            Self::Deflated { bytes, .. } => bytes.len() as u64,
        }
    }
}

#[derive(Default)]
pub(super) struct AtlasCache {
    entries: BTreeMap<Key, Entry>,
    clock: u64,
}

impl AtlasCache {
    fn viewport_key(picture: &skia_safe::Picture, background: Color) -> Key {
        Key {
            picture: picture.unique_id(),
            phase_x: 0.0_f32.to_bits(),
            phase_y: 0.0_f32.to_bits(),
            background: u32::from_be_bytes([
                background.a(),
                background.r(),
                background.g(),
                background.b(),
            ]),
        }
    }

    pub(super) fn viewport(
        &mut self,
        picture: &skia_safe::Picture,
        background: Color,
    ) -> Option<Image> {
        self.clock = self.clock.saturating_add(1);
        let entry = self
            .entries
            .get_mut(&Self::viewport_key(picture, background))?;
        entry.used = self.clock;
        entry.image.image()
    }

    pub(super) fn retain_viewport(
        &mut self,
        picture: &skia_safe::Picture,
        background: Color,
        image: Image,
    ) {
        let (width, height) = (image.width(), image.height());
        if background.a() != 255
            || width > 4096
            || height > 4096
            || width as u64 * height as u64 > MAX_ATLAS_PIXELS
        {
            return;
        }
        let image = StoredImage::compact(image);
        let bytes = image.bytes();
        while self.entries.len() >= MAX_ENTRIES
            || self.entries.values().map(|entry| entry.bytes).sum::<u64>() + bytes > CAPACITY_BYTES
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.clock = self.clock.saturating_add(1);
        self.entries.insert(
            Self::viewport_key(picture, background),
            Entry {
                image,
                left: 0,
                top: 0,
                bytes,
                used: self.clock,
            },
        );
    }

    pub(super) fn draw(
        &mut self,
        canvas: &Canvas,
        picture: &skia_safe::Picture,
        transform: &Matrix,
        background: Color,
        budget: &Rc<backdrop::RasterBudget>,
        pool: &mut raster_pool::RasterPool,
    ) -> Result<bool> {
        // Transparent rendering retains the established layer/viewport semantics.
        if background.a() != 255 {
            return Ok(false);
        }
        let (dx, dy) = (transform.translate_x(), transform.translate_y());
        let bounds = picture.cull_rect();
        let coordinates = [
            dx,
            dy,
            bounds.left(),
            bounds.top(),
            bounds.right(),
            bounds.bottom(),
        ];
        if coordinates
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 8_000_000.0)
        {
            return Ok(false);
        }
        let (ix, iy) = (dx.floor() as i32, dy.floor() as i32);
        let (px, py) = (dx - ix as f32, dy - iy as f32);
        let key = Key {
            picture: picture.unique_id(),
            phase_x: px.to_bits(),
            phase_y: py.to_bits(),
            background: u32::from_be_bytes([
                background.a(),
                background.r(),
                background.g(),
                background.b(),
            ]),
        };
        self.clock = self.clock.saturating_add(1);
        if !self.entries.contains_key(&key) {
            // Preserve the current viewport's device origin when the whole scene
            // fits. Extend only to capture content outside that viewport.
            let viewport = canvas.base_layer_size();
            let left = ((bounds.left() + dx).floor() as i32).min(0);
            let top = ((bounds.top() + dy).floor() as i32).min(0);
            let right = ((bounds.right() + dx).ceil() as i32).max(viewport.width);
            let bottom = ((bounds.bottom() + dy).ceil() as i32).max(viewport.height);
            let (width, height) = (right - left, bottom - top);
            if width <= 0
                || height <= 0
                || width > 4096
                || height > 4096
                || width as u64 * height as u64 > MAX_ATLAS_PIXELS
            {
                return Ok(false);
            }
            let bytes = width as u64 * height as u64 * 4;
            while self.entries.len() >= MAX_ENTRIES
                || self.entries.values().map(|entry| entry.bytes).sum::<u64>() + bytes
                    > CAPACITY_BYTES
            {
                let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(key, _)| *key)
                else {
                    break;
                };
                self.entries.remove(&oldest);
            }
            let retained = self.entries.values().map(|entry| entry.bytes).sum::<u64>();
            let _reservation = budget.reserve(retained + bytes * 3)?;
            let mut surface = skia_safe::surfaces::raster_n32_premul((width, height))
                .ok_or_else(|| invalid("Cannot allocate retained preview raster"))?;
            surface.canvas().clear(background);
            let translation = Matrix::translate((dx - left as f32, dy - top as f32));
            pool.draw(
                surface.canvas(),
                picture,
                &translation,
                (width, height),
                background,
                budget,
            )?;
            let image = surface.image_snapshot();
            // Warm crops need no raster workers or their backing surfaces.
            *pool = Default::default();
            self.entries.insert(
                key,
                Entry {
                    image: StoredImage::Native(image),
                    left: left - ix,
                    top: top - iy,
                    bytes,
                    used: self.clock,
                },
            );
        }
        let _reservation = budget.reserve(self.entries.values().map(|entry| entry.bytes).sum())?;
        let entry = self
            .entries
            .get_mut(&key)
            .expect("Requested raster was retained");
        entry.used = self.clock;
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::Src);
        // Exact phase keys plus integer destination coordinates preserve the native
        // pixels. Zoom changes select a different recorded picture and raster.
        let Some(image) = entry.image.image() else {
            return Ok(false);
        };
        canvas.draw_image(
            &image,
            ((entry.left + ix) as f32, (entry.top + iy) as f32),
            Some(&paint),
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_rasters_restore_exact_premultiplied_device_pixels() {
        let info = skia_safe::ImageInfo::new_n32_premul((64, 64), None);
        let pixels: Vec<u8> = (0..4096)
            .flat_map(|index| {
                if index % 2 == 0 {
                    [8, 16, 32, 64]
                } else {
                    [18, 52, 86, 255]
                }
            })
            .collect();
        let original =
            skia_safe::images::raster_from_data(&info, Data::new_copy(&pixels), 256).unwrap();
        let stored = StoredImage::compact(original);
        assert!(stored.bytes() < pixels.len() as u64 / 8);
        let restored = stored.image().unwrap();
        let mut actual = vec![0; pixels.len()];
        assert!(restored.read_pixels(
            &info,
            &mut actual,
            256,
            (0, 0),
            skia_safe::image::CachingHint::Disallow
        ));
        assert_eq!(actual, pixels);
    }

    fn checkerboard(invert: bool) -> skia_safe::Picture {
        let mut recorder = skia_safe::PictureRecorder::new();
        let canvas = recorder.begin_recording(SkRect::new(-2.0, -2.0, 130.0, 130.0), true);
        let mut paint = Paint::default();
        for y in 0..128 {
            for x in 0..128 {
                paint.set_color(if ((x + y) % 2 == 0) ^ invert {
                    Color::RED
                } else {
                    Color::BLUE
                });
                canvas.draw_rect(SkRect::from_xywh(x as f32, y as f32, 1.0, 1.0), &paint);
            }
        }
        recorder.finish_recording_as_picture(None).unwrap()
    }

    #[test]
    fn integer_crops_keep_single_pixel_detail_and_invalidate_phase_background_and_scene() {
        let mut cache = AtlasCache::default();
        let mut pool = raster_pool::RasterPool::default();
        for picture in [checkerboard(false), checkerboard(true)] {
            for background in [Color::WHITE, Color::from_rgb(248, 250, 252)] {
                for phase in [0.0, 0.25, 0.5, 0.75, 0.0] {
                    for offset in [(0.0, 0.0), (16.0, -32.0), (-80.0, 60.0), (0.0, 0.0)] {
                        let transform = Matrix::translate((offset.0 + phase, offset.1 + phase));
                        let mut expected =
                            skia_safe::surfaces::raster_n32_premul((73, 61)).unwrap();
                        let mut actual = skia_safe::surfaces::raster_n32_premul((73, 61)).unwrap();
                        expected.canvas().clear(background);
                        actual.canvas().clear(background);
                        expected
                            .canvas()
                            .draw_picture(&picture, Some(&transform), None);
                        assert!(
                            cache
                                .draw(
                                    actual.canvas(),
                                    &picture,
                                    &transform,
                                    background,
                                    &Rc::new(backdrop::RasterBudget::default()),
                                    &mut pool
                                )
                                .unwrap()
                        );
                        assert!(
                            encode_image(&expected.image_snapshot()).unwrap()
                                == encode_image(&actual.image_snapshot()).unwrap(),
                            "offset={offset:?}, phase={phase}"
                        );
                    }
                }
            }
        }
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert!(cache.entries.values().map(|entry| entry.bytes).sum::<u64>() <= CAPACITY_BYTES);
    }

    #[test]
    fn transparent_and_oversized_scenes_preserve_the_direct_raster_fallback() {
        let mut recorder = skia_safe::PictureRecorder::new();
        let canvas = recorder.begin_recording(SkRect::new(0.0, 0.0, 5000.0, 5000.0), true);
        canvas.clear(Color::RED);
        let picture = recorder.finish_recording_as_picture(None).unwrap();
        let mut cache = AtlasCache::default();
        let mut pool = raster_pool::RasterPool::default();
        let mut surface = skia_safe::surfaces::raster_n32_premul((8, 8)).unwrap();
        surface.canvas().clear(Color::BLUE);
        let before = encode_image(&surface.image_snapshot()).unwrap();
        for background in [Color::TRANSPARENT, Color::WHITE] {
            assert!(
                !cache
                    .draw(
                        surface.canvas(),
                        &picture,
                        &Matrix::new_identity(),
                        background,
                        &Rc::new(backdrop::RasterBudget::default()),
                        &mut pool
                    )
                    .unwrap()
            );
        }
        assert!(before == encode_image(&surface.image_snapshot()).unwrap());
        assert!(cache.entries.is_empty());
    }
}
