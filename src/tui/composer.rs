//! Conversation history and its attached prompt share one visual surface.
use super::*;
use ratatui::{
    layout::Margin,
    symbols::border,
    text::Span,
    widgets::{HighlightSpacing, Padding},
};

pub(super) struct ToolDisplay {
    pub title: String,
    pub status: String,
}

impl Default for ToolDisplay {
    fn default() -> Self {
        Self {
            title: "Tool".into(),
            status: "running".into(),
        }
    }
}

impl App {
    pub(super) fn context_block(
        &self,
        label: &str,
        icon: &str,
        count: usize,
        focused: bool,
        width: u16,
    ) -> Block<'static> {
        let title = Line::from(vec![
            Span::styled(format!(" {icon} "), Style::default().fg(theme::NODES)),
            Span::styled(
                label.to_owned(),
                Style::default()
                    .fg(if focused { theme::FOCUS } else { Color::Reset })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {count} "),
                Style::default().fg(theme::SECONDARY),
            ),
        ]);
        let available = usize::from(width).saturating_sub(title.width() + 2);
        let mut block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(if focused { theme::FOCUS } else { theme::FRAME }))
            .padding(Padding {
                left: 1,
                right: 3,
                top: 0,
                bottom: 0,
            })
            .title(title);
        if focused {
            let key = self.navigation.label(Nav::RemoveAttachment);
            let key = key.split(" / ").next().unwrap_or("");
            let hint = format!(" {key} remove · Esc input ");
            if !key.is_empty() && Span::raw(&hint).width() <= available {
                block = block.title_top(
                    Line::styled(hint, Style::default().fg(theme::SECONDARY)).right_aligned(),
                );
            } else {
                let hint = if key.is_empty() {
                    " Esc input ".into()
                } else {
                    format!(" {key} remove ")
                };
                if Span::raw(&hint).width() <= available {
                    block = block.title_top(
                        Line::styled(hint, Style::default().fg(theme::SECONDARY)).right_aligned(),
                    );
                }
            }
        }
        block
    }

    pub(super) fn activity_line(&self, now: Instant) -> Option<Line<'static>> {
        if !self.conversation.active() {
            return None;
        }
        let elapsed = self.turn_started.map_or(Duration::ZERO, |started| {
            now.saturating_duration_since(started)
        });
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let (icon, label, color) = if self.permissions.is_empty() {
            (
                frames[(elapsed.as_millis() / 200 % frames.len() as u128) as usize],
                "Working…",
                theme::WORKING,
            )
        } else {
            ("!", "Waiting for approval", theme::RUNNING)
        };
        Some(Line::from(vec![
            Span::styled(format!("{icon} {label}"), Style::default().fg(color)),
            Span::styled(
                format!(" {}s", elapsed.as_secs()),
                Style::default().fg(theme::SECONDARY),
            ),
        ]))
    }

    pub(super) fn draw_conversation(&mut self, frame: &mut Frame, area: Rect) {
        let badge = self.agent_activity();
        let block = panel(
            theme::title("◉", "Agent", theme::AGENT, self.focus.in_conversation()),
            self.focus.in_conversation(),
        )
        .border_set(border::ROUNDED);
        let block = if area.width >= 32 {
            block.title_top(Line::from(vec![Span::raw(" "), badge, Span::raw(" ")]).right_aligned())
        } else {
            block
        };
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let count = if self.conversation.active() {
            0
        } else {
            self.conversation.images().len()
        };
        let attachment_rows = if count == 0 {
            0
        } else {
            count.min(if self.outbox.references.is_empty() {
                3
            } else {
                1
            }) as u16
                + 1
        };
        let reference_rows = if self.outbox.references.is_empty() {
            0
        } else {
            self.outbox
                .references
                .len()
                .min(if count == 0 { 3 } else { 1 }) as u16
                + 1
        };
        let queue_rows = u16::from(!self.outbox.queued.is_empty());
        let activity = self.activity_line(Instant::now());
        let activity_rows = u16::from(activity.is_some()) * 2;
        let prompt_space = inner
            .height
            .saturating_sub(attachment_rows + reference_rows + queue_rows + activity_rows + 1);
        let prompt_height =
            (self.editor.lines().len().clamp(3, 6) as u16 + 2).min(prompt_space.max(3));
        let sections = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(attachment_rows),
            Constraint::Length(queue_rows),
            Constraint::Length(reference_rows),
            Constraint::Length(activity_rows),
            Constraint::Length(prompt_height),
        ])
        .split(inner);
        let logarea = sections[0].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        let top_gap = u16::from(logarea.height > 1);
        let logarea = Rect::new(
            logarea.x,
            logarea.y + top_gap,
            logarea.width,
            logarea.height - top_gap,
        );
        let text_area = logarea.inner(Margin {
            horizontal: 2,
            vertical: 0,
        });
        let paragraph = if self.log.is_empty() {
            None
        } else {
            Some(self.history.take().unwrap_or_else(|| {
                Paragraph::new(history_text(&self.log, &self.tool_calls)).wrap(Wrap { trim: false })
            }))
        };
        let lines = paragraph.as_ref().map_or(0, |paragraph| {
            paragraph.line_count(text_area.width).min(u16::MAX as usize) as u16
        });
        let scroll = lines
            .saturating_sub(logarea.height)
            .saturating_sub(self.scroll);
        if let Some(paragraph) = paragraph {
            let paragraph = paragraph.scroll((scroll, 0));
            frame.render_widget(&paragraph, text_area);
            fill_user_backgrounds(frame.buffer_mut(), logarea);
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
                            format!("{} ", index + 1),
                            Style::default().fg(if selected {
                                theme::NODE_SELECTION
                            } else {
                                theme::SECONDARY
                            }),
                        ),
                        Span::raw(clean(&image.display_name)),
                        Span::styled(
                            format!("  {}", images::dimensions(image)),
                            Style::default().fg(if selected {
                                Color::White
                            } else {
                                theme::SECONDARY
                            }),
                        ),
                    ]))
                })
                .collect::<Vec<_>>();
            frame.render_stateful_widget(
                List::new(attachments)
                    .block(self.context_block(
                        "Attachments",
                        "▧",
                        count,
                        self.focus == Focus::Attachments,
                        sections[1].width,
                    ))
                    .highlight_symbol(if self.focus == Focus::Attachments {
                        "› "
                    } else {
                        "  "
                    })
                    .highlight_spacing(HighlightSpacing::Always)
                    .highlight_style(if self.focus == Focus::Attachments {
                        theme::node_selected(true)
                    } else {
                        Style::default()
                    }),
                sections[1],
                &mut self.attachments,
            );
        }
        if let Some(next) = self.outbox.queued.front() {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("{} queued", self.outbox.queued.len()),
                        Style::default().fg(theme::WORKING),
                    ),
                    Span::styled(
                        if self.outbox.paused { " (paused)" } else { "" },
                        Style::default().fg(theme::RUNNING),
                    ),
                    Span::styled(
                        format!(" · {}", next.text.replace('\n', " ")),
                        Style::default().fg(theme::SECONDARY),
                    ),
                ])),
                sections[2].inner(Margin {
                    horizontal: 2,
                    vertical: 0,
                }),
            );
        }
        if !self.outbox.references.is_empty() {
            self.draw_references(frame, sections[3]);
        }
        if let Some(activity) = activity {
            frame.render_widget(
                Paragraph::new(activity),
                sections[4].inner(Margin {
                    horizontal: 1,
                    vertical: 0,
                }),
            );
        }
        let prompt = sections[5];
        let input_style = Style::default()
            .fg(if self.focus == Focus::Prompt {
                Color::White
            } else {
                Color::Gray
            })
            .bg(theme::PROMPT_BACKGROUND);
        frame.render_widget(Block::default().style(input_style), prompt);
        self.editor.set_style(input_style);
        let mut input_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(if self.focus == Focus::Prompt {
                theme::FOCUS
            } else {
                theme::FRAME
            }))
            .title(theme::title(
                "✎",
                "Prompt",
                theme::AGENT,
                self.focus == Focus::Prompt,
            ))
            .padding(Padding {
                left: 3,
                right: 3,
                top: 0,
                bottom: 1,
            });
        if self.focus == Focus::Prompt && (count > 0 || !self.outbox.references.is_empty()) {
            let previous = self.navigation.label(Nav::PreviousSection);
            let previous = previous.split(" / ").next().unwrap_or("");
            let hint = format!(" {previous} context ");
            if !previous.is_empty() && Span::raw(&hint).width() + 12 <= usize::from(prompt.width) {
                input_block = input_block.title_top(
                    Line::styled(hint, Style::default().fg(theme::SECONDARY)).right_aligned(),
                );
            }
        }
        self.editor.set_block(input_block);
        let input = self.editor.block().unwrap().inner(prompt);
        let placeholder = if self.conversation.active() {
            "Add a follow-up…"
        } else if input.width >= 26 {
            "Describe a design change…"
        } else if input.width >= 19 {
            "Describe a change…"
        } else {
            "Message…"
        };
        // TextArea prepends a blank cursor cell to its placeholder. Paint the
        // hint ourselves so the native caret covers its first character.
        self.editor.set_placeholder_text("");
        let editing = self.focus == Focus::Prompt && !self.has_modal();
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
        if self.editor.is_empty() {
            frame.render_widget(
                Paragraph::new(placeholder).style(input_style.fg(Color::Gray)),
                input,
            );
        }
    }
}

fn fill_user_backgrounds(buffer: &mut ratatui::buffer::Buffer, area: Rect) {
    // Paragraph applies line styles to text cells. Extend each rendered user
    // row across the viewport, including rows created by wrapping or scrolling.
    let mut previous_user = false;
    for y in area.y..area.bottom() {
        let user = (area.x..area.right()).any(|x| buffer[(x, y)].bg == theme::USER_BACKGROUND);
        if user {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(theme::USER_BACKGROUND);
            }
            if !previous_user && area.width > 0 {
                buffer[(area.x, y)].set_symbol("›").set_fg(theme::USER);
            }
        }
        previous_user = user;
    }
}

fn history_text(
    log: &str,
    tools: &std::collections::HashMap<String, ToolDisplay>,
) -> Text<'static> {
    let mut lines = Vec::new();
    let mut body = String::new();
    let ranges = markdown::code_ranges(log);
    let mut ranges = ranges.iter().peekable();
    let mut offset = 0;
    let mut activity = false;
    let mut user = false;
    for raw in log.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        while ranges.peek().is_some_and(|range| range.end <= offset) {
            ranges.next();
        }
        let code = ranges.peek().is_some_and(|range| range.contains(&offset));
        offset += raw.len();
        if code {
            close_tools(&mut lines, &mut activity);
            body.push_str(raw);
            continue;
        }
        let tool = line
            .strip_prefix("[tool_call] ")
            .and_then(|id| serde_json::from_str::<String>(id).ok())
            .and_then(|id| tools.get(&id));
        if let Some((status, title)) = tool
            .map(|tool| (tool.status.as_str(), tool.title.as_str()))
            .or_else(|| {
                line.strip_prefix('[')
                    .and_then(|line| line.split_once("] "))
            })
            && let Some((icon, color)) = match status {
                "running" | "in_progress" => Some(("●", theme::RUNNING)),
                "pending" => Some(("○", theme::KEY)),
                "completed" => Some(("✓", theme::SUCCESS)),
                "failed" => Some(("✕", theme::ERROR)),
                "cancelled" | "canceled" => Some(("−", theme::MUTED)),
                _ => None,
            }
        {
            flush_body(&mut body, &mut lines, !activity, user);
            if !activity {
                lines.push(Line::from(vec![
                    Span::styled("┌ ", Style::default().fg(theme::MUTED)),
                    Span::styled(
                        "Tools",
                        Style::default()
                            .fg(theme::SECONDARY)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            let title = if let Some(name) = title.strip_prefix("mcp.lazypencil.") {
                name.replace('_', " ")
            } else {
                title.to_owned()
            };
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!("{icon} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    title,
                    Style::default().fg(if status == "failed" {
                        theme::ERROR
                    } else {
                        theme::SECONDARY
                    }),
                ),
            ]));
            activity = true;
            continue;
        }
        if line == "[0 images]" {
            continue;
        }
        if activity && line.trim().is_empty() {
            continue;
        }
        close_tools(&mut lines, &mut activity);
        let styled = if let Some(message) = line.strip_prefix("You:") {
            flush_body(&mut body, &mut lines, true, user);
            if lines.last().is_some_and(|line| !line.spans.is_empty()) {
                lines.push(Line::default());
            }
            user = true;
            body.push_str(message.trim_start());
            body.push('\n');
            activity = false;
            continue;
        } else if matches!(line, "Codex:" | "Claude:") {
            flush_body(&mut body, &mut lines, true, user);
            user = false;
            activity = false;
            continue;
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
        flush_body(&mut body, &mut lines, true, user);
        lines.push(styled);
        activity = false;
    }
    close_tools(&mut lines, &mut activity);
    flush_body(&mut body, &mut lines, false, user);
    while lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }
    Text::from(lines)
}

fn close_tools(lines: &mut Vec<Line<'static>>, active: &mut bool) {
    if *active {
        lines.push(Line::styled("└", Style::default().fg(theme::MUTED)));
        lines.push(Line::default());
        *active = false;
    }
}

fn flush_body(body: &mut String, lines: &mut Vec<Line<'static>>, spacing: bool, user: bool) {
    let mut rendered = markdown::render(body);
    let has_content = !rendered.is_empty();
    if user {
        for line in &mut rendered {
            line.style = line.style.fg(theme::USER).bg(theme::USER_BACKGROUND);
        }
    }
    lines.extend(rendered);
    if spacing && has_content && lines.last().is_some_and(|line| !line.spans.is_empty()) {
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
        let rendered = history_text(source, &Default::default());
        let text = rendered.to_string();
        assert!(text.contains("どの画面を、どう変更したいか教えてください。"));
        assert!(text.contains("┌ Tools\n│ ● read nodes\n│ ✓ Tool\n└"));
        assert!(text.contains("│ [completed] literal status"));
        assert!(text.contains("│ You: literal role"));
        assert!(text.contains("│ [0 images]"));
        assert!(text.contains("│ **literal code**"));
        assert_eq!(text.matches("[0 images]").count(), 1);
        assert!(text.contains("✓ Request complete"));
        assert!(!text.contains("› You") && !text.contains("◆ Codex"));
        assert_eq!(rendered.lines[0].style.bg, Some(theme::USER_BACKGROUND));
        assert!(
            rendered
                .lines
                .iter()
                .find(|line| line.to_string().starts_with("どの画面"))
                .unwrap()
                .style
                .bg
                .is_none()
        );
    }

    #[test]
    fn role_colors_cover_wrapped_rows_without_taking_header_rows() {
        let rendered = history_text(
            "You: A long user message\n[0 images]\n\nClaude:\nA reply\n",
            &Default::default(),
        );
        assert_eq!(rendered.to_string(), "A long user message\n\nA reply");
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 14, 6));
        ratatui::widgets::Widget::render(
            Paragraph::new(rendered).wrap(Wrap { trim: false }),
            Rect::new(2, 0, 12, 6),
            &mut buffer,
        );
        fill_user_backgrounds(&mut buffer, Rect::new(0, 0, 14, 6));
        for y in 0..2 {
            assert_eq!(
                buffer[(13, y)].bg,
                theme::USER_BACKGROUND,
                "Wrapped user rows retain their background"
            );
        }
        assert_eq!(buffer[(0, 0)].symbol(), "›");
        assert_eq!(buffer[(2, 3)].symbol(), "A");
        assert_eq!(buffer[(13, 3)].bg, Color::Reset);
    }
}
