//! Owned pixels for fast terminal previews and exact on-demand PNG encoding.
use super::*;

#[derive(Clone)]
pub(crate) struct PreviewPixels {
    width: u32,
    height: u32,
    // Preserve premultiplied pixels for on-demand PNG export. No Skia object
    // crosses the preview worker's thread boundary.
    rgba: std::sync::Arc<[u8]>,
    opaque: bool,
    // Nonopaque frames need a separate straight-alpha copy. Opaque camera
    // feedback borrows rgba directly; large windows retain no camera copy.
    camera_pixels: Option<std::sync::Arc<image::RgbaImage>>,
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
        let opaque = rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255);
        let bitmap = if opaque {
            // Straight and premultiplied pixels are identical for opaque images.
            image::RgbaImage::from_raw(width, height, rgba.clone())
                .expect("Read pixels have the checked image dimensions")
        } else {
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
            bitmap
        };
        Ok((
            Self {
                width,
                height,
                rgba: rgba.into(),
                opaque,
                camera_pixels: (!opaque
                    && u64::from(width) * u64::from(height) * 4 <= 16 * 1024 * 1024)
                    .then(|| std::sync::Arc::new(bitmap.clone())),
            },
            image::DynamicImage::ImageRgba8(bitmap),
        ))
    }

    pub(crate) fn camera_pixels(&self) -> Option<image::ImageBuffer<image::Rgba<u8>, &[u8]>> {
        if self.rgba.len() > 16 * 1024 * 1024 {
            return None;
        }
        let bytes = if self.opaque {
            self.rgba.as_ref()
        } else {
            self.camera_pixels.as_ref()?.as_raw().as_slice()
        };
        image::ImageBuffer::from_raw(self.width, self.height, bytes)
    }

    pub(crate) fn identity(&self) -> usize {
        self.rgba.as_ptr() as usize
    }
    pub(crate) fn bitmap(&self) -> image::RgbaImage {
        if let Some(image) = &self.camera_pixels {
            return (**image).clone();
        }
        let mut bytes = self.rgba.to_vec();
        if !self.opaque {
            for pixel in bytes.as_chunks_mut::<4>().0 {
                let alpha = u16::from(pixel[3]);
                for channel in &mut pixel[..3] {
                    *channel = (u16::from(*channel) * 255 + alpha / 2)
                        .checked_div(alpha)
                        .unwrap_or(0)
                        .min(255) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(self.width, self.height, bytes)
            .expect("Retained pixel dimensions are valid")
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
    pub retain_raw_pixels: bool,
}

/// Geometry belongs to one captured layout and stays on the preview thread.
#[derive(Default)]
pub(crate) struct PreviewRenderer {
    cache: Option<RasterCache>,
    recordable: Option<bool>,
}

impl PreviewRenderer {
    pub(crate) fn clear(&mut self) {
        self.cache = None;
        self.recordable = None;
    }

    pub(crate) fn render(
        &mut self,
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
        fn static_value(value: &Value) -> bool {
            match value {
                Value::Object(fields) => {
                    !matches!(
                        fields.get("type").and_then(Value::as_str),
                        Some("shader" | "mesh_gradient" | "background_blur")
                    ) && fields.values().all(static_value)
                }
                Value::Array(values) => values.iter().all(static_value),
                _ => true,
            }
        }
        let cache = self.cache.get_or_insert_with(|| RasterCache::new(snapshot));
        let recordable = *self.recordable.get_or_insert_with(|| {
            snapshot.items.iter().all(|item| {
                item.node.kind() != "browser" && item.node.properties.values().all(static_value)
            }) && !cache.background_nodes.iter().any(|flag| *flag)
        });
        let result = (|| {
            let raster = raster_impl_cached(
                project,
                snapshot,
                options,
                Some(&mut *runtime),
                Some(cache),
                recordable,
            )?;
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
                retain_raw_pixels: recordable,
            })
        })();
        runtime.render_result(result)
    }
}

#[cfg(test)]
pub(crate) fn preview_with_runtime_observed(
    project: &Project,
    snapshot: &LayoutSnapshot,
    options: &RenderOptions,
    runtime: &mut crate::dynamic::DynamicRuntime,
    cancel: Option<tokio_util::sync::CancellationToken>,
    observer: Option<&mut dyn FnMut(&'static str)>,
) -> Result<RenderedPreview> {
    PreviewRenderer::default().render(project, snapshot, options, runtime, cancel, observer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_pixel_storage_preserves_export_and_borrows_opaque_camera_bytes() {
        for pixel in [[18, 52, 86, 255], [8, 16, 32, 64]] {
            let info = skia_safe::ImageInfo::new(
                (2, 2),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Premul,
                None,
            );
            let image =
                skia_safe::images::raster_from_data(&info, Data::new_copy(&pixel.repeat(4)), 8)
                    .unwrap();
            let (pixels, bitmap) = PreviewPixels::from_image(&image).unwrap();
            let camera = pixels.camera_pixels().unwrap();
            assert_eq!(camera.as_raw(), &bitmap.to_rgba8().as_raw().as_slice());
            assert_eq!(
                camera.as_raw().as_ptr() == pixels.rgba.as_ptr(),
                pixel[3] == 255
            );
            let mut options = skia_safe::png_encoder::Options::default();
            options.z_lib_level = 1;
            assert_eq!(
                pixels.png().unwrap(),
                skia_safe::png_encoder::encode_image(None, &image, &options)
                    .unwrap()
                    .as_bytes()
            );
        }
    }

    #[test]
    fn retained_geometry_preserves_exact_camera_pixels_and_clears_on_reload() {
        for images in [false, true] {
            retained_geometry_case(images);
        }
    }

    fn retained_geometry_case(images: bool) {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("design.pen");
        let asset = directory.path().join("image.png");
        let mut document = json!({"version":"2.21","children":[{
            "id":"frame","type":"frame","layout":"none","x":-30,"y":-10,
            "width":200,"height":140,"clip":true,"fill":"#F8FAFC",
            "children":[{
                "id":"path","type":"path","x":13.5,"y":7.25,"width":80,"height":60,
                "geometry":"M0 0 L80 0 L40 60 Z","fill":"#2563EB80",
                "stroke":"#000000","strokeWidth":2,
                "effect":{"type":"shadow","blur":4,"offset":{"x":2,"y":3},"color":"#00000040"}
            },{
                "id":"text","type":"text","x":20,"y":80,"content":"日本語 abc",
                "fontFamily":"Noto Sans JP","fontSize":14,"fill":"#123456"
            },{
                "id":"image","type":"rectangle","x":110.25,"y":17.5,
                "width":61.5,"height":43.25,"rotation":13,"cornerRadius":7,
                "fill":{"type":"image","url":"image.png","mode":"cover","opacity":0.75}
            }]
        }]});
        if !images {
            document["children"][0]["children"]
                .as_array_mut()
                .unwrap()
                .pop();
        }
        std::fs::write(&path, document.to_string()).unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let mut retained = PreviewRenderer::default();
        let mut dynamic = crate::dynamic::DynamicRuntime::new(store.data_dir());
        for changed in [false, true] {
            image::RgbaImage::from_fn(7, 5, |x, y| {
                image::Rgba([
                    x as u8 * 30,
                    y as u8 * 40,
                    if changed { 210 } else { 50 },
                    180,
                ])
            })
            .save(&asset)
            .unwrap();
            if changed {
                let mut document = document.clone();
                document["children"][0]["children"][0]["geometry"] = json!("M0 0 L80 60 L0 60 Z");
                std::fs::write(&path, document.to_string()).unwrap();
                retained.clear();
            }
            let project = Project::load(&store).unwrap();
            let snapshot = crate::layout::compute(
                project.resolve().unwrap(),
                &mut crate::text::Fonts::new(&[]).unwrap(),
            )
            .unwrap();
            let too_small_export = RenderOptions {
                scale: 0.005,
                ..Default::default()
            };
            let error = raster_impl(&project, &snapshot, &too_small_export, None)
                .err()
                .unwrap();
            assert!(
                error.message.contains("0.01..16"),
                "Public export scale limits must stay unchanged"
            );
            for scale in [0.375, 1.0, 1.25, 2.0] {
                for x in [-48.5, 16.0, -48.5] {
                    let options = RenderOptions {
                        scale,
                        region: Some(Rect::new(x, -24.25, 640.0, 480.0)),
                        ..Default::default()
                    };
                    let original = preview_png_with_runtime_observed(
                        &project,
                        &snapshot,
                        &options,
                        &mut dynamic,
                        None,
                        None,
                    )
                    .unwrap();
                    let cached = retained
                        .render(&project, &snapshot, &options, &mut dynamic, None, None)
                        .unwrap();
                    assert_eq!(
                        crate::document::hash(&cached.pixels.png().unwrap()),
                        crate::document::hash(&original.png),
                        "scale={scale}, x={x}, changed={changed}"
                    );
                }
            }
            assert!(!retained.cache.as_ref().unwrap().shapes.borrow().is_empty());
            assert_eq!(
                retained
                    .cache
                    .as_ref()
                    .unwrap()
                    .pictures
                    .borrow()
                    .values()
                    .any(|entry| !entry.images.is_empty()),
                images,
                "The pixel comparison must exercise both vector and image commands"
            );
        }
    }

    #[test]
    fn oversized_static_images_use_the_uncached_path_without_changing_pixels() {
        let directory = tempfile::TempDir::new().unwrap();
        image::RgbaImage::from_pixel(3000, 3000, image::Rgba([12, 34, 56, 200]))
            .save(directory.path().join("large.png"))
            .unwrap();
        let path = directory.path().join("design.pen");
        std::fs::write(
            &path,
            json!({"version":"2.21","children":[{
                "type":"rectangle","id":"image","width":80,"height":60,
                "fill":{"type":"image","url":"large.png","mode":"cover"}
            }]})
            .to_string(),
        )
        .unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let project = Project::load(&store).unwrap();
        let snapshot = crate::layout::compute(
            project.resolve().unwrap(),
            &mut crate::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let options = RenderOptions::default();
        let mut dynamic = crate::dynamic::DynamicRuntime::new(store.data_dir());
        let original = preview_png_with_runtime_observed(
            &project,
            &snapshot,
            &options,
            &mut dynamic,
            None,
            None,
        )
        .unwrap();
        let mut retained = PreviewRenderer::default();
        for _ in 0..2 {
            let cached = retained
                .render(&project, &snapshot, &options, &mut dynamic, None, None)
                .unwrap();
            assert_eq!(cached.pixels.png().unwrap(), original.png);
            assert!(
                retained
                    .cache
                    .as_ref()
                    .unwrap()
                    .pictures
                    .borrow()
                    .is_empty(),
                "Decoded image storage above 32 MiB must not be retained"
            );
        }
    }

    #[test]
    fn retained_dashboard_matches_uncached_pixels_at_fitted_zoom_and_pan() {
        let directory = tempfile::TempDir::new().unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/performance");
        for file in ["dashboard.pen", "avatar.png"] {
            std::fs::copy(fixture.join(file), directory.path().join(file)).unwrap();
        }
        // Pixel parity must not depend on hosted debug-render timing.
        std::fs::write(
            directory.path().join("lazypencil.toml"),
            "version=1\n[runtime.dynamic]\nrender_timeout_ms=180000\n",
        )
        .unwrap();
        let store = crate::store::Store::open(
            directory.path().join("dashboard.pen"),
            Some(directory.path().join("data")),
        )
        .unwrap();
        let project = Project::load(&store).unwrap();
        let snapshot = crate::layout::compute(
            project.resolve().unwrap(),
            &mut crate::text::Fonts::new(&[]).unwrap(),
        )
        .unwrap();
        let fitted = (900.0 / snapshot.bounds.width).min(980.0 / snapshot.bounds.height) * 0.94;
        let mut retained = PreviewRenderer::default();
        let mut dynamic = crate::dynamic::DynamicRuntime::new(store.data_dir());
        let views = [
            (fitted, 0.0, 0.0),
            (fitted * 1.25, 0.0, 0.0),
            (fitted, 64.0, 0.0),
            (fitted, 0.0, 64.0),
            (fitted, 128.0, 0.0),
            (fitted, 192.0, 0.0),
            (fitted, 256.0, 0.0),
            (fitted, 320.0, 0.0),
            (fitted, 384.0, 0.0),
            (fitted, 448.0, 0.0),
            (fitted, 512.0, 0.0),
            (fitted, 576.0, 0.0),
            (fitted * 1.25, 0.0, 0.0),
            (fitted, 0.0, 0.0),
        ]
        .into_iter()
        .map(|view| (view, Color::from_rgb(248, 250, 252)))
        .chain(std::iter::once((
            (fitted, 0.0, 0.0),
            Color::from_rgb(30, 40, 50),
        )));
        for ((scale, pan, vertical), background) in views {
            let width = 900.0 / scale;
            let height = 980.0 / scale;
            let options = RenderOptions {
                scale,
                region: Some(Rect::new(
                    snapshot.bounds.x + snapshot.bounds.width / 2.0 - width / 2.0 + pan / scale,
                    snapshot.bounds.y + snapshot.bounds.height / 2.0 - height / 2.0
                        + vertical / scale,
                    width,
                    height,
                )),
                background,
                ..Default::default()
            };
            let cached = retained
                .render(&project, &snapshot, &options, &mut dynamic, None, None)
                .unwrap();
            let original = preview_png_with_runtime_observed(
                &project,
                &snapshot,
                &options,
                &mut dynamic,
                None,
                None,
            )
            .unwrap();
            let actual = cached.pixels.png().unwrap();
            assert_eq!(
                crate::document::hash(&actual),
                crate::document::hash(&original.png),
                "scale={scale}, pan={pan}, vertical={vertical}"
            );
        }
    }
}
