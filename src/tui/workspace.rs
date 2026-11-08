//! Shared workspace geometry for an open document and the startup shell.
use crate::config::keys::PanelLayout;
use ratatui::layout::{Constraint, Layout, Rect};

pub(super) const FOOTER_HEIGHT: u16 = 2;

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
    Areas {
        header: sections[0],
        panes: [panes[0], panes[1], panes[2]],
        footer: sections[2],
    }
}
