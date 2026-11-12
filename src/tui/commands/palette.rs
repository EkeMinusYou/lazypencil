//! Searchable launcher rows and contextual details for the command palette.
use super::*;
use ratatui::style::Stylize;

pub(in crate::tui) fn area(workspace: Rect) -> Rect {
    modal::area(workspace)
}

pub(in crate::tui) fn matching(query: &str) -> Vec<usize> {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let haystack = format!("{} {} {} {}", c.label, c.category, c.description, c.usage())
                .to_lowercase();
            terms
                .iter()
                .all(|term| haystack.contains(term))
                .then_some(i)
        })
        .collect()
}

fn details(command: &Command, unavailable: Option<&str>, compact: bool) -> Text<'static> {
    let colors = theme::colors();
    let slash_equivalent = command.slash_enabled() && command.action != Action::Help;
    let usage = if command.action == Action::Help {
        command.label.to_owned()
    } else {
        command.usage()
    };
    if compact {
        return Text::from(vec![
            Line::styled(usage, Style::default().fg(colors.preview)),
            Line::styled(
                unavailable.map_or_else(
                    || command.description.to_owned(),
                    |reason| format!("Unavailable: {reason}"),
                ),
                Style::default().fg(if unavailable.is_some() {
                    colors.running
                } else {
                    colors.secondary
                }),
            ),
        ]);
    }
    let mut lines = vec![
        Line::styled(
            command.category.to_uppercase(),
            Style::default().fg(colors.preview).bold(),
        ),
        Line::default(),
        Line::styled(command.label, Style::default().fg(colors.text).bold()),
        Line::default(),
        Line::styled(command.description, Style::default().fg(colors.secondary)),
        Line::default(),
        Line::styled(
            if slash_equivalent {
                "SLASH COMMAND"
            } else {
                "ACTION"
            },
            Style::default().fg(colors.secondary).bold(),
        ),
        Line::styled(
            usage,
            Style::default()
                .fg(colors.preview)
                .bg(colors.user_background),
        ),
    ];
    if command.slash_enabled() && !command.aliases.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled(
            format!(
                "Alias: {}",
                command
                    .aliases
                    .iter()
                    .map(|name| format!("/{name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Style::default().fg(colors.secondary),
        ));
    }
    if let Some(reason) = unavailable {
        lines.push(Line::default());
        lines.push(Line::styled(
            "UNAVAILABLE",
            Style::default().fg(colors.running).bold(),
        ));
        lines.push(Line::styled(
            reason.to_owned(),
            Style::default().fg(colors.text),
        ));
    }
    Text::from(lines)
}

impl App {
    pub(in crate::tui) fn paste_command_filter(&mut self, text: &str) {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.command_filter.len() + c.len_utf8() > 4096 {
                break;
            }
            self.command_filter.push(c);
        }
        if let Some(checkpoints::Modal::Commands { selected }) = &mut self.modal
            && let Some(index) = matching(&self.command_filter).first()
        {
            *selected = *index;
        }
        self.pointer.reattach(mouse::ListTarget::Commands);
    }

    pub(in crate::tui) fn draw_command_palette(&mut self, frame: &mut Frame) {
        let Some(checkpoints::Modal::Commands { selected }) = self.modal.as_ref() else {
            return;
        };
        let selected = *selected;
        let matches = matching(&self.command_filter);
        let command = &COMMANDS[selected];
        let unavailable = self.command_unavailable(command);
        let colors = theme::colors();
        self.pointer.overlay();
        let popup = area(frame.area());
        let shell = modal::Shell::draw(frame, popup, "Commands", "Search commands…", 1);
        shell.close(frame, &mut self.pointer);
        shell.query(frame, "Commands", &self.command_filter, "Search commands…");
        let body = shell.body;
        let wide = body.width >= 96 && body.height >= 16;
        let (list, detail) = if wide {
            let columns =
                Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)])
                    .split(body);
            (columns[0], modal::detail_surface(frame, columns[1]))
        } else {
            let height = Paragraph::new(details(command, unavailable, true))
                .wrap(Wrap { trim: false })
                .line_count(body.width)
                .min(usize::from(body.height.saturating_sub(3))) as u16;
            let rows =
                Layout::vertical([Constraint::Min(3), Constraint::Length(height)]).split(body);
            (rows[0], rows[1])
        };
        let visible = self.pointer.command_rows(list, selected, &matches);
        for &(index, y) in &visible {
            let item = &COMMANDS[index];
            let chosen = index == selected;
            let disabled = self.command_unavailable(item).is_some();
            let background = if chosen {
                colors.selected_background
            } else {
                colors.canvas
            };
            let style = Style::default()
                .fg(if chosen {
                    colors.preview
                } else if disabled {
                    colors.secondary
                } else {
                    colors.text
                })
                .bg(background);
            let row = Rect::new(list.x, y, list.width, 1);
            frame.render_widget(Block::default().bg(background), row);
            let metadata = if disabled {
                "Unavailable"
            } else {
                item.category
            };
            let content = modal::result(item.label, metadata, row.width, chosen).style(style);
            let mut state = ratatui::widgets::ListState::default();
            state.select(chosen.then_some(0));
            frame.render_stateful_widget(
                ratatui::widgets::List::new([content])
                    .style(style)
                    .highlight_style(Style::default())
                    .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
                    .highlight_symbol("› "),
                row,
                &mut state,
            );
        }
        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No matching commands").style(Style::default().fg(colors.secondary)),
                list,
            );
        } else {
            frame.render_widget(
                Paragraph::new(details(command, unavailable, !wide)).wrap(Wrap { trim: false }),
                detail,
            );
        }
        let position = |index| {
            matches
                .iter()
                .position(|i| *i == index)
                .map_or(0, |i| i + 1)
        };
        let range = format!(
            "{}–{} of {}",
            visible.first().map_or(0, |r| position(r.0)),
            visible.last().map_or(0, |r| position(r.0)),
            matches.len()
        );
        shell.context(
            frame,
            if self.command_filter.is_empty() {
                "All commands"
            } else {
                "Matching commands"
            },
            &range,
        );
        let action = if matches.is_empty() {
            "[No matches]"
        } else if unavailable.is_some() {
            "[Unavailable]"
        } else if command.arguments.is_empty() {
            "[Enter Run]"
        } else {
            "[Enter Continue]"
        };
        let action_width = Line::raw(action).width() as u16;
        let footer = shell.footer;
        let parts = Layout::horizontal([Constraint::Min(0), Constraint::Length(action_width)])
            .split(footer);
        frame.render_widget(
            Paragraph::new(modal::hints(&[
                ("↑ ↓", "Navigate"),
                ("Esc", "Clear / close"),
            ])),
            parts[0],
        );
        frame.render_widget(
            Paragraph::new(action).style(
                Style::default()
                    .fg(if unavailable.is_some() {
                        colors.muted
                    } else {
                        colors.preview
                    })
                    .bold(),
            ),
            parts[1],
        );
        if !matches.is_empty() && unavailable.is_none() && self.pointer.controls {
            self.pointer
                .add(parts[1], mouse::Target::Button(mouse::Action::Accept));
        }
    }
}
