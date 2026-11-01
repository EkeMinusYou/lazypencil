//! CommonMark presentation for streamed conversation text, without changing saved logs.
use super::theme;
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use std::ops::Range;

fn parser(text: &str) -> Parser<'_> {
    Parser::new_ext(
        text,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
    )
}

/// Metadata-looking lines inside code must remain literal code.
pub(super) fn code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (event, range) in parser(text).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = start.take() {
                    ranges.push(start..range.end);
                }
            }
            _ => {}
        }
    }
    ranges
}

pub(super) fn render(text: &str) -> Vec<Line<'static>> {
    let mut output = Renderer::default();
    for event in parser(text) {
        output.event(event);
    }
    output.flush();
    while output
        .lines
        .last()
        .is_some_and(|line| line.spans.is_empty())
    {
        output.lines.pop();
    }
    output.lines
}

#[derive(Default)]
struct Renderer {
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    style: Style,
    styles: Vec<Style>,
    lists: Vec<Option<u64>>,
    links: Vec<Option<String>>,
    quotes: usize,
    code: bool,
}

impl Renderer {
    fn flush(&mut self) {
        if !self.spans.is_empty() {
            self.lines.push(Line::from(std::mem::take(&mut self.spans)));
        }
    }

    fn separate(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|line| !line.spans.is_empty()) {
            self.lines.push(Line::default());
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                if self.spans.is_empty() {
                    self.lines.push(Line::default());
                } else {
                    self.flush();
                }
            }
            if part.is_empty() {
                continue;
            }
            if self.spans.is_empty() {
                if self.quotes > 0 {
                    self.spans.push(Span::styled(
                        "│ ".repeat(self.quotes),
                        Style::default().fg(theme::MUTED),
                    ));
                }
                if !self.lists.is_empty() {
                    self.spans.push(Span::raw("  ".repeat(self.lists.len())));
                }
                if self.code {
                    self.spans
                        .push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
                }
            }
            self.spans.push(Span::styled(part.to_owned(), style));
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => {
                self.styles.push(self.style);
                match tag {
                    Tag::Heading { .. } => {
                        self.separate();
                        self.style = self.style.fg(theme::AGENT).add_modifier(Modifier::BOLD);
                    }
                    Tag::Strong => self.style = self.style.add_modifier(Modifier::BOLD),
                    Tag::TableHead => {
                        self.flush();
                        self.style = self.style.add_modifier(Modifier::BOLD);
                    }
                    Tag::Emphasis => self.style = self.style.add_modifier(Modifier::ITALIC),
                    Tag::Strikethrough => {
                        self.style = self.style.add_modifier(Modifier::CROSSED_OUT)
                    }
                    Tag::CodeBlock(kind) => {
                        self.separate();
                        let language = match kind {
                            CodeBlockKind::Fenced(info) => {
                                info.split_whitespace().next().unwrap_or("").to_owned()
                            }
                            CodeBlockKind::Indented => String::new(),
                        };
                        self.push(&format!("┌ {language}"), Style::default().fg(theme::MUTED));
                        self.flush();
                        self.code = true;
                        self.style = self.style.fg(theme::PREVIEW);
                    }
                    Tag::BlockQuote(_) => {
                        self.separate();
                        self.quotes += 1;
                    }
                    Tag::List(start) => {
                        if self.lists.is_empty() {
                            self.separate();
                        } else {
                            self.flush();
                        }
                        self.lists.push(start);
                    }
                    Tag::Item => {
                        self.flush();
                        let indent = "  ".repeat(self.lists.len().saturating_sub(1));
                        let marker = match self.lists.last_mut() {
                            Some(Some(number)) => {
                                let marker = format!("{number}. ");
                                *number = number.saturating_add(1);
                                marker
                            }
                            _ => "• ".to_owned(),
                        };
                        self.spans.push(Span::styled(
                            format!("{indent}{marker}"),
                            Style::default().fg(theme::KEY),
                        ));
                    }
                    Tag::Link {
                        link_type,
                        dest_url,
                        ..
                    } => {
                        self.links.push(
                            if matches!(link_type, LinkType::Autolink | LinkType::Email) {
                                None
                            } else {
                                Some(dest_url.to_string())
                            },
                        );
                        self.style = self.style.fg(theme::KEY).add_modifier(Modifier::UNDERLINED);
                    }
                    Tag::Image { dest_url, .. } => {
                        self.links.push(Some(dest_url.to_string()));
                        self.push("▧ ", Style::default().fg(theme::KEY));
                    }
                    Tag::Table(_) => self.separate(),
                    Tag::TableRow => self.flush(),
                    _ => {}
                }
            }
            Event::End(tag) => {
                self.style = self.styles.pop().unwrap_or_default();
                match tag {
                    TagEnd::Heading(_) => self.separate(),
                    TagEnd::Paragraph => {
                        self.flush();
                        if self.lists.is_empty() {
                            self.separate();
                        }
                    }
                    TagEnd::CodeBlock => {
                        self.flush();
                        self.code = false;
                        self.push("└", Style::default().fg(theme::MUTED));
                        self.separate();
                    }
                    TagEnd::BlockQuote(_) => {
                        self.flush();
                        self.quotes = self.quotes.saturating_sub(1);
                        self.separate();
                    }
                    TagEnd::Item => self.flush(),
                    TagEnd::List(_) => {
                        self.flush();
                        self.lists.pop();
                        if self.lists.is_empty() {
                            self.separate();
                        }
                    }
                    TagEnd::Link | TagEnd::Image => {
                        if let Some(Some(destination)) = self.links.pop() {
                            self.push(
                                &format!(" ({destination})"),
                                self.style.fg(theme::SECONDARY),
                            );
                        }
                    }
                    TagEnd::TableCell => self.push(" │ ", self.style.fg(theme::MUTED)),
                    TagEnd::TableHead | TagEnd::TableRow => self.flush(),
                    TagEnd::Table => self.separate(),
                    _ => {}
                }
            }
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => self.push(&text, self.style),
            Event::Code(text) => self.push(&text, self.style.fg(theme::PREVIEW)),
            Event::SoftBreak | Event::HardBreak => self.flush(),
            Event::Rule => {
                self.separate();
                self.push("────────", Style::default().fg(theme::MUTED));
                self.separate();
            }
            Event::TaskListMarker(checked) => self.push(
                if checked { "☑ " } else { "☐ " },
                self.style.fg(if checked {
                    theme::SUCCESS
                } else {
                    theme::MUTED
                }),
            ),
            Event::FootnoteReference(label) => {
                self.push(&format!("[{label}]"), self.style.fg(theme::KEY))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_japanese_markdown_without_losing_links_lists_or_literal_code() {
        let source = "# 変更内容\n\n**どの画面を、どう変更したいか**教えてください。\n\n- **余白**を広げる\n  - 子項目\n- [x] 確認済み\n\n3. 次の作業\n4. 最終確認\n\n> *引用*と~~取消~~\n\n[設計資料](https://example.com/design) と `**literal**`。\n\n| 項目 | 状態 |\n| --- | --- |\n| 余白 | 完了 |\n\n```rust\nlet value = \"**literal**\";\n```\n";
        let lines = render(source);
        let text = plain(&lines);
        for expected in [
            "変更内容",
            "どの画面を、どう変更したいか",
            "• 余白を広げる",
            "  • 子項目",
            "☑ 確認済み",
            "3. 次の作業",
            "4. 最終確認",
            "│ 引用と取消",
            "設計資料 (https://example.com/design)",
            "**literal**",
            "項目 │ 状態",
            "let value = \"**literal**\";",
        ] {
            assert!(text.contains(expected), "Missing {expected:?}: {text}");
        }
        assert!(!text.contains("**どの画面"));
        assert!(!text.contains("```"));
        let spans = lines
            .iter()
            .flat_map(|line| &line.spans)
            .collect::<Vec<_>>();
        assert!(
            spans
                .iter()
                .any(|span| span.content == "どの画面を、どう変更したいか"
                    && span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(spans.iter().any(
            |span| span.content == "引用" && span.style.add_modifier.contains(Modifier::ITALIC)
        ));
        assert!(spans.iter().any(|span| span.content == "取消"
            && span.style.add_modifier.contains(Modifier::CROSSED_OUT)));
        assert!(spans.iter().any(|span| span.content == "設計資料"
            && span.style.add_modifier.contains(Modifier::UNDERLINED)));
    }

    #[test]
    fn partial_responses_and_escaped_markers_preserve_their_content() {
        for source in [
            "**入力中",
            "**入力中**",
            "```rust\nlet value = 1;",
            "```rust\nlet value = 1;\n```",
        ] {
            let text = plain(&render(source));
            assert!(
                text.contains(if source.starts_with("**") {
                    "入力中"
                } else {
                    "let value = 1;"
                }),
                "{text}"
            );
        }
        let lines = render("\\*\\*そのまま\\*\\* と **太字と*斜体*** の続き");
        assert!(plain(&lines).contains("**そのまま** と 太字と斜体 の続き"));
        let suffix = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.contains("の続き"))
            .unwrap();
        assert!(
            !suffix
                .style
                .add_modifier
                .intersects(Modifier::BOLD | Modifier::ITALIC)
        );
    }
}
