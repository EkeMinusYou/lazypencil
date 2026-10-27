//! Owned pixels for terminal previews; PNG encoding is deferred until export.
use super::*;

pub(crate) struct PreviewPixels {
    width: u32,
    height: u32,
    // Preserve premultiplied pixels for on-demand PNG export. No Skia object
    // crosses the preview worker's thread boundary.
    rgba: Vec<u8>,
    // Only the displayed frame retains a reusable straight-alpha copy. Large
    // windows keep the ordinary render path without another large allocation.
    camera_pixels: Option<image::RgbaImage>,
}

impl PreviewPixels {
    pub(super) fn from_image(image: &Image) -> Result<(Self, image::DynamicImage)> {
        let width = image.width() as u32;
        let height = image.height() as u32;
        let info = skia_safe::ImageInfo::new(
            (width as i32, height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let mut rgba = vec![0_u8; width as usize * height as usize * 4];
        if !image.read_pixels(
            &info,
            &mut rgba,
            width as usize * 4,
            (0, 0),
            skia_safe::image::CachingHint::Disallow,
        ) {
            return Err(Error::new("RENDER_ERROR", "Cannot copy preview pixels"));
        }
        let mut bitmap = image::RgbaImage::new(width, height);
        let unpremul = info.with_alpha_type(skia_safe::AlphaType::Unpremul);
        if !image.read_pixels(
            &unpremul,
            bitmap.as_mut(),
            width as usize * 4,
            (0, 0),
            skia_safe::image::CachingHint::Disallow,
        ) {
            return Err(Error::new("RENDER_ERROR", "Cannot convert preview pixels"));
        }
        Ok((
            Self {
                width,
                height,
                rgba,
                camera_pixels: (u64::from(width) * u64::from(height) * 4 <= 16 * 1024 * 1024)
                    .then(|| bitmap.clone()),
            },
            image::DynamicImage::ImageRgba8(bitmap),
        ))
    }

    pub(crate) fn camera_pixels(&self) -> Option<&image::RgbaImage> {
        self.camera_pixels.as_ref()
    }

    pub(crate) fn png(&self) -> Result<Vec<u8>> {
        let info = skia_safe::ImageInfo::new(
            (self.width as i32, self.height as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let image = skia_safe::images::raster_from_data(
            &info,
            Data::new_copy(&self.rgba),
            self.width as usize * 4,
        )
        .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot restore preview pixels"))?;
        let mut encoding = skia_safe::png_encoder::Options::default();
        encoding.z_lib_level = 1;
        skia_safe::png_encoder::encode_image(None, &image, &encoding)
            .map(|data| data.as_bytes().to_vec())
            .ok_or_else(|| Error::new("RENDER_ERROR", "Cannot encode preview pixels"))
    }
}

pub(crate) struct RenderedPreview {
    pub pixels: PreviewPixels,
    pub bitmap: image::DynamicImage,
    pub dynamic_artifacts: Vec<Value>,
}

pub(crate) fn preview_with_runtime_observed(
    project: &Project,
    snapshot: &LayoutSnapshot,
    options: &RenderOptions,
    runtime: &mut crate::dynamic::DynamicRuntime,
    cancel: Option<tokio_util::sync::CancellationToken>,
    mut observer: Option<&mut dyn FnMut(&'static str)>,
) -> Result<RenderedPreview> {
    runtime.configure(project.runtime_settings())?;
    runtime.begin_render(cancel, options.refresh);
    if options.refresh_pages {
        runtime.refresh_pages();
    }
    let result = (|| {
        let raster = raster_impl(project, snapshot, options, Some(&mut *runtime))?;
        if let Some(observer) = observer.as_mut() {
            observer("render_raster_ms");
        }
        let (pixels, bitmap) = PreviewPixels::from_image(&raster.image)?;
        if let Some(observer) = observer.as_mut() {
            observer("render_pixel_copy_ms");
        }
        runtime.check()?;
        Ok(RenderedPreview {
            pixels,
            bitmap,
            dynamic_artifacts: raster.dynamic_artifacts,
        })
    })();
    runtime.render_result(result)
}
