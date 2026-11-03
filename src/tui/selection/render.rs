//! View decoration runs on the existing preview worker and reuses its pixels.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(in crate::tui) struct Request {
    pub source: preview::PreviewImage,
    pub nodes: Arc<[preview::Node]>,
    pub selected: Vec<String>,
    pub before: preview::View,
    pub view: preview::View,
    pub size: ratatui::layout::Size,
    pub viewport: [u32; 2],
    pub outlines: Vec<Rect>,
    pub marquee: Option<Rect>,
    pub picker: graphics::Backend,
    pub image_id: u32,
    pub serial: u64,
    pub clock: Arc<AtomicU64>,
}
pub(in crate::tui) struct Cache {
    source: preview::PreviewImage,
    image: image::RgbaImage,
}
pub(in crate::tui) struct Finished {
    pub serial: u64,
    pub view: preview::View,
    pub result: Result<Option<graphics::Encoded>>,
}
pub(in crate::tui) fn run(
    request: Request,
    cache: &mut Option<Cache>,
    labels: &mut preview::labels::Labels,
) -> Finished {
    let serial = &request.clock;
    let result = (|| {
        if request.serial != serial.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if cache
            .as_ref()
            .is_none_or(|cache| cache.source.identity() != request.source.identity())
        {
            *cache = Some(Cache {
                image: request.source.bitmap()?,
                source: request.source.clone(),
            });
        }
        if request.serial != serial.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let source = &cache.as_ref().unwrap().image;
        let mut image = if request.before == request.view {
            source.clone()
        } else {
            let Some(projected) = super::super::camera::project_region(
                source,
                request.before,
                request.view,
                request.viewport,
            ) else {
                return Ok(None);
            };
            projected
        };
        let region = request.view.region(request.viewport);
        let project = |bounds: Rect| -> [i64; 4] {
            [
                ((bounds.x - region.x) / region.width * image.width() as f32).round() as i64,
                ((bounds.y - region.y) / region.height * image.height() as f32).round() as i64,
                ((bounds.right() - region.x) / region.width * image.width() as f32).round() as i64,
                ((bounds.bottom() - region.y) / region.height * image.height() as f32).round()
                    as i64,
            ]
        };
        let outlines = request
            .outlines
            .iter()
            .copied()
            .map(project)
            .collect::<Vec<_>>();
        let marquee = request.marquee.map(project);
        for outline in outlines {
            if request.serial != serial.load(Ordering::Relaxed) {
                return Ok(None);
            }
            if !outline_pixels_checked(&mut image, outline, false, &|| {
                request.serial != serial.load(Ordering::Relaxed)
            }) {
                return Ok(None);
            }
        }
        if let Some(marquee) = marquee
            && !outline_pixels_checked(&mut image, marquee, true, &|| {
                request.serial != serial.load(Ordering::Relaxed)
            })
        {
            return Ok(None);
        }
        if request.serial != serial.load(Ordering::Relaxed) {
            return Ok(None);
        }
        labels.paint(
            &mut image,
            &request.nodes,
            request.view,
            request.viewport,
            &request.selected,
        )?;
        request
            .picker
            .encode_preview(
                image::DynamicImage::ImageRgba8(image),
                request.size,
                request.image_id,
            )
            .map(Some)
    })();
    Finished {
        serial: request.serial,
        view: request.view,
        result,
    }
}
