//! Semantic terminal colors and single-cell symbols shared by the workspace.
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

// Keep terminal chrome aligned with the semantic tokens in design.pen.
pub(super) const CANVAS: Color = Color::Rgb(30, 30, 46);
pub(super) const TEXT: Color = Color::Rgb(205, 214, 244);
pub(super) const BLUE_RGB: [u8; 3] = [137, 180, 250];
pub(super) const SECONDARY_RGB: [u8; 3] = [166, 173, 200];
pub(super) const SELECTION_TINT_ALPHA: u16 = 26;
pub(super) const FRAME: Color = Color::Rgb(108, 112, 134);
pub(super) const MUTED: Color = FRAME;
pub(super) const SECONDARY: Color = rgb(SECONDARY_RGB);
pub(super) const FOCUS: Color = Color::Rgb(166, 227, 161);
pub(super) const KEY: Color = rgb(BLUE_RGB);
pub(super) const WORKING: Color = KEY;
pub(super) const NODES: Color = KEY;
pub(super) const NODE_SELECTION: Color = KEY;
pub(super) const SELECTION_BACKGROUND: Color = Color::Rgb(69, 71, 90);
pub(super) const PREVIEW: Color = Color::Rgb(137, 220, 235);
pub(super) const AGENT: Color = Color::Rgb(203, 166, 247);
pub(super) const USER: Color = TEXT;
pub(super) const USER_BACKGROUND: Color = Color::Rgb(49, 50, 68);
pub(super) const RUNNING: Color = Color::Rgb(249, 226, 175);
pub(super) const SUCCESS: Color = FOCUS;
pub(super) const ERROR: Color = Color::Rgb(243, 139, 168);
pub(super) const PROMPT_BACKGROUND: Color = Color::Rgb(24, 24, 37);

const fn rgb([r, g, b]: [u8; 3]) -> Color {
    Color::Rgb(r, g, b)
}

pub(super) fn base() -> Style {
    Style::default().fg(TEXT).bg(CANVAS)
}

pub(super) fn modal_selected() -> Style {
    Style::default().fg(CANVAS).bg(PREVIEW)
}

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
                .fg(if focused { FOCUS } else { color })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ])
}

pub(super) fn node_selected(focused: bool) -> Style {
    Style::default()
        .bg(SELECTION_BACKGROUND)
        .fg(NODE_SELECTION)
        .add_modifier(if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}
