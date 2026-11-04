//! Large temporary rasters release their backing pages when the image is dropped.
use super::*;

pub(super) struct RasterSurface {
    // Keep the mapping alive until the native surface has been destroyed.
    surface: skia_safe::Surface,
    #[cfg(unix)]
    mapping: Option<(Backing, skia_safe::ImageInfo)>,
}

impl RasterSurface {
    pub(super) fn new(size: (i32, i32)) -> Option<Self> {
        #[cfg(unix)]
        if let Some(length) = usize::try_from(size.0)
            .ok()
            .and_then(|width| {
                usize::try_from(size.1)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            && length >= 4 * 1024 * 1024
            && let Some(mut mapping) = Backing::new(length)
        {
            let info = skia_safe::ImageInfo::new_n32_premul(size, None);
            let row_bytes = size.0 as usize * 4;
            // The native clone stays inside this owner. Only canvas borrows are
            // exposed, and finish consumes the owner before exposing its pixels.
            let surface = {
                let borrowed =
                    skia_safe::surfaces::wrap_pixels(&info, mapping.pixels(), row_bytes, None)?;
                (*borrowed).clone()
            };
            return Some(Self {
                surface,
                mapping: Some((mapping, info)),
            });
        }
        Some(Self {
            surface: skia_safe::surfaces::raster_n32_premul(size)?,
            #[cfg(unix)]
            mapping: None,
        })
    }

    pub(super) fn canvas(&mut self) -> &Canvas {
        self.surface.canvas()
    }

    pub(super) fn finish(mut self) -> Image {
        #[cfg(unix)]
        if let Some((mapping, info)) = &self.mapping
            && let Some(data) = Data::from_filename(mapping.file.path())
            && let Some(image) =
                skia_safe::images::raster_from_data(info, data, info.min_row_bytes())
        {
            // No writer survives this return. The immutable image owns a second
            // mapping, so worker completion never copies a full-sized bitmap.
            return image;
        }
        self.surface.image_snapshot()
    }
}

#[cfg(unix)]
struct Backing {
    file: tempfile::NamedTempFile,
    address: std::ptr::NonNull<u8>,
    length: usize,
}

#[cfg(unix)]
impl Backing {
    fn new(length: usize) -> Option<Self> {
        use std::os::fd::AsRawFd;
        let file = tempfile::NamedTempFile::new().ok()?;
        file.as_file().set_len(length as u64).ok()?;
        // A newly extended private file supplies zero-initialized pixels.
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if address == libc::MAP_FAILED {
            return None;
        }
        let Some(address) = std::ptr::NonNull::new(address.cast()) else {
            unsafe {
                libc::munmap(address, length);
            }
            return None;
        };
        Some(Self {
            file,
            address,
            length,
        })
    }

    fn pixels(&mut self) -> &mut [u8] {
        // This exclusive borrow is used only while constructing the surface.
        // Later writes go through the native surface, which drops before us.
        unsafe { std::slice::from_raw_parts_mut(self.address.as_ptr(), self.length) }
    }
}

#[cfg(unix)]
impl Drop for Backing {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.address.as_ptr().cast(), self.length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_raster_snapshots_keep_native_pixels_after_the_writer_is_destroyed() {
        let size = (1100, 1000);
        let mut expected = skia_safe::surfaces::raster_n32_premul(size).unwrap();
        let mut mapped = RasterSurface::new(size).unwrap();
        for canvas in [expected.canvas(), mapped.canvas()] {
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
}
