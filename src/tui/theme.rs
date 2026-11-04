//! Semantic terminal colors shared by the workspace and document picker.
use crate::config::keys::Theme;
use ratatui::{
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
};
use std::cell::Cell;

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub canvas: Color,
    pub text: Color,
    pub frame: Color,
    pub muted: Color,
    pub secondary: Color,
    pub focus: Color,
    pub key: Color,
    pub working: Color,
    pub nodes: Color,
    pub node_selection: Color,
    pub preview: Color,
    pub agent: Color,
    pub user: Color,
    pub user_background: Color,
    pub running: Color,
    pub success: Color,
    pub error: Color,
    pub prompt_background: Color,
    pub selected_background: Color,
    pub inactive_background: Color,
    focused_border: Option<border::Set<'static>>,
}
impl Palette {
    // Keep the dark theme aligned with the semantic tokens in design.pen.
    const DARK: Self = Self {
        canvas: Color::Rgb(30, 30, 46),
        text: Color::Rgb(205, 214, 244),
        frame: Color::Rgb(108, 112, 134),
        muted: Color::Rgb(108, 112, 134),
        secondary: Color::Rgb(166, 173, 200),
        focus: Color::Rgb(166, 227, 161),
        key: Color::Rgb(137, 180, 250),
        working: Color::Rgb(137, 180, 250),
        nodes: Color::Rgb(137, 180, 250),
        node_selection: Color::Rgb(137, 180, 250),
        preview: Color::Rgb(137, 220, 235),
        agent: Color::Rgb(203, 166, 247),
        user: Color::Rgb(205, 214, 244),
        user_background: Color::Rgb(49, 50, 68),
        running: Color::Rgb(249, 226, 175),
        success: Color::Rgb(166, 227, 161),
        error: Color::Rgb(243, 139, 168),
        prompt_background: Color::Rgb(24, 24, 37),
        selected_background: Color::Rgb(69, 71, 90),
        inactive_background: Color::Rgb(69, 71, 90),
        focused_border: None,
    };
    const LIGHT: Self = Self {
        canvas: Color::Rgb(250, 250, 252),
        text: Color::Rgb(28, 35, 45),
        frame: Color::Rgb(94, 102, 115),
        muted: Color::Rgb(94, 102, 115),
        secondary: Color::Rgb(68, 77, 91),
        focus: Color::Rgb(0, 95, 184),
        key: Color::Rgb(28, 75, 172),
        working: Color::Rgb(28, 75, 172),
        nodes: Color::Rgb(28, 75, 172),
        node_selection: Color::Rgb(24, 63, 145),
        preview: Color::Rgb(0, 99, 117),
        agent: Color::Rgb(125, 45, 145),
        user: Color::Rgb(28, 35, 45),
        user_background: Color::Rgb(230, 234, 240),
        running: Color::Rgb(132, 83, 0),
        success: Color::Rgb(25, 105, 50),
        error: Color::Rgb(180, 35, 40),
        prompt_background: Color::Rgb(242, 244, 247),
        selected_background: Color::Rgb(215, 226, 247),
        inactive_background: Color::Rgb(232, 237, 245),
        focused_border: Some(border::DOUBLE),
    };
    fn monochrome(light: bool) -> Self {
        let rgb = |n| Color::Rgb(n, n, n);
        let mut palette = if light { Self::LIGHT } else { Self::DARK };
        let ink = rgb(if light { 32 } else { 230 });
        palette.canvas = rgb(if light { 250 } else { 30 });
        palette.text = ink;
        palette.frame = rgb(if light { 96 } else { 155 });
        palette.muted = palette.frame;
        palette.secondary = rgb(if light { 64 } else { 190 });
        palette.focus = ink;
        palette.key = ink;
        palette.working = ink;
        palette.nodes = ink;
        palette.node_selection = ink;
        palette.preview = ink;
        palette.agent = ink;
        palette.user = ink;
        palette.running = ink;
        palette.success = ink;
        palette.error = ink;
        palette.user_background = rgb(if light { 235 } else { 45 });
        palette.prompt_background = rgb(if light { 245 } else { 28 });
        palette.selected_background = rgb(if light { 218 } else { 58 });
        palette.inactive_background = rgb(if light { 235 } else { 36 });
        palette
    }
}

// UI colors are scoped to the drawing thread, including parallel test renderers.
thread_local! {
    static CURRENT: Cell<Palette> = const { Cell::new(Palette::DARK) };
}
pub(super) fn colors() -> Palette {
    CURRENT.get()
}
pub(super) struct Guard(Palette);
impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.set(self.0);
    }
}
pub(super) fn activate(theme: Theme, background: Option<[u8; 3]>) -> Guard {
    let previous = colors();
    select(theme, background);
    Guard(previous)
}
pub(super) fn select(theme: Theme, background: Option<[u8; 3]>) {
    CURRENT.set(resolve(
        theme,
        background,
        std::env::var("COLORFGBG").ok().as_deref(),
    ));
}
fn resolve(theme: Theme, background: Option<[u8; 3]>, colorfgbg: Option<&str>) -> Palette {
    let light = background
        .map(|[r, g, b]| {
            // Relative luminance separates light terminals from dark and mid-gray ones.
            let linear = |c: u8| {
                let c = f64::from(c) / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b) > 0.179
        })
        .or_else(|| {
            let index = colorfgbg?.rsplit(';').next()?.parse::<u8>().ok()?;
            (index < 16).then_some(matches!(index, 7 | 9..=15))
        });
    match theme {
        Theme::Dark => Palette::DARK,
        Theme::Light => Palette::LIGHT,
        Theme::Auto => match light {
            Some(true) => Palette::LIGHT,
            Some(false) => Palette::DARK,
            None => Palette::DARK,
        },
        Theme::Monochrome => Palette::monochrome(light.unwrap_or(false)),
    }
}

// Preview overlays retain document-independent semantic colors.
pub(super) const BLUE_RGB: [u8; 3] = [137, 180, 250];
pub(super) const SECONDARY_RGB: [u8; 3] = [166, 173, 200];
pub(super) const SELECTION_TINT_ALPHA: u16 = 26;

pub(super) fn base() -> Style {
    Style::default().fg(colors().text).bg(colors().canvas)
}

pub(super) fn canvas_background() -> image::Rgba<u8> {
    let Color::Rgb(r, g, b) = colors().canvas else {
        unreachable!("UI themes use explicit RGB canvas colors")
    };
    image::Rgba([r, g, b, 255])
}

pub(super) fn modal_selected() -> Style {
    Style::default().fg(colors().canvas).bg(colors().preview)
}

pub(super) fn permission_selected() -> Style {
    Style::default().fg(colors().canvas).bg(colors().running)
}

pub(super) fn border_set(focused: bool, regular: border::Set<'static>) -> border::Set<'static> {
    if focused {
        colors().focused_border.unwrap_or(regular)
    } else {
        regular
    }
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
                .fg(if focused { colors().focus } else { color })
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ])
}

pub(super) fn node_selected(focused: bool) -> Style {
    let colors = colors();
    Style::default()
        .bg(if focused {
            colors.selected_background
        } else {
            colors.inactive_background
        })
        .fg(colors.node_selection)
        .add_modifier(if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("Expected RGB palette")
        };
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }
    fn contrast(ink: Color, paper: Color) -> f64 {
        let (a, b) = (luminance(ink), luminance(paper));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn auto_selects_light_and_dark_from_terminal_color_before_environment_hints() {
        for background in [[255, 255, 255], [245, 245, 245], [230, 234, 240]] {
            let palette = resolve(Theme::Auto, Some(background), Some("15;0"));
            assert_eq!(palette.user, Palette::LIGHT.user);
            assert_eq!(palette.prompt_background, Palette::LIGHT.prompt_background);
        }
        for background in [[0, 0, 0], [17, 17, 27], [40, 44, 52]] {
            let palette = resolve(Theme::Auto, Some(background), Some("0;15"));
            assert_eq!(palette.prompt_background, Palette::DARK.prompt_background);
        }
        assert_eq!(
            resolve(Theme::Light, Some([0, 0, 0]), None).user,
            Palette::LIGHT.user
        );
        assert_eq!(
            resolve(Theme::Dark, Some([255, 255, 255]), None).user,
            Palette::DARK.user
        );
    }

    #[test]
    fn absent_background_uses_colorfgbg_or_a_consistent_dark_fallback() {
        assert_eq!(
            resolve(Theme::Auto, None, Some("0;15")).user,
            Palette::LIGHT.user
        );
        assert_eq!(
            resolve(Theme::Auto, None, Some("15;0")).user,
            Palette::DARK.user
        );
        for hint in [None, Some("invalid"), Some("0;256")] {
            let palette = resolve(Theme::Auto, None, hint);
            assert_eq!(palette.canvas, Palette::DARK.canvas);
            assert_eq!(palette.prompt_background, Palette::DARK.prompt_background);
            assert_eq!(palette.user_background, Palette::DARK.user_background);
            assert_eq!(palette.secondary, Palette::DARK.secondary);
        }
    }

    #[test]
    fn light_and_monochrome_text_keep_readable_contrast_on_their_surfaces() {
        for theme in [Theme::Light, Theme::Monochrome] {
            let palette = resolve(theme, Some([255, 255, 255]), None);
            for ink in [
                palette.muted,
                palette.secondary,
                palette.focus,
                palette.key,
                palette.nodes,
                palette.preview,
                palette.agent,
                palette.running,
                palette.success,
                palette.error,
            ] {
                assert!(
                    contrast(ink, Color::Rgb(255, 255, 255)) >= 4.5,
                    "{theme:?}: {ink:?}"
                );
                assert!(
                    contrast(ink, palette.prompt_background) >= 4.5,
                    "{theme:?}: {ink:?}"
                );
            }
            assert!(contrast(palette.user, palette.user_background) >= 4.5);
            assert!(contrast(palette.user, palette.prompt_background) >= 4.5);
            assert!(contrast(palette.node_selection, palette.selected_background) >= 4.5);
        }
    }

    #[test]
    fn scoped_themes_restore_colors_and_leave_other_rendering_threads_unchanged() {
        let previous = colors().prompt_background;
        {
            let _guard = activate(Theme::Light, Some([255, 255, 255]));
            assert_eq!(colors().prompt_background, Palette::LIGHT.prompt_background);
            let other = std::thread::spawn(|| colors().prompt_background)
                .join()
                .unwrap();
            assert_eq!(other, Palette::DARK.prompt_background);
        }
        assert_eq!(colors().prompt_background, previous);
    }
}
