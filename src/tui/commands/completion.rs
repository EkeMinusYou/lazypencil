//! Conversation command completion never submits text or executes an action.
use super::*;

#[derive(Default)]
pub(in crate::tui) struct State {
    query: Option<String>,
    dismissed: bool,
    pub selected: usize,
}

pub(in crate::tui) fn query<'a>(input: &'a TextArea<'_>) -> Option<&'a str> {
    let [line] = input.lines() else { return None };
    let token = line.split_whitespace().next()?;
    if !line.starts_with('/') || !token.is_ascii() {
        return None;
    }
    let cursor = input.cursor();
    if cursor.0 != 0 || cursor.1 == 0 || cursor.1 > token.len() {
        return None;
    }
    token.strip_prefix('/')
}

pub(in crate::tui) fn candidates(prefix: &str) -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|command| {
            command.slash_enabled()
                && (command.name.starts_with(prefix)
                    || command
                        .aliases
                        .iter()
                        .any(|alias| alias.starts_with(prefix)))
        })
        .collect()
}

fn completion_name(prefix: &str, command: &Command) -> &'static str {
    if command.name.starts_with(prefix) {
        command.name
    } else {
        command
            .aliases
            .iter()
            .copied()
            .find(|alias| alias.starts_with(prefix))
            .expect("matching completion alias")
    }
}

impl State {
    fn sync(&mut self, query: Option<&str>) {
        if self.query.as_deref() != query {
            self.query = query.map(str::to_owned);
            self.dismissed = false;
            self.selected = 0;
        }
    }
}

impl App {
    fn command_candidates(&mut self) -> Vec<&'static Command> {
        let prefix = (self.focus == Focus::Prompt && !self.has_modal())
            .then(|| query(&self.editor))
            .flatten();
        self.completion.sync(prefix);
        if self.completion.dismissed {
            return vec![];
        }
        prefix.map(candidates).unwrap_or_default()
    }

    pub(in crate::tui) fn complete_command_key(&mut self, key: KeyEvent) -> bool {
        let commands = self.command_candidates();
        if commands.is_empty() {
            return false;
        }
        let plain = key.modifiers.is_empty();
        match key.code {
            KeyCode::Up if plain => {
                self.completion.selected =
                    (self.completion.selected + commands.len() - 1) % commands.len()
            }
            KeyCode::Down if plain => {
                self.completion.selected = (self.completion.selected + 1) % commands.len()
            }
            KeyCode::Esc if plain => self.completion.dismissed = true,
            KeyCode::Tab if plain => self.fill_command(commands[self.completion.selected]),
            _ if (key.code == KeyCode::Enter && plain) || self.keys.submits(key) => {
                let prefix = query(&self.editor).expect("completion query");
                if commands
                    .iter()
                    .any(|command| command.name == prefix || command.aliases.contains(&prefix))
                {
                    return false;
                }
                self.fill_command(commands[self.completion.selected]);
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    fn fill_command(&mut self, command: &Command) {
        let line = &self.editor.lines()[0];
        let end = line.find(char::is_whitespace).unwrap_or(line.len());
        let prefix = &line[1..end];
        let name = completion_name(prefix, command);
        let text = format!(
            "{}{}",
            &name[prefix.len()..],
            if end == line.len() && !command.arguments.is_empty() {
                " "
            } else {
                ""
            }
        );
        self.editor.move_cursor(CursorMove::Jump(0, end as u16));
        self.editor.insert_str(&text);
        self.editor.move_cursor(CursorMove::End);
        self.draft_changed = true;
        self.completion.sync(query(&self.editor));
        self.completion.dismissed = true;
    }

    pub(in crate::tui) fn complete_command(&mut self, index: usize) {
        if let Some(command) = self.command_candidates().get(index) {
            self.fill_command(command);
            self.dirty = true;
        }
    }

    pub(in crate::tui) fn draw_command_completion(
        &mut self,
        frame: &mut Frame,
        inner: Rect,
        prompt: Rect,
    ) {
        let commands = self.command_candidates();
        let available = prompt.y.saturating_sub(inner.y).min(9);
        if commands.is_empty() || available < 3 || inner.width < 8 {
            return;
        }
        let height = available.min(commands.len() as u16 + 2);
        let area = Rect::new(inner.x, prompt.y - height, inner.width, height);
        let block = Block::bordered()
            .title("/ Commands")
            .border_style(Style::default().fg(theme::colors().agent))
            .style(Style::default().bg(theme::colors().prompt_background));
        let rows = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        // Cover underlying history/context hits while leaving the prompt's IME caret visible.
        self.pointer
            .add(area, mouse::Target::CommandCompletion(None));
        let count = usize::from(rows.height);
        let prefix = query(&self.editor).expect("visible completion query");
        let first = self
            .completion
            .selected
            .saturating_sub(count.saturating_sub(1));
        for (index, command) in commands.iter().enumerate().skip(first).take(count) {
            self.pointer.add(
                Rect::new(rows.x, rows.y + (index - first) as u16, rows.width, 1),
                mouse::Target::CommandCompletion(Some(index)),
            );
            let selected = index == self.completion.selected;
            let unavailable = self.command_unavailable(command);
            let style = if selected {
                theme::node_selected(true)
            } else {
                theme::base()
            };
            let label = if rows.width >= 46 {
                format!(
                    "{} /{}  {}",
                    if selected { "›" } else { " " },
                    completion_name(prefix, command),
                    unavailable.unwrap_or(command.description)
                )
            } else {
                format!(
                    "{} /{}",
                    if selected { "›" } else { " " },
                    completion_name(prefix, command)
                )
            };
            frame.render_widget(
                Paragraph::new(label).style(if unavailable.is_some() {
                    style.fg(theme::colors().secondary)
                } else {
                    style
                }),
                Rect::new(rows.x, rows.y + (index - first) as u16, rows.width, 1),
            );
        }
        let hint = if area.width >= 33 {
            " ↑↓ select · Tab/Enter fill · Esc "
        } else {
            " ↑↓ Tab Esc "
        };
        let hint_width = (hint.chars().count() as u16).min(area.width.saturating_sub(2));
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(theme::colors().secondary)),
            Rect::new(area.x + 1, area.bottom() - 1, hint_width, 1),
        );
    }
}
