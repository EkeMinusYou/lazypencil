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
    pub fn draw(
        &self,
        frame: &mut Frame,
        state: &mut ListState,
        pointer: &mut super::super::mouse::Pointer,
    ) {
        pointer.begin();
        // Background discovery can reorder rows without changing their rectangles.
        use std::hash::{Hash, Hasher};
        let mut snapshot = std::collections::hash_map::DefaultHasher::new();
        self.paths.hash(&mut snapshot);
        self.selected.hash(&mut snapshot);
        self.path.hash(&mut snapshot);
        self.listing.root.hash(&mut snapshot);
        let scope = format!("documents:{:x}", snapshot.finish());
        if pointer.scope != scope {
            pointer.cancel();
            pointer.scope = scope;
        }
        empty_workspace(frame, self.layout, self.agent);
        let area = frame.area();
        let popup = popup(area);
        frame.render_widget(Clear, popup);
        let title = theme::title("▤", "Open a .pen file", theme::colors().nodes, true);
        let title_width = title.width();
        let mut block = panel(title, true).padding(Padding::horizontal(1));
        let count = Line::styled(
            format!(
                " {} / {} files ",
                self.paths.len(),
                self.listing.documents.len()
            ),
            Style::default().fg(theme::colors().secondary),
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
            pointer.clear();
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
            .style(Style::default().fg(theme::colors().secondary)),
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
                Span::styled(label, Style::default().fg(theme::colors().secondary)),
                Span::raw(input.clone()),
            ]))
            .style(Style::default().bg(theme::colors().prompt_background)),
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
                Paragraph::new(message).style(Style::default().fg(theme::colors().secondary)),
                sections[3],
            );
        } else {
            let width = usize::from(sections[3].width).saturating_sub(2);
            let items: Vec<_> = self
                .paths
                .iter()
                .map(|path| {
                    ListItem::new(Line::from(vec![
                        Span::styled("✎ ", Style::default().fg(theme::colors().nodes)),
                        Span::raw(fit_end(&clean(&path.to_string_lossy()), width)),
                    ]))
                })
                .collect();
            pointer.list(
                frame,
                List::new(items).highlight_style(theme::node_selected(true)),
                sections[3],
                state,
                (1, super::super::mouse::ListTarget::Documents),
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
            (
                format!("Partial results · {reason}"),
                theme::colors().running,
            )
        } else if self.done {
            (
                format!("Search complete · {} files", self.listing.documents.len()),
                theme::colors().secondary,
            )
        } else {
            (
                format!("Searching… · {} files", self.listing.documents.len()),
                theme::colors().secondary,
            )
        };
        let mut messages = vec![Line::styled(status, Style::default().fg(color))];
        if !self.notice.is_empty() {
            messages.push(Line::styled(
                clean(self.notice),
                Style::default().fg(theme::colors().error),
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
        pointer.button(
            frame,
            Rect::new(
                popup.right().saturating_sub(9),
                popup.bottom().saturating_sub(2),
                8,
                1,
            ),
            "[Open]",
            super::super::mouse::Action::Accept,
        );
        pointer.button(
            frame,
            Rect::new(
                popup.right().saturating_sub(9),
                popup.bottom().saturating_sub(3),
                8,
                1,
            ),
            "[Path]",
            super::super::mouse::Action::DocumentPath,
        );
        pointer.finish();
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
    workspace_frame(frame, layout, agent, Some(path));
}

fn empty_workspace(frame: &mut Frame, layout: &PanelLayout, agent: Option<&str>) {
    workspace_frame(frame, layout, agent, None);
}

fn workspace_frame(
    frame: &mut Frame,
    layout: &PanelLayout,
    agent: Option<&str>,
    opening: Option<&Path>,
) {
    frame.render_widget(
        ratatui::widgets::Block::default().style(theme::base()),
        frame.area(),
    );
    let areas = workspace::areas(frame.area(), layout, false, false, workspace::FOOTER_HEIGHT);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "✎ lazypencil",
                Style::default()
                    .fg(theme::colors().agent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    " · {}{}{}",
                    opening
                        .map(|path| clean(
                            &path
                                .file_name()
                                .unwrap_or(path.as_os_str())
                                .to_string_lossy()
                        ))
                        .unwrap_or_else(|| "No document open".into()),
                    if opening.is_some() {
                        " · Loading…"
                    } else {
                        ""
                    },
                    agent
                        .map(|status| format!(" · {status}"))
                        .unwrap_or_default()
                ),
                Style::default().fg(theme::colors().secondary),
            ),
        ])),
        areas.header,
    );
    for (area, icon, label, color) in [
        (areas.panes[0], "▤", "Nodes", theme::colors().nodes),
        (areas.panes[1], "▣", "Preview", theme::colors().preview),
        (areas.panes[2], "◉", "Agent", theme::colors().agent),
    ] {
        frame.render_widget(panel(theme::title(icon, label, color, false), false), area);
    }
    if opening.is_some() {
        let nodes = areas.panes[0].inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        frame.render_widget(
            Paragraph::new("Loading nodes…").style(Style::default().fg(theme::colors().secondary)),
            nodes,
        );
    }
    let preview = areas.panes[1].inner(Margin {
        horizontal: if opening.is_some() { 1 } else { 2 },
        vertical: 1,
    });
    let preview_text = if opening.is_some() {
        "Loading preview…"
    } else {
        "No document open"
    };
    frame.render_widget(
        Paragraph::new(preview_text)
            .style(if opening.is_some() {
                Style::default()
            } else {
                Style::default().fg(theme::colors().secondary)
            })
            .wrap(Wrap { trim: false }),
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
        Paragraph::new(if opening.is_some() {
            "Loading document…"
        } else {
            "Open a document first"
        })
        .block(block)
        .style(
            Style::default()
                .fg(theme::colors().secondary)
                .bg(theme::colors().prompt_background),
        ),
        prompt,
    );
    frame.render_widget(
        Paragraph::new(vec![Line::default(), hints(&[("Ctrl+C", " quit")])]),
        areas.footer,
    );
}

fn hints(items: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![];
    for (index, (key, label)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                " · ",
                Style::default().fg(theme::colors().muted),
            ));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default()
                .fg(theme::colors().key)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            label.to_string(),
            Style::default().fg(theme::colors().secondary),
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
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

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
            .draw(|frame| {
                view.draw(
                    frame,
                    &mut ListState::default(),
                    &mut super::super::super::mouse::Pointer::default(),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        crate::tui::test_artifacts::artifact(&buffer, name);
        (buffer, terminal.get_cursor_position().unwrap())
    }

    #[test]
    fn theme_presets_keep_picker_selection_and_input_readable() {
        use crate::config::keys::Theme;
        for (name, preset, background) in [
            ("light", Theme::Auto, [245, 245, 245]),
            ("dark", Theme::Auto, [17, 17, 27]),
            ("monochrome-light", Theme::Monochrome, [245, 245, 245]),
        ] {
            let _theme = theme::activate(preset, Some(background));
            let (buffer, cursor) = render(130, 34, &format!("document-picker-theme-{name}"));
            assert!(text(&buffer).contains("Open a .pen file"));
            assert_eq!(
                buffer[(cursor.x, cursor.y)].bg,
                theme::colors().prompt_background
            );
            assert!(buffer.content.iter().any(|cell| {
                cell.bg == theme::colors().selected_background
                    && cell.fg == theme::colors().node_selection
                    && cell.symbol() == "s"
            }));
        }
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
        assert_eq!(buffer[(popup.x, popup.y)].fg, theme::colors().focus);
        let selected = buffer
            .content
            .iter()
            .find(|cell| cell.bg == theme::colors().selected_background && cell.symbol() == "s")
            .expect("Workspace selection palette");
        assert_eq!(selected.fg, theme::colors().node_selection);
        assert_eq!(
            buffer[(popup.x + 1, popup.bottom() - 2)].bg,
            theme::colors().canvas
        );
        assert_eq!(buffer[(0, 0)].bg, theme::colors().canvas);
        assert!(popup.contains(cursor));
        assert_eq!(
            buffer[(cursor.x, cursor.y)].bg,
            theme::colors().prompt_background
        );
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
            .draw(|frame| {
                view.draw(
                    frame,
                    &mut ListState::default(),
                    &mut super::super::super::mouse::Pointer::default(),
                )
            })
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
            theme::colors().prompt_background
        );
        crate::tui::test_artifacts::artifact(terminal.backend().buffer(), "document-picker-error");
        view.done = false;
        view.path = None;
        view.notice = "";
        terminal
            .draw(|frame| {
                view.draw(
                    frame,
                    &mut ListState::default(),
                    &mut super::super::super::mouse::Pointer::default(),
                )
            })
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
        let areas = workspace::areas(
            Rect::new(0, 0, 130, 34),
            &layout,
            false,
            false,
            workspace::FOOTER_HEIGHT,
        );
        for pane in areas.panes {
            assert_eq!(terminal.backend().buffer()[(pane.x, pane.y)].symbol(), "╭");
            assert_eq!(
                terminal.backend().buffer()[(pane.x, pane.y)].fg,
                theme::colors().frame
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
    fn opening_dismisses_the_popup_and_shows_loading_inside_the_workspace_panes() {
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
                for label in ["Loading preview…", "Nodes", "Preview", "Agent"] {
                    assert!(screen.contains(label), "Missing {label}: {screen}");
                }
                assert!(screen.lines().next().unwrap().contains("設定.pen"));
                assert!(!screen.contains("nested/設定.pen"));
                let areas = workspace::areas(
                    buffer.area,
                    &PanelLayout::default(),
                    false,
                    false,
                    workspace::FOOTER_HEIGHT,
                );
                let preview = areas.panes[1];
                for y in preview.y + 2..preview.bottom() - 1 {
                    for x in preview.x + 1..preview.right() - 1 {
                        assert_eq!(
                            buffer[(x, y)].symbol(),
                            " ",
                            "Unexpected preview content below loading status"
                        );
                    }
                }
                for (pane, message) in [
                    (areas.panes[0], "Loading nodes…"),
                    (areas.panes[1], "Loading preview…"),
                ] {
                    assert_eq!(buffer[(pane.x, pane.y)].symbol(), "╭");
                    for (index, symbol) in message
                        .chars()
                        .take(pane.width.saturating_sub(2) as usize)
                        .enumerate()
                    {
                        assert_eq!(
                            buffer[(pane.x + 1 + index as u16, pane.y + 1)].symbol(),
                            symbol.to_string()
                        );
                    }
                }
                let area = popup(buffer.area);
                assert_ne!(buffer[(area.x, area.y)].symbol(), "╭");
            }
            assert!(
                !screen.contains("Opening…")
                    && !screen.contains("Open a .pen file")
                    && !screen.contains("Filter ›")
            );
        }
    }
}
