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

/// Geometry belongs to one captured layout and stays on the preview thread.
#[derive(Default)]
pub(crate) struct PreviewRenderer {
    shapes: RefCell<BTreeMap<usize, Path>>,
    pictures: RefCell<BTreeMap<u32, skia_safe::Picture>>,
    recordable: Option<bool>,
    raster_pool: RefCell<raster_pool::RasterPool>,
}

impl PreviewRenderer {
    pub(crate) fn clear(&mut self) {
        self.shapes.get_mut().clear();
        self.pictures.get_mut().clear();
        self.recordable = None;
        *self.raster_pool.get_mut() = Default::default();
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
                        Some("image" | "shader" | "mesh_gradient" | "background_blur")
                    ) && fields.values().all(static_value)
                }
                Value::Array(values) => values.iter().all(static_value),
                _ => true,
            }
        }
        let recordable = *self.recordable.get_or_insert_with(|| {
            snapshot.items.iter().all(|item| {
                item.node.kind() != "browser" && item.node.properties.values().all(static_value)
            }) && !background_nodes(snapshot).iter().any(|flag| *flag)
        });
        let result = (|| {
            let raster = raster_impl_with_shapes(
                project,
                snapshot,
                options,
                Some(&mut *runtime),
                Some(&self.shapes),
                recordable.then_some(&self.pictures),
                Some(&self.raster_pool),
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
    fn retained_geometry_preserves_exact_camera_pixels_and_clears_on_reload() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("design.pen");
        let document = json!({"version":"2.21","children":[{
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
            }]
        }]});
        std::fs::write(&path, document.to_string()).unwrap();
        let store = crate::store::Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let mut retained = PreviewRenderer::default();
        let mut dynamic = crate::dynamic::DynamicRuntime::new(store.data_dir());
        for changed in [false, true] {
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
                        cached.pixels.png().unwrap(),
                        original.png,
                        "scale={scale}, x={x}, changed={changed}"
                    );
                }
            }
            assert!(!retained.shapes.borrow().is_empty());
        }
    }
}
