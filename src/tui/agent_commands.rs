//! Search the connected agent's advertised commands and attach one to a draft.
use super::*;
use crate::agent::available_commands::invocation;
use agent_client_protocol::schema::v1::AvailableCommand;
use ratatui::widgets::Wrap;
use std::sync::Arc;

pub(super) struct Picker {
    pub queued: bool,
    current: Option<String>,
    snapshot: Option<Arc<[AvailableCommand]>>,
    filter: String,
    revision: u64,
    editor: search::Editor,
    pub selected: usize,
}

impl Picker {
    pub fn new(queued: bool, current: Option<String>) -> Self {
        Self {
            queued,
            current,
            snapshot: None,
            filter: String::new(),
            revision: 0,
            editor: search::Editor::with_limit(1024),
            selected: 0,
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.editor.paste(&mut self.filter, text);
        self.selected = 0;
    }

    fn entries(&self) -> Vec<Option<&AvailableCommand>> {
        let terms: Vec<_> = self
            .filter
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut entries = Vec::new();
        if self.current.is_some() && terms.is_empty() {
            entries.push(None);
        }
        entries.extend(
            self.snapshot
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|entry| {
                    let searchable =
                        format!("{} {}", invocation(&entry.name), entry.description).to_lowercase();
                    terms
                        .iter()
                        .all(|term| searchable.contains(term))
                        .then_some(Some(entry))
                }),
        );
        entries
    }

    pub fn refresh(&mut self, snapshot: Option<Arc<[AvailableCommand]>>) {
        if match (&self.snapshot, &snapshot) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        } {
            return;
        }
        let selected = self
            .entries()
            .get(self.selected)
            .copied()
            .flatten()
            .map(|entry| entry.name.clone());
        self.snapshot = snapshot;
        self.revision = self.revision.wrapping_add(1);
        let preferred = selected.as_ref().or(self.current.as_ref());
        self.selected = self
            .entries()
            .iter()
            .position(|entry| entry.is_some_and(|entry| Some(&entry.name) == preferred))
            .unwrap_or(0);
    }

    pub fn pointer_identity(&self) -> String {
        format!(
            "agent-commands:{}:{}:{}",
            self.queued, self.revision, self.filter
        )
    }

    pub fn key(&mut self, key: KeyEvent) {
        let last = self.entries().len().saturating_sub(1);
        self.selected = match key.code {
            KeyCode::Up => self.selected.saturating_sub(1),
            KeyCode::Down => (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected.saturating_sub(8),
            KeyCode::PageDown => (self.selected + 8).min(last),
            KeyCode::Home if key.modifiers.is_empty() => 0,
            KeyCode::End if key.modifiers.is_empty() => last,
            _ => match self.editor.edit(&mut self.filter, key) {
                search::Edit::Changed => 0,
                _ => self.selected,
            },
        };
    }

    pub fn apply(&self, app: &mut App) -> Result<bool> {
        let entries = self.entries();
        let Some(entry) = entries.get(self.selected) else {
            return Ok(false);
        };
        let name = entry.map(|entry| entry.name.clone());
        if let Some(name) = &name
            && !app
                .conversation
                .available_agent_commands()
                .is_some_and(|commands| commands.iter().any(|command| command.name == *name))
        {
            app.set_attention_status(
                "The agent's commands changed. Select a current command.".into(),
            );
            return Ok(false);
        }
        if self.queued {
            let prompt = app.outbox.queued.front_mut().ok_or_else(|| {
                Error::new("INVALID_ARGUMENT", "There is no queued request to update")
            })?;
            let previous = std::mem::replace(&mut prompt.agent_command, name);
            if let Err(error) = app.outbox.save(app.conversation.directory()) {
                app.outbox
                    .queued
                    .front_mut()
                    .expect("queued request")
                    .agent_command = previous;
                return Err(error);
            }
            app.set_status("Queued request updated · Resume queued prompts to send".into());
        } else {
            let previous = std::mem::replace(&mut app.outbox.draft_command, name);
            let previous_draft = app.outbox.draft.replace(app.editor.lines().join("\n"));
            if let Err(error) = app.outbox.save(app.conversation.directory()) {
                app.outbox.draft_command = previous;
                app.outbox.draft = previous_draft;
                return Err(error);
            }
            app.agent_collapsed = false;
            app.searching = false;
            app.preview_search.editing = false;
            app.focus = Focus::Prompt;
        }
        Ok(true)
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, pointer: &mut mouse::Pointer) {
        let mode = if self.queued {
            "Queued command"
        } else {
            "Agent commands"
        };
        let shell = modal::Shell::draw(frame, area, mode, "Search commands…", 1);
        shell.close(frame, pointer);
        shell.query(frame, mode, &self.filter, "Search commands…", &self.editor);
        let entries = self.entries();
        let status = match self.snapshot.as_deref() {
            None => "The agent has not reported commands yet.",
            Some([]) => "The agent reported no commands.",
            Some(_) if entries.is_empty() => "No matching commands.",
            Some(_) if self.queued => "Update the command on the first paused request.",
            Some(_) => "Add a command to the draft.",
        };
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(theme::colors().secondary)),
            shell.context,
        );
        let details = match entries.get(self.selected) {
            Some(Some(entry)) => {
                let description = clean(&entry.description);
                match &entry.input {
                    Some(input) => format!("{description}\nArguments: {}", argument_hint(input)),
                    None => description,
                }
            }
            Some(None) => "Keep the request and clear its agent command.".to_owned(),
            None => String::new(),
        };
        let details = Paragraph::new(details)
            .style(Style::default().fg(theme::colors().secondary))
            .wrap(Wrap { trim: false });
        let details_height = details
            .line_count(shell.body.width)
            .min(usize::from(shell.body.height.saturating_sub(3)))
            as u16;
        let sections = Layout::vertical([Constraint::Min(1), Constraint::Length(details_height)])
            .split(shell.body);
        let choices = entries.iter().enumerate().map(|(index, entry)| {
            let (name, status) = match entry {
                Some(entry) => (
                    invocation(&entry.name),
                    if self.current.as_ref() == Some(&entry.name) {
                        "Selected"
                    } else {
                        ""
                    },
                ),
                None => ("Remove command".into(), ""),
            };
            modal::result(
                &name,
                status,
                sections[0].width.saturating_sub(2),
                index == self.selected,
            )
        });
        pointer.modal_list(
            frame,
            List::new(choices)
                .highlight_style(theme::modal_selected())
                .highlight_symbol("› "),
            sections[0],
            self.selected,
            mouse::ListTarget::AgentCommands,
        );
        frame.render_widget(details, sections[1]);
        frame.render_widget(
            Paragraph::new(modal::hints(&[("↑ ↓", "Select"), ("Esc", "Close")])),
            shell.footer,
        );
        if !entries.is_empty() {
            modal::action(frame, shell.footer, "[Enter Choose]", pointer);
        }
    }
}

fn argument_hint(input: &agent_client_protocol::schema::v1::AvailableCommandInput) -> String {
    match input {
        agent_client_protocol::schema::v1::AvailableCommandInput::Unstructured(input) => {
            clean(&input.hint)
        }
        _ => String::new(),
    }
}
