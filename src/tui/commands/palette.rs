//! Grouped command navigation and contextual details for the command palette.
use super::*;
use ratatui::{layout::Margin, style::Stylize};

/// Return command indices and their label rows; category headings are not selectable.
pub(in crate::tui) fn rows(first: usize, height: u16) -> Vec<(usize, u16)> {
    let mut rows = Vec::new();
    let mut y = 0;
    let mut category = "";
    for (index, command) in COMMANDS.iter().enumerate().skip(first) {
        let heading = command.category != category;
        let gap = if heading && height > 1 {
            if rows.is_empty() { 1 } else { 2 }
        } else {
            0
        };
        if y + gap >= height {
            break;
        }
        y += gap;
        rows.push((index, y));
        y += 1;
        category = command.category;
    }
    rows
}

pub(in crate::tui) fn maximum_start(height: u16) -> usize {
    (0..COMMANDS.len())
        .find(|first| {
            rows(*first, height)
                .last()
                .is_some_and(|row| row.0 + 1 == COMMANDS.len())
        })
        .unwrap_or(COMMANDS.len() - 1)
}

fn dim(color: Color, background: Color) -> Color {
    match (color, background) {
        (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) => {
            let mix = |a, b| ((u16::from(a) * 2 + u16::from(b) * 3) / 5) as u8;
            Color::Rgb(mix(r, br), mix(g, bg), mix(b, bb))
        }
        _ => color,
    }
}

fn rule(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(area.width)))
            .style(Style::default().fg(theme::colors().frame)),
        area,
    );
}

fn details(command: &Command, unavailable: Option<&str>, compact: bool) -> Text<'static> {
    let colors = theme::colors();
    if compact {
        return Text::from(vec![
            Line::styled(command.usage(), Style::default().fg(colors.preview)),
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
        Line::styled("COMMAND", Style::default().fg(colors.secondary).bold()),
        Line::styled(
            command.usage(),
            Style::default()
                .fg(colors.preview)
                .bg(colors.user_background),
        ),
    ];
    if !command.aliases.is_empty() {
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
    pub(in crate::tui) fn draw_command_palette(&mut self, frame: &mut Frame) {
        let Some(checkpoints::Modal::Commands { selected }) = self.modal.as_ref() else {
            return;
        };
        let selected = *selected;
        let command = &COMMANDS[selected];
        let unavailable = self.command_unavailable(command);
        let colors = theme::colors();
        // Dim the workspace without changing its focus, selection or theme.
        for cell in &mut frame.buffer_mut().content {
            cell.fg = dim(cell.fg, colors.canvas);
            cell.bg = dim(cell.bg, colors.canvas);
        }
        self.pointer.overlay();
        let popup = centered(frame.area(), 120, 35);
        frame.render_widget(Clear, popup);
        let block = Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .border_style(Style::default().fg(colors.frame))
            .style(theme::base());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let sections = Layout::vertical([
            Constraint::Length(if popup.height >= 20 { 2 } else { 1 }),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
        let header = sections[0].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        frame.render_widget(
            Paragraph::new("Commands").style(Style::default().fg(colors.text).bold()),
            header,
        );
        let categories = COMMANDS
            .iter()
            .enumerate()
            .filter(|(i, c)| *i == 0 || COMMANDS[*i - 1].category != c.category)
            .count();
        if header.width >= 65 {
            frame.render_widget(
                Paragraph::new(format!(
                    "{} commands · {categories} categories",
                    COMMANDS.len()
                ))
                .style(Style::default().fg(colors.secondary)),
                Rect::new(header.x + 18, header.y, header.width - 30, 1),
            );
        }
        let close = Rect::new(header.right().saturating_sub(7), header.y, 7, 1);
        frame.render_widget(
            Paragraph::new("[Close]").style(Style::default().fg(colors.key)),
            close,
        );
        if self.pointer.controls {
            self.pointer
                .add(close, mouse::Target::Button(mouse::Action::Close));
        }
        rule(frame, sections[1]);
        rule(frame, sections[3]);
        let body = sections[2].inner(Margin {
            horizontal: 1,
            vertical: u16::from(popup.height >= 24),
        });
        let wide = body.width >= 96 && body.height >= 20;
        let (list, detail) = if wide {
            let columns = Layout::horizontal([
                Constraint::Fill(3),
                Constraint::Length(3),
                Constraint::Fill(2),
            ])
            .split(body);
            let separator = Rect::new(columns[1].x + 1, body.y, 1, body.height);
            frame.render_widget(
                Paragraph::new("│\n".repeat(usize::from(separator.height)))
                    .style(Style::default().fg(colors.frame)),
                separator,
            );
            frame.render_widget(Block::default().bg(colors.prompt_background), columns[2]);
            (
                columns[0],
                columns[2].inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                }),
            )
        } else {
            let height = Paragraph::new(details(command, unavailable, true))
                .wrap(Wrap { trim: false })
                .line_count(body.width)
                .min(usize::from(body.height.saturating_sub(3))) as u16;
            let rows =
                Layout::vertical([Constraint::Min(3), Constraint::Length(height)]).split(body);
            (rows[0], rows[1])
        };
        let visible = self.pointer.grouped_command_rows(list, selected);
        let mut category = "";
        for &(index, y) in &visible {
            let item = &COMMANDS[index];
            if item.category != category && y > list.y {
                frame.render_widget(
                    Paragraph::new(item.category.to_uppercase())
                        .style(Style::default().fg(colors.secondary).bold()),
                    Rect::new(list.x + 1, y - 1, list.width.saturating_sub(1), 1),
                );
            }
            category = item.category;
            let chosen = index == selected;
            let disabled = self.command_unavailable(item).is_some();
            let background = if chosen {
                colors.selected_background
            } else {
                colors.canvas
            };
            let foreground = if disabled {
                colors.secondary
            } else if chosen {
                colors.preview
            } else {
                colors.text
            };
            let row = Rect::new(list.x, y, list.width, 1);
            frame.render_widget(Block::default().bg(background), row);
            let style = Style::default().fg(foreground).bg(background);
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {}",
                    if chosen { "▏›" } else { "  " },
                    item.label
                ))
                .style(if chosen { style.bold() } else { style }),
                row,
            );
            if chosen && disabled && list.width >= 55 {
                frame.render_widget(
                    Paragraph::new("Unavailable")
                        .style(Style::default().fg(colors.running).bg(background)),
                    Rect::new(row.right() - 12, row.y, 11, 1),
                );
            }
        }
        frame.render_widget(
            Paragraph::new(details(command, unavailable, !wide)).wrap(Wrap { trim: false }),
            detail,
        );
        let action = if unavailable.is_some() {
            "[Unavailable]"
        } else if command.arguments.is_empty() {
            "[Enter Run]"
        } else {
            "[Enter Continue]"
        };
        let action_width = Line::raw(action).width() as u16;
        let range = if wide {
            format!(
                "{}–{} of {}",
                visible.first().map_or(selected + 1, |r| r.0 + 1),
                visible.last().map_or(selected + 1, |r| r.0 + 1),
                COMMANDS.len()
            )
        } else {
            format!("{}/{}", selected + 1, COMMANDS.len())
        };
        let footer = sections[4].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        let parts = Layout::horizontal([
            Constraint::Min(18),
            Constraint::Length(Line::raw(&range).width() as u16 + 2),
            Constraint::Length(action_width),
        ])
        .split(footer);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "↑ ↓",
                    Style::default()
                        .fg(colors.key)
                        .bg(colors.user_background)
                        .bold(),
                ),
                Span::styled(" Navigate  ", Style::default().fg(colors.secondary)),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(colors.key)
                        .bg(colors.user_background)
                        .bold(),
                ),
                Span::styled(" Close", Style::default().fg(colors.secondary)),
            ])),
            parts[0],
        );
        frame.render_widget(
            Paragraph::new(format!(" {range} ")).style(Style::default().fg(colors.secondary)),
            parts[1],
        );
        frame.render_widget(
            Paragraph::new(action).style(
                Style::default()
                    .fg(if unavailable.is_some() {
                        colors.muted
                    } else {
                        colors.preview
                    })
                    .bg(if unavailable.is_some() {
                        colors.prompt_background
                    } else {
                        colors.user_background
                    })
                    .bold(),
            ),
            parts[2],
        );
        if unavailable.is_none() && self.pointer.controls {
            self.pointer
                .add(parts[2], mouse::Target::Button(mouse::Action::Accept));
        }
    }
}
