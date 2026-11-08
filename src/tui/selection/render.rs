//! View decoration runs on the existing preview worker and reuses its pixels.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Default, PartialEq)]
pub(in crate::tui) struct State {
    pub selected: Vec<String>,
    pub marquee: Option<Rect>,
}

#[derive(Clone)]
pub(in crate::tui) struct Completed {
    pub state: State,
    pub image: graphics::Encoded,
}

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
                request.picker.canvas_background(),
            ) else {
                return Ok(None);
            };
            projected
        };
        if !paint(
            &mut image,
            request.view,
            request.viewport,
            &request.outlines,
            request.marquee,
            &|| request.serial != serial.load(Ordering::Relaxed),
        ) {
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

/// Paint screen-width outlines into an existing viewport before encoding it.
pub(in crate::tui) fn paint(
    image: &mut image::RgbaImage,
    view: preview::View,
    viewport: [u32; 2],
    outlines: &[Rect],
    marquee: Option<Rect>,
    cancelled: &impl Fn() -> bool,
) -> bool {
    let region = view.region(viewport);
    let [width, height] = [image.width() as f32, image.height() as f32];
    let project = |bounds: Rect| -> [i64; 4] {
        [
            ((bounds.x - region.x) / region.width * width).round() as i64,
            ((bounds.y - region.y) / region.height * height).round() as i64,
            ((bounds.right() - region.x) / region.width * width).round() as i64,
            ((bounds.bottom() - region.y) / region.height * height).round() as i64,
        ]
    };
    for outline in outlines {
        if cancelled() || !outline_pixels_checked(image, project(*outline), false, cancelled) {
            return false;
        }
    }
    marquee.is_none_or(|marquee| outline_pixels_checked(image, project(marquee), true, cancelled))
}
