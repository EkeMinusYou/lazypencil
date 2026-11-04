//! Optional terminal-cell artifacts for visual review of focused tests.
use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier},
};
use std::path::PathBuf;

pub(super) fn snapshot(app: &mut super::App, name: &str) -> Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 34)).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    artifact(&buffer, name);
    buffer
}

pub(super) fn artifact(buffer: &Buffer, name: &str) {
    let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS")
        .or_else(|| std::env::var_os("LAZYPENCIL_TEST_PICKER_ARTIFACTS"))
    else {
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{name}.txt")),
        (0..buffer.area.height)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < buffer.area.width {
                    let symbol = buffer[(x, y)].symbol();
                    line.push_str(symbol);
                    x += ratatui::text::Line::raw(symbol).width().max(1) as u16;
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    fn color(color: Color, foreground: bool) -> String {
        match color {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Green => "#98c379".into(),
            Color::Blue => "#61afef".into(),
            Color::Magenta => "#c678dd".into(),
            Color::Cyan => "#56b6c2".into(),
            Color::Yellow => "#e5c07b".into(),
            Color::Red => "#e06c75".into(),
            Color::DarkGray => "#5c6370".into(),
            Color::Gray => "#abb2bf".into(),
            Color::Reset if !foreground => default_paper().into(),
            Color::Reset => default_ink().into(),
            _ => "#e5e5e5".into(),
        }
    }
    fn light() -> bool {
        matches!(super::theme::colors().canvas, Color::Rgb(r, g, b) if r > 128 && g > 128 && b > 128)
    }
    fn default_paper() -> &'static str {
        if light() { "#fafafc" } else { "#171923" }
    }
    fn default_ink() -> &'static str {
        if light() { "#20242c" } else { "#e5e5e5" }
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"{}\"/>",
        buffer.area.width * 10,
        buffer.area.height * 20,
        buffer.area.width * 10,
        buffer.area.height * 20,
        default_paper()
    );
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"20\" fill=\"{}\"/>",
                x * 10,
                y * 20,
                color(cell.bg, false)
            ));
        }
    }
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let symbol = cell
                .symbol()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            svg.push_str(&format!("<text x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"Menlo,Hiragino Sans,monospace\" font-size=\"16\" font-weight=\"{}\">{symbol}</text>",x*10,y*20+16,color(cell.fg,true),if cell.modifier.contains(Modifier::BOLD) {"bold"} else {"normal"}));
        }
    }
    svg.push_str("</svg>");
    std::fs::write(directory.join(format!("{name}.svg")), svg).unwrap();
}
