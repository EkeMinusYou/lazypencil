//! Exact static camera frames and bounded prediction for repeated gestures.
use super::*;

pub(super) struct Frame<'a> {
    pub project: &'a Project,
    pub layout: &'a crate::layout::LayoutSnapshot,
    pub picker: &'a Backend,
    pub cells: Size,
    pub pixels: [u32; 2],
    pub raster_scale: f32,
    pub image_id: u32,
    pub nodes: Arc<[Node]>,
    pub summary: Value,
    pub cancel: CancellationToken,
    pub interrupted: Option<&'a dyn Fn() -> bool>,
}

impl Frame<'_> {
    pub(super) fn hydrate(
        &self,
        renderer: &mut crate::render::PreviewRenderer,
        dynamic: &mut crate::dynamic::DynamicRuntime,
        view: View,
    ) -> Result<()> {
        self.check_interrupted()?;
        let options = self.options(view);
        renderer.hydrate(
            self.project,
            self.layout,
            &options,
            dynamic,
            Some(self.cancel.clone()),
        )?;
        self.check_interrupted()
    }

    fn options(&self, view: View) -> crate::render::RenderOptions {
        let image::Rgba([r, g, b, a]) = self.picker.canvas_background();
        crate::render::RenderOptions {
            region: Some(view.region(self.pixels)),
            scale: view.zoom.unwrap() * self.raster_scale,
            background: skia_safe::Color::from_argb(a, r, g, b),
            time: view.time,
            ..Default::default()
        }
    }
    pub(super) fn render(
        &self,
        renderer: &mut crate::render::PreviewRenderer,
        dynamic: &mut crate::dynamic::DynamicRuntime,
        labels: &mut labels::Labels,
        view: View,
    ) -> Result<Preview> {
        self.check_interrupted()?;
        let options = self.options(view);
        let image = renderer.render(
            self.project,
            self.layout,
            &options,
            dynamic,
            Some(self.cancel.clone()),
            None,
        )?;
        self.check_interrupted()?;
        if !image.retain_raw_pixels {
            return Err(Error::new(
                "RENDER_ERROR",
                "Scene cannot retain static camera frames",
            ));
        }
        let mut bitmap = image.bitmap.into_rgba8();
        labels.paint(&mut bitmap, &self.nodes, view, self.pixels, &[])?;
        let protocol = encode(
            self.picker,
            image::DynamicImage::ImageRgba8(bitmap),
            self.cells,
            self.image_id,
        )?;
        Ok(Preview {
            protocol,
            image: PreviewImage::Pixels(image.pixels),
            view,
            nodes: self.nodes.clone(),
            summary: self.summary.clone(),
            attachment: false,
            time_dependent: false,
            live_pages: false,
            frame_time: std::time::Duration::ZERO,
        })
    }

    fn check_interrupted(&self) -> Result<()> {
        if self.cancel.is_cancelled() || self.interrupted.is_some_and(|interrupted| interrupted()) {
            return Err(Error::new("CANCELLED", "Camera preparation was superseded"));
        }
        Ok(())
    }
}

pub(super) fn predict(previous: View, current: View, pixels: [u32; 2]) -> Option<View> {
    if previous.time != current.time {
        return None;
    }
    let [before, now] = [previous.zoom?, current.zoom?];
    let mut next = current;
    if before != now {
        next.zoom = Some(if now == (before * 1.25).min(16.0) {
            (now * 1.25).min(16.0)
        } else if now == (before / 1.25).max(0.01) {
            (now / 1.25).max(0.01)
        } else {
            return None;
        });
    }
    let region = current.region(pixels);
    for (axis, extent) in [region.width, region.height].into_iter().enumerate() {
        let delta = current.center[axis] - previous.center[axis];
        if !delta.is_finite() || delta.abs() > f64::from(extent) {
            return None;
        }
        next.center[axis] += delta;
    }
    (!cache::same_pixels(next, current, pixels)).then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicts_repeated_native_pan_and_zoom_without_moving_the_actual_camera() {
        for zoom in [0.01, 0.06482758, 1.0, 16.0] {
            let original = View {
                center: [1234.25, -3456.75],
                zoom: Some(zoom),
                time: 0.0,
            };
            for delta in [[64.0, 0.0], [0.0, -64.0], [12.5, -9.0]] {
                let mut current = original;
                current.pan(delta);
                let mut expected = current;
                expected.pan(delta);
                let predicted = predict(original, current, [1404, 1040]).unwrap();
                assert!(cache::same_pixels(predicted, expected, [1404, 1040]));
            }
            let mut current = original;
            current.zoom = Some((zoom * 1.25).min(16.0));
            if current.zoom != original.zoom {
                let predicted = predict(original, current, [1404, 1040]).unwrap();
                assert_eq!(
                    predicted.zoom,
                    Some((current.zoom.unwrap() * 1.25).min(16.0))
                );
            }
        }
        let original = View {
            zoom: Some(1.0),
            ..Default::default()
        };
        assert!(predict(original, original, [1404, 1040]).is_none());
        assert!(
            predict(
                original,
                View {
                    time: 1.0,
                    ..original
                },
                [1404, 1040]
            )
            .is_none()
        );
    }
}
