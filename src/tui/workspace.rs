//! Shared workspace geometry for an open document and the startup shell.
use crate::config::keys::PanelLayout;
use ratatui::layout::{Constraint, Layout, Rect};

pub(super) const FOOTER_HEIGHT: u16 = 2;

#[derive(Clone, Copy, Debug)]
pub(super) struct Areas {
    pub header: Rect,
    pub panes: [Rect; 3],
    pub footer: Rect,
}

pub(super) fn areas(
    area: Rect,
    layout: &PanelLayout,
    nodes_collapsed: bool,
    agent_collapsed: bool,
    footer_height: u16,
) -> Areas {
    sized_areas(
        area,
        layout,
        None,
        nodes_collapsed,
        agent_collapsed,
        footer_height,
    )
}

/// Preferred side widths as shares of the whole workspace, including hidden panes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Widths {
    pub nodes: f64,
    pub agent: f64,
}

impl Widths {
    pub fn configured(layout: &PanelLayout) -> Self {
        let total = f64::from(layout.nodes) + f64::from(layout.preview) + f64::from(layout.agent);
        Self {
            nodes: f64::from(layout.nodes) / total,
            agent: f64::from(layout.agent) / total,
        }
    }
}

pub(super) const MIN_WIDTHS: [u16; 3] = [12, 20, 18];

pub(super) fn sized_areas(
    area: Rect,
    layout: &PanelLayout,
    widths: Option<Widths>,
    nodes_collapsed: bool,
    agent_collapsed: bool,
    footer_height: u16,
) -> Areas {
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(footer_height),
    ])
    .split(area);
    let total = u32::from(layout.nodes) + u32::from(layout.preview) + u32::from(layout.agent);
    let nodes = if nodes_collapsed {
        0
    } else {
        u32::from(layout.nodes)
    };
    let agent = if agent_collapsed {
        0
    } else {
        u32::from(layout.agent)
    };
    let panes = Layout::horizontal([
        Constraint::Ratio(nodes, total),
        Constraint::Ratio(total - nodes - agent, total),
        Constraint::Ratio(agent, total),
    ])
    .split(sections[1]);
    let width = sections[1].width;
    let minimum = if width >= 50 { MIN_WIDTHS } else { [0; 3] };
    let min_nodes = if nodes_collapsed { 0 } else { minimum[0] };
    let min_agent = if agent_collapsed { 0 } else { minimum[2] };
    let mut nodes = if nodes_collapsed {
        0
    } else {
        widths
            .map_or(panes[0].width, |w| {
                (w.nodes * f64::from(width)).round() as u16
            })
            .max(min_nodes)
    };
    let mut agent = if agent_collapsed {
        0
    } else {
        widths
            .map_or(panes[2].width, |w| {
                (w.agent * f64::from(width)).round() as u16
            })
            .max(min_agent)
    };
    let available = width.saturating_sub(minimum[1]);
    if u32::from(nodes) + u32::from(agent) > u32::from(available) {
        let extra = u32::from(available - min_nodes - min_agent);
        let node_extra = u32::from(nodes - min_nodes);
        let total_extra = node_extra + u32::from(agent - min_agent);
        nodes = min_nodes + (extra * node_extra / total_extra) as u16;
        agent = available - nodes;
    }
    let body = sections[1];
    let preview = width - nodes - agent;
    Areas {
        header: sections[0],
        panes: [
            Rect::new(body.x, body.y, nodes, body.height),
            Rect::new(body.x + nodes, body.y, preview, body.height),
            Rect::new(body.x + nodes + preview, body.y, agent, body.height),
        ],
        footer: sections[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_widths_respect_minima_and_restore_preferences_after_terminal_resize() {
        let layout = PanelLayout::default();
        let preferred = Some(Widths {
            nodes: 0.35,
            agent: 0.4,
        });
        for width in 50..500 {
            for nodes_hidden in [false, true] {
                for agent_hidden in [false, true] {
                    let panes = sized_areas(
                        Rect::new(3, 4, width, 30),
                        &layout,
                        preferred,
                        nodes_hidden,
                        agent_hidden,
                        2,
                    )
                    .panes;
                    assert_eq!(panes.iter().map(|p| p.width).sum::<u16>(), width);
                    assert_eq!(panes[0].right(), panes[1].x);
                    assert_eq!(panes[1].right(), panes[2].x);
                    assert_eq!(panes[2].right(), width + 3);
                    assert!(panes[1].width >= MIN_WIDTHS[1]);
                    for (i, hidden) in [(0, nodes_hidden), (2, agent_hidden)] {
                        if hidden {
                            assert_eq!(panes[i].width, 0);
                        } else {
                            assert!(panes[i].width >= MIN_WIDTHS[i]);
                        }
                    }
                }
            }
        }
        let full = sized_areas(
            Rect::new(0, 0, 200, 30),
            &layout,
            preferred,
            false,
            false,
            2,
        );
        assert_eq!(full.panes.map(|p| p.width), [70, 50, 80]);
        let narrow = sized_areas(Rect::new(0, 0, 50, 30), &layout, preferred, false, false, 2);
        assert_eq!(narrow.panes.map(|p| p.width), MIN_WIDTHS);
        let restored = sized_areas(
            Rect::new(0, 0, 200, 30),
            &layout,
            preferred,
            false,
            false,
            2,
        );
        assert_eq!(full.panes, restored.panes);
    }

    #[test]
    fn hidden_panel_preferences_can_exceed_available_space_when_restored() {
        let preferred = Some(Widths {
            nodes: 0.8,
            agent: 0.8,
        });
        let layout = PanelLayout::default();
        let hidden = sized_areas(Rect::new(0, 0, 200, 30), &layout, preferred, true, false, 2);
        assert_eq!(hidden.panes.map(|p| p.width), [0, 40, 160]);
        let shown = sized_areas(
            Rect::new(0, 0, 200, 30),
            &layout,
            preferred,
            false,
            false,
            2,
        );
        assert_eq!(shown.panes[1].width, 20);
        assert_eq!(shown.panes.iter().map(|p| p.width).sum::<u16>(), 200);
    }
}
