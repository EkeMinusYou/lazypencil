//! Conversation history and its attached prompt share one visual surface.
use super::transcript::{Entry, Kind};
use super::*;
use ratatui::{
    layout::Margin,
    text::Span,
    widgets::{HighlightSpacing, Padding},
};

pub(super) fn prompt_height(lines: usize, available: u16) -> u16 {
    (lines.clamp(3, 6) as u16 + 2).min(available.max(3))
}

pub(super) fn prompt_block(focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(if focused {
            theme::colors().focus
        } else {
            theme::colors().frame
        }))
        .title(theme::title("✎", "Prompt", theme::colors().agent, focused))
        .padding(Padding {
            left: 3,
            right: 3,
            top: 0,
            bottom: 1,
        })
}

pub(super) struct ToolDisplay {
    pub title: String,
    pub status: String,
    details: String,
}

pub(super) struct History {
    paragraph: Paragraph<'static>,
    user_starts: Vec<usize>,
    width: u16,
}

impl History {
    fn new((text, starts): (Text<'static>, Vec<usize>), width: u16) -> Self {
        let mut previous = 0;
        let mut row = 0;
        let user_starts = starts
            .into_iter()
            .map(|start| {
                row += Paragraph::new(Text::from(text.lines[previous..start].to_vec()))
                    .wrap(Wrap { trim: false })
                    .line_count(width);
                previous = start;
                row
            })
            .collect();
        Self {
            paragraph: Paragraph::new(text).wrap(Wrap { trim: false }),
            user_starts,
            width,
        }
    }

    fn render(
        mut self,
        buffer: &mut ratatui::buffer::Buffer,
        area: Rect,
        text_area: Rect,
        scroll: u16,
    ) -> Self {
        self.paragraph = self.paragraph.scroll((scroll, 0));
        ratatui::widgets::Widget::render(&self.paragraph, text_area, buffer);
        fill_user_backgrounds(buffer, area);
        for &start in &self.user_starts {
            if let Some(row) = start.checked_sub(usize::from(scroll))
                && row < usize::from(area.height)
                && area.width > 0
                && text_area.width > 0
            {
                buffer[(area.x, area.y + row as u16)]
                    .set_symbol("›")
                    .set_fg(theme::colors().user);
            }
        }
        self
    }
}

impl Default for ToolDisplay {
    fn default() -> Self {
        Self {
            title: "Tool".into(),
            status: "running".into(),
            details: String::new(),
        }
    }
}

impl ToolDisplay {
    pub fn update(&mut self, update: &Value) {
        if let Some(title) = update["title"].as_str().filter(|title| !title.is_empty()) {
            self.title = clean(title).replace(['\n', '\t'], " ");
        }
        if let Some(status) = update["status"].as_str() {
            self.status = clean(status).replace(['\n', '\t'], " ");
        }
        if let Some(content) = update["content"].as_array() {
            self.details = clean(
                &content
                    .iter()
                    .filter(|item| item["type"] == "content" && item["content"]["type"] == "text")
                    .filter_map(|item| item["content"]["text"].as_str())
                    .flat_map(|text| text.chars().chain(std::iter::once('\n')))
                    .take(4096)
                    .collect::<String>(),
            );
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
            Span::styled(
                format!(" {icon} "),
                Style::default().fg(theme::colors().nodes),
            ),
            Span::styled(
                label.to_owned(),
                Style::default()
                    .fg(if focused {
                        theme::colors().focus
                    } else {
                        theme::colors().nodes
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" · {count} "),
                Style::default().fg(theme::colors().secondary),
            ),
        ]);
        let available = usize::from(width).saturating_sub(title.width() + 2);
        let mut block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(if focused {
                theme::colors().focus
            } else {
                theme::colors().frame
            }))
            .padding(Padding {
                left: 1,
                right: 3,
                top: 0,
                bottom: 0,
            })
            .title(title);
        if focused {
            let key = self
                .navigation
                .preferred_label(Nav::RemoveAttachment, false)
                .unwrap_or_default();
            let hint = format!(" {key} remove · Esc input ");
            if !key.is_empty() && Span::raw(&hint).width() <= available {
                block = block.title_top(
                    Line::styled(hint, Style::default().fg(theme::colors().secondary))
                        .right_aligned(),
                );
            } else {
                let hint = if key.is_empty() {
                    " Esc input ".into()
                } else {
                    format!(" {key} remove ")
                };
                if Span::raw(&hint).width() <= available {
                    block = block.title_top(
                        Line::styled(hint, Style::default().fg(theme::colors().secondary))
                            .right_aligned(),
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
                theme::colors().working,
            )
        } else {
            ("!", "Waiting for approval", theme::colors().running)
        };
        Some(Line::from(vec![
            Span::styled(format!("{icon} {label}"), Style::default().fg(color)),
            Span::styled(
                format!(" {}s", elapsed.as_secs()),
                Style::default().fg(theme::colors().secondary),
            ),
        ]))
    }

    pub(super) fn draw_conversation(&mut self, frame: &mut Frame, area: Rect) {
        let badge = self.agent_activity();
        let title = theme::title("◉", "Agent", theme::colors().agent, false);
        let available = usize::from(area.width).saturating_sub(title.width() + 2);
        let mut block = panel(title, self.focus.in_conversation()).padding(Padding::top(1));
        let mut right = Line::default();
        if area.width >= 32 {
            right = Line::from(vec![Span::raw(" "), badge, Span::raw(" ")]);
        }
        if right.width() <= available {
            block = block.title_top(right.right_aligned());
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let count = self.pending_images().len();
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
        let queue_rows = if self.outbox.queued.is_empty() {
            0
        } else {
            self.outbox.queued.len() as u16 + 1
        };
        let activity = self.activity_line(Instant::now());
        let activity_rows = u16::from(activity.is_some()) * 2;
        let context_rows = attachment_rows + reference_rows + queue_rows;
        let gap_space = inner
            .height
            .saturating_sub(context_rows + activity_rows + 4);
        let history_gap =
            u16::from(!(self.log.is_empty() && self.saved_log.is_empty()) && gap_space > 0);
        let context_gap = u16::from(context_rows > 0 && gap_space > history_gap);
        let prompt_space = inner
            .height
            .saturating_sub(context_rows + activity_rows + history_gap + context_gap + 1);
        let input_width = prompt_block(false).inner(inner).width;
        let lines = Paragraph::new(Text::from(
            self.editor
                .lines()
                .iter()
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>(),
        ))
        .wrap(Wrap { trim: false })
        .line_count(input_width);
        let prompt_height = prompt_height(lines, prompt_space);
        let sections = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(history_gap),
            Constraint::Length(activity_rows),
            Constraint::Length(attachment_rows),
            Constraint::Length(queue_rows),
            Constraint::Length(reference_rows),
            Constraint::Length(context_gap),
            Constraint::Length(prompt_height),
        ])
        .split(inner);
        let logarea = sections[0].inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        let text_area = logarea.inner(Margin {
            horizontal: 2,
            vertical: 0,
        });
        self.pointer.add(sections[0], mouse::Target::History);
        let history = if self.log.is_empty() && self.saved_log.is_empty() {
            None
        } else {
            Some(
                self.history
                    .take()
                    .filter(|history| history.width == text_area.width)
                    .unwrap_or_else(|| {
                        History::new(
                            history_text(
                                self.saved_log.iter().chain(self.log.iter()),
                                &self.tool_calls,
                            ),
                            text_area.width,
                        )
                    }),
            )
        };
        let lines = history.as_ref().map_or(0, |history| {
            history
                .paragraph
                .line_count(text_area.width)
                .min(u16::MAX as usize) as u16
        });
        if self.scroll > 0 && self.history_metrics.0 == text_area.width {
            self.scroll = self
                .scroll
                .saturating_add(lines.saturating_sub(self.history_metrics.1));
        }
        self.history_metrics = (text_area.width, lines, lines.saturating_sub(logarea.height));
        let scroll = lines
            .saturating_sub(logarea.height)
            .saturating_sub(self.scroll);
        if let Some(history) = history {
            self.history = Some(history.render(frame.buffer_mut(), logarea, text_area, scroll));
        }
        if count > 0 {
            let attachments = self
                .pending_images()
                .iter()
                .enumerate()
                .map(|(index, image)| {
                    let selected = self.focus == Focus::Attachments
                        && self.attachments.selected() == Some(index);
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{} ", index + 1),
                            Style::default().fg(if selected {
                                theme::colors().node_selection
                            } else {
                                theme::colors().secondary
                            }),
                        ),
                        Span::raw(clean(&image.display_name)),
                        Span::styled(
                            format!("  {}", images::dimensions(image)),
                            Style::default().fg(if selected {
                                theme::colors().text
                            } else {
                                theme::colors().secondary
                            }),
                        ),
                    ]))
                })
                .collect::<Vec<_>>();
            let block = self.context_block(
                "Attachments",
                "▧",
                count,
                self.focus == Focus::Attachments,
                sections[3].width,
            );
            let rows = block.inner(sections[3]);
            self.pointer.list(
                frame,
                List::new(attachments)
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
                rows,
                &mut self.attachments,
                (1, mouse::ListTarget::Attachments),
            );
            frame.render_widget(block, sections[3]);
        }
        if !self.outbox.queued.is_empty() {
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    format!("{} queued", self.outbox.queued.len()),
                    Style::default().fg(theme::colors().working),
                ),
                Span::styled(
                    if self.outbox.paused { " (paused)" } else { "" },
                    Style::default().fg(theme::colors().running),
                ),
            ])];
            lines.extend(
                self.outbox
                    .queued
                    .iter()
                    .enumerate()
                    .map(|(index, prompt)| {
                        Line::styled(
                            format!(
                                "{}. {}{}",
                                index + 1,
                                if prompt.images.is_empty() {
                                    String::new()
                                } else {
                                    format!("[{} images] ", prompt.images.len())
                                },
                                prompt.display_text().replace('\n', " ")
                            ),
                            Style::default().fg(theme::colors().secondary),
                        )
                    }),
            );
            frame.render_widget(
                Paragraph::new(lines),
                sections[4].inner(Margin {
                    horizontal: 2,
                    vertical: 0,
                }),
            );
        }
        if !self.outbox.references.is_empty() {
            self.draw_references(frame, sections[5]);
        }
        if let Some(activity) = activity {
            let left_padding = 3.min(
                sections[2]
                    .width
                    .saturating_sub(activity.spans[0].width() as u16 + 1),
            );
            frame.render_widget(
                Paragraph::new(activity).block(Block::default().padding(Padding::new(
                    left_padding,
                    1,
                    0,
                    0,
                ))),
                sections[2],
            );
        }
        let prompt = sections[7];
        self.pointer
            .add(prompt, mouse::Target::Focus(Focus::Prompt));
        let input_style = Style::default()
            .fg(if self.focus == Focus::Prompt {
                theme::colors().text
            } else {
                theme::colors().secondary
            })
            .bg(theme::colors().prompt_background);
        frame.render_widget(Block::default().style(input_style), prompt);
        self.editor.set_style(input_style);
        let mut block = prompt_block(self.focus == Focus::Prompt);
        if let Some(name) = &self.outbox.draft_command {
            block = block.title(Span::styled(
                format!(" {} ", crate::agent::available_commands::invocation(name)),
                Style::default().fg(theme::colors().preview),
            ));
        }
        self.editor.set_block(block);
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
                    // A full wrapped row leaves the caret just beyond its text.
                    // The prompt's right padding provides that extra cursor cell.
                    && col <= usize::from(input.width)
                    && input.width > 0
                    && input.right() < prompt.right()
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
                Paragraph::new(placeholder).style(input_style.fg(theme::colors().secondary)),
                input,
            );
        }
        self.draw_command_completion(frame, inner, prompt);
    }
}

fn fill_user_backgrounds(buffer: &mut ratatui::buffer::Buffer, area: Rect) {
    if theme::colors().user_background == Color::Reset {
        return;
    }
    // Paragraph applies line styles to text cells. Extend each rendered user
    // row across the viewport, including rows created by wrapping or scrolling.
    for y in area.y..area.bottom() {
        let user =
            (area.x..area.right()).any(|x| buffer[(x, y)].bg == theme::colors().user_background);
        if user {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(theme::colors().user_background);
            }
        }
    }
}

fn history_text<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    tools: &std::collections::HashMap<String, ToolDisplay>,
) -> (Text<'static>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut user_starts = Vec::new();
    let mut body = String::new();
    let mut activity = Vec::new();
    let mut role = None;
    for entry in entries {
        match &entry.kind {
            Kind::User | Kind::Agent | Kind::Thought | Kind::Notice => {
                if !activity.is_empty() && entry.text.trim().is_empty() {
                    continue;
                }
                close_tools(&mut lines, &mut activity);
                if role != Some(&entry.kind)
                    || entry.start && matches!(entry.kind, Kind::User | Kind::Notice)
                {
                    flush_body(
                        &mut body,
                        &mut lines,
                        &mut user_starts,
                        true,
                        role == Some(&Kind::User),
                    );
                    if entry.kind == Kind::Thought {
                        body.push_str("Thinking:\n");
                    }
                }
                role = Some(&entry.kind);
                body.push_str(&entry.text);
            }
            Kind::Tool(_) | Kind::Activity(_) => {
                let tool = match &entry.kind {
                    Kind::Tool(id) => tools.get(id),
                    _ => None,
                };
                let (status, title, details) = if let Some(tool) = tool {
                    (
                        tool.status.as_str(),
                        tool.title.as_str(),
                        tool.details.as_str(),
                    )
                } else if let Kind::Activity(status) = &entry.kind {
                    (status.as_str(), entry.text.as_str(), "")
                } else {
                    ("running", "Tool", "")
                };
                flush_body(
                    &mut body,
                    &mut lines,
                    &mut user_starts,
                    activity.is_empty(),
                    role == Some(&Kind::User),
                );
                role = None;
                if !render_activity(&mut lines, &mut activity, status, title, details) {
                    close_tools(&mut lines, &mut activity);
                    lines.push(Line::raw(format!("[{status}] {title}")));
                }
            }
            Kind::Images(count) => {
                close_tools(&mut lines, &mut activity);
                flush_body(
                    &mut body,
                    &mut lines,
                    &mut user_starts,
                    true,
                    role == Some(&Kind::User),
                );
                role = None;
                if *count > 0 {
                    lines.push(Line::styled(
                        format!("▧ {count} images"),
                        Style::default().fg(theme::colors().key),
                    ));
                    lines.push(Line::default());
                }
            }
            Kind::Failure => {
                close_tools(&mut lines, &mut activity);
                flush_body(
                    &mut body,
                    &mut lines,
                    &mut user_starts,
                    true,
                    role == Some(&Kind::User),
                );
                role = None;
                lines.extend(
                    format!("✕ Request failed: {}", entry.text)
                        .lines()
                        .map(|line| {
                            Line::styled(
                                line.to_owned(),
                                Style::default().fg(theme::colors().error),
                            )
                        }),
                );
                lines.push(Line::default());
            }
            Kind::Complete => {
                close_tools(&mut lines, &mut activity);
                flush_body(
                    &mut body,
                    &mut lines,
                    &mut user_starts,
                    true,
                    role == Some(&Kind::User),
                );
                role = None;
            }
        }
    }
    close_tools(&mut lines, &mut activity);
    flush_body(
        &mut body,
        &mut lines,
        &mut user_starts,
        false,
        role == Some(&Kind::User),
    );
    while lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }
    (Text::from(lines), user_starts)
}

fn render_activity(
    lines: &mut Vec<Line<'static>>,
    activity: &mut Vec<Line<'static>>,
    status: &str,
    title: &str,
    details: &str,
) -> bool {
    let (icon, color) = match status {
        "running" | "in_progress" => ("●", theme::colors().working),
        "pending" => ("○", theme::colors().key),
        "completed" => ("✓", theme::colors().success),
        "failed" => ("✕", theme::colors().error),
        "cancelled" | "canceled" => ("−", theme::colors().muted),
        _ => return false,
    };
    let title = if let Some(name) = title.strip_prefix("mcp.lazypencil.") {
        name.replace('_', " ")
    } else if let Some((_, name)) = title
        .strip_prefix("mcp__lazypencil_bound_")
        .and_then(|name| name.split_once("__"))
        .or_else(|| {
            title
                .strip_prefix("mcp.lazypencil_bound_")
                .and_then(|name| name.split_once('.'))
        })
    {
        name.replace('_', " ")
    } else {
        title.to_owned()
    };
    activity.push(Line::from(vec![
        Span::styled("│ ", Style::default().fg(theme::colors().muted)),
        Span::styled(
            format!("{icon} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            title,
            Style::default().fg(if status == "failed" {
                theme::colors().error
            } else {
                theme::colors().secondary
            }),
        ),
    ]));
    if status == "failed" && !details.trim().is_empty() {
        close_tools(lines, activity);
        lines.extend(
            details.trim().lines().map(|line| {
                Line::styled(line.to_owned(), Style::default().fg(theme::colors().error))
            }),
        );
        lines.push(Line::default());
    }
    true
}

fn close_tools(lines: &mut Vec<Line<'static>>, activity: &mut Vec<Line<'static>>) {
    const VISIBLE_TOOL_CALLS: usize = 3;

    if activity.is_empty() {
        return;
    }
    lines.push(Line::from(vec![
        Span::styled("┌ ", Style::default().fg(theme::colors().muted)),
        Span::styled(
            "Tools",
            Style::default()
                .fg(theme::colors().secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let hidden = activity.len().saturating_sub(VISIBLE_TOOL_CALLS);
    if hidden > 0 {
        let label = if hidden == 1 { "call" } else { "calls" };
        lines.push(Line::styled(
            format!("│ … {hidden} earlier {label}"),
            Style::default().fg(theme::colors().muted),
        ));
    }
    lines.extend(activity.drain(..).skip(hidden));
    lines.push(Line::styled(
        "└",
        Style::default().fg(theme::colors().muted),
    ));
    lines.push(Line::default());
}

fn flush_body(
    body: &mut String,
    lines: &mut Vec<Line<'static>>,
    user_starts: &mut Vec<usize>,
    spacing: bool,
    user: bool,
) {
    let mut rendered = markdown::render(body);
    let has_content = !rendered.is_empty();
    if user {
        if has_content {
            user_starts.push(lines.len());
        }
        for line in &mut rendered {
            // Empty Markdown rows need a cell for Paragraph to retain their background.
            if line.spans.is_empty() {
                line.spans.push(Span::raw(" "));
            }
            line.style = line
                .style
                .fg(theme::colors().user)
                .bg(theme::colors().user_background);
            if theme::colors().user_background == Color::Reset {
                line.style = line.style.add_modifier(Modifier::BOLD);
            }
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
    use serde_json::json;

    #[test]
    fn literal_message_lines_do_not_become_roles_statuses_or_tool_calls() {
        let literal = "You: literal role\nYou (mid-turn): literal input\nAgent:\nCodex:\nClaude:\nRequest complete\n[0 images]\n[completed] literal activity\n[tool_call] \"real\"\nRequest failed: literal text\n";
        let mut tool = ToolDisplay::default();
        tool.update(&json!({"title":"Actual tool", "status":"completed"}));
        let tools = std::collections::HashMap::from([("real".into(), tool)]);
        for role in [Kind::User, Kind::Agent] {
            let entries = [Entry::new(role, literal)];
            let (rendered, _) = history_text(&entries, &tools);
            let text = rendered.to_string();
            for line in literal.lines() {
                assert!(
                    text.lines().any(|actual| actual == line),
                    "Missing literal {line:?}:\n{text}"
                );
            }
            assert!(!text.contains("Actual tool"));
        }
    }

    #[test]
    fn restored_transcript_keeps_user_and_agent_styling_separate() {
        let (rendered, _) = history_text(
            &[
                Entry::new(Kind::User, "Saved request"),
                Entry::new(Kind::Agent, "Saved response"),
            ],
            &Default::default(),
        );
        let request = rendered
            .lines
            .iter()
            .find(|line| line.to_string() == "Saved request")
            .unwrap();
        let response = rendered
            .lines
            .iter()
            .find(|line| line.to_string() == "Saved response")
            .unwrap();
        assert_eq!(request.style.bg, Some(theme::colors().user_background));
        assert_ne!(response.style.bg, Some(theme::colors().user_background));
    }

    #[test]
    fn failed_tools_keep_the_error_body_visible_across_status_updates() {
        let mut tool = ToolDisplay::default();
        tool.update(&json!({"title":"mcp__lazypencil_bound_test__startup","status":"in_progress",
            "content":[{"type":"content","content":{"type":"text","text":"MCP server was not started: duplicate server name."}}]}));
        let mut tools = std::collections::HashMap::from([("startup".into(), tool)]);
        let log = [Entry::new(Kind::Tool("startup".into()), "")];
        assert!(
            !history_text(&log, &tools)
                .0
                .to_string()
                .contains("duplicate server")
        );
        tools
            .get_mut("startup")
            .unwrap()
            .update(&json!({"status":"failed"}));
        let rendered = history_text(&log, &tools).0.to_string();
        assert!(rendered.contains("✕ startup"));
        assert!(rendered.contains("MCP server was not started: duplicate server name."));
    }

    #[test]
    fn tool_runs_show_only_the_latest_three_calls() {
        for count in 0_usize..=6 {
            let source = (0..count)
                .map(|index| {
                    Entry::new(Kind::Activity("completed".into()), format!("call {index}"))
                })
                .collect::<Vec<_>>();
            let rendered = history_text(&source, &Default::default()).0.to_string();
            if count == 0 {
                assert!(rendered.is_empty());
                continue;
            }
            let hidden = count.saturating_sub(3);
            let mut expected = String::from("┌ Tools\n");
            if hidden > 0 {
                expected.push_str(match hidden {
                    1 => "│ … 1 earlier call\n",
                    2 => "│ … 2 earlier calls\n",
                    3 => "│ … 3 earlier calls\n",
                    _ => unreachable!(),
                });
            }
            for index in hidden..count {
                expected.push_str(&format!("│ ✓ call {index}\n"));
            }
            expected.push('└');
            assert_eq!(rendered, expected, "{count} calls");
        }
    }

    #[test]
    fn tool_runs_collapse_independently_across_prose_and_code() {
        let calls = || {
            [
                ("completed", "old"),
                ("completed", "read"),
                ("failed", "edit"),
                ("in_progress", "render"),
            ]
            .map(|(status, title)| Entry::new(Kind::Activity(status.into()), title))
        };
        let source = calls()
            .into_iter()
            .chain([Entry::new(Kind::Agent, "Checking the result.")])
            .chain(calls())
            .chain([
                Entry::new(Kind::Agent, "```text\n[completed] literal\n```"),
                Entry::new(Kind::Activity("completed".into()), "final"),
            ])
            .collect::<Vec<_>>();
        let rendered = history_text(&source, &Default::default()).0.to_string();
        let block = "┌ Tools\n│ … 1 earlier call\n│ ✓ read\n│ ✕ edit\n│ ● render\n└";
        assert_eq!(rendered.matches(block).count(), 2);
        assert!(rendered.contains(&format!("{block}\n\nChecking the result.")));
        assert!(rendered.contains("│ [completed] literal"));
        assert!(rendered.ends_with("┌ Tools\n│ ✓ final\n└"));
        assert!(!rendered.contains("✓ old"));
    }

    #[test]
    fn transcript_formats_markdown_and_activity_but_preserves_code() {
        let source = [
            Entry::new(Kind::User, "よろしく"),
            Entry::new(Kind::Images(0), ""),
            Entry::new(
                Kind::Agent,
                "**どの画面を、どう変更したいか**教えてください。",
            ),
            Entry::new(
                Kind::Activity("in_progress".into()),
                "mcp.lazypencil.read_nodes",
            ),
            Entry::new(Kind::Activity("completed".into()), "Tool"),
            Entry::new(
                Kind::Agent,
                "```text\n[completed] literal status\nYou: literal role\n[0 images]\n**literal code**\n```",
            ),
            Entry::new(Kind::Complete, ""),
        ];
        let (rendered, _) = history_text(&source, &Default::default());
        let text = rendered.to_string();
        assert!(text.contains("どの画面を、どう変更したいか教えてください。"));
        assert!(text.contains("┌ Tools\n│ ● read nodes\n│ ✓ Tool\n└"));
        assert!(text.contains("│ [completed] literal status"));
        assert!(text.contains("│ You: literal role"));
        assert!(text.contains("│ [0 images]"));
        assert!(text.contains("│ **literal code**"));
        assert_eq!(text.matches("[0 images]").count(), 1);
        assert!(!text.contains("Request complete"));
        assert!(!text.contains("› You") && !text.contains("◆ Codex"));
        assert_eq!(
            rendered.lines[0].style.bg,
            Some(theme::colors().user_background)
        );
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
    fn mid_turn_messages_use_the_same_style_as_regular_prompts() {
        let entries = [
            Entry::new(Kind::User, "First request"),
            Entry::new(Kind::Agent, "Checking."),
            Entry::new(Kind::User, "Use blue."),
            Entry::new(Kind::Agent, "Understood."),
        ];
        let (rendered, _) = history_text(&entries, &Default::default());
        assert_eq!(
            rendered.to_string(),
            "First request\n\nChecking.\n\nUse blue.\n\nUnderstood."
        );
        assert_eq!(rendered.lines[0].style, rendered.lines[4].style);
        assert_ne!(rendered.lines[4].style.bg, rendered.lines[6].style.bg);
    }

    #[test]
    fn role_colors_cover_wrapped_rows_without_taking_header_rows() {
        let (rendered, starts) = history_text(
            &[
                Entry::new(Kind::User, "A long user message"),
                Entry::new(Kind::Agent, "A reply"),
            ],
            &Default::default(),
        );
        assert_eq!(rendered.to_string(), "A long user message\n\nA reply");
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 14, 6));
        History::new((rendered, starts), 12).render(
            &mut buffer,
            Rect::new(0, 0, 14, 6),
            Rect::new(2, 0, 12, 6),
            0,
        );
        for y in 0..2 {
            assert_eq!(
                buffer[(13, y)].bg,
                theme::colors().user_background,
                "Wrapped user rows retain their background"
            );
        }
        assert_eq!(buffer[(0, 0)].symbol(), "›");
        assert_eq!(buffer[(2, 3)].symbol(), "A");
        assert_eq!(buffer[(13, 3)].bg, ratatui::style::Color::Reset);
    }

    #[test]
    fn multiblock_user_message_keeps_one_background_and_marker_while_scrolling() {
        let entries = [
            Entry::new(
                Kind::User,
                "iPad向けのこの画面配置はだいぶ問題がある\n\n- 音声再生ボタンの位置\n- 品詞表示の位置\n- 詳しい意味、例文の位置\n\nこれらの観点で配置とデザインを見直し",
            ),
            Entry::new(Kind::Agent, "Reply."),
        ];
        for width in [12, 24, 80] {
            let mut history = History::new(history_text(&entries, &Default::default()), width);
            let height = history.paragraph.line_count(width) as u16;
            let area = Rect::new(1, 1, width + 4, height);
            let text_area = area.inner(Margin {
                horizontal: 2,
                vertical: 0,
            });
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, width + 6, height + 2));
            history = history.render(&mut buffer, area, text_area, 0);
            assert_eq!(
                buffer
                    .content
                    .iter()
                    .filter(|cell| cell.symbol() == "›")
                    .count(),
                1
            );
            assert_eq!(buffer[(area.x, area.y)].symbol(), "›");
            let user_rows = height - 2;
            for y in area.y..area.y + user_rows {
                for x in area.x..area.right() {
                    assert_eq!(
                        buffer[(x, y)].bg,
                        theme::colors().user_background,
                        "width {width}, row {y}, column {x}"
                    );
                }
            }
            for y in area.y + user_rows..area.bottom() {
                assert_eq!(buffer[(area.x, y)].bg, Color::Reset);
            }
            // A viewport beginning inside the same submission must not invent a new start.
            for scroll in 1..user_rows {
                buffer.reset();
                history = history.render(&mut buffer, area, text_area, scroll);
                assert!(
                    !buffer.content.iter().any(|cell| cell.symbol() == "›"),
                    "width {width}, scroll {scroll}"
                );
                assert_eq!(buffer[(area.x, area.y)].bg, theme::colors().user_background);
            }
        }
    }

    #[test]
    fn adjacent_user_submissions_keep_distinct_markers_and_backgrounds() {
        let entries = [
            Entry::new(
                Kind::User,
                "First request with several wrapped rows\n\n- Point\n\nEnding",
            ),
            Entry::new(Kind::User, "Second"),
            Entry::new(Kind::User, "Third"),
            Entry::new(Kind::Agent, "Reply."),
        ];
        let mut history = History::new(history_text(&entries, &Default::default()), 12);
        let height = history.paragraph.line_count(12) as u16;
        let area = Rect::new(0, 0, 16, height);
        let text_area = area.inner(Margin {
            horizontal: 2,
            vertical: 0,
        });
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        history = history.render(&mut buffer, area, text_area, 0);
        assert_eq!(
            buffer
                .content
                .iter()
                .filter(|cell| cell.symbol() == "›")
                .count(),
            3
        );
        for text in ["First", "Second", "Third"] {
            let row = (0..height)
                .find(|&y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        .contains(text)
                })
                .unwrap();
            assert_eq!(buffer[(0, row)].symbol(), "›", "{text}");
            if row > 0 {
                assert_eq!(buffer[(0, row - 1)].bg, Color::Reset);
            }
            buffer.reset();
            history = history.render(&mut buffer, area, text_area, row);
            assert_eq!(buffer[(0, 0)].symbol(), "›", "{text} at viewport start");
            buffer.reset();
            history = history.render(&mut buffer, area, text_area, 0);
        }
    }
}
