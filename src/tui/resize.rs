//! Shared boundary movement for pointer drags and the keyboard resize mode.
use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use workspace::{Areas, MIN_WIDTHS, Widths};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Boundary {
    Nodes,
    Agent,
}

impl Boundary {
    fn index(self) -> usize {
        match self {
            Self::Nodes => 0,
            Self::Agent => 1,
        }
    }

    pub fn area(self, panes: [Rect; 3]) -> Option<Rect> {
        let i = self.index();
        let left = panes[i];
        let right = panes[i + 1];
        (left.width > 0 && right.width > 0 && left.height > 2)
            .then(|| Rect::new(left.right() - 1, left.y + 1, 2, left.height - 2))
    }
}

#[derive(Clone, Copy)]
struct Drag {
    boundary: Boundary,
    origin: u16,
    panes: [Rect; 3],
    widths: Widths,
    original: Option<Widths>,
}

#[derive(Default)]
pub(super) struct Resize {
    pub keyboard: bool,
    drag: Option<Drag>,
    pub areas: Option<Areas>,
    pub widths: Option<Widths>,
}

impl Resize {
    pub fn active(&self) -> bool {
        self.keyboard || self.drag.is_some()
    }

    pub fn stop(&mut self) {
        self.keyboard = false;
        self.drag = None;
    }

    fn realized(&self, layout: &crate::config::keys::PanelLayout, panes: [Rect; 3]) -> Widths {
        let mut widths = self.widths.unwrap_or_else(|| Widths::configured(layout));
        let total = f64::from(panes.iter().map(|p| p.width).sum::<u16>());
        if panes[0].width > 0 {
            widths.nodes = f64::from(panes[0].width) / total;
        }
        if panes[2].width > 0 {
            widths.agent = f64::from(panes[2].width) / total;
        }
        widths
    }
}

fn moved(boundary: Boundary, delta: i32, panes: [Rect; 3], mut widths: Widths) -> Option<Widths> {
    let i = boundary.index();
    let delta = delta.clamp(
        i32::from(MIN_WIDTHS[i]) - i32::from(panes[i].width),
        i32::from(panes[i + 1].width) - i32::from(MIN_WIDTHS[i + 1]),
    );
    if delta == 0 {
        return None;
    }
    let total = f64::from(panes.iter().map(|p| p.width).sum::<u16>());
    match boundary {
        Boundary::Nodes => widths.nodes = f64::from(i32::from(panes[0].width) + delta) / total,
        Boundary::Agent => widths.agent = f64::from(i32::from(panes[2].width) - delta) / total,
    }
    Some(widths)
}

impl App {
    fn resize_areas(&self) -> Option<Areas> {
        self.resize.areas.map(|areas| {
            workspace::sized_areas(
                Rect::new(
                    areas.header.x,
                    areas.header.y,
                    areas.header.width,
                    areas.footer.bottom() - areas.header.y,
                ),
                &self.panel_layout,
                self.resize.widths,
                self.nodes_collapsed,
                self.agent_collapsed,
                self.footer_height(),
            )
        })
    }

    pub(super) fn start_panel_resize(&mut self) {
        self.resize.keyboard = true;
        self.resize.drag = None;
        self.pointer.cancel();
        self.native_input.cancel();
        self.selection.abort();
        self.dirty = true;
    }

    pub(super) fn resize_key(&mut self, key: KeyEvent, worker: &Worker) -> bool {
        if self.has_overlay() {
            self.resize.stop();
            return false;
        }
        if self.resize.drag.is_some() {
            if key.code == KeyCode::Esc {
                self.resize.widths = self.resize.drag.take().unwrap().original;
            } else {
                self.resize.drag = None;
            }
            self.dirty = true;
            return true;
        }
        if !self.resize.keyboard {
            return false;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
            || self.navigation.matches(Nav::ResizePanels, key)
        {
            self.resize.stop();
        } else if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            let focus = self.focus.next(
                key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                self.nodes_collapsed,
                self.agent_collapsed,
            );
            self.pointer_focus(focus, worker);
        } else if key.modifiers.is_empty()
            && let Some(right) = match key.code {
                KeyCode::Char('h') | KeyCode::Left => Some(false),
                KeyCode::Char('l') | KeyCode::Right => Some(true),
                _ => None,
            }
            && let Some(areas) = self.resize_areas()
        {
            let preferred = match self.focus {
                Focus::Nodes => Boundary::Nodes,
                Focus::Preview if !right => Boundary::Nodes,
                _ => Boundary::Agent,
            };
            let other = match preferred {
                Boundary::Nodes => Boundary::Agent,
                Boundary::Agent => Boundary::Nodes,
            };
            if let Some(boundary) = [preferred, other]
                .into_iter()
                .find(|b| b.area(areas.panes).is_some())
            {
                let total = areas.panes.iter().map(|p| p.width).sum::<u16>();
                let step = (i32::from(total) + 10) / 20;
                let widths = self.resize.realized(&self.panel_layout, areas.panes);
                if let Some(widths) = moved(
                    boundary,
                    if right { step } else { -step },
                    areas.panes,
                    widths,
                ) {
                    self.resize.widths = Some(widths);
                }
            }
        }
        self.dirty = true;
        true
    }

    pub(super) fn resize_mouse(&mut self, event: MouseEvent) -> bool {
        if self.has_overlay() {
            self.resize.stop();
            return false;
        }
        if let Some(drag) = self.resize.drag {
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    self.resize.widths = moved(
                        drag.boundary,
                        i32::from(event.column) - i32::from(drag.origin),
                        drag.panes,
                        drag.widths,
                    )
                    .or(drag.original);
                    if matches!(event.kind, MouseEventKind::Up(_)) {
                        self.resize.drag = None;
                    }
                    self.dirty = true;
                }
                MouseEventKind::Down(_) => self.resize.drag = None,
                _ => {}
            }
            return true;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(mouse::Target::Resize(boundary)) = self
                .pointer
                .at(ratatui::layout::Position::new(event.column, event.row))
            && let Some(areas) = self.resize_areas()
        {
            self.pointer.cancel();
            self.native_input.cancel();
            self.selection.abort();
            self.resize.drag = Some(Drag {
                boundary,
                origin: event.column,
                panes: areas.panes,
                widths: self.resize.realized(&self.panel_layout, areas.panes),
                original: self.resize.widths,
            });
            self.dirty = true;
            return true;
        }
        self.resize.keyboard
    }

    pub(super) fn draw_resize(&mut self, frame: &mut Frame, areas: Areas) {
        if self.has_overlay() {
            self.resize.stop();
            return;
        }
        for boundary in [Boundary::Nodes, Boundary::Agent] {
            if let Some(area) = boundary.area(areas.panes) {
                if self.mouse.enabled {
                    self.pointer.add(area, mouse::Target::Resize(boundary));
                }
                let selected = self.resize.drag.is_some_and(|d| d.boundary == boundary)
                    || self.resize.keyboard
                        && match self.focus {
                            Focus::Nodes => boundary == Boundary::Nodes,
                            Focus::Preview => true,
                            _ => boundary == Boundary::Agent,
                        };
                if selected {
                    for y in area.y..area.bottom() {
                        for x in area.x..area.right() {
                            frame.buffer_mut()[(x, y)].set_fg(theme::colors().focus);
                        }
                    }
                }
            }
        }
        if self.resize.active() {
            let name = match self.resize.drag.map(|drag| drag.boundary) {
                Some(Boundary::Nodes) => "Nodes / Preview",
                Some(Boundary::Agent) => "Preview / Agent",
                None => match self.focus {
                    Focus::Nodes => "Nodes",
                    Focus::Preview => "Preview",
                    _ => "Agent",
                },
            };
            let panes = areas.panes;
            let footer = areas.footer;
            frame.render_widget(
                Paragraph::new(format!(
                    "RESIZE · {name} · Nodes {} | Preview {} | Agent {}",
                    panes[0].width, panes[1].width, panes[2].width
                ))
                .style(Style::default().fg(theme::colors().focus)),
                Rect::new(footer.x, footer.bottom() - 1, footer.width, 1),
            );
            if !self.status_kind.is_error() && self.status_kind != footer::StatusKind::Attention {
                let hints = if self.resize.drag.is_some() {
                    "Drag to resize · Release: keep · Esc: undo"
                } else {
                    "h/← left · l/→ right · Tab panel · Esc/Enter done"
                };
                frame.render_widget(
                    Paragraph::new(hints).style(Style::default().fg(theme::colors().key)),
                    Rect::new(footer.x, footer.y, footer.width, 1),
                );
            }
        }
    }
}
