//! Shared single-line editing for search queries and file paths.
use super::*;

pub(super) enum Edit {
    Changed,
    Keep,
    Clear,
    Ignored,
}

#[derive(Clone)]
pub(super) struct Editor {
    input: TextArea<'static>,
    max_bytes: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Self::with_limit(editing::MAX_BYTES)
    }
}

impl Editor {
    pub fn with_limit(max_bytes: usize) -> Self {
        Self {
            input: editor(""),
            max_bytes,
        }
    }

    fn sync(&mut self, query: &str) {
        // Searches can also be cleared by selection and panel navigation.
        if self.input.lines()[0] != query {
            self.input = editor(query);
        }
    }

    pub fn edit(&mut self, query: &mut String, key: KeyEvent) -> Edit {
        self.sync(query);
        match key.code {
            KeyCode::Esc => {
                query.clear();
                self.input = editor("");
                return Edit::Clear;
            }
            KeyCode::Enter => return Edit::Keep,
            KeyCode::Char(c)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL
                        | KeyModifiers::ALT
                        | KeyModifiers::SUPER
                        | KeyModifiers::HYPER
                        | KeyModifiers::META,
                ) && !c.is_control() =>
            {
                self.paste(query, &c.to_string());
                return Edit::Changed;
            }
            _ => {}
        }

        let changed = if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('u') => self.input.delete_line_by_head(),
                KeyCode::Char('_' | '/') => self.input.undo(),
                KeyCode::Char('y') => {
                    self.paste(query, &self.input.yank_text());
                    return Edit::Changed;
                }
                KeyCode::Char('a' | 'b' | 'd' | 'e' | 'f' | 'h' | 'k' | 'w') => {
                    self.input.input(key)
                }
                _ => false,
            }
        } else if (key.modifiers.is_empty()
            && matches!(
                key.code,
                KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::Backspace
                    | KeyCode::Delete
            ))
            || (key.modifiers == KeyModifiers::ALT
                && matches!(
                    key.code,
                    KeyCode::Char('b' | 'f' | 'd' | 'h') | KeyCode::Backspace | KeyCode::Delete
                ))
        {
            self.input.input(key)
        } else {
            false
        };
        if changed {
            query.clone_from(&self.input.lines()[0]);
            Edit::Changed
        } else {
            Edit::Ignored
        }
    }

    pub fn paste(&mut self, query: &mut String, text: &str) {
        self.sync(query);
        let mut insertion = String::new();
        for c in text.chars().filter(|c| !c.is_control()) {
            if query.len() + insertion.len() + c.len_utf8() > self.max_bytes {
                break;
            }
            insertion.push(c);
        }
        if self.input.insert_str(insertion) {
            query.clone_from(&self.input.lines()[0]);
        }
    }

    pub fn title_cursor(&self, query: &str, title_width: usize) -> usize {
        if self.input.lines()[0] != query {
            return title_width;
        }
        let offset = query
            .char_indices()
            .nth(self.input.cursor().1)
            .map_or(query.len(), |(offset, _)| offset);
        title_width.saturating_sub(Line::raw(&query[offset..]).width())
    }

    pub fn viewport<'a>(&self, query: &'a str, width: usize) -> (&'a str, usize) {
        let column = if self.input.lines()[0] == query {
            self.input.cursor().1
        } else {
            query.chars().count()
        };
        let cursor = query
            .char_indices()
            .nth(column)
            .map_or(query.len(), |(i, _)| i);
        let start = query[..cursor]
            .char_indices()
            .rev()
            .map(|(i, _)| i)
            .take_while(|&i| Line::raw(&query[i..cursor]).width() <= width)
            .last()
            .unwrap_or(cursor);
        let end = query[start..]
            .char_indices()
            .map(|(i, _)| start + i)
            .chain(std::iter::once(query.len()))
            .take_while(|&i| Line::raw(&query[start..i]).width() <= width)
            .last()
            .unwrap_or(start);
        (&query[start..end], Line::raw(&query[start..cursor]).width())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emacs_single_line_edits_keep_utf8_limits_and_cursor_viewports() {
        let mut editor = Editor::with_limit(12);
        let mut query = String::new();
        editor.paste(&mut query, "日本語 abc");
        assert_eq!(query, "日本語 ab");
        assert_eq!(editor.viewport(&query, 4), (" ab", 3));
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        );
        assert_eq!(editor.viewport(&query, 3), ("日", 0));
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        );
        assert_eq!(editor.viewport(&query, 4), ("日本", 2));
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
        );
        assert_eq!(query, "本語 ab");
        editor.paste(&mut query, "X\n\t");
        assert_eq!(query, "X本語 ab");
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert_eq!(query, "本語 ab");
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
        );
        assert_eq!(query, "X本語 ab");
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('_'), KeyModifiers::CONTROL),
        );
        assert_eq!(query, "本語 ab");
        assert_eq!(editor.viewport(&query, 0), ("", 0));
    }

    #[test]
    fn search_yanks_and_pastes_stay_within_the_utf8_query_limit() {
        let mut editor = Editor::default();
        let mut query = String::new();
        editor.paste(&mut query, &"x".repeat(1024 * 1024));
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert!(query.is_empty());
        editor.paste(&mut query, "日本語");
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
        );
        assert_eq!(query.len(), 1024 * 1024);
        assert!(query.starts_with("日本語"));
        editor.edit(
            &mut query,
            KeyEvent::new(KeyCode::Char('あ'), KeyModifiers::NONE),
        );
        assert_eq!(query.len(), 1024 * 1024);
        query.clear();
        editor.paste(&mut query, &"あ".repeat(1024 * 1024 / 3 + 1));
        assert_eq!(query.len(), 1024 * 1024 - 1);
    }
}
