//! Semantic terminal colors and single-cell symbols shared by the workspace.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub(super) const FRAME: Color = Color::DarkGray;
pub(super) const MUTED: Color = Color::DarkGray;
pub(super) const SECONDARY: Color = Color::Gray;
pub(super) const FOCUS: Color = Color::Green;
pub(super) const KEY: Color = Color::Blue;
pub(super) const WORKING: Color = KEY;
pub(super) const NODES: Color = Color::Blue;
pub(super) const NODE_SELECTION: Color = Color::Rgb(150, 182, 250);
pub(super) const PREVIEW: Color = Color::Cyan;
pub(super) const AGENT: Color = Color::Magenta;
pub(super) const USER: Color = Color::White;
pub(super) const USER_BACKGROUND: Color = Color::Rgb(47, 49, 60);
pub(super) const RUNNING: Color = Color::Yellow;
pub(super) const SUCCESS: Color = Color::Green;
pub(super) const ERROR: Color = Color::Red;
pub(super) const PROMPT_BACKGROUND: Color = Color::Rgb(28, 30, 42);

pub(super) fn title(
    icon: &str,
    label: impl Into<String>,
    color: Color,
    focused: bool,
) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(
            label.into(),
            Style::default()
                .fg(if focused { FOCUS } else { Color::Reset })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ])
}

pub(super) fn node_selected(focused: bool) -> Style {
    Style::default()
        .bg(if focused {
            Color::Rgb(43, 45, 61)
        } else {
            Color::Rgb(30, 32, 44)
        })
        .fg(NODE_SELECTION)
        .add_modifier(if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}
