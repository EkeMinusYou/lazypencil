//! Conversation history and its attached prompt share one visual surface.
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
            self.status = status.into();
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
        let history_gap = u16::from(!self.log.is_empty() && gap_space > 0);
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
        let paragraph = if self.log.is_empty() && self.saved_log.is_empty() {
            None
        } else {
            Some(self.history.take().unwrap_or_else(|| {
                let log = format!("{}{}", self.saved_log, self.log);
                Paragraph::new(history_text(&log, &self.tool_calls)).wrap(Wrap { trim: false })
            }))
        };
        let lines = paragraph.as_ref().map_or(0, |paragraph| {
            paragraph.line_count(text_area.width).min(u16::MAX as usize) as u16
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
        if let Some(paragraph) = paragraph {
            let paragraph = paragraph.scroll((scroll, 0));
            frame.render_widget(&paragraph, text_area);
            fill_user_backgrounds(frame.buffer_mut(), logarea);
            self.history = Some(paragraph);
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
                                prompt.text.replace('\n', " ")
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
        self.editor
            .set_block(prompt_block(self.focus == Focus::Prompt));
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
    let mut previous_user = false;
    for y in area.y..area.bottom() {
        let user =
            (area.x..area.right()).any(|x| buffer[(x, y)].bg == theme::colors().user_background);
        if user {
            for x in area.x..area.right() {
                buffer[(x, y)].set_bg(theme::colors().user_background);
            }
            if !previous_user && area.width > 0 {
                buffer[(area.x, y)]
                    .set_symbol("›")
                    .set_fg(theme::colors().user);
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
    let mut activity = Vec::new();
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
                "running" | "in_progress" => Some(("●", theme::colors().working)),
                "pending" => Some(("○", theme::colors().key)),
                "completed" => Some(("✓", theme::colors().success)),
                "failed" => Some(("✕", theme::colors().error)),
                "cancelled" | "canceled" => Some(("−", theme::colors().muted)),
                _ => None,
            }
        {
            flush_body(&mut body, &mut lines, activity.is_empty(), user);
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
            if status == "failed"
                && let Some(tool) = tool
                && !tool.details.trim().is_empty()
            {
                close_tools(&mut lines, &mut activity);
                lines.extend(tool.details.trim().lines().map(|line| {
                    Line::styled(line.to_owned(), Style::default().fg(theme::colors().error))
                }));
                lines.push(Line::default());
            }
            continue;
        }
        if line == "[0 images]" {
            continue;
        }
        if !activity.is_empty() && line.trim().is_empty() {
            continue;
        }
        close_tools(&mut lines, &mut activity);
        let styled = if let Some(message) = line
            .strip_prefix("You:")
            .or_else(|| line.strip_prefix("You (mid-turn):"))
        {
            flush_body(&mut body, &mut lines, true, user);
            if lines.last().is_some_and(|line| !line.spans.is_empty()) {
                lines.push(Line::default());
            }
            user = true;
            body.push_str(message.trim_start());
            body.push('\n');
            continue;
        } else if matches!(line, "Codex:" | "Claude:" | "Agent:") {
            flush_body(&mut body, &mut lines, true, user);
            user = false;
            continue;
        } else if line == "Request complete" {
            Line::styled(
                format!("✓ {line}"),
                Style::default().fg(theme::colors().success),
            )
        } else if line.starts_with("Request failed:") {
            Line::styled(
                format!("✕ {line}"),
                Style::default().fg(theme::colors().error),
            )
        } else if line.starts_with('[') && line.ends_with(" images]") {
            Line::styled(
                format!("▧ {}", line.trim_matches(['[', ']'])),
                Style::default().fg(theme::colors().key),
            )
        } else {
            body.push_str(raw);
            continue;
        };
        flush_body(&mut body, &mut lines, true, user);
        lines.push(styled);
    }
    close_tools(&mut lines, &mut activity);
    flush_body(&mut body, &mut lines, false, user);
    while lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }
    Text::from(lines)
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

fn flush_body(body: &mut String, lines: &mut Vec<Line<'static>>, spacing: bool, user: bool) {
    let mut rendered = markdown::render(body);
    let has_content = !rendered.is_empty();
    if user {
        for line in &mut rendered {
            line.style = line
                .style
                .fg(theme::colors().user)
                .bg(theme::colors().user_background);
            if theme::colors().user_background == Color::Reset {
                line.spans.insert(0, Span::raw("› "));
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
    fn restored_transcript_keeps_user_and_agent_styling_separate() {
        let rendered = history_text(
            "You:\nSaved request\n\nAgent:\nSaved response\n",
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
        let log = "[tool_call] \"startup\"\n";
        assert!(
            !history_text(log, &tools)
                .to_string()
                .contains("duplicate server")
        );
        tools
            .get_mut("startup")
            .unwrap()
            .update(&json!({"status":"failed"}));
        let rendered = history_text(log, &tools).to_string();
        assert!(rendered.contains("✕ startup"));
        assert!(rendered.contains("MCP server was not started: duplicate server name."));
    }

    #[test]
    fn tool_runs_show_only_the_latest_three_calls() {
        for count in 0_usize..=6 {
            let source = (0..count)
                .map(|index| format!("[completed] call {index}\n\n"))
                .collect::<String>();
            let rendered = history_text(&source, &Default::default()).to_string();
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
        let calls =
            "[completed] old\n\n[completed] read\n\n[failed] edit\n\n[in_progress] render\n\n";
        let source = format!(
            "{calls}Checking the result.\n\n{calls}```text\n[completed] literal\n```\n\n[completed] final\n"
        );
        let rendered = history_text(&source, &Default::default()).to_string();
        let block = "┌ Tools\n│ … 1 earlier call\n│ ✓ read\n│ ✕ edit\n│ ● render\n└";
        assert_eq!(rendered.matches(block).count(), 2);
        assert!(rendered.contains(&format!("{block}\n\nChecking the result.")));
        assert!(rendered.contains("│ [completed] literal"));
        assert!(rendered.ends_with("┌ Tools\n│ ✓ final\n└"));
        assert!(!rendered.contains("✓ old"));
    }

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
        for agent in ["Codex", "Claude"] {
            let rendered = history_text(
                &format!(
                    "You: First request\n\n{agent}:\nChecking.\n\nYou (mid-turn): Use blue.\n\n{agent}:\nUnderstood.\n"
                ),
                &Default::default(),
            );
            assert_eq!(
                rendered.to_string(),
                "First request\n\nChecking.\n\nUse blue.\n\nUnderstood."
            );
            assert_eq!(rendered.lines[0].style, rendered.lines[4].style);
            assert_ne!(rendered.lines[4].style.bg, rendered.lines[6].style.bg);
        }
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
                theme::colors().user_background,
                "Wrapped user rows retain their background"
            );
        }
        assert_eq!(buffer[(0, 0)].symbol(), "›");
        assert_eq!(buffer[(2, 3)].symbol(), "A");
        assert_eq!(buffer[(13, 3)].bg, ratatui::style::Color::Reset);
    }
}
