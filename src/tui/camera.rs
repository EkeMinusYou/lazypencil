//! Completed native camera frames and temporary bitmap feedback for fallback terminals.
use super::preview::View;
use image::{Rgba, RgbaImage};

const NAVIGATION_REFRESH: std::time::Duration = std::time::Duration::from_millis(33);
const NAVIGATION_IDLE: std::time::Duration = std::time::Duration::from_millis(80);

const EXACT_FRAME_HEAD_START: std::time::Duration = std::time::Duration::from_millis(1);

pub(super) struct InputProfile {
    enabled: bool,
    origin: std::time::Instant,
    input_id: u64,
    last_input: Option<std::time::Instant>,
    last_serial: u64,
}

impl Default for InputProfile {
    fn default() -> Self {
        Self {
            enabled: std::env::var("LAZYPENCIL_PROFILE_INPUT").as_deref()
                == Ok("owned-local-measurement"),
            origin: std::time::Instant::now(),
            input_id: 0,
            last_input: None,
            last_serial: 0,
        }
    }
}

impl InputProfile {
    pub(super) fn input(&mut self, action: &str, kind: crossterm::event::KeyEventKind) {
        if !self.enabled || self.input_id >= 2048 {
            return;
        }
        self.input_id += 1;
        self.last_input = Some(std::time::Instant::now());
        eprintln!(
            "LAZYPENCIL_INPUT_PROFILE {}",
            serde_json::json!({
                "event":"input", "input_id":self.input_id, "action":action,
                "repeat":kind == crossterm::event::KeyEventKind::Repeat,
                "app_elapsed_ms":self.origin.elapsed().as_secs_f64()*1000.0,
            })
        );
    }

    pub(super) fn presented(&mut self, serial: u64, cached: bool) {
        if !self.enabled || serial == self.last_serial || self.input_id >= 2048 {
            return;
        }
        self.last_serial = serial;
        eprintln!(
            "LAZYPENCIL_INPUT_PROFILE {}",
            serde_json::json!({
                "event":"output", "input_id":self.last_input.map(|_| self.input_id),
                "placement_serial":serial, "cached":cached,
                "app_elapsed_ms":self.origin.elapsed().as_secs_f64()*1000.0,
                "input_to_flush_ms":self.last_input.map(|at| at.elapsed().as_secs_f64()*1000.0),
            })
        );
    }
}

pub(super) struct Camera {
    pub image: ratatui_image::protocol::Protocol,
    pub generation: u64,
}

impl super::App {
    pub(super) fn request_camera_navigation(
        &mut self,
        worker: &super::preview::Worker,
        picker: &super::graphics::Backend,
    ) -> crate::Result<()> {
        let progressive = self.presentation.source_size().is_some()
            && self.pending_preview.is_none()
            && !self.playback.active()
            && self.preview.as_ref().is_some_and(|preview| {
                !preview.attachment
                    && !preview.time_dependent
                    && !preview.live_pages
                    && preview.protocol.size() == self.cells
            });
        if !progressive {
            self.request(worker, false, false);
            return self.camera_feedback(picker);
        }
        if !self.camera_navigation {
            self.request(worker, false, false);
            self.camera_navigation = true;
            self.camera_refresh_deadline = Some(std::time::Instant::now() + NAVIGATION_REFRESH);
        }
        // Keep the render generation stable throughout navigation. New keys
        // update the target only; an in-flight source render must finish.
        self.preview_generation = None;
        self.camera = None;
        self.camera_settle_deadline = Some(std::time::Instant::now() + NAVIGATION_IDLE);
        Ok(())
    }

    pub(super) fn accept_interactive_preview(
        &mut self,
        output: super::preview::Output,
        worker: &super::preview::Worker,
        picker: &super::graphics::Backend,
    ) -> bool {
        if !self.camera_navigation || output.generation != self.generation {
            return self.accept_controlled_preview(output, worker, picker);
        }
        match &output.result {
            Ok(preview)
                if !preview.attachment
                    && !preview.time_dependent
                    && !preview.live_pages
                    && preview.protocol.size() == self.cells
                    && preview.view.time == self.view.time =>
            {
                self.deferred_preview = Some(output);
                if self.camera_settle_deadline.is_none() {
                    self.refresh_camera_navigation(worker, picker);
                }
                false
            }
            Ok(preview) if preview.view != self.view => {
                // A changed scene may require dynamic rendering. Restart at the
                // current target instead of displaying an obsolete viewport.
                self.request(worker, false, false);
                false
            }
            _ => self.accept_controlled_preview(output, worker, picker),
        }
    }

    pub(super) fn refresh_camera_navigation(
        &mut self,
        worker: &super::preview::Worker,
        picker: &super::graphics::Backend,
    ) {
        self.camera_refresh_deadline = None;
        if !self.camera_navigation {
            return;
        }
        let target = self.view;
        let idle = self.camera_settle_deadline;
        if let Some(output) = self.deferred_preview.take() {
            if !self.accept_controlled_preview(output, worker, picker) {
                return;
            }
            self.view = target;
            self.camera_navigation = true;
            self.camera_settle_deadline = idle;
            if self
                .preview
                .as_ref()
                .is_some_and(|preview| preview.view != target)
            {
                self.preview_generation = None;
                // Display only the completed render at its own viewport. A
                // cell-rounded cached projection would change scale/position
                // again when the final exact frame arrives.
                self.camera = None;
            }
        }
        let exact = self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.view == target);
        if idle.is_none() && exact {
            self.camera_navigation = false;
            // Any in-flight render belongs to the finished navigation epoch.
            // The retained source already exactly matches the final target.
            self.generation += 1;
            self.preview_generation = Some(self.generation);
            self.camera = None;
            return;
        }
        if !exact {
            // Start the newest target only when the previous source render has
            // completed. No queued history and no cancellation on each repeat.
            worker.request_if_idle(super::preview::Request {
                generation: self.generation,
                cells: self.cells,
                view: target,
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            });
        }
        self.camera_refresh_deadline = Some(std::time::Instant::now() + NAVIGATION_REFRESH);
    }

    pub(super) fn finish_camera_gesture(
        &mut self,
        worker: &super::preview::Worker,
        picker: &super::graphics::Backend,
    ) {
        self.camera_settle_deadline = None;
        self.refresh_camera_navigation(worker, picker);
    }

    pub(super) fn camera_feedback(
        &mut self,
        picker: &super::graphics::Backend,
    ) -> crate::Result<()> {
        if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty
            && !picker.tmux_detected()
        {
            self.camera = None;
            self.camera_deadline = None;
            self.status.clear();
            return Ok(());
        }
        if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty
            && !self.camera_was_visible
            && self
                .preview
                .as_ref()
                .is_some_and(|preview| preview.frame_time <= EXACT_FRAME_HEAD_START)
        {
            // Let short exact renders finish without a competing projection,
            // compression pass and image transfer on the input thread. Keep the
            // first deadline across superseding gestures so feedback cannot starve.
            self.camera_deadline
                .get_or_insert_with(|| std::time::Instant::now() + EXACT_FRAME_HEAD_START);
            return Ok(());
        }
        self.camera_deadline = None;
        self.render_camera_feedback(picker)
    }

    pub(super) fn render_camera_feedback(
        &mut self,
        picker: &super::graphics::Backend,
    ) -> crate::Result<()> {
        self.camera = None;
        // Native Kitty must keep completed frame geometry unchanged. Sixel
        // also leaves rendering/encoding on the coalescing worker because an
        // interim bitmap would require another expensive palette/transfer.
        if picker.protocol_type() == ratatui_image::picker::ProtocolType::Sixel
            || (picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty
                && !picker.tmux_detected())
        {
            return Ok(());
        }
        let Some(preview) = &self.preview else {
            return Ok(());
        };
        if preview.attachment
            || preview.time_dependent
            || preview.live_pages
            || self.playback.active()
            || !matches!(self.focus, super::Focus::Preview | super::Focus::Nodes)
            || (preview.view.center == self.view.center && preview.view.zoom == self.view.zoom)
        {
            return Ok(());
        }
        let decoded = if let super::preview::PreviewImage::Png(png) = &preview.image {
            let (width, height) =
                image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png)
                    .into_dimensions()
                    .map_err(|error| crate::Error::new("RENDER_ERROR", error.to_string()))?;
            if u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
                return Ok(());
            }
            Some(
                image::load_from_memory_with_format(png, image::ImageFormat::Png)
                    .map_err(|error| crate::Error::new("RENDER_ERROR", error.to_string()))?
                    .into_rgba8(),
            )
        } else {
            None
        };
        let bitmap = match &preview.image {
            super::preview::PreviewImage::Pixels(pixels) => pixels.camera_pixels(),
            super::preview::PreviewImage::Png(_) => decoded.as_ref().and_then(|bitmap| {
                image::ImageBuffer::from_raw(
                    bitmap.width(),
                    bitmap.height(),
                    bitmap.as_raw().as_slice(),
                )
            }),
        };
        let Some(bitmap) = bitmap else {
            return Ok(());
        };
        let [width, height] = super::preview::pixel_size(picker, self.cells)?;
        // Raster bounds round outward. Discard at most one trailing pixel for
        // temporary feedback; the final frame still uses the exact renderer.
        if [bitmap.width(), bitmap.height()] != [width, height]
            && !((width..=width + 1).contains(&bitmap.width())
                && (height..=height + 1).contains(&bitmap.height()))
        {
            return Ok(());
        }
        let Some(bitmap) = project_region(&bitmap, preview.view, self.view, [width, height]) else {
            return Ok(());
        };
        // The projection owns its pixels; release a decoded fallback before
        // the terminal encoder allocates its own temporary transport buffers.
        drop(decoded);
        let protocol = picker.encode(
            image::DynamicImage::ImageRgba8(bitmap),
            self.cells,
            self.camera_id,
        )?;
        self.camera = Some(Camera {
            image: protocol,
            generation: self.generation,
        });
        self.status.clear();
        Ok(())
    }
}

/// Reproject the original cached image for every gesture. Reprojecting a prior
/// temporary frame would accumulate resampling loss and uncovered borders.
#[cfg(test)]
pub(super) fn project<C>(
    source: &image::ImageBuffer<Rgba<u8>, C>,
    before: View,
    after: View,
) -> Option<RgbaImage>
where
    C: std::ops::Deref<Target = [u8]>,
{
    project_region(source, before, after, [source.width(), source.height()])
}

fn project_region<C>(
    source: &image::ImageBuffer<Rgba<u8>, C>,
    before: View,
    after: View,
    size: [u32; 2],
) -> Option<RgbaImage>
where
    C: std::ops::Deref<Target = [u8]>,
{
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
    if size[0] > source.width()
        || size[1] > source.height()
        || size.contains(&0)
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
    if ratio == 1.0 {
        let (dx, dy) = ((dx + 0.5).floor(), (dy + 0.5).floor());
        let mut result = RgbaImage::from_pixel(size[0], size[1], Rgba([248, 250, 252, 255]));
        if dx <= -f64::from(size[0])
            || dx >= f64::from(size[0])
            || dy <= -f64::from(size[1])
            || dy >= f64::from(size[1])
        {
            return Some(result);
        }
        let (dx, dy) = (dx as i64, dy as i64);
        let left = (-dx).clamp(0, i64::from(size[0])) as usize;
        let right = (i64::from(size[0]) - dx).clamp(0, i64::from(size[0])) as usize;
        if right > left {
            let length = (right - left) * 4;
            for (y, row) in result.chunks_exact_mut(size[0] as usize * 4).enumerate() {
                let source_y = y as i64 + dy;
                if !(0..i64::from(size[1])).contains(&source_y) {
                    continue;
                }
                let offset =
                    (source_y as usize * source.width() as usize + (left as i64 + dx) as usize) * 4;
                row[left * 4..left * 4 + length]
                    .copy_from_slice(&source.as_raw()[offset..offset + length]);
            }
        }
        return Some(result);
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
        let offset = source_y as usize * source.width() as usize * 4;
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
    fn rounded_trailing_pixels_are_cropped_without_a_copy_or_rescale() {
        let source = RgbaImage::from_fn(121, 81, |x, y| Rgba([x as u8, y as u8, 99, 255]));
        let cropped = image::imageops::crop_imm(&source, 0, 0, 120, 80).to_image();
        let borrowed = image::ImageBuffer::from_raw(121, 81, source.as_raw().as_slice()).unwrap();
        let before = View {
            zoom: Some(0.5),
            ..Default::default()
        };
        for zoom in [0.5, 1.0] {
            let after = View {
                center: [16.0, -8.0],
                zoom: Some(zoom),
                ..before
            };
            assert_eq!(
                project_region(&borrowed, before, after, [120, 80]),
                project(&cropped, before, after)
            );
        }
    }

    #[test]
    fn row_copy_pan_matches_pixel_centers_and_clips_fractional_and_distant_views() {
        let image = RgbaImage::from_fn(12, 8, |x, y| Rgba([x as u8, y as u8, 100, 255]));
        let before = View {
            zoom: Some(1.0),
            ..Default::default()
        };
        for dx in [-160.0_f64, -3.25, -0.49, 0.49, 3.75, 160.0, 1.0e30] {
            for dy in [-90.0_f64, -1.25, 0.0, 1.75, 90.0] {
                let mut after = before;
                after.pan([dx, dy]);
                let actual = project(&image, before, after).unwrap();
                for y in 0..8 {
                    for x in 0..12 {
                        let sx = (f64::from(x) + 0.5 + dx).floor();
                        let sy = (f64::from(y) + 0.5 + dy).floor();
                        let expected = if (0.0..12.0).contains(&sx) && (0.0..8.0).contains(&sy) {
                            *image.get_pixel(sx as u32, sy as u32)
                        } else {
                            Rgba([248, 250, 252, 255])
                        };
                        assert_eq!(
                            *actual.get_pixel(x, y),
                            expected,
                            "pan ({dx}, {dy}), pixel ({x}, {y})"
                        );
                    }
                }
            }
        }
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
