//! Canvas selection is view state; exported pixels and document bytes stay untouched.
use super::{App, Focus, graphics, preview, theme};
use crate::{
    Result,
    geometry::{Affine, Rect},
};
use crossterm::event::KeyModifiers;
use ratatui::layout::Position;
pub(super) mod render;

#[derive(Clone, Copy)]
pub(super) struct HitGeometry {
    pub enabled: bool,
    pub rect: Rect,
    pub transform: Affine,
    pub clip: bool,
}
impl Default for HitGeometry {
    fn default() -> Self {
        Self {
            enabled: true,
            rect: Rect::default(),
            transform: Affine::default(),
            clip: false,
        }
    }
}
fn contains(rect: Rect, point: [f64; 2]) -> bool {
    rect.width > 0.0
        && rect.height > 0.0
        && point[0] >= f64::from(rect.x)
        && point[0] <= f64::from(rect.right())
        && point[1] >= f64::from(rect.y)
        && point[1] <= f64::from(rect.bottom())
}
fn local(node: &preview::Node, point: [f64; 2]) -> Option<[f64; 2]> {
    let inverse = node.hit.transform.inverse()?;
    let (x, y) = inverse.point(point[0] as f32, point[1] as f32);
    Some([f64::from(x), f64::from(y)])
}
fn visible_at(nodes: &[preview::Node], index: usize, point: [f64; 2]) -> bool {
    let mut ancestor = Some(index);
    while let Some(index) = ancestor {
        let node = &nodes[index];
        if !node.hit.enabled
            || (node.hit.clip
                && local(node, point).is_none_or(|point| !contains(node.hit.rect, point)))
        {
            return false;
        }
        ancestor = node.parent;
    }
    true
}
pub(super) fn shape_bounds(node: &preview::Node) -> Rect {
    if node.hit.rect.width <= 0.0 || node.hit.rect.height <= 0.0 {
        node.bounds
    } else {
        node.hit.rect.transformed(node.hit.transform)
    }
}
fn hit_test(nodes: &[preview::Node], point: [f64; 2]) -> Option<usize> {
    nodes.iter().enumerate().rev().find_map(|(index, node)| {
        let p = local(node, point)?;
        let degenerate = node.hit.rect.width <= 0.0 || node.hit.rect.height <= 0.0;
        let inside = if degenerate {
            contains(node.bounds, point)
        } else {
            contains(node.hit.rect, p)
        };
        if !inside || !visible_at(nodes, index, point) {
            return None;
        }
        if node.kind == "ellipse" && !degenerate {
            let rx = f64::from(node.hit.rect.width) / 2.0;
            let ry = f64::from(node.hit.rect.height) / 2.0;
            if ((p[0] - rx) / rx).powi(2) + ((p[1] - ry) / ry).powi(2) > 1.0 {
                return None;
            }
        }
        Some(index)
    })
}

#[derive(Default)]
pub(super) struct Selection {
    pub ids: Vec<String>,
    document: Option<std::path::PathBuf>,
    pub presentation: Option<graphics::Encoded>,
    presentation_view: Option<preview::View>,
    clock: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    marquee: Option<Rect>,
    base: Vec<String>,
    origin: Option<[f64; 2]>,
    additive: bool,
}
impl Selection {
    pub fn accept_overlay(&mut self, output: render::Finished) -> Result<bool> {
        if self
            .clock
            .as_ref()
            .is_some_and(|clock| clock.load(std::sync::atomic::Ordering::Relaxed) == output.serial)
            && let Some(image) = output.result?
        {
            self.presentation = Some(image);
            self.presentation_view = Some(output.view);
            return Ok(true);
        }
        Ok(false)
    }
    fn invalidate(&self) {
        if let Some(clock) = &self.clock {
            clock.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    pub fn presentation_for(
        &self,
        view: preview::View,
        size: ratatui::layout::Size,
    ) -> Option<&graphics::Encoded> {
        self.presentation
            .as_ref()
            .filter(|image| self.presentation_view == Some(view) && image.size() == size)
    }
    pub fn bind(&mut self, path: &std::path::Path) {
        if self.document.as_deref() != Some(path) {
            self.clear();
            self.document = Some(path.to_owned());
        }
    }
    pub fn clear(&mut self) {
        self.ids.clear();
        self.cancel();
        self.presentation = None;
        self.invalidate();
    }
    pub fn abort(&mut self) {
        if self.dragging() {
            self.ids.clone_from(&self.base);
            self.cancel();
            self.presentation = None;
            self.invalidate();
        }
    }
    pub fn cancel(&mut self) {
        self.marquee = None;
        self.origin = None;
        self.base.clear();
    }
    pub fn retain(&mut self, nodes: &[preview::Node]) {
        let valid = nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.ids.retain(|id| valid.contains(id.as_str()));
        self.presentation = None;
        self.invalidate();
    }
    pub fn dragging(&self) -> bool {
        self.origin.is_some()
    }
}

impl App {
    fn canvas_point(&self, position: Position, picker: &graphics::Backend) -> Option<[f64; 2]> {
        let preview = self.preview.as_ref().filter(|p| !p.attachment)?;
        let view = if self.camera.is_some() {
            self.view
        } else {
            preview.view
        };
        let area = self.pointer.area(super::mouse::Target::Canvas)?;
        let font = picker.font_size();
        let zoom = f64::from(view.zoom?);
        Some([
            view.center[0]
                + (f64::from(position.x) + 0.5 - f64::from(area.x) - f64::from(area.width) / 2.0)
                    * f64::from(font.width)
                    / zoom,
            view.center[1]
                + (f64::from(position.y) + 0.5 - f64::from(area.y) - f64::from(area.height) / 2.0)
                    * f64::from(font.height)
                    / zoom,
        ])
    }
    pub(super) fn begin_canvas_selection(
        &mut self,
        position: Position,
        modifiers: KeyModifiers,
        picker: &graphics::Backend,
    ) {
        self.selection.cancel();
        self.selection.invalidate();
        self.selection.origin = self.canvas_point(position, picker);
        self.selection.base.clone_from(&self.selection.ids);
        self.selection.additive = modifiers.contains(KeyModifiers::SHIFT);
    }
    pub(super) fn drag_canvas_selection(
        &mut self,
        position: Position,
        picker: &graphics::Backend,
        worker: &super::Worker,
    ) -> Result<()> {
        let Some(origin) = self.selection.origin else {
            return Ok(());
        };
        let Some(point) = self.canvas_point(position, picker) else {
            self.selection.cancel();
            return Ok(());
        };
        let rect = Rect::new(
            origin[0].min(point[0]) as f32,
            origin[1].min(point[1]) as f32,
            (origin[0] - point[0]).abs() as f32,
            (origin[1] - point[1]).abs() as f32,
        );
        self.selection.marquee = Some(rect);
        let nodes = &self.preview.as_ref().unwrap().nodes;
        let mut selected = vec![false; nodes.len()];
        let mut covered = vec![false; nodes.len()];
        for (index, node) in nodes.iter().enumerate() {
            covered[index] = node
                .parent
                .is_some_and(|parent| selected[parent] || covered[parent]);
            let bounds = shape_bounds(node);
            let center = [
                f64::from(bounds.x + bounds.width / 2.0),
                f64::from(bounds.y + bounds.height / 2.0),
            ];
            selected[index] = contains(rect, [f64::from(bounds.x), f64::from(bounds.y)])
                && contains(
                    rect,
                    [f64::from(bounds.right()), f64::from(bounds.bottom())],
                )
                && visible_at(nodes, index, center)
                && !covered[index];
        }
        self.selection.ids = if self.selection.additive {
            self.selection.base.clone()
        } else {
            Vec::new()
        };
        let mut included = self
            .selection
            .ids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        for (index, node) in nodes.iter().enumerate() {
            if selected[index] && included.insert(node.id.clone()) {
                self.selection.ids.push(node.id.clone());
            }
        }
        self.refresh_selection_presentation(picker, worker)
    }
    pub(super) fn finish_canvas_selection(
        &mut self,
        position: Position,
        moved: bool,
        picker: &graphics::Backend,
        worker: &super::Worker,
    ) -> Result<()> {
        if !self.selection.dragging() {
            return Ok(());
        }
        if !moved {
            let hit = self
                .canvas_point(position, picker)
                .and_then(|point| hit_test(&self.preview.as_ref()?.nodes, point));
            let id = hit.map(|index| self.preview.as_ref().unwrap().nodes[index].id.clone());
            if !self.selection.additive {
                self.selection.ids.clear();
            }
            if let Some(id) = id {
                if self.selection.additive && self.selection.ids.contains(&id) {
                    self.selection.ids.retain(|previous| previous != &id);
                } else {
                    self.selection.ids.push(id);
                }
            }
        }
        self.selection.cancel();
        let primary = self.selection.ids.first().cloned();
        if primary.is_some() {
            self.filter.clear();
            self.searching = false;
            self.diagnostics.active = false;
        }
        self.refresh_nodes(primary.as_deref());
        self.pointer.reattach(super::mouse::ListTarget::Nodes);
        self.focus = Focus::Preview;
        self.refresh_selection_presentation(picker, worker)?;
        let count = self.selection.ids.len();
        self.set_status(if count == 0 {
            "Selection cleared".into()
        } else {
            format!("{count} node{} selected", if count == 1 { "" } else { "s" })
        });
        self.dirty = true;
        Ok(())
    }
    pub(super) fn refresh_selection_presentation(
        &mut self,
        picker: &graphics::Backend,
        worker: &super::Worker,
    ) -> Result<()> {
        // Keep the last visible marquee during a drag, and let an in-flight
        // frame finish even when newer pointer positions are already queued.
        // Releasing/cancelling the drag still invalidates all marquee frames.
        if !self.selection.dragging() {
            self.selection.presentation = None;
            self.selection.invalidate();
        }
        if self.selection.ids.is_empty() && self.selection.marquee.is_none() {
            return Ok(());
        }
        let Some(preview) = self.preview.as_ref().filter(|preview| !preview.attachment) else {
            return Ok(());
        };
        let view = if self.camera.is_some() {
            self.view
        } else {
            preview.view
        };
        let size = if self.camera.is_some() {
            self.cells
        } else {
            preview.protocol.size()
        };
        let viewport = preview::pixel_size(picker, size)?;
        let selected = self
            .selection
            .ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let outlines = preview
            .nodes
            .iter()
            .filter(|node| selected.contains(node.id.as_str()))
            .map(shape_bounds)
            .collect::<Vec<_>>();
        self.selection.clock = Some(worker.present(render::Request {
            source: preview.image.clone(),
            nodes: preview.nodes.clone(),
            selected: self.selection.ids.clone(),
            before: preview.view,
            view,
            size,
            viewport,
            outlines,
            marquee: self.selection.marquee,
            picker: picker.clone(),
            image_id: self.camera_id,
            serial: 0,
            clock: std::sync::Arc::default(),
        }));
        Ok(())
    }
}
#[cfg(test)]
fn outline_pixels(image: &mut image::RgbaImage, rect: [i64; 4], fill: bool) {
    outline_pixels_checked(image, rect, fill, &|| false);
}
fn outline_pixels_checked(
    image: &mut image::RgbaImage,
    [left, top, right, bottom]: [i64; 4],
    fill: bool,
    cancelled: &impl Fn() -> bool,
) -> bool {
    let width = i64::from(image.width());
    let height = i64::from(image.height());
    if fill {
        for y in top.max(0)..bottom.saturating_add(1).min(height) {
            if cancelled() {
                return false;
            }
            for x in left.max(0)..right.saturating_add(1).min(width) {
                let pixel = image.get_pixel_mut(x as u32, y as u32);
                let alpha = theme::SELECTION_TINT_ALPHA;
                for (channel, color) in pixel.0[..3].iter_mut().zip(theme::BLUE_RGB) {
                    *channel = ((u16::from(*channel) * (255 - alpha) + u16::from(color) * alpha)
                        / 255) as u8;
                }
            }
        }
    }
    for [x0, y0, x1, y1] in [
        [left, top, right.saturating_add(1), top.saturating_add(2)],
        [
            left,
            bottom.saturating_sub(1),
            right.saturating_add(1),
            bottom.saturating_add(1),
        ],
        [left, top, left.saturating_add(2), bottom.saturating_add(1)],
        [
            right.saturating_sub(1),
            top,
            right.saturating_add(1),
            bottom.saturating_add(1),
        ],
    ] {
        for y in y0.max(0)..y1.min(height) {
            if cancelled() {
                return false;
            }
            for x in x0.max(0)..x1.min(width) {
                let [r, g, b] = theme::BLUE_RGB;
                *image.get_pixel_mut(x as u32, y as u32) = image::Rgba([r, g, b, 255]);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn node(
        id: &str,
        transform: Affine,
        rect: Rect,
        parent: Option<usize>,
        clip: bool,
    ) -> preview::Node {
        preview::Node {
            id: id.into(),
            name: id.into(),
            display_name: id.into(),
            bounds: rect.transformed(transform),
            hit: HitGeometry {
                enabled: true,
                rect,
                transform,
                clip,
            },
            target: json!({"id":id}),
            kind: "rectangle".into(),
            text_excerpt: String::new(),
            diagnostic_count: 0,
            has_error_diagnostic: false,
            parent,
            children: vec![],
        }
    }
    #[test]
    fn hit_testing_respects_paint_order_transforms_disabled_nodes_and_clipping() {
        let size = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut nodes = vec![
            node("background", Affine::default(), size, None, false),
            node(
                "clip",
                Affine::translation(20.0, 20.0),
                Rect::new(0.0, 0.0, 40.0, 40.0),
                None,
                true,
            ),
            node(
                "child",
                Affine::translation(40.0, 40.0),
                size,
                Some(1),
                false,
            ),
            node("disabled", Affine::default(), size, None, false),
        ];
        nodes[3].hit.enabled = false;
        assert_eq!(hit_test(&nodes, [50.0, 50.0]), Some(2));
        assert_eq!(hit_test(&nodes, [90.0, 90.0]), Some(0));
        assert_eq!(hit_test(&nodes, [-1.0, 50.0]), None);
        nodes[1].hit.enabled = false;
        assert_eq!(hit_test(&nodes, [50.0, 50.0]), Some(0));
        let mut stroke = node(
            "stroke",
            Affine::translation(0.0, 110.0),
            Rect::new(0.0, 0.0, 100.0, 0.0),
            None,
            false,
        );
        stroke.bounds = Rect::new(0.0, 109.0, 100.0, 2.0);
        assert_eq!(hit_test(&[stroke], [50.0, 110.0]), Some(0));
        let rotated = node(
            "rotated",
            Affine::translation(200.0, 200.0).then(Affine::rotation(45.0)),
            size,
            None,
            false,
        );
        let inside = rotated.hit.transform.point(50.0, 50.0);
        assert_eq!(
            hit_test(&[rotated], [f64::from(inside.0), f64::from(inside.1)]),
            Some(0)
        );
    }
    #[test]
    fn outlines_are_clipped_without_painting_an_offscreen_edge() {
        let mut image = image::RgbaImage::from_pixel(40, 40, image::Rgba([255, 255, 255, 255]));
        outline_pixels(&mut image, [-100, -100, 20, 20], false);
        assert_eq!(*image.get_pixel(0, 0), image::Rgba([255, 255, 255, 255]));
        assert_eq!(*image.get_pixel(20, 10), image::Rgba([137, 180, 250, 255]));
    }
}
