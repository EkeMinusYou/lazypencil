//! The document modal shares the workspace's panels, palette and selection style.
use super::super::{centered, clean, composer, panel, theme, workspace};
use crate::{config::keys::PanelLayout, documents::Listing};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
};
use std::path::{Path, PathBuf};

pub(super) struct View<'a> {
    pub layout: &'a PanelLayout,
    pub listing: &'a Listing,
    pub paths: &'a [PathBuf],
    pub selected: usize,
    pub filter: &'a str,
    pub path: Option<&'a str>,
    pub notice: &'a str,
    pub done: bool,
    pub agent: Option<&'a str>,
}

impl View<'_> {
    pub fn draw(&self, frame: &mut Frame, state: &mut ListState) {
        empty_workspace(frame, self.layout, self.agent);
        let area = frame.area();
        let popup = popup(area);
        frame.render_widget(Clear, popup);
        let title = theme::title("▤", "Open a .pen file", theme::NODES, true);
        let title_width = title.width();
        let mut block = panel(title, true).padding(Padding::horizontal(1));
        let count = Line::styled(
            format!(
                " {} / {} files ",
                self.paths.len(),
                self.listing.documents.len()
            ),
            Style::default().fg(theme::SECONDARY),
        );
        if title_width + count.width() + 2 <= usize::from(popup.width) {
            block = block.title_top(count.right_aligned());
        }
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        if inner.height < 6 || inner.width < 12 {
            frame.render_widget(
                Paragraph::new("Resize the terminal to choose a document. Ctrl+C exits.")
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(u16::from(inner.height >= 12)),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(if self.notice.is_empty() { 1 } else { 3 }),
            Constraint::Length(2),
        ])
        .split(inner);
        frame.render_widget(
            Paragraph::new(fit_end(
                &clean(&self.listing.root.display().to_string()),
                sections[0].width as usize,
            ))
            .style(Style::default().fg(theme::SECONDARY)),
            sections[0],
        );
        let input = self.path.unwrap_or(self.filter);
        let label = if self.path.is_some() {
            "Path › "
        } else {
            "Filter › "
        };
        let label_width = Line::raw(label).width();
        let available = usize::from(sections[2].width).saturating_sub(label_width + 1);
        let input = input_tail(input, available);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label, Style::default().fg(theme::SECONDARY)),
                Span::raw(input.clone()),
            ]))
            .style(Style::default().bg(theme::PROMPT_BACKGROUND)),
            sections[2],
        );
        state.select((!self.paths.is_empty()).then_some(self.selected));
        if self.paths.is_empty() {
            let message = if self.done {
                "No matching .pen files"
            } else {
                "Searching for .pen files…"
            };
            frame.render_widget(
                Paragraph::new(message).style(Style::default().fg(theme::SECONDARY)),
                sections[3],
            );
        } else {
            let width = usize::from(sections[3].width).saturating_sub(2);
            let items: Vec<_> = self
                .paths
                .iter()
                .map(|path| {
                    ListItem::new(Line::from(vec![
                        Span::styled("✎ ", Style::default().fg(theme::NODES)),
                        Span::raw(fit_end(&clean(&path.to_string_lossy()), width)),
                    ]))
                })
                .collect();
            frame.render_stateful_widget(
                List::new(items).highlight_style(theme::node_selected(true)),
                sections[3],
                state,
            );
        }
        let (status, color) = if let Some(reason) = self.listing.stop_reason {
            let reason = match reason {
                "document_limit" => "file limit reached",
                "entry_limit" => "search limit reached",
                "time_limit" => "time limit reached",
                "search_error" => "search unavailable",
                _ => "some folders could not be searched",
            };
            (format!("Partial results · {reason}"), theme::RUNNING)
        } else if self.done {
            (
                format!("Search complete · {} files", self.listing.documents.len()),
                theme::SECONDARY,
            )
        } else {
            (
                format!("Searching… · {} files", self.listing.documents.len()),
                theme::SECONDARY,
            )
        };
        let mut messages = vec![Line::styled(status, Style::default().fg(color))];
        if !self.notice.is_empty() {
            messages.push(Line::styled(
                clean(self.notice),
                Style::default().fg(theme::ERROR),
            ));
        }
        frame.render_widget(
            Paragraph::new(messages).wrap(Wrap { trim: false }),
            sections[4],
        );
        frame.render_widget(
            Paragraph::new(vec![
                hints(&[("↑/↓", " select"), ("Enter", " open")]),
                hints(&[("Ctrl+L", " path / folder"), ("Esc", " clear")]),
            ]),
            sections[5],
        );
        if sections[2].width > 0 && sections[2].height > 0 {
            let x = label_width + Line::raw(input).width();
            frame.set_cursor_position((
                sections[2].x + x.min(usize::from(sections[2].width - 1)) as u16,
                sections[2].y,
            ));
        }
    }
}

fn popup(area: Rect) -> Rect {
    centered(
        area,
        area.width.saturating_sub(6).min(94),
        area.height.saturating_sub(6).min(22),
    )
}

pub(super) fn opening(frame: &mut Frame, layout: &PanelLayout, path: &Path, agent: Option<&str>) {
    empty_workspace(frame, layout, agent);
    let popup = popup(frame.area());
    frame.render_widget(Clear, popup);
    let block = panel(theme::title("↻", "Opening…", theme::RUNNING, true), true)
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let content = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .split(inner)[1];
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Loading…",
                Style::default()
                    .fg(theme::RUNNING)
                    .add_modifier(Modifier::BOLD),
            )
            .centered(),
            Line::raw(""),
            Line::styled(
                fit_end(&path.display().to_string(), content.width as usize),
                Style::default().fg(theme::SECONDARY),
            )
            .centered(),
        ]),
        content,
    );
}

fn empty_workspace(frame: &mut Frame, layout: &PanelLayout, agent: Option<&str>) {
    let areas = workspace::areas(frame.area(), layout, false, false, 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "✎ lazypencil",
                Style::default()
                    .fg(theme::AGENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " · No document open{}",
                    agent
                        .map(|status| format!(" · {status}"))
                        .unwrap_or_default()
                ),
                Style::default().fg(theme::SECONDARY),
            ),
        ])),
        areas.header,
    );
    for (area, icon, label, color) in [
        (areas.panes[0], "▤", "Nodes", theme::NODES),
        (areas.panes[1], "▣", "Preview", theme::PREVIEW),
        (areas.panes[2], "◉", "Agent", theme::AGENT),
    ] {
        frame.render_widget(panel(theme::title(icon, label, color, false), false), area);
    }
    let preview = areas.panes[1].inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    frame.render_widget(
        Paragraph::new("No document open").style(Style::default().fg(theme::SECONDARY)),
        preview,
    );
    let agent = areas.panes[2].inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let height = composer::prompt_height(1, agent.height.saturating_sub(1)).min(agent.height);
    let prompt = Rect::new(
        agent.x,
        agent.bottom().saturating_sub(height),
        agent.width,
        height,
    );
    let block = composer::prompt_block(false);
    frame.render_widget(
        Paragraph::new("Open a document first").block(block).style(
            Style::default()
                .fg(theme::SECONDARY)
                .bg(theme::PROMPT_BACKGROUND),
        ),
        prompt,
    );
    frame.render_widget(Paragraph::new(hints(&[("Ctrl+C", " quit")])), areas.footer);
}

fn hints(items: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![];
    for (index, (key, label)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(theme::MUTED)));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            label.to_string(),
            Style::default().fg(theme::SECONDARY),
        ));
    }
    Line::from(spans)
}

fn input_tail(text: &str, width: usize) -> String {
    let text = clean(text).replace(['\n', '\t'], " ");
    for (index, _) in text.char_indices() {
        if Line::raw(&text[index..]).width() <= width {
            return text[index..].to_owned();
        }
    }
    String::new()
}

fn fit_end(text: &str, width: usize) -> String {
    let text = clean(text).replace(['\n', '\t'], " ");
    if Line::raw(text.as_str()).width() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    format!("…{}", input_tail(&text, width - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Color};

    fn listing() -> Listing {
        Listing {
            root: PathBuf::from("/work/designs"),
            documents: vec![
                "screens/login.pen".into(),
                "screens/settings.pen".into(),
                "設定.pen".into(),
            ],
            complete: true,
            stop_reason: None,
            entries_scanned: 5,
            unreadable_directories: 0,
        }
    }
    fn text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    x += Line::raw(symbol).width().max(1) as u16;
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn render(width: u16, height: u16, name: &str) -> (Buffer, ratatui::layout::Position) {
        let listing = listing();
        let layout = PanelLayout::default();
        let view = View {
            layout: &layout,
            listing: &listing,
            paths: &listing.documents,
            selected: 1,
            filter: "",
            path: None,
            notice: "",
            done: true,
            agent: None,
        };
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, &mut ListState::default()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        artifact(&buffer, name);
        (buffer, terminal.get_cursor_position().unwrap())
    }

    #[test]
    fn modal_preserves_the_unopened_workspace_and_uses_its_theme() {
        let (buffer, cursor) = render(130, 34, "document-picker-wide");
        let screen = text(&buffer);
        for label in [
            "✎ lazypencil",
            "No document open",
            "Nodes",
            "Preview",
            "Agent",
            "Open a .pen file",
            "screens/settings.pen",
            "Search complete",
        ] {
            assert!(screen.contains(label), "Missing {label}:\n{screen}");
        }
        for label in ["Loading preview", "connecting", "Working", "0 nodes"] {
            assert!(!screen.contains(label));
        }
        let popup = centered(buffer.area, 94, 22);
        assert_eq!(buffer[(popup.x, popup.y)].symbol(), "╭");
        assert_eq!(buffer[(popup.x, popup.y)].fg, theme::FOCUS);
        let selected = buffer
            .content
            .iter()
            .find(|cell| cell.bg == Color::Rgb(43, 45, 61) && cell.symbol() == "s")
            .expect("Workspace selection palette");
        assert_eq!(selected.fg, theme::NODE_SELECTION);
        assert!(popup.contains(cursor));
        assert_eq!(buffer[(cursor.x, cursor.y)].bg, theme::PROMPT_BACKGROUND);
        assert!(popup.x > 0 && popup.y > 1 && popup.bottom() < buffer.area.bottom() - 1);
    }

    #[test]
    fn modal_resizes_keeps_unicode_input_visible_and_handles_empty_or_partial_results() {
        for (width, height) in [(80, 24), (50, 16), (40, 12), (20, 8), (1, 1)] {
            let (buffer, _) = render(width, height, &format!("document-picker-{width}x{height}"));
            if width >= 50 {
                assert!(text(&buffer).contains("Open a .pen file"));
            }
        }
        let mut listing = listing();
        listing.complete = false;
        listing.stop_reason = Some("time_limit");
        let layout = PanelLayout {
            nodes: 30,
            preview: 45,
            agent: 25,
        };
        let mut view = View {
            layout: &layout,
            listing: &listing,
            paths: &[],
            selected: 0,
            filter: "no-match",
            path: Some(
                "/a/very/long/path/that/keeps/going/through/more/folders/and/ends/with/日本語の設定.pen",
            ),
            notice: "INVALID_DOCUMENT: The selected file is invalid",
            done: true,
            agent: None,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| view.draw(frame, &mut ListState::default()))
            .unwrap();
        let screen = text(terminal.backend().buffer());
        for label in [
            "No matching .pen files",
            "Partial results",
            "INVALID_DOCUMENT",
            "日本語の設定.pen",
        ] {
            assert!(screen.contains(label), "Missing {label}:\n{screen}");
        }
        assert!(!screen.contains("Search complete"));
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!(
            terminal.backend().buffer()[(cursor.x, cursor.y)].bg,
            theme::PROMPT_BACKGROUND
        );
        artifact(terminal.backend().buffer(), "document-picker-error");
        view.done = false;
        view.path = None;
        view.notice = "";
        terminal
            .draw(|frame| view.draw(frame, &mut ListState::default()))
            .unwrap();
        assert!(text(terminal.backend().buffer()).contains("Searching for .pen files"));
    }

    #[test]
    fn background_respects_the_same_configured_panel_geometry_as_an_open_document() {
        let layout = PanelLayout {
            nodes: 30,
            preview: 40,
            agent: 30,
        };
        let mut terminal = Terminal::new(TestBackend::new(130, 34)).unwrap();
        terminal
            .draw(|frame| empty_workspace(frame, &layout, None))
            .unwrap();
        let areas = workspace::areas(Rect::new(0, 0, 130, 34), &layout, false, false, 1);
        for pane in areas.panes {
            assert_eq!(terminal.backend().buffer()[(pane.x, pane.y)].symbol(), "╭");
            assert_eq!(
                terminal.backend().buffer()[(pane.x, pane.y)].fg,
                theme::FRAME
            );
        }
    }

    #[test]
    fn startup_workspace_keeps_prompt_geometry_when_document_opens() {
        use clap::Parser;
        let temp = tempfile::tempdir().unwrap();
        let arguments = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "tui".into(),
            temp.path().join("design.pen").into_os_string(),
            "--create".into(),
        ])
        .unwrap();
        let crate::cli::Command::Tui(args) = arguments.command else {
            unreachable!()
        };
        let conversation =
            crate::conversation::Conversation::open(args.prompt, Some(temp.path().join("data")))
                .unwrap();
        let mut app = crate::tui::App::new(conversation);
        for (width, height) in [(130, 34), (80, 24), (50, 16)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| empty_workspace(frame, &app.panel_layout, None))
                .unwrap();
            let before = text(terminal.backend().buffer());
            let before = before
                .lines()
                .position(|line| line.contains("Prompt"))
                .unwrap();
            terminal
                .draw(|frame| {
                    app.draw(frame);
                })
                .unwrap();
            let after = text(terminal.backend().buffer());
            let after = after
                .lines()
                .position(|line| line.contains("Prompt"))
                .unwrap();
            assert_eq!(
                before, after,
                "Prompt moved while opening at {width}x{height}"
            );
        }
    }

    #[test]
    fn opening_reuses_the_popup_and_shows_loading_instead_of_selection_controls() {
        for (width, height) in [(130, 34), (80, 24), (50, 16), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    opening(
                        frame,
                        &PanelLayout::default(),
                        Path::new("nested/設定.pen"),
                        None,
                    )
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let screen = text(buffer);
            if width >= 50 {
                for label in ["Loading…", "nested/設定.pen", "Nodes", "Preview", "Agent"] {
                    assert!(screen.contains(label), "Missing {label}: {screen}");
                }
                let area = popup(buffer.area);
                assert_eq!(buffer[(area.x, area.y)].symbol(), "╭");
            }
            assert!(!screen.contains("Enter open") && !screen.contains("Filter ›"));
        }
    }

    fn artifact(buffer: &Buffer, name: &str) {
        let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_PICKER_ARTIFACTS") else {
            return;
        };
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{name}.txt")), text(buffer)).unwrap();
        fn color(color: Color, foreground: bool) -> String {
            match color {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                Color::Green => "#98c379".into(),
                Color::Blue => "#61afef".into(),
                Color::Magenta => "#c678dd".into(),
                Color::Cyan => "#56b6c2".into(),
                Color::Yellow => "#e5c07b".into(),
                Color::Red => "#e06c75".into(),
                Color::DarkGray => "#5c6370".into(),
                Color::Gray => "#abb2bf".into(),
                Color::Reset if !foreground => "#171923".into(),
                _ => "#e5e5e5".into(),
            }
        }
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#171923\"/>",
            buffer.area.width * 10,
            buffer.area.height * 20,
            buffer.area.width * 10,
            buffer.area.height * 20
        );
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                svg.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"20\" fill=\"{}\"/>",
                    x * 10,
                    y * 20,
                    color(cell.bg, false)
                ));
            }
        }
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                if cell.symbol().trim().is_empty() {
                    continue;
                }
                let symbol = cell
                    .symbol()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                svg.push_str(&format!("<text x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Menlo,Hiragino Sans,monospace\" font-size=\"16\" font-weight=\"{}\">{symbol}</text>",x*10,y*20+16,color(cell.fg,true),if cell.modifier.contains(Modifier::BOLD) {"bold"} else {"normal"}));
            }
        }
        svg.push_str("</svg>");
        std::fs::write(directory.join(format!("{name}.svg")), svg).unwrap();
    }
}
