//! Own large native raster pixels until the last immutable image releases them.
use super::*;
#[cfg(target_os = "macos")]
use std::sync::Arc;

pub(super) struct RasterSurface {
    // Destroy the external-pixel surface before releasing its pixel owner.
    surface: skia_safe::Surface,
    #[cfg(target_os = "macos")]
    pixels: Option<Arc<Vec<u8>>>,
}

impl RasterSurface {
    pub(super) fn new(size: (i32, i32)) -> Option<Self> {
        #[cfg(target_os = "macos")]
        if let Some(length) = usize::try_from(size.0)
            .ok()
            .and_then(|width| {
                usize::try_from(size.1)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            && length >= 4 * 1024 * 1024
        {
            let mut pixels = Arc::new(vec![0; length]);
            let info = skia_safe::ImageInfo::new_n32_premul(size, None);
            let surface = {
                let borrowed = skia_safe::surfaces::wrap_pixels(
                    &info,
                    Arc::get_mut(&mut pixels)?.as_mut_slice(),
                    info.min_row_bytes(),
                    None,
                )?;
                (*borrowed).clone()
            };
            return Some(Self {
                surface,
                pixels: Some(pixels),
            });
        }
        Some(Self {
            surface: skia_safe::surfaces::raster_n32_premul(size)?,
            #[cfg(target_os = "macos")]
            pixels: None,
        })
    }

    pub(super) fn canvas(&mut self) -> Option<&Canvas> {
        #[cfg(target_os = "macos")]
        if self
            .pixels
            .as_mut()
            .is_some_and(|pixels| Arc::get_mut(pixels).is_none())
        {
            // A native image still owns this buffer. Never overwrite its pixels.
            *self = Self::new((self.surface.width(), self.surface.height()))?;
        }
        Some(self.surface.canvas())
    }

    pub(super) fn snapshot(&mut self) -> Image {
        #[cfg(target_os = "macos")]
        if let Some(pixels) = &self.pixels {
            let mut bitmap = skia_safe::Bitmap::new();
            // The transparent wrapper matches the pinned native bitmap layout;
            // the bridge also checks its size/alignment before touching it.
            let installed = unsafe {
                C_Lazypencil_installRasterBitmap(
                    (&mut bitmap as *mut skia_safe::Bitmap).cast(),
                    std::mem::size_of::<skia_safe::Bitmap>(),
                    std::mem::align_of::<skia_safe::Bitmap>(),
                    self.surface.width(),
                    self.surface.height(),
                    pixels.as_ref().as_ptr().cast_mut().cast(),
                    self.surface.width() as usize * 4,
                    release_pixels,
                    Arc::into_raw(pixels.clone()).cast_mut().cast(),
                )
            };
            if installed {
                bitmap.set_immutable();
                if let Some(image) = skia_safe::images::raster_from_bitmap(&bitmap) {
                    return image;
                }
            }
        }
        self.surface.image_snapshot()
    }

    pub(super) fn finish(mut self) -> Image {
        self.snapshot()
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn C_Lazypencil_installRasterBitmap(
        bitmap: *mut std::ffi::c_void,
        bitmap_size: usize,
        bitmap_alignment: usize,
        width: i32,
        height: i32,
        pixels: *mut std::ffi::c_void,
        row_bytes: usize,
        release: unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void),
        context: *mut std::ffi::c_void,
    ) -> bool;
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn release_pixels(_: *mut std::ffi::c_void, context: *mut std::ffi::c_void) {
    // Native PixelRef retains this one Arc until all image/bitmap clones are gone.
    unsafe { drop(Arc::<Vec<u8>>::from_raw(context.cast())) };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_raster_snapshots_keep_native_pixels_after_the_writer_is_destroyed() {
        let size = (1100, 1000);
        let mut expected = skia_safe::surfaces::raster_n32_premul(size).unwrap();
        let mut mapped = RasterSurface::new(size).unwrap();
        for canvas in [expected.canvas(), mapped.canvas().unwrap()] {
            canvas.clear(Color::TRANSPARENT);
            let mut paint = Paint::default();
            paint
                .set_color(Color::from_argb(173, 12, 34, 56))
                .set_anti_alias(true);
            canvas.draw_circle((550.5, 498.25), 390.75, &paint);
        }
        let expected = expected.image_snapshot();
        let actual = mapped.finish();
        let encode = |image: &Image| {
            skia_safe::png_encoder::encode_image(None, image, &Default::default()).unwrap()
        };
        assert_eq!(encode(&actual).as_bytes(), encode(&expected).as_bytes());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_images_keep_their_pixels_when_the_writer_reuses_or_releases_storage() {
        let mut surface = RasterSurface::new((1100, 1000)).unwrap();
        surface.canvas().unwrap().clear(Color::RED);
        let storage = Arc::downgrade(surface.pixels.as_ref().unwrap());
        let red = surface.snapshot();
        assert!(
            storage.strong_count() >= 2,
            "Native images must retain their pixel owner"
        );
        surface.canvas().unwrap().clear(Color::BLUE);
        let blue = surface.finish();
        let clone = red.clone();
        drop(red);
        std::thread::spawn(move || {
            let info = skia_safe::ImageInfo::new(
                (1, 1),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Unpremul,
                None,
            );
            for (image, expected) in [(clone, [255, 0, 0, 255]), (blue, [0, 0, 255, 255])] {
                let mut pixel = [0_u8; 4];
                assert!(image.read_pixels(
                    &info,
                    &mut pixel,
                    4,
                    (0, 0),
                    skia_safe::image::CachingHint::Disallow,
                ));
                assert_eq!(pixel, expected);
            }
        })
        .join()
        .unwrap();
        assert!(
            storage.upgrade().is_none(),
            "The last image must release its pixels"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_incompatible_native_bitmap_layout_releases_the_transferred_owner() {
        let pixels = Arc::new(vec![0_u8; 16]);
        let mut bitmap = skia_safe::Bitmap::new();
        let accepted = unsafe {
            C_Lazypencil_installRasterBitmap(
                (&mut bitmap as *mut skia_safe::Bitmap).cast(),
                0,
                0,
                2,
                2,
                pixels.as_ref().as_ptr().cast_mut().cast(),
                8,
                release_pixels,
                Arc::into_raw(pixels.clone()).cast_mut().cast(),
            )
        };
        assert!(!accepted);
        assert_eq!(Arc::strong_count(&pixels), 1);
    }
}
