//! Owned pixels for fast terminal previews and exact on-demand PNG encoding.
use super::*;

#[derive(Clone)]
pub(crate) struct PreviewPixels {
    width: u32,
    height: u32,
    // Preserve premultiplied pixels for on-demand PNG export. No Skia object
    // crosses the preview worker's thread boundary.
    rgba: std::sync::Arc<Vec<u8>>,
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
                rgba: std::sync::Arc::new(rgba),
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
    pub(crate) fn retained_bytes(&self) -> usize {
        self.rgba.len()
            + self
                .camera_pixels
                .as_ref()
                .map_or(0, |image| image.as_raw().len())
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
        // The immutable source outlives this local image and its synchronous
        // encoder. Only the encoded PNG leaves this scope, never the borrowed Data.
        let data = unsafe { Data::new_bytes(&self.rgba) };
        let image = skia_safe::images::raster_from_data(&info, data, self.width as usize * 4)
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
    viewport: Option<[u32; 2]>,
}

impl PreviewRenderer {
    pub(crate) fn fork(&self) -> Self {
        Self {
            cache: self.cache.as_ref().map(RasterCache::fork),
            recordable: self.recordable,
            viewport: self.viewport,
        }
    }

    pub(crate) fn set_viewport(&mut self, pixels: [u32; 2]) {
        self.viewport = Some(pixels);
    }
    pub(crate) fn retain_prepared_commands(&mut self, prepared: &Self) {
        if let (Some(cache), Some(prepared)) = (&self.cache, &prepared.cache) {
            // Commands are keyed by exact scale and, for image scenes, viewport.
            // Keep atlas origins from actual display order: speculative raster
            // origins can change f32 antialias rounding on a later camera move.
            let mut pictures = cache.pictures.borrow_mut();
            let mut bytes = pictures
                .values()
                .map(|picture| picture.bytes)
                .sum::<usize>();
            let prepared_pictures = prepared.pictures.borrow();
            let mut entries = prepared_pictures.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(_, picture)| std::cmp::Reverse(picture.used.get()));
            for (key, picture) in entries {
                if pictures.contains_key(key)
                    || pictures.len() >= if key.viewport.is_some() { 8 } else { 48 }
                    || bytes.saturating_add(picture.bytes) > 32 * 1024 * 1024
                {
                    continue;
                }
                bytes += picture.bytes;
                pictures.insert(*key, picture.clone());
            }
            cache
                .picture_clock
                .set(cache.picture_clock.get().max(prepared.picture_clock.get()));
        }
    }
    pub(crate) fn retained_commands(&self, scale: f32) -> (usize, usize, bool, Option<u32>) {
        self.cache.as_ref().map_or((0, 0, false, None), |cache| {
            let pictures = cache.pictures.borrow();
            (
                pictures.len(),
                pictures.values().map(|entry| entry.bytes).sum(),
                pictures.keys().any(|key| key.scale == scale.to_bits()),
                pictures
                    .keys()
                    .map(|key| key.scale.abs_diff(scale.to_bits()))
                    .min(),
            )
        })
    }
    pub(crate) fn clear(&mut self) {
        self.cache = None;
        self.recordable = None;
    }

    pub(crate) fn release_working_buffers(&self) {
        if let Some(cache) = &self.cache {
            cache.pool.borrow().release_buffers();
        }
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
        self.with_raster(
            project,
            snapshot,
            options,
            runtime,
            cancel,
            |raster, recordable| {
                if let Some(observer) = observer.as_mut() {
                    observer("render_raster_ms");
                }
                let (pixels, bitmap) = PreviewPixels::from_image(&raster.image)?;
                if let Some(observer) = observer.as_mut() {
                    observer("render_pixel_copy_ms");
                }
                Ok(RenderedPreview {
                    pixels,
                    bitmap,
                    dynamic_artifacts: raster.dynamic_artifacts,
                    retain_raw_pixels: recordable,
                })
            },
        )
    }

    pub(crate) fn hydrate(
        &mut self,
        project: &Project,
        snapshot: &LayoutSnapshot,
        options: &RenderOptions,
        runtime: &mut crate::dynamic::DynamicRuntime,
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<()> {
        crate::cancel::check(cancel.as_ref())?;
        if self.recordable == Some(true)
            && self.cache.as_ref().is_some_and(|cache| !cache.has_images)
        {
            let plan =
                RasterPlan::new(project, snapshot, options, true)?.with_viewport(self.viewport)?;
            if plan.width * plan.height > atlas::MAX_ATLAS_PIXELS {
                // Vector commands use world coordinates. With no possible atlas,
                // revisiting a cached pan has no device origin to synchronize.
                project.verify_unchanged()?;
                return Ok(());
            }
        }
        // Update exact raster origins without copying pixels that will be discarded.
        self.with_raster(project, snapshot, options, runtime, cancel, |_, _| Ok(()))
    }

    fn with_raster<T>(
        &mut self,
        project: &Project,
        snapshot: &LayoutSnapshot,
        options: &RenderOptions,
        runtime: &mut crate::dynamic::DynamicRuntime,
        cancel: Option<tokio_util::sync::CancellationToken>,
        finish: impl FnOnce(RenderedRaster, bool) -> Result<T>,
    ) -> Result<T> {
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
                self.viewport,
            )?;
            let output = finish(raster, recordable)?;
            runtime.check()?;
            Ok(output)
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
    fn preview_viewport_preserves_pixels_at_fractional_zoom_without_resampling() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("design.pen");
        std::fs::write(
            &path,
            json!({"version":"2.21","children":[{
                "type":"frame","id":"root","width":7562,"height":17560,"clip":true,
                "fill":"#12345680","children":[
                    {"type":"path","id":"diagonal","width":7562,"height":17560,
                     "geometry":"M0 0 L7562 17560 L0 17560 Z","fill":"#F59E0BA0"},
                    {"type":"rectangle","id":"edge","x":2200.25,"y":1300.5,
                     "width":1,"height":15000,"fill":"#FFFFFF"}
                ]
            }]})
            .to_string(),
        )
        .unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let (project, snapshot) = crate::service::layout_snapshot(&store, &[], None, None).unwrap();
        let mut runtime = crate::dynamic::DynamicRuntime::new(store.data_dir());
        for [width, height] in [[996, 980], [1992, 1960]] {
            let fitted = height as f32 / 17560.0 * 0.94;
            for background in [Color::TRANSPARENT, Color::from_rgb(248, 250, 252)] {
                let mut retained = PreviewRenderer::default();
                retained.set_viewport([width, height]);
                for scale in [fitted, fitted * 1.01] {
                    for pan in [0.0, 64.0] {
                        let options = RenderOptions {
                            scale,
                            region: Some(Rect::new(
                                3781.0 - width as f32 / scale / 2.0 + pan / scale,
                                8780.0 - height as f32 / scale / 2.0,
                                width as f32 / scale,
                                height as f32 / scale,
                            )),
                            background,
                            ..Default::default()
                        };
                        let export =
                            png_with_runtime(&project, &snapshot, &options, &mut runtime, None)
                                .unwrap();
                        if scale == fitted {
                            assert_eq!(
                                export.height,
                                height + 1,
                                "Exercise the measured outward-rounding case"
                            );
                        }
                        // Compare against the existing retained raster before
                        // terminal resizing, preserving its native alpha and AA.
                        let original = PreviewRenderer::default()
                            .render(&project, &snapshot, &options, &mut runtime, None, None)
                            .unwrap();
                        let reference = image::load_from_memory(&original.pixels.png().unwrap())
                            .unwrap()
                            .crop_imm(0, 0, width, height)
                            .into_rgba8();
                        for _ in 0..2 {
                            let mut renderer = retained.fork();
                            let preview = renderer
                                .render(&project, &snapshot, &options, &mut runtime, None, None)
                                .unwrap();
                            let bitmap = image::load_from_memory(&preview.pixels.png().unwrap())
                                .unwrap()
                                .into_rgba8();
                            assert_eq!(bitmap.dimensions(), (width, height));
                            assert_eq!(preview.bitmap.width(), width);
                            assert_eq!(preview.bitmap.height(), height);
                            assert!(
                                bitmap == reference,
                                "Preserve native pixels and alpha without filtering at scale={scale}, pan={pan}: first differing pixels {:?}",
                                bitmap
                                    .pixels()
                                    .zip(reference.pixels())
                                    .enumerate()
                                    .filter(|(_, (a, b))| a != b)
                                    .take(4)
                                    .map(|(i, (a, b))| ((i as u32 % width, i as u32 / width), a, b))
                                    .collect::<Vec<_>>()
                            );
                            renderer
                                .hydrate(&project, &snapshot, &options, &mut runtime, None)
                                .unwrap();
                            retained = renderer;
                        }
                        assert_eq!(
                            crate::document::hash(
                                &png_with_runtime(
                                    &project,
                                    &snapshot,
                                    &options,
                                    &mut runtime,
                                    None
                                )
                                .unwrap()
                                .png
                            ),
                            crate::document::hash(&export.png)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn preview_viewport_keeps_image_scene_pixels_and_transparency_at_the_trailing_edge() {
        let directory = tempfile::tempdir().unwrap();
        image::RgbaImage::from_fn(7, 5, |x, y| {
            image::Rgba([x as u8 * 30, y as u8 * 40, 90, 180])
        })
        .save(directory.path().join("image.png"))
        .unwrap();
        let path = directory.path().join("design.pen");
        std::fs::write(
            &path,
            json!({"version":"2.21","children":[{
                "type":"rectangle","id":"image","width":7562,"height":17560,
                "fill":{"type":"image","url":"image.png","mode":"cover","opacity":0.75}
            }]})
            .to_string(),
        )
        .unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let (project, snapshot) = crate::service::layout_snapshot(&store, &[], None, None).unwrap();
        let mut runtime = crate::dynamic::DynamicRuntime::new(store.data_dir());
        let scale = 0.052460138;
        let mut retained = PreviewRenderer::default();
        retained.set_viewport([128, 980]);
        for background in [Color::TRANSPARENT, Color::from_rgb(248, 250, 252)] {
            for pan in [0.0, 64.0] {
                let options = RenderOptions {
                    scale,
                    background,
                    region: Some(Rect::new(pan / scale, 0.0, 128.0 / scale, 980.0 / scale)),
                    ..Default::default()
                };
                let original = PreviewRenderer::default()
                    .render(&project, &snapshot, &options, &mut runtime, None, None)
                    .unwrap();
                let reference = image::load_from_memory(&original.pixels.png().unwrap()).unwrap();
                assert_eq!(reference.height(), 981);
                let reference = reference.crop_imm(0, 0, 128, 980).into_rgba8();
                let preview = retained
                    .render(&project, &snapshot, &options, &mut runtime, None, None)
                    .unwrap();
                let actual = image::load_from_memory(&preview.pixels.png().unwrap())
                    .unwrap()
                    .into_rgba8();
                assert_eq!(actual.dimensions(), (128, 980));
                assert_eq!(
                    crate::document::hash(actual.as_raw()),
                    crate::document::hash(reference.as_raw())
                );
            }
        }
    }

    #[test]
    fn large_vector_camera_hydration_needs_no_raster_or_pixel_copy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("design.pen");
        std::fs::write(&path, r##"{"version":"2.21","children":[{"type":"rectangle","id":"box","width":20,"height":20,"fill":"#123456"}]}"##).unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let (project, snapshot) = crate::service::layout_snapshot(&store, &[], None, None).unwrap();
        let mut retained = PreviewRenderer {
            cache: Some(RasterCache::new(&snapshot)),
            recordable: Some(true),
            viewport: None,
        };
        let options = RenderOptions {
            region: Some(Rect::new(0.0, 0.0, 4096.0, 4096.0)),
            ..Default::default()
        };
        let mut runtime = crate::dynamic::DynamicRuntime::new(store.data_dir());
        retained
            .hydrate(&project, &snapshot, &options, &mut runtime, None)
            .unwrap();
        assert!(
            retained
                .cache
                .as_ref()
                .unwrap()
                .pictures
                .borrow()
                .is_empty()
        );
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        assert!(
            retained
                .hydrate(&project, &snapshot, &options, &mut runtime, Some(cancel))
                .is_err()
        );
    }

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
                    retained
                        .hydrate(&project, &snapshot, &options, &mut dynamic, None)
                        .unwrap();
                    let cached = retained
                        .render(&project, &snapshot, &options, &mut dynamic, None, None)
                        .unwrap();
                    if scale == 0.375 && x == 16.0 {
                        let identities = retained
                            .cache
                            .as_ref()
                            .unwrap()
                            .pictures
                            .borrow()
                            .iter()
                            .map(|(key, picture)| (*key, picture.picture.unique_id()))
                            .collect::<Vec<_>>();
                        let mut prepared = PreviewRenderer::default();
                        prepared
                            .render(&project, &snapshot, &options, &mut dynamic, None, None)
                            .unwrap();
                        retained.retain_prepared_commands(&prepared);
                        for (key, identity) in identities {
                            assert_eq!(
                                retained.cache.as_ref().unwrap().pictures.borrow()[&key]
                                    .picture
                                    .unique_id(),
                                identity,
                                "Preparation must preserve active raster identities and origins"
                            );
                        }
                        let revisited = retained
                            .render(&project, &snapshot, &options, &mut dynamic, None, None)
                            .unwrap();
                        assert_eq!(
                            revisited.pixels.png().unwrap(),
                            cached.pixels.png().unwrap()
                        );
                    }
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
