//! Shared command-palette surfaces, spacing and controls for modal workflows.
use super::*;
use ratatui::{layout::Margin, style::Stylize};

pub(super) fn area(workspace: Rect) -> Rect {
    centered(workspace, 104, 30)
}

pub(super) fn dim(frame: &mut Frame) {
    let background = theme::colors().canvas;
    let mix = |color| match (color, background) {
        (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) => {
            let channel = |a, b| ((u16::from(a) * 2 + u16::from(b) * 3) / 5) as u8;
            Color::Rgb(channel(r, br), channel(g, bg), channel(b, bb))
        }
        _ => color,
    };
    for cell in &mut frame.buffer_mut().content {
        // Kitty placeholder foregrounds encode image IDs, not text colors.
        if !cell.symbol().contains('\u{10eeee}') {
            cell.fg = mix(cell.fg);
        }
        cell.bg = mix(cell.bg);
    }
}

pub(super) fn rule(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(area.width)))
            .style(Style::default().fg(theme::colors().user_background)),
        area,
    );
}

pub(super) struct Shell {
    pub header: Rect,
    pub context: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub close: Rect,
}

impl Shell {
    pub fn draw(frame: &mut Frame, popup: Rect, mode: &str, title: &str, footer_rows: u16) -> Self {
        dim(frame);
        frame.render_widget(Clear, popup);
        let colors = theme::colors();
        let block = Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .border_style(Style::default().fg(colors.frame))
            .style(theme::base());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let sections = Layout::vertical([
            Constraint::Length(if popup.height >= 12 { 2 } else { 1 }),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(footer_rows),
        ])
        .split(inner);
        let inset = |rect: Rect| {
            rect.inner(Margin {
                horizontal: 1,
                vertical: 0,
            })
        };
        let header = inset(sections[0]);
        let close_width = 7.min(header.width);
        let close = Rect::new(
            header.right() - close_width,
            header.y,
            close_width,
            1.min(header.height),
        );
        frame.render_widget(Block::default().bg(colors.prompt_background), sections[1]);
        frame.render_widget(Block::default().bg(colors.prompt_background), sections[4]);
        rule(frame, sections[3]);
        let shell = Self {
            header,
            context: inset(sections[1]),
            body: sections[2].inner(Margin {
                horizontal: 1,
                vertical: u16::from(popup.height >= 24),
            }),
            footer: inset(sections[4]),
            close,
        };
        shell.heading(frame, mode, title, false);
        shell
    }

    pub fn heading(&self, frame: &mut Frame, mode: &str, query: &str, placeholder: bool) -> Rect {
        let colors = theme::colors();
        let content = Rect::new(
            self.header.x,
            self.header.y,
            self.close.x.saturating_sub(self.header.x),
            self.header.height,
        );
        frame.render_widget(Clear, content);
        frame.render_widget(Block::default().style(theme::base()), content);
        let badge_width =
            (Line::raw(mode).width() as u16 + 2).min(self.header.width.saturating_sub(12));
        let badge = Rect::new(
            self.header.x,
            self.header.y,
            badge_width,
            1.min(self.header.height),
        );
        frame.render_widget(
            Paragraph::new(format!(" {mode} ")).style(
                Style::default()
                    .fg(colors.preview)
                    .bg(colors.prompt_background)
                    .bold(),
            ),
            badge,
        );
        let prompt = Rect::new(
            badge.right(),
            badge.y,
            3.min(self.header.width.saturating_sub(badge_width)),
            badge.height,
        );
        frame.render_widget(Paragraph::new(" › ").style(action_style()), prompt);
        let mut input = Rect::new(
            prompt.right(),
            badge.y,
            self.close.x.saturating_sub(prompt.right() + 1),
            badge.height,
        );
        if !placeholder
            && self.header.height > 1
            && Line::raw(query).width() > usize::from(input.width)
        {
            input = Rect::new(self.header.x, self.header.y + 1, self.header.width, 1);
        }
        frame.render_widget(
            Paragraph::new(query).style(Style::default().fg(if placeholder {
                colors.secondary
            } else {
                colors.text
            })),
            input,
        );
        input
    }

    pub fn query(&self, frame: &mut Frame, mode: &str, value: &str, placeholder: &str) -> Rect {
        let input = self.heading(frame, mode, "", false);
        let width = usize::from(input.width.saturating_sub(1));
        let tail = value
            .char_indices()
            .find_map(|(i, _)| (Line::raw(&value[i..]).width() <= width).then_some(&value[i..]))
            .unwrap_or("");
        self.heading(
            frame,
            mode,
            if value.is_empty() { placeholder } else { tail },
            value.is_empty(),
        );
        if input.width > 0 && input.height > 0 {
            frame.set_cursor_position((input.x + Line::raw(tail).width() as u16, input.y));
        }
        input
    }

    pub fn context(&self, frame: &mut Frame, scope: &str, count: &str) {
        let colors = theme::colors();
        frame.render_widget(Clear, self.context);
        frame.render_widget(Block::default().bg(colors.prompt_background), self.context);
        let width = (Line::raw(count).width() as u16).min(self.context.width / 2);
        let parts = Layout::horizontal([Constraint::Min(0), Constraint::Length(width)])
            .spacing(u16::from(width > 0))
            .split(self.context);
        let style = Style::default()
            .fg(colors.secondary)
            .bg(colors.prompt_background);
        frame.render_widget(Paragraph::new(scope).style(style), parts[0]);
        frame.render_widget(Paragraph::new(count).style(style).right_aligned(), parts[1]);
    }

    pub fn close(&self, frame: &mut Frame, pointer: &mut mouse::Pointer) {
        frame.render_widget(
            Paragraph::new("[Esc]").style(
                Style::default()
                    .fg(theme::colors().secondary)
                    .bg(theme::colors().user_background),
            ),
            self.close,
        );
        if pointer.controls {
            pointer.add(self.close, mouse::Target::Button(mouse::Action::Close));
        }
    }
}

pub(super) fn action_style() -> Style {
    let colors = theme::colors();
    Style::default().fg(colors.preview).bold()
}

pub(super) fn action(frame: &mut Frame, footer: Rect, label: &str, pointer: &mut mouse::Pointer) {
    let width = (Line::raw(label).width() as u16).min(footer.width);
    let area = Rect::new(
        footer.right() - width,
        footer.y,
        width,
        1.min(footer.height),
    );
    frame.render_widget(Paragraph::new(label).style(action_style()), area);
    if pointer.controls {
        pointer.add(area, mouse::Target::Button(mouse::Action::Accept));
    }
}

pub(super) fn hints(items: &[(&str, &str)]) -> Line<'static> {
    let colors = theme::colors();
    let mut spans = Vec::new();
    for (index, (key, label)) in items.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            Style::default()
                .fg(colors.secondary)
                .bg(colors.user_background),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(colors.secondary),
        ));
    }
    Line::from(spans)
}

pub(super) fn result(
    label: &str,
    metadata: &str,
    width: u16,
    selected: bool,
) -> ratatui::widgets::ListItem<'static> {
    let colors = theme::colors();
    let width = usize::from(width).saturating_sub(2);
    let metadata_width = Line::raw(metadata).width().min(width / 3);
    let label_width = width.saturating_sub(metadata_width + usize::from(metadata_width > 0));
    let fit = |text: &str, width: usize| {
        let mut fitted = String::new();
        for ch in text.chars() {
            if Line::raw(format!("{fitted}{ch}")).width() > width {
                break;
            }
            fitted.push(ch);
        }
        fitted
    };
    let label = fit(label, label_width);
    let padding = width.saturating_sub(Line::raw(&label).width() + metadata_width);
    ratatui::widgets::ListItem::new(Line::from(vec![
        if selected {
            Span::styled(label, Style::default().fg(colors.preview).bold())
        } else {
            Span::raw(label)
        },
        Span::raw(" ".repeat(padding)),
        Span::styled(
            fit(metadata, metadata_width),
            Style::default().fg(colors.secondary),
        ),
    ]))
    .style(
        Style::default()
            .fg(if selected {
                colors.preview
            } else {
                colors.text
            })
            .bg(if selected {
                colors.selected_background
            } else {
                colors.canvas
            }),
    )
}

pub(super) fn detail_surface(frame: &mut Frame, area: Rect) -> Rect {
    frame.render_widget(Block::default().bg(theme::colors().prompt_background), area);
    area.inner(Margin {
        horizontal: 2.min(area.width / 4),
        vertical: u16::from(area.height >= 8),
    })
}
