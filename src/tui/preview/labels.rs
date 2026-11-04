//! Root names decorate terminal pixels without changing document exports.
use super::{Node, View};
use crate::{
    Result,
    text::{Fonts, TextBlock, TextSpec},
};
use std::collections::BTreeMap;

pub(in crate::tui) struct Labels {
    fonts: Option<Fonts>,
    blocks: BTreeMap<(String, bool), TextBlock>,
    screen_scale: f32,
    ellipses: [Option<TextBlock>; 2],
}

impl Default for Labels {
    fn default() -> Self {
        Self::new(ratatui_image::FontSize::new(10, 20))
    }
}

impl Labels {
    pub fn new(font: ratatui_image::FontSize) -> Self {
        Self {
            fonts: None,
            blocks: BTreeMap::new(),
            // Match terminal text density instead of treating physical image
            // pixels as UI pixels. Camera zoom never changes this scale.
            screen_scale: (f32::from(font.height) / 20.0).max(1.0),
            ellipses: [None, None],
        }
    }

    pub(super) fn top_padding(&self) -> u32 {
        (28.0 * self.screen_scale).ceil() as u32
    }

    fn shape(&mut self, content: &str, selected: bool) -> Result<TextBlock> {
        if self.fonts.is_none() {
            self.fonts = Some(Fonts::new(&[])?);
        }
        let [r, g, b] = if selected {
            super::super::theme::BLUE_RGB
        } else {
            super::super::theme::SECONDARY_RGB
        };
        self.fonts.as_mut().unwrap().layout(
            &TextSpec {
                content: content.into(),
                size: 13.0 * self.screen_scale,
                color: skia_safe::Color::from_rgb(r, g, b),
                ..Default::default()
            },
            None,
        )
    }

    pub fn paint(
        &mut self,
        image: &mut image::RgbaImage,
        nodes: &[Node],
        view: View,
        viewport: [u32; 2],
        selected: &[String],
    ) -> Result<()> {
        let region = view.region(viewport);
        let scale = image.width() as f32 / viewport[0] as f32;
        let zoom = view.zoom.unwrap_or(1.0);
        let info = skia_safe::ImageInfo::new(
            (image.width() as i32, image.height() as i32),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let row_bytes = image.width() as usize * 4;
        let mut surface = skia_safe::surfaces::wrap_pixels(&info, image.as_mut(), row_bytes, None)
            .ok_or_else(|| crate::Error::new("RENDER_ERROR", "Cannot decorate preview pixels"))?;
        let canvas = surface.canvas();
        canvas.scale((scale, scale));
        for node in nodes
            .iter()
            .filter(|node| node.parent.is_none() && node.hit.enabled)
        {
            let bounds = super::super::selection::shape_bounds(node);
            let x = (bounds.x - region.x) * zoom;
            let width = bounds.width * zoom;
            let bottom = (bounds.y - region.y) * zoom - 6.0 * self.screen_scale;
            if bottom <= 0.0
                || bottom > viewport[1] as f32 + 24.0 * self.screen_scale
                || x >= viewport[0] as f32
                || width <= 0.0
                || x + width <= 0.0
            {
                continue;
            }
            let name: String = node.display_name.chars().take(512).collect();
            if name.trim().is_empty() {
                continue;
            }
            let key = (name, selected.contains(&node.id));
            if !self.blocks.contains_key(&key) {
                if self.blocks.len() >= 256 {
                    self.blocks.clear();
                }
                let block = self.shape(&key.0, key.1)?;
                self.blocks.insert(key.clone(), block);
            }
            let truncated = self.blocks[&key].metrics.width > width;
            let style = usize::from(key.1);
            if truncated && self.ellipses[style].is_none() {
                self.ellipses[style] = Some(self.shape("…", key.1)?);
            }
            let block = self.blocks.get(&key).unwrap();
            let ellipsis = truncated.then(|| self.ellipses[style].as_ref().unwrap());
            let text_width =
                ellipsis.map_or(width, |ellipsis| (width - ellipsis.metrics.width).max(0.0));
            // Clip decoration to its own frame; shaping stays cached across zooms.
            canvas.save();
            canvas.clip_rect(
                skia_safe::Rect::from_xywh(x, 0.0, text_width, viewport[1] as f32),
                skia_safe::ClipOp::Intersect,
                false,
            );
            block
                .paragraph
                .paint(canvas, (x, bottom - block.metrics.height));
            canvas.restore();
            if let Some(ellipsis) = ellipsis {
                canvas.save();
                canvas.clip_rect(
                    skia_safe::Rect::from_xywh(x, 0.0, width, viewport[1] as f32),
                    skia_safe::ClipOp::Intersect,
                    false,
                );
                ellipsis.paragraph.paint(
                    canvas,
                    (
                        x + text_width,
                        bottom - block.metrics.height + block.metrics.baseline
                            - ellipsis.metrics.baseline,
                    ),
                );
                canvas.restore();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::{Affine, Rect},
        tui::selection::HitGeometry,
    };

    fn node(name: &str, x: f32, parent: Option<usize>, enabled: bool) -> Node {
        Node {
            id: name.into(),
            name: name.into(),
            display_name: name.into(),
            bounds: Rect::new(x, 0.0, 80.0, 60.0),
            hit: HitGeometry {
                enabled,
                rect: Rect::new(0.0, 0.0, 80.0, 60.0),
                transform: Affine::translation(x, 0.0),
                clip: false,
            },
            target: serde_json::Value::Null,
            kind: "frame".into(),
            text_excerpt: String::new(),
            diagnostic_count: 0,
            has_error_diagnostic: false,
            parent,
            children: vec![],
        }
    }

    #[test]
    fn root_labels_follow_pan_keep_screen_size_and_skip_nested_or_hidden_nodes() {
        let mut nodes = vec![
            node("旧案 / Page 1", -100.0, None, true),
            node("Nested", 0.0, Some(0), true),
            node("Hidden", 100.0, None, false),
            node("   ", 200.0, None, true),
        ];
        nodes[0].hit.rect.width = 400.0;
        nodes[0].bounds.width = 400.0;
        let mut labels = Labels::default();
        let white = image::Rgba([255, 255, 255, 255]);
        let mut previous_ink = None;
        for zoom in [0.5, 1.0, 2.0] {
            for pan in [0.0, 20.0] {
                let mut image = image::RgbaImage::from_pixel(640, 320, white);
                let view = View {
                    center: [pan, 0.0],
                    zoom: Some(zoom),
                    time: 0.0,
                };
                labels
                    .paint(&mut image, &nodes, view, [640, 320], &[])
                    .unwrap();
                let ink: Vec<_> = image
                    .enumerate_pixels()
                    .filter(|(_, _, p)| **p != white)
                    .map(|(x, y, _)| (x, y))
                    .collect();
                assert!(!ink.is_empty(), "Japanese root name must render");
                let left = ink.iter().map(|p| p.0).min().unwrap();
                let top = ink.iter().map(|p| p.1).min().unwrap();
                assert!(left as f32 >= 320.0 + (-100.0 - pan as f32) * zoom);
                assert!(
                    ink.iter().all(|(_, y)| *y < 154),
                    "Name stays above the root"
                );
                let relative: Vec<_> = ink.iter().map(|(x, y)| (x - left, y - top)).collect();
                if let Some(previous) = &previous_ink {
                    assert_eq!(&relative, previous);
                }
                previous_ink = Some(relative);
                assert_eq!(labels.blocks.len(), 1, "Only the visible root gets a label");
            }
        }
    }

    #[test]
    fn root_labels_match_terminal_metrics_and_stay_constant_across_zoom() {
        let mut root = node("旧案 / Page 1", -100.0, None, true);
        root.hit.rect.width = 400.0;
        root.bounds.width = 400.0;
        let white = image::Rgba([255, 255, 255, 255]);
        let mut base_size: Option<[u32; 2]> = None;
        for factor in [1_u32, 2, 3] {
            let font = ratatui_image::FontSize::new(10 * factor as u16, 20 * factor as u16);
            let viewport = [640 * factor, 320 * factor];
            let mut labels = Labels::new(font);
            let mut previous_size = None;
            for zoom in [0.5, 1.0, 2.0] {
                let view = View {
                    zoom: Some(zoom * factor as f32),
                    ..Default::default()
                };
                let mut bitmap = image::RgbaImage::from_pixel(viewport[0], viewport[1], white);
                labels
                    .paint(
                        &mut bitmap,
                        std::slice::from_ref(&root),
                        view,
                        viewport,
                        &[],
                    )
                    .unwrap();
                let ink = bitmap
                    .enumerate_pixels()
                    .filter(|(_, _, pixel)| **pixel != white)
                    .map(|(x, y, _)| (x, y))
                    .collect::<Vec<_>>();
                assert!(!ink.is_empty());
                let size = [
                    ink.iter().map(|p| p.0).max().unwrap() - ink.iter().map(|p| p.0).min().unwrap()
                        + 1,
                    ink.iter().map(|p| p.1).max().unwrap() - ink.iter().map(|p| p.1).min().unwrap()
                        + 1,
                ];
                if let Some(previous) = previous_size {
                    assert_eq!(size, previous, "Canvas zoom must not resize the title");
                }
                previous_size = Some(size);
                if let Some(base) = base_size {
                    for axis in 0..2 {
                        assert!(
                            size[axis].abs_diff(base[axis] * factor) <= factor + 1,
                            "Title must track terminal metrics: font={font:?}, base={base:?}, size={size:?}"
                        );
                    }
                }
            }
            if base_size.is_none() {
                base_size = previous_size;
            }
            assert_eq!(labels.blocks.len(), 1, "Zooms reuse the same shaped title");
        }
    }

    #[test]
    fn long_root_titles_keep_their_font_size_and_stay_inside_the_frame_width() {
        let root = node(
            "Long title that must not overlap the next frame",
            -100.0,
            None,
            true,
        );
        let white = image::Rgba([255, 255, 255, 255]);
        for factor in [1_u32, 2, 3] {
            let font = ratatui_image::FontSize::new(10 * factor as u16, 20 * factor as u16);
            let viewport = [640 * factor, 320 * factor];
            let mut labels = Labels::new(font);
            for zoom in [0.5, 1.0, 2.0] {
                for selected in [vec![], vec![root.id.clone()]] {
                    let view = View {
                        zoom: Some(zoom * factor as f32),
                        ..Default::default()
                    };
                    let mut bitmap = image::RgbaImage::from_pixel(viewport[0], viewport[1], white);
                    labels
                        .paint(
                            &mut bitmap,
                            std::slice::from_ref(&root),
                            view,
                            viewport,
                            &selected,
                        )
                        .unwrap();
                    let ink = bitmap
                        .enumerate_pixels()
                        .filter(|(_, _, pixel)| **pixel != white)
                        .map(|(x, y, _)| (x, y))
                        .collect::<Vec<_>>();
                    assert!(!ink.is_empty());
                    let left = (root.bounds.x - view.region(viewport).x) * view.zoom.unwrap();
                    let right = left + root.bounds.width * view.zoom.unwrap();
                    assert!(
                        ink.iter()
                            .all(|(x, _)| *x as f32 >= left && (*x as f32) < right),
                        "Titles must not paint over adjacent frame names"
                    );
                    let height = ink.iter().map(|p| p.1).max().unwrap()
                        - ink.iter().map(|p| p.1).min().unwrap()
                        + 1;
                    assert!(height >= 9 * factor, "Truncation must not shrink the font");
                }
            }
            assert_eq!(
                labels.blocks.len(),
                2,
                "Widths and zooms reuse the shaped titles"
            );
        }
    }

    #[test]
    fn root_labels_have_room_above_tall_artwork_after_initial_fit() {
        for factor in [1_u32, 2, 3] {
            let mut root = node("旧案 / Page 1", -100.0, None, true);
            root.hit.rect.height = 800.0;
            root.bounds.height = 800.0;
            let font = ratatui_image::FontSize::new(10 * factor as u16, 20 * factor as u16);
            let mut labels = Labels::new(font);
            let pixels = [240 * factor, 120 * factor];
            let mut view = View::default();
            view.fit_canvas(root.bounds, pixels, labels.top_padding());
            let region = view.region(pixels);
            assert!((root.bounds.y - region.y) * view.zoom.unwrap() >= labels.top_padding() as f32);
            assert!((root.bounds.bottom() - region.y) * view.zoom.unwrap() <= pixels[1] as f32);
            let white = image::Rgba([255, 255, 255, 255]);
            let mut image = image::RgbaImage::from_pixel(pixels[0], pixels[1], white);
            labels
                .paint(&mut image, &[root], view, pixels, &[])
                .unwrap();
            let ink: Vec<_> = image
                .enumerate_pixels()
                .filter(|(_, _, p)| **p != white)
                .collect();
            assert!(!ink.is_empty());
            assert!(ink.iter().all(|(_, y, _)| *y > 0));
        }
    }

    #[test]
    fn root_labels_highlight_selection_and_clip_at_viewport_edges() {
        let nodes = vec![node("Page", -100.0, None, true)];
        let mut labels = Labels::default();
        let white = image::Rgba([255, 255, 255, 255]);
        let mut image = image::RgbaImage::from_pixel(240, 120, white);
        let view = View {
            zoom: Some(1.0),
            ..Default::default()
        };
        labels
            .paint(&mut image, &nodes, view, [240, 120], &["Page".into()])
            .unwrap();
        assert!(
            image
                .pixels()
                .any(|p| i16::from(p.0[2]) - i16::from(p.0[0]) > 60)
        );
        let mut image = image::RgbaImage::from_pixel(240, 120, white);
        let offscreen = View {
            center: [0.0, 100.0],
            ..view
        };
        labels
            .paint(&mut image, &nodes, offscreen, [240, 120], &[])
            .unwrap();
        assert!(image.pixels().all(|p| *p == white));
        let mut clipped = image::RgbaImage::from_pixel(240, 120, white);
        let edge = View {
            center: [30.0, -50.0],
            ..view
        };
        labels
            .paint(&mut clipped, &nodes, edge, [240, 120], &[])
            .unwrap();
        assert!(clipped.pixels().any(|p| *p != white));
    }
}
