//! Bound every insertion before changing text, selection, yank state or history.
use crossterm::event::KeyEvent;
use ratatui_textarea::{Input, Key, TextArea};

pub(super) const MAX_BYTES: usize = 1024 * 1024;

fn len(editor: &TextArea<'_>) -> usize {
    editor.lines().iter().map(String::len).sum::<usize>() + editor.lines().len().saturating_sub(1)
}

fn offset(editor: &TextArea<'_>, (row, column): (usize, usize)) -> usize {
    let lines = editor.lines();
    lines[..row].iter().map(String::len).sum::<usize>()
        + row
        + lines[row]
            .char_indices()
            .nth(column)
            .map_or(lines[row].len(), |(byte, _)| byte)
}

fn fits(editor: &TextArea<'_>, incoming: usize) -> bool {
    let selected = editor.selection_range().map_or(0, |(start, end)| {
        offset(editor, end) - offset(editor, start)
    });
    len(editor)
        .saturating_sub(selected)
        .saturating_add(incoming)
        <= MAX_BYTES
}

pub(super) fn insert(editor: &mut TextArea<'_>, text: &str) -> bool {
    if !fits(editor, text.len()) {
        return false;
    }
    editor.insert_str(text);
    true
}

/// None rejects a growing edit without touching the editor's undo history.
pub(super) fn input(editor: &mut TextArea<'_>, key: KeyEvent) -> Option<bool> {
    let input = Input::from(key);
    let incoming = match input {
        Input {
            key: Key::Enter, ..
        }
        | Input {
            key: Key::Char('m'),
            ctrl: true,
            alt: false,
            ..
        } => Some(1),
        Input {
            key: Key::Char(c),
            ctrl: false,
            alt: false,
            ..
        } => Some(c.len_utf8()),
        Input {
            key: Key::Tab,
            ctrl: false,
            alt: false,
            ..
        } => {
            let tab = usize::from(editor.tab_length());
            Some(if tab == 0 {
                0
            } else if editor.hard_tab_indent() {
                1
            } else {
                let cursor = editor.cursor();
                let (row, column) = editor
                    .selection_range()
                    .map_or((cursor.0, cursor.1), |(start, _)| start);
                let width: usize = editor.lines()[row]
                    .chars()
                    .take(column)
                    .map(|c| {
                        let mut bytes = [0; 4];
                        ratatui::text::Span::raw(c.encode_utf8(&mut bytes) as &str).width()
                    })
                    .sum();
                tab - width % tab
            })
        }
        Input {
            key: Key::Paste, ..
        }
        | Input {
            key: Key::Char('y'),
            ctrl: true,
            alt: false,
            ..
        } => Some(editor.yank_text().len()),
        _ => None,
    };
    if incoming.is_some_and(|bytes| !fits(editor, bytes)) {
        return None;
    }
    Some(editor.input(input))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui_textarea::CursorMove;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn newline_utf8_and_tab_insertions_use_the_resulting_serialized_byte_count() {
        let mut editor = super::super::editor(&"x".repeat(MAX_BYTES - 1));
        assert_eq!(input(&mut editor, key(KeyCode::Char('🙂'))), None);
        assert_eq!(input(&mut editor, key(KeyCode::Tab)), Some(true));
        assert_eq!(len(&editor), MAX_BYTES);
        assert_eq!(input(&mut editor, key(KeyCode::Enter)), None);
        assert_eq!(
            input(
                &mut editor,
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL)
            ),
            None
        );
        assert_eq!(len(&editor), MAX_BYTES);
        assert!(editor.undo());
        assert_eq!(len(&editor), MAX_BYTES - 1);
        assert!(insert(&mut editor, "\n"));
        assert_eq!(len(&editor), MAX_BYTES);
        assert!(!insert(&mut editor, "x"));
    }

    #[test]
    fn full_buffer_selection_replacement_preserves_unicode_and_bounded_undo_redo() {
        let original = "🙂".repeat(MAX_BYTES / 4);
        let mut editor = super::super::editor(&original);
        editor.start_selection();
        editor.move_cursor(CursorMove::Head);
        assert!(insert(&mut editor, "置換\n🙂"));
        assert_eq!(editor.lines().join("\n"), "置換\n🙂");
        while editor.undo() {
            assert!(len(&editor) <= MAX_BYTES);
        }
        assert_eq!(editor.lines().join("\n"), original);
        while editor.redo() {
            assert!(len(&editor) <= MAX_BYTES);
        }
        assert_eq!(editor.lines().join("\n"), "置換\n🙂");
    }

    #[test]
    fn rejected_yanks_do_not_change_text_or_consume_undo_history() {
        let original = "x".repeat(MAX_BYTES - 1);
        let mut editor = super::super::editor(&original);
        editor.start_selection();
        editor.move_cursor(CursorMove::Head);
        assert_eq!(
            input(
                &mut editor,
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
            ),
            Some(true)
        );
        assert!(insert(&mut editor, "🙂"));
        assert_eq!(
            input(
                &mut editor,
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            None
        );
        assert_eq!(editor.lines().join("\n"), "🙂");
        assert!(editor.undo());
        assert_eq!(len(&editor), 0);
        assert_eq!(
            input(
                &mut editor,
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            Some(true)
        );
        assert_eq!(editor.lines().join("\n"), original);
    }
}
