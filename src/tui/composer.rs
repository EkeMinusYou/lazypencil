//! Conversation history and its attached prompt share one visual surface.
use super::*;
use ratatui::{layout::Margin, symbols::border, text::Span, widgets::Padding};

impl App {
    pub(super) fn draw_conversation(&mut self, frame: &mut Frame, area: Rect) {
        let activity = self.agent_activity();
        let block = panel(
            theme::title(
                "◉",
                if area.width < 24 {
                    "Chat"
                } else {
                    "Conversation"
                },
                theme::AGENT,
                self.focus.in_conversation(),
            ),
            self.focus.in_conversation(),
        )
        .border_set(border::ROUNDED);
        let block = if area.width >= 36 {
            block.title_top(
                Line::from(vec![Span::raw(" "), activity, Span::raw(" ")]).right_aligned(),
            )
        } else {
            block
        };
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let count = self.conversation.images().len();
        let prompt_rows = self.editor.lines().len().clamp(1, 3) as u16;
        let sections = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(if count == 0 {
                0
            } else {
                count.min(3) as u16 + 1
            }),
            Constraint::Length(prompt_rows + 2),
        ])
        .split(inner);
        let logarea = sections[0].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        if !self.log.is_empty() {
            let paragraph = self.history.take().unwrap_or_else(|| {
                Paragraph::new(history_text(&self.log)).wrap(Wrap { trim: false })
            });
            let lines = paragraph.line_count(logarea.width).min(u16::MAX as usize) as u16;
            let scroll = lines
                .saturating_sub(logarea.height)
                .saturating_sub(self.scroll);
            let paragraph = paragraph.scroll((scroll, 0));
            frame.render_widget(&paragraph, logarea);
            self.history = Some(paragraph);
        }
        if count > 0 {
            let attachments = self
                .conversation
                .images()
                .iter()
                .enumerate()
                .map(|(index, image)| {
                    let selected = self.focus == Focus::Attachments
                        && self.attachments.selected() == Some(index);
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("▧ {} ", index + 1),
                            Style::default().fg(if selected { Color::White } else { theme::NODES }),
                        ),
                        Span::raw(clean(&image.display_name)),
                        Span::styled(
                            format!("  {}", images::dimensions(image)),
                            Style::default().fg(if selected { Color::White } else { theme::MUTED }),
                        ),
                    ]))
                })
                .collect::<Vec<_>>();
            frame.render_stateful_widget(
                List::new(attachments)
                    .block(
                        Block::default()
                            .title(theme::title(
                                "▧",
                                format!("Attachments · {count}"),
                                theme::NODES,
                                self.focus == Focus::Attachments,
                            ))
                            .padding(Padding::horizontal(1)),
                    )
                    .highlight_symbol("› ")
                    .highlight_style(theme::selected(self.focus == Focus::Attachments)),
                sections[1],
                &mut self.attachments,
            );
        }
        let prompt = sections[2];
        let input_style = Style::default()
            .fg(if self.focus == Focus::Prompt {
                Color::White
            } else {
                Color::Gray
            })
            .bg(theme::PROMPT_BACKGROUND);
        frame.render_widget(Block::default().style(input_style), prompt);
        self.editor.set_style(input_style);
        self.editor.set_block(Block::default().padding(Padding {
            left: 3,
            right: 1,
            top: 1,
            bottom: 1,
        }));
        let input = self.editor.block().unwrap().inner(prompt);
        self.editor
            .set_placeholder_text(if self.conversation.active() {
                "Waiting for response…"
            } else if input.width >= 26 {
                "Describe a design change…"
            } else if input.width >= 19 {
                "Describe a change…"
            } else {
                "Message…"
            });
        self.editor.set_placeholder_style(
            Style::default()
                .fg(Color::Gray)
                .bg(theme::PROMPT_BACKGROUND),
        );
        let editing =
            self.focus == Focus::Prompt && !self.conversation.active() && !self.has_modal();
        // Find the widget's rendered caret so Unicode widths and scrolling keep
        // the native IME anchor aligned with the actual input cells.
        self.editor.set_cursor_style(if editing {
            Style::default().add_modifier(Modifier::HIDDEN)
        } else {
            Style::default()
        });
        frame.render_widget(&self.editor, prompt);
        frame.render_widget(
            Paragraph::new("›").style(input_style),
            Rect::new(prompt.x + 1, input.y, 1, 1),
        );
        if editing {
            let mut cursor = None;
            for y in input.y..input.bottom() {
                for x in input.x..input.right() {
                    let cell = &mut frame.buffer_mut()[(x, y)];
                    if cell.modifier.contains(Modifier::HIDDEN) {
                        cursor.get_or_insert((x, y));
                        cell.modifier.remove(Modifier::HIDDEN);
                    }
                }
            }
            if cursor.is_none() {
                let logical = self.editor.screen_cursor();
                let (top, left) = self.editor.scroll_offset();
                if let (Some(row), Some(col)) = (
                    logical.row.checked_sub(usize::from(top)),
                    logical.col.checked_sub(usize::from(left)),
                ) && row < usize::from(input.height)
                    && col < usize::from(input.width)
                {
                    cursor = Some((input.x + col as u16, input.y + row as u16));
                }
            }
            if let Some(position) = cursor {
                frame.set_cursor_position(position);
            }
        }
    }
}

fn history_text(log: &str) -> Text<'static> {
    let mut lines = Vec::new();
    let mut body = String::new();
    let ranges = markdown::code_ranges(log);
    let mut ranges = ranges.iter().peekable();
    let mut offset = 0;
    let mut activity = false;
    for raw in log.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        while ranges.peek().is_some_and(|range| range.end <= offset) {
            ranges.next();
        }
        let code = ranges.peek().is_some_and(|range| range.contains(&offset));
        offset += raw.len();
        if code {
            body.push_str(raw);
            activity = false;
            continue;
        }
        if let Some((status, title)) = line
            .strip_prefix('[')
            .and_then(|line| line.split_once("] "))
            && let Some((icon, color)) = match status {
                "running" | "in_progress" => Some(("●", theme::RUNNING)),
                "pending" => Some(("○", theme::KEY)),
                "completed" => Some(("✓", theme::SUCCESS)),
                "failed" => Some(("✕", theme::ERROR)),
                "cancelled" | "canceled" => Some(("−", theme::MUTED)),
                _ => None,
            }
        {
            flush_body(&mut body, &mut lines, !activity);
            let title = if let Some(name) = title.strip_prefix("mcp.lazypencil.") {
                name.replace('_', " ")
            } else {
                title.to_owned()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{icon} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(title),
            ]));
            activity = true;
            continue;
        }
        if line == "[0 images]" {
            continue;
        }
        let styled = if let Some(message) = line.strip_prefix("You:") {
            flush_body(&mut body, &mut lines, true);
            lines.push(Line::styled(
                "› You",
                Style::default()
                    .fg(theme::USER)
                    .add_modifier(Modifier::BOLD),
            ));
            body.push_str(message.trim_start());
            body.push('\n');
            activity = false;
            continue;
        } else if matches!(line, "Codex:" | "Claude:") {
            Line::styled(
                format!("◆ {}", line.trim_end_matches(':')),
                Style::default()
                    .fg(theme::AGENT)
                    .add_modifier(Modifier::BOLD),
            )
        } else if line == "Request complete" {
            Line::styled(format!("✓ {line}"), Style::default().fg(theme::SUCCESS))
        } else if line.starts_with("Request failed:") {
            Line::styled(format!("✕ {line}"), Style::default().fg(theme::ERROR))
        } else if line.starts_with('[') && line.ends_with(" images]") {
            Line::styled(
                format!("▧ {}", line.trim_matches(['[', ']'])),
                Style::default().fg(theme::KEY),
            )
        } else {
            body.push_str(raw);
            continue;
        };
        flush_body(&mut body, &mut lines, true);
        lines.push(styled);
        activity = false;
    }
    flush_body(&mut body, &mut lines, false);
    Text::from(lines)
}

fn flush_body(body: &mut String, lines: &mut Vec<Line<'static>>, spacing: bool) {
    lines.extend(markdown::render(body));
    if spacing && body.contains('\n') && lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(Line::default());
    }
    body.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_formats_markdown_and_activity_but_preserves_code() {
        let source = "You: よろしく\n[0 images]\n\nCodex:\n**どの画面を、どう変更したいか**教えてください。\n\n[in_progress] mcp.lazypencil.read_nodes\n\n[completed] Tool\n\n```text\n[completed] literal status\nYou: literal role\n[0 images]\n**literal code**\n```\n\nRequest complete\n";
        let rendered = history_text(source);
        let text = rendered.to_string();
        assert!(text.contains("どの画面を、どう変更したいか教えてください。"));
        assert!(text.contains("● read nodes\n✓ Tool"));
        assert!(text.contains("│ [completed] literal status"));
        assert!(text.contains("│ You: literal role"));
        assert!(text.contains("│ [0 images]"));
        assert!(text.contains("│ **literal code**"));
        assert_eq!(text.matches("[0 images]").count(), 1);
        assert!(text.contains("✓ Request complete"));
    }
}
