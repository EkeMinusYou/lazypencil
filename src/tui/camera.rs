//! Temporary camera feedback from the last exact frame; never an export source.
use super::preview::View;
use image::{Rgba, RgbaImage};

pub(super) struct Camera {
    pub protocol: ratatui_image::protocol::Protocol,
    pub generation: u64,
}

impl super::App {
    pub(super) fn camera_feedback(
        &mut self,
        picker: &super::graphics::Backend,
    ) -> crate::Result<()> {
        self.camera = None;
        let Some(preview) = &self.preview else {
            return Ok(());
        };
        if preview.attachment
            || preview.time_dependent
            || preview.live_pages
            || self.playback.active()
            || self.focus != super::Focus::Preview
            || (preview.view.center == self.view.center && preview.view.zoom == self.view.zoom)
        {
            return Ok(());
        }
        let super::preview::PreviewImage::Pixels(pixels) = &preview.image else {
            return Ok(());
        };
        let Some(bitmap) = pixels.camera_pixels() else {
            return Ok(());
        };
        let [width, height] = super::preview::pixel_size(picker, self.cells)?;
        // Raster bounds round outward. Discard at most one trailing pixel for
        // temporary feedback; the final frame still uses the exact renderer.
        let cropped = if [bitmap.width(), bitmap.height()] == [width, height] {
            None
        } else if (width..=width + 1).contains(&bitmap.width())
            && (height..=height + 1).contains(&bitmap.height())
        {
            Some(image::imageops::crop_imm(bitmap, 0, 0, width, height).to_image())
        } else {
            return Ok(());
        };
        let Some(bitmap) = project(cropped.as_ref().unwrap_or(bitmap), preview.view, self.view)
        else {
            return Ok(());
        };
        let protocol = picker.encode(
            image::DynamicImage::ImageRgba8(bitmap),
            self.cells,
            self.camera_id,
        )?;
        self.camera = Some(Camera {
            protocol,
            generation: self.generation,
        });
        self.status = "Cached preview · rendering full detail".into();
        Ok(())
    }
}

/// Reproject the original cached image for every gesture. Reprojecting a prior
/// temporary frame would accumulate resampling loss and uncovered borders.
pub(super) fn project(source: &RgbaImage, before: View, after: View) -> Option<RgbaImage> {
    let [old_scale, new_scale] = [before.zoom?, after.zoom?];
    if [old_scale, new_scale]
        .iter()
        .any(|scale| !scale.is_finite() || !(0.01..=16.0).contains(scale))
        || before.time != after.time
        || !before.time.is_finite()
        || before.time < 0.0
        || [before.center, after.center]
            .into_iter()
            .flatten()
            .any(|x| !x.is_finite())
    {
        return None;
    }
    let size = [source.width(), source.height()];
    if size.contains(&0)
        || size.iter().any(|size| *size > 8192)
        || u64::from(size[0]) * u64::from(size[1]) * 4 > 16 * 1024 * 1024
    {
        return None;
    }
    let old_region = before.region(size);
    let new_region = after.region(size);
    let ratio = f64::from(old_scale) / f64::from(new_scale);
    let dx = f64::from(new_region.x - old_region.x) * f64::from(old_scale);
    let dy = f64::from(new_region.y - old_region.y) * f64::from(old_scale);
    if !dx.is_finite() || !dy.is_finite() {
        return None;
    }
    let columns = (0..size[0])
        .map(|x| {
            let source_x = ((f64::from(x) + 0.5) * ratio + dx).floor();
            (source_x >= 0.0 && source_x < f64::from(size[0])).then_some(source_x as usize)
        })
        .collect::<Vec<_>>();
    let mut result = RgbaImage::from_pixel(size[0], size[1], Rgba([248, 250, 252, 255]));
    for (y, row) in result.chunks_exact_mut(size[0] as usize * 4).enumerate() {
        let source_y = ((y as f64 + 0.5) * ratio + dy).floor();
        if source_y < 0.0 || source_y >= f64::from(size[1]) {
            continue;
        }
        let offset = source_y as usize * size[0] as usize * 4;
        for (pixel, x) in row.chunks_exact_mut(4).zip(&columns) {
            if let Some(x) = x {
                let offset = offset + x * 4;
                pixel.copy_from_slice(&source.as_raw()[offset..offset + 4]);
            }
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_feedback_preserves_pan_direction_and_reuses_original_pixels_without_drift() {
        let image = RgbaImage::from_fn(120, 80, |x, y| Rgba([x as u8, y as u8, 100, 255]));
        let initial = View {
            center: [-40.0, 20.0],
            zoom: Some(0.5),
            time: 0.0,
        };
        let mut moved = initial;
        moved.pan([16.0, -8.0]);
        let projected = project(&image, initial, moved).unwrap();
        assert_eq!(projected.get_pixel(0, 8), image.get_pixel(16, 0));
        assert_eq!(projected.get_pixel(103, 79), image.get_pixel(119, 71));
        assert_eq!(projected.get_pixel(104, 8).0, [248, 250, 252, 255]);
        assert_eq!(projected.get_pixel(0, 7).0, [248, 250, 252, 255]);
        moved.pan([-16.0, 8.0]);
        assert_eq!(project(&image, initial, moved).unwrap(), image);
        moved.zoom = Some(1.0);
        let zoomed = project(&image, initial, moved).unwrap();
        assert_eq!(zoomed.get_pixel(0, 0), image.get_pixel(30, 20));
        assert_eq!(zoomed.get_pixel(119, 79), image.get_pixel(89, 59));
        // A round trip reads the original source again, not the enlarged frame.
        assert_eq!(project(&image, initial, initial).unwrap(), image);
    }

    #[test]
    fn camera_feedback_rejects_time_changes_invalid_cameras_and_unbounded_images() {
        let image = RgbaImage::new(1, 1);
        let view = View {
            zoom: Some(1.0),
            ..Default::default()
        };
        for invalid in [
            View::default(),
            View {
                zoom: Some(f32::NAN),
                ..view
            },
            View {
                zoom: Some(0.0),
                ..view
            },
            View {
                center: [f64::INFINITY, 0.0],
                ..view
            },
            View { time: 1.0, ..view },
        ] {
            assert!(project(&image, view, invalid).is_none());
        }
        assert!(project(&RgbaImage::new(0, 1), view, view).is_none());
        assert!(project(&RgbaImage::new(2049, 2048), view, view).is_none());
    }
}
