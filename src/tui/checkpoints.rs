//! Asynchronous checkpoint browsing and shared, guarded restoration.

use crate::{Error, Result, project::Project, service, store::Store};
use serde_json::Value;
use std::{path::PathBuf, thread::JoinHandle};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const PAGE_SIZE: usize = 50;

#[derive(Clone)]
pub struct Checkpoint {
    pub id: String,
    pub sequence: u64,
    pub created_at: String,
    pub request_id: String,
    pub revision: String,
}
#[derive(Clone)]
pub struct Page {
    pub revision: String,
    pub dependency_revision: String,
    pub entries: Vec<Checkpoint>,
    pub next_before: Option<u64>,
    pub cursors: Vec<Option<u64>>,
    pub selected: usize,
    pub notice: Option<String>,
}
pub enum Output {
    History(Page),
    Restored(Value),
    Saved(Box<super::saved::Page>),
}

pub struct Task {
    pub receiver: oneshot::Receiver<Result<Output>>,
    pub cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}
impl Task {
    pub(super) fn spawn(
        label: &str,
        action: impl FnOnce(&CancellationToken) -> Result<Output> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = oneshot::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let thread = std::thread::Builder::new()
            .name(format!("lazypencil-{label}"))
            .spawn(move || {
                let result = action(&token);
                let _ = sender.send(result);
            })
            .map_err(super::terminal::io)?;
        Ok(Self {
            receiver,
            cancel,
            thread: Some(thread),
        })
    }
    pub fn history(store: Store, fonts: Vec<PathBuf>, cursors: Vec<Option<u64>>) -> Result<Self> {
        Self::spawn("history", move |cancel| {
            crate::cancel::check(Some(cancel))?;
            let mut project = Project::load_with_cancellation(&store, Some(cancel))?;
            for path in &fonts {
                crate::cancel::check(Some(cancel))?;
                project.capture_asset(path, "font")?;
            }
            let revision = project.root_document().revision();
            let dependency_revision = project.dependency_revision();
            let mut entries = store.history_with_cancellation(
                PAGE_SIZE + 1,
                cursors.last().copied().flatten(),
                Some(cancel),
            )?;
            crate::store::require_revision(&store.read()?, &revision)?;
            project.verify_unchanged()?;
            crate::cancel::check(Some(cancel))?;
            let next_before = (entries.len() > PAGE_SIZE).then(|| entries[PAGE_SIZE - 1].sequence);
            entries.truncate(PAGE_SIZE);
            let entries = entries
                .into_iter()
                .map(|entry| Checkpoint {
                    id: entry.id,
                    sequence: entry.sequence,
                    created_at: entry.created_at,
                    request_id: entry.request_id,
                    revision: entry.after_revision,
                })
                .collect();
            Ok(Output::History(Page {
                revision,
                dependency_revision,
                entries,
                next_before,
                cursors,
                selected: 0,
                notice: None,
            }))
        })
    }
    pub fn restore(store: Store, fonts: Vec<PathBuf>, page: &Page) -> Result<Self> {
        let entry = page
            .entries
            .get(page.selected)
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Select a checkpoint first"))?
            .clone();
        let revision = page.revision.clone();
        let dependency_revision = page.dependency_revision.clone();
        let request_id = uuid::Uuid::new_v4().to_string();
        Self::spawn("restore", move |cancel| {
            service::restore_checkpoint_with_cancellation(&store, &service::RestoreRequest { request_id: &request_id, expected_revision: &revision, expected_dependency_revision: Some(&dependency_revision), checkpoint: &entry.id }, &fonts, Some(cancel)).map(Output::Restored)
                .map_err(|error| { let cause = error.details.clone(); error.details(serde_json::json!({"document":store.read().map(|document| document.summary()).unwrap_or_else(|error| serde_json::json!({"error":error})),"cause":cause})) })
        })
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub enum Modal {
    Commands { selected: usize },
    Loading { message: &'static str },
    History(Page),
    Confirm(Page),
    Saved(Box<super::saved::Page>),
}
impl Modal {
    pub fn commands() -> Self {
        Self::Commands { selected: 0 }
    }
}
pub const COMMANDS: &[&str] = &[
    "Restore checkpoint…",
    "Refresh preview",
    "Open preview externally",
    "Help",
    "Quit",
    "Browse current conversation logs…",
    "Browse saved conversations…",
    "New empty conversation (save current draft)…",
    "Toggle Nodes panel",
    "Toggle Agent panel",
    "View controls…",
    "Enable pinch zoom…",
];

impl super::App {
    pub(super) fn has_modal(&self) -> bool {
        self.help || self.modal.is_some() || !self.permissions.is_empty() || self.pointer.more
    }
    pub(super) fn open_history(&mut self, cursors: Vec<Option<u64>>) -> Result<()> {
        if self.conversation.active() || self.document_task.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the active operation or cancel it before browsing checkpoints",
            ));
        }
        self.document_task = Some(Task::history(
            self.conversation.store().clone(),
            self.conversation.settings().fonts.clone(),
            cursors,
        )?);
        self.saved_return = None;
        self.modal = Some(Modal::Loading {
            message: "Loading checkpoints… · Esc cancels",
        });
        self.restore_return = None;
        Ok(())
    }
    pub(super) fn document_result(
        &mut self,
        output: Result<Output>,
        worker: &super::preview::Worker,
    ) {
        match output {
            Ok(Output::Saved(page)) => {
                let deleted = page.deletion_completed;
                self.saved_return = None;
                self.modal = Some(Modal::Saved(page));
                self.set_status(
                    if deleted {
                        "Local conversation deleted · project assets and provider history kept"
                    } else if self.conversation.active() {
                        "Saved logs are read-only · active request continues · Ctrl+C cancels"
                    } else {
                        "Saved logs are read-only; browsing sends no requests"
                    }
                    .into(),
                );
            }
            Ok(Output::History(page)) => {
                self.modal = Some(Modal::History(page));
                self.set_status("Select a checkpoint; Enter reviews the restoration".into());
            }
            Ok(Output::Restored(value)) => {
                self.modal = None;
                self.restore_return = None;
                self.nodes.select(None);
                self.view.zoom = None;
                self.set_status(format!(
                    "Restored checkpoint · revision {}",
                    value["revision"].as_str().unwrap_or("unknown")
                ));
                self.append(&format!("\nCheckpoint restored: {value}\n"));
                self.request(worker, false, false);
            }
            Err(error) => {
                if let Some(mut page) = self.saved_return.take() {
                    page.operation_error(super::clean(&error.to_string()));
                    self.modal = Some(Modal::Saved(page));
                } else if let Some(mut page) = self.restore_return.take() {
                    page.notice = Some(super::clean(&format!(
                        "{error} · r reloads current revisions"
                    )));
                    self.modal = Some(Modal::History(page));
                } else if matches!(self.modal, Some(Modal::Loading { .. })) {
                    self.modal = None;
                }
                self.error(error);
            }
        }
    }
    pub(super) fn finish_document_result(
        &mut self,
        output: Result<Output>,
        worker: &super::preview::Worker,
    ) {
        let cancelled = self
            .document_task
            .as_ref()
            .is_some_and(|task| task.cancel.is_cancelled());
        self.document_task.take();
        let read_only = match &output {
            Ok(Output::History(_)) => true,
            Ok(Output::Saved(page)) => !page.deletion_completed,
            _ => false,
        };
        if cancelled && read_only {
            if let Some(page) = self.saved_return.take() {
                self.modal = Some(Modal::Saved(page));
            } else if matches!(self.modal, Some(Modal::Loading { .. })) {
                self.modal = None;
            }
            self.set_status("History loading cancelled".into());
        } else {
            self.document_result(output, worker);
        }
    }
    pub(super) fn activate_modal(&mut self, worker: &super::preview::Worker) -> Result<bool> {
        let Some(modal) = self.modal.take() else {
            return Ok(false);
        };
        match modal {
            Modal::Commands { selected } => match selected {
                0 => self.open_history(vec![None])?,
                1 => self.request(worker, false, true),
                2 => self.open_preview()?,
                3 => self.show_help(),
                4 => return Ok(true),
                5 => self.open_saved(
                    super::saved::Page::current(self.conversation.id().into()),
                    None,
                )?,
                6 => self.open_saved(
                    super::saved::Page::sessions(self.conversation.id().into()),
                    None,
                )?,
                7 => self.pending_session = Some(super::saved::SessionChange::New),
                8 => self.toggle_panel(super::Nav::ToggleNodesPanel, worker),
                9 => self.toggle_panel(super::Nav::ToggleAgentPanel, worker),
                10 if self.mouse.enabled => self.pointer.more = true,
                10 => self.set_status("Pointer input is disabled in user settings".into()),
                11 => {
                    let status = if !self.mouse.enabled || !self.mouse.native_pinch {
                        "Pinch input is disabled in user settings"
                    } else if !cfg!(target_os = "macos")
                        || std::env::var_os("SSH_CONNECTION").is_some()
                    {
                        "Native pinch input requires a local macOS session; terminal-forwarded Ctrl+wheel zoom remains available"
                    } else if self.native_pinch.start(true) {
                        "Pinch zoom enabled"
                    } else if !self.native_pinch.authorized() {
                        "Pinch zoom needs macOS Accessibility access; allow the requested app in System Settings and retry"
                    } else {
                        "Cannot start native pinch input; restart the TUI after allowing Accessibility access"
                    };
                    self.set_status(status.into());
                }
                _ => unreachable!(),
            },
            Modal::History(page) if !page.entries.is_empty() => {
                self.modal = Some(Modal::Confirm(page))
            }
            Modal::Confirm(page) => {
                if self.conversation.active() || self.document_task.is_some() {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "Wait for the active operation before restoring",
                    ));
                }
                self.save_draft()?;
                self.document_task = Some(Task::restore(
                    self.conversation.store().clone(),
                    self.conversation.settings().fonts.clone(),
                    &page,
                )?);
                self.restore_return = Some(page.clone());
                self.modal = Some(Modal::Loading {
                    message: "Restoring document… · Esc requests cancellation before save",
                });
                self.set_status("Restoring checkpoint…".into());
            }
            Modal::Saved(mut page) => {
                let action = page.accept();
                self.saved_action(page, action)?;
            }
            other => self.modal = Some(other),
        }
        Ok(false)
    }
    pub(super) fn modal_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        worker: &super::preview::Worker,
    ) -> Result<bool> {
        use crossterm::event::KeyCode;
        if key.code == KeyCode::Enter && !matches!(self.modal, Some(Modal::Saved(_))) {
            return self.activate_modal(worker);
        }
        let Some(mut modal) = self.modal.take() else {
            return Ok(false);
        };
        match &mut modal {
            Modal::Saved(_) => {
                let Modal::Saved(page) = modal else {
                    unreachable!()
                };
                return self.saved_key(page, key);
            }
            Modal::Commands { selected } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(COMMANDS.len() - 1)
                }
                _ => {}
            },
            Modal::Loading { .. } => {
                if key.code == KeyCode::Esc {
                    self.saved_return = None;
                    if let Some(task) = &self.document_task {
                        task.cancel.cancel();
                    }
                    self.set_attention_status("Cancelling document operation…".into());
                    return Ok(false);
                }
            }
            Modal::History(page) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
                KeyCode::Up | KeyCode::Char('k') => page.selected = page.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    page.selected = (page.selected + 1).min(page.entries.len().saturating_sub(1))
                }
                KeyCode::Char('r') => {
                    self.open_history(page.cursors.clone())?;
                    return Ok(false);
                }
                KeyCode::Home => {
                    self.open_history(vec![None])?;
                    return Ok(false);
                }
                KeyCode::PageDown | KeyCode::Char('n') => {
                    if let Some(before) = page.next_before {
                        let mut cursors = page.cursors.clone();
                        cursors.push(Some(before));
                        self.open_history(cursors)?;
                        return Ok(false);
                    }
                }
                KeyCode::PageUp | KeyCode::Char('p') if page.cursors.len() > 1 => {
                    let mut cursors = page.cursors.clone();
                    cursors.pop();
                    self.open_history(cursors)?;
                    return Ok(false);
                }
                _ => {}
            },
            Modal::Confirm(page) => match key.code {
                KeyCode::Esc | KeyCode::Backspace => {
                    self.modal = Some(Modal::History(page.clone()));
                    return Ok(false);
                }
                _ => {}
            },
        }
        self.modal = Some(modal);
        Ok(false)
    }
    pub(super) fn draw_checkpoint_modal(&mut self, frame: &mut ratatui::Frame) {
        use ratatui::{
            layout::{Constraint, Layout},
            style::Style,
            text::Line,
            widgets::{Clear, List, ListItem, Paragraph, Wrap},
        };
        let Some(modal) = &mut self.modal else {
            return;
        };
        self.pointer.overlay();
        let popup = super::centered(frame.area(), 100, 26);
        frame.render_widget(Clear, popup);
        let highlight = super::theme::modal_selected();
        match modal {
            Modal::Saved(page) => super::saved::draw(page, frame, popup, &mut self.pointer),
            Modal::Commands { selected } => {
                let items = COMMANDS.iter().map(|text| ListItem::new(*text));
                let block = super::dialog("Commands");
                let inner = block.inner(popup);
                let sections =
                    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
                frame.render_widget(block, popup);
                self.pointer.modal_list(
                    frame,
                    List::new(items)
                        .highlight_style(highlight)
                        .highlight_symbol("› "),
                    sections[0],
                    *selected,
                    super::mouse::ListTarget::Commands,
                );
                frame.render_widget(
                    Paragraph::new("↑/↓ select · Enter run · Esc closes")
                        .style(Style::default().fg(super::theme::SECONDARY)),
                    sections[1],
                );
            }
            Modal::Loading { message } => frame.render_widget(
                Paragraph::new(*message)
                    .style(Style::default().fg(super::theme::SECONDARY))
                    .block(super::dialog("Document operation"))
                    .wrap(Wrap { trim: false }),
                popup,
            ),
            Modal::History(page) => {
                let block = super::dialog("Checkpoints");
                let inner = block.inner(popup);
                frame.render_widget(block, popup);
                let header_height = if page.notice.is_some() { 5 } else { 2 };
                let footer = Paragraph::new(format!(
                    "Page {} · older checkpoints {}\nEnter review · r reload · PgDn/n older · PgUp/p newer · Home latest\nRestoration creates a new history entry and preserves the draft.",
                    page.cursors.len(),
                    if page.next_before.is_some() { "available" } else { "unavailable" },
                ))
                .style(Style::default().fg(super::theme::SECONDARY))
                .wrap(Wrap { trim: false });
                let footer_height = footer
                    .line_count(inner.width)
                    .min(usize::from(inner.height.saturating_sub(header_height + 1)))
                    as u16;
                let sections = Layout::vertical([
                    Constraint::Length(header_height),
                    Constraint::Min(1),
                    Constraint::Length(footer_height),
                ])
                .split(inner);
                let mut lines = vec![Line::styled(
                    format!(
                        "Current document: {} · dependencies: {}",
                        short(&page.revision),
                        short(&page.dependency_revision)
                    ),
                    Style::default().fg(super::theme::SECONDARY),
                )];
                if let Some(notice) = &page.notice {
                    lines.push(Line::styled(
                        notice.clone(),
                        Style::default().fg(super::theme::RUNNING),
                    ));
                }
                if page.entries.is_empty() {
                    lines.push(Line::styled(
                        "No committed checkpoints yet",
                        Style::default().fg(super::theme::SECONDARY),
                    ));
                }
                frame.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }),
                    sections[0],
                );
                let items = page.entries.iter().map(|entry| {
                    ListItem::new(super::clean(&format!(
                        "#{} · {} · {} · {}",
                        entry.sequence,
                        entry.created_at,
                        short(&entry.revision),
                        entry.request_id
                    )))
                });
                self.pointer.modal_list(
                    frame,
                    List::new(items)
                        .highlight_style(highlight)
                        .highlight_symbol("› "),
                    sections[1],
                    page.selected,
                    super::mouse::ListTarget::Checkpoints,
                );
                frame.render_widget(footer, sections[2]);
            }
            Modal::Confirm(page) => {
                let entry = &page.entries[page.selected];
                let text = super::clean(&format!(
                    "Restore selected checkpoint?\n\nCheckpoint #{}\n{}\nCreated: {}\nRequest: {}\n\nCurrent revision: {}\nTarget revision: {}\n\nThis restores the document using current referenced assets.\nIt creates a new history entry and preserves the draft and attachments.\nConcurrent document or dependency changes stop the restoration.\n\nEnter restores · Esc returns to the checkpoint list",
                    entry.sequence,
                    entry.id,
                    entry.created_at,
                    entry.request_id,
                    page.revision,
                    entry.revision
                ));
                frame.render_widget(
                    Paragraph::new(text)
                        .block(super::dialog("Review restoration"))
                        .wrap(Wrap { trim: false }),
                    popup,
                );
            }
        }
        self.pointer.button(
            frame,
            super::Rect::new(popup.right().saturating_sub(9), popup.y, 8, 1),
            "[Close]",
            super::mouse::Action::Close,
        );
        let label = match self.modal {
            Some(Modal::Commands { .. }) => Some("[Run]"),
            Some(Modal::History(_)) => Some("[Review]"),
            Some(Modal::Confirm(_)) => Some("[Restore]"),
            _ => None,
        };
        if let Some(label) = label {
            self.pointer.button(
                frame,
                super::Rect::new(
                    popup.right().saturating_sub(12),
                    popup.bottom().saturating_sub(1),
                    11,
                    1,
                ),
                label,
                super::mouse::Action::Accept,
            );
        }
    }
}
fn short(text: &str) -> String {
    text.chars().take(12).collect()
}
