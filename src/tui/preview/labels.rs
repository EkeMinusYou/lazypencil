//! Root names decorate terminal pixels without changing document exports.
use super::{Node, View};
use crate::{
    Result,
    text::{Fonts, TextBlock, TextSpec},
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(in crate::tui) struct Labels {
    fonts: Option<Fonts>,
    blocks: BTreeMap<(String, bool), TextBlock>,
}

impl Labels {
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
            let bottom = (bounds.y - region.y) * zoom - 6.0;
            if bottom <= 0.0 || bottom > viewport[1] as f32 + 24.0 || x >= viewport[0] as f32 {
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
                if self.fonts.is_none() {
                    self.fonts = Some(Fonts::new(&[])?);
                }
                let block = self.fonts.as_mut().unwrap().layout(
                    &TextSpec {
                        content: key.0.clone(),
                        size: 13.0,
                        color: {
                            let [r, g, b] = if key.1 {
                                super::super::theme::BLUE_RGB
                            } else {
                                super::super::theme::SECONDARY_RGB
                            };
                            skia_safe::Color::from_rgb(r, g, b)
                        },
                        ..Default::default()
                    },
                    None,
                )?;
                self.blocks.insert(key.clone(), block);
            }
            let block = self.blocks.get(&key).unwrap();
            block
                .paragraph
                .paint(canvas, (x, bottom - block.metrics.height));
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
        let nodes = vec![
            node("旧案 / Page 1", -100.0, None, true),
            node("Nested", 0.0, Some(0), true),
            node("Hidden", 100.0, None, false),
            node("   ", 200.0, None, true),
        ];
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
    fn root_labels_have_room_above_tall_artwork_after_initial_fit() {
        let mut root = node("旧案 / Page 1", -100.0, None, true);
        root.hit.rect.height = 800.0;
        root.bounds.height = 800.0;
        let mut view = View::default();
        view.fit_canvas(root.bounds, [240, 120]);
        let region = view.region([240, 120]);
        assert!((root.bounds.y - region.y) * view.zoom.unwrap() >= 28.0);
        assert!((root.bounds.bottom() - region.y) * view.zoom.unwrap() <= 120.0);
        let white = image::Rgba([255, 255, 255, 255]);
        let mut image = image::RgbaImage::from_pixel(240, 120, white);
        Labels::default()
            .paint(&mut image, &[root], view, [240, 120], &[])
            .unwrap();
        let ink: Vec<_> = image
            .enumerate_pixels()
            .filter(|(_, _, p)| **p != white)
            .collect();
        assert!(!ink.is_empty());
        assert!(ink.iter().all(|(_, y, _)| *y > 0));
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
