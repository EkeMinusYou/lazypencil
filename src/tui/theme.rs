//! Semantic terminal colors and single-cell symbols shared by the workspace.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub(super) const FRAME: Color = Color::DarkGray;
pub(super) const MUTED: Color = Color::DarkGray;
pub(super) const FOCUS: Color = Color::Green;
pub(super) const KEY: Color = Color::Blue;
pub(super) const NODES: Color = Color::Blue;
pub(super) const PREVIEW: Color = Color::Cyan;
pub(super) const AGENT: Color = Color::Magenta;
pub(super) const USER: Color = Color::Cyan;
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

pub(super) fn selected(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }
}
