//! Shared command-palette surfaces, spacing and controls for modal workflows.
use super::*;
use ratatui::{layout::Margin, style::Stylize};

pub(super) fn area(workspace: Rect) -> Rect {
    centered(workspace, 120, 35)
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
            .style(Style::default().fg(theme::colors().frame)),
        area,
    );
}

pub(super) struct Shell {
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub close: Rect,
}

impl Shell {
    pub fn draw(frame: &mut Frame, popup: Rect, title: &str, footer_rows: u16) -> Self {
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
            Constraint::Length(if popup.height >= 20 { 2 } else { 1 }),
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
        frame.render_widget(
            Paragraph::new(title).style(Style::default().fg(colors.text).bold()),
            Rect::new(
                header.x,
                header.y,
                header.width.saturating_sub(9),
                header.height,
            ),
        );
        rule(frame, sections[1]);
        rule(frame, sections[3]);
        Self {
            header,
            body: sections[2].inner(Margin {
                horizontal: 1,
                vertical: u16::from(popup.height >= 24),
            }),
            footer: inset(sections[4]),
            close,
        }
    }

    pub fn close(&self, frame: &mut Frame, pointer: &mut mouse::Pointer) {
        frame.render_widget(
            Paragraph::new("[Close]").style(Style::default().fg(theme::colors().key)),
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
            Style::default().fg(colors.key).bold(),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(colors.secondary),
        ));
    }
    Line::from(spans)
}

pub(super) fn detail_surface(frame: &mut Frame, area: Rect) -> Rect {
    frame.render_widget(Block::default().bg(theme::colors().prompt_background), area);
    area.inner(Margin {
        horizontal: 2.min(area.width / 4),
        vertical: u16::from(area.height >= 8),
    })
}
