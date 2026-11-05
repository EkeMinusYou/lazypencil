//! Shared query editing and panel titles for Nodes and Preview search.
use super::*;

pub(super) enum Edit {
    Changed,
    Keep,
    Clear,
    Ignored,
}

pub(super) fn edit(query: &mut String, key: KeyEvent) -> Edit {
    match key.code {
        KeyCode::Esc => {
            query.clear();
            Edit::Clear
        }
        KeyCode::Enter => Edit::Keep,
        KeyCode::Backspace => {
            query.pop();
            Edit::Changed
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !c.is_control() =>
        {
            paste(query, &c.to_string());
            Edit::Changed
        }
        _ => Edit::Ignored,
    }
}

pub(super) fn paste(query: &mut String, text: &str) {
    for c in text.chars().filter(|c| !c.is_control()) {
        if query.len() + c.len_utf8() > 1024 * 1024 {
            break;
        }
        query.push(c);
    }
}

pub(super) fn title(label: &str, query: &str, editing: bool) -> String {
    if editing || !query.is_empty() {
        format!("{label} /{query}")
    } else {
        label.into()
    }
}

impl App {
    pub(super) fn search_cursor(&self, frame: &mut Frame, pane: Rect, title_width: usize) {
        if pane.width > 2 && pane.height > 1 && !self.has_overlay() {
            let offset = title_width.min(usize::from(pane.width - 2)) as u16;
            frame.set_cursor_position((pane.x + offset, pane.y));
        }
    }
}
