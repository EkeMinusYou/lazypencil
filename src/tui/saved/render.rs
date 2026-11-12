//! Read-only conversation presentation built from bounded transcript records.
use super::super::{clean, markdown, theme};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Text},
};
use serde_json::Value;

pub(super) fn entry(data: &Value, session: bool) -> Text<'static> {
    let colors = theme::colors();
    let label = |text: &str| Line::styled(text.to_owned(), Style::default().fg(colors.secondary));
    let value = |key: &str| Line::raw(clean(data[key].as_str().unwrap_or("unknown")));
    let mut lines = vec![
        Line::styled(
            if session {
                "CONVERSATION"
            } else {
                "SAVED TURN"
            },
            Style::default()
                .fg(colors.preview)
                .add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    if session {
        lines.extend([
            value("agent"),
            Line::default(),
            label("SESSION"),
            value("id"),
            Line::default(),
            label("ACCOUNT"),
            value("requested_auth"),
        ]);
    } else {
        lines.extend([
            value("id"),
            Line::default(),
            label("SIZE"),
            Line::raw(format!("{} bytes", data["bytes"].as_u64().unwrap_or(0))),
        ]);
    }
    lines.push(Line::default());
    if data["unavailable"] == true {
        lines.push(Line::styled(
            "Unavailable",
            Style::default().fg(colors.running),
        ));
        lines.push(Line::raw(clean(&data["error"].to_string())));
    } else {
        lines.push(label(if session {
            "Browsing sends no requests. Saved data stays unchanged."
        } else {
            "Enter opens the saved log. Your draft is preserved."
        }));
    }
    Text::from(lines)
}

pub(super) fn conversation(data: &Value) -> Text<'static> {
    let Some(records) = data["records"].as_array() else {
        return Text::from(markdown::render(&clean(
            data["text"].as_str().unwrap_or(""),
        )));
    };
    let mut lines = Vec::new();
    let mut kind = "";
    let mut body = String::new();
    let mut last_event = String::new();
    for record in records {
        let next = record["kind"].as_str().unwrap_or("record");
        let text = clean(record["text"].as_str().unwrap_or(""));
        if next == "turn" || text.is_empty() {
            continue;
        }
        if matches!(next, "user" | "agent" | "thought") {
            if next != kind || (next == "user" && record["start"] == true) {
                flush(&mut lines, kind, &mut body);
            }
            kind = next;
            body.push_str(&text);
            last_event.clear();
            continue;
        }
        flush(&mut lines, kind, &mut body);
        kind = "";
        if next == "status" && text.trim() == "Request complete" {
            continue;
        }
        if next == "record" {
            if let Some(summary) = event_summary(&text) {
                if summary != last_event {
                    event(&mut lines, &summary, theme::colors().secondary);
                    last_event = summary;
                }
            } else {
                let text = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|value| serde_json::to_string_pretty(&value).ok())
                    .unwrap_or(text);
                heading(&mut lines, "OTHER SAVED EVENT", theme::colors().secondary);
                lines.extend(text.lines().map(|line| {
                    Line::styled(
                        format!("│ {line}"),
                        Style::default().fg(theme::colors().secondary),
                    )
                }));
            }
        } else {
            last_event.clear();
            let colors = theme::colors();
            let color = if next == "error" || text.contains("[failed]") {
                colors.error
            } else if text.trim().starts_with("[completed]") {
                colors.success
            } else {
                colors.secondary
            };
            if next == "tool" {
                let text = text.trim();
                let (status, title) = text
                    .strip_prefix('[')
                    .and_then(|text| text.split_once("] "))
                    .unwrap_or(("running", text));
                let icon = match status {
                    "completed" => "✓",
                    "failed" => "✕",
                    "pending" => "○",
                    "cancelled" | "canceled" => "−",
                    _ => "●",
                };
                event(&mut lines, &format!("{icon} {title}"), color);
            } else {
                let message = match next {
                    "error" => format!("✕ Request failed: {}", text.trim()),
                    _ => text.trim().to_owned(),
                };
                event(&mut lines, &message, color);
            }
        }
    }
    flush(&mut lines, kind, &mut body);
    Text::from(lines)
}

fn heading(lines: &mut Vec<Line<'static>>, title: &str, color: ratatui::style::Color) {
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    lines.push(Line::styled(
        title.to_owned(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
}

fn flush(lines: &mut Vec<Line<'static>>, kind: &str, body: &mut String) {
    if body.trim().is_empty() {
        body.clear();
        return;
    }
    let colors = theme::colors();
    let (label, color) = match kind {
        "user" => ("YOU", colors.key),
        "thought" => ("THINKING", colors.secondary),
        _ => ("ASSISTANT", colors.agent),
    };
    heading(lines, label, color);
    if kind == "user" {
        lines.last_mut().unwrap().style = Style::default().bg(colors.user_background);
    }
    let content = markdown::render(body.trim());
    lines.extend(content.into_iter().map(|mut line| {
        if kind == "thought" {
            line.style = line.style.fg(colors.secondary);
        } else if kind == "user" {
            line.style = line.style.fg(colors.user).bg(colors.user_background);
        }
        line
    }));
    body.clear();
}

fn event(lines: &mut Vec<Line<'static>>, text: &str, color: ratatui::style::Color) {
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    lines.extend(text.lines().map(|line| {
        Line::styled(
            if line.starts_with(['✓', '✕', '●', '○', '−']) {
                line.to_owned()
            } else {
                format!("· {line}")
            },
            Style::default().fg(color),
        )
    }));
}

fn event_summary(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let update = &value["notification"]["update"];
    if value["type"] == "update" && update["sessionUpdate"] == "session_info_update" {
        return Some(
            update["_meta"]["codex"]["threadStatus"]["type"]
                .as_str()
                .map_or_else(
                    || "Session updated".into(),
                    |status| format!("Session {}", clean(status)),
                ),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn saved_conversation_groups_chunks_styles_markdown_and_keeps_literal_roles_in_messages() {
        let data = json!({"records":[
            {"kind":"turn","text":"Turn internal-uuid"},
            {"kind":"user","start":true,"text":"Please improve the design.\nYou:\nliteral role"},
            {"kind":"record","text":"{\"type\":\"update\",\"notification\":{\"update\":{\"sessionUpdate\":\"session_info_update\",\"_meta\":{\"codex\":{\"threadStatus\":{\"type\":\"active\"}}}}}}"},
            {"kind":"thought","text":"**Redesigning "},
            {"kind":"thought","text":"role cards**"},
            {"kind":"agent","text":"## Result\nSpacing is"},
            {"kind":"agent","text":" "},
            {"kind":"agent","text":"**improved**.\n```text\nYou:\n```"},
            {"kind":"tool","text":"[completed] Read document"},
            {"kind":"status","text":"Request complete"}
        ]});
        let result = conversation(&data);
        let text = result
            .lines
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches("THINKING").count(), 1);
        assert_eq!(text.matches("ASSISTANT").count(), 1);
        assert_eq!(text.matches("YOU").count(), 1);
        for expected in [
            "literal role",
            "You:",
            "Session active",
            "Redesigning role cards",
            "Result",
            "Spacing is improved.",
            "✓ Read document",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        for hidden in [
            "sessionUpdate",
            "_meta",
            "internal-uuid",
            "**",
            "Request complete",
        ] {
            assert!(!text.contains(hidden), "{text}");
        }
        assert!(
            result
                .lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.content == "improved"
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
    }
}
