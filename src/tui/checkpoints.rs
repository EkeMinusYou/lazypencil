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
}

pub struct Task {
    pub receiver: oneshot::Receiver<Result<Output>>,
    pub cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}
impl Task {
    fn spawn(
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
            let mut project = Project::load(&store)?;
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
];

impl super::App {
    pub(super) fn has_modal(&self) -> bool {
        self.help || self.modal.is_some() || !self.permissions.is_empty()
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
            Ok(Output::History(page)) => {
                self.modal = Some(Modal::History(page));
                self.status = "Select a checkpoint; Enter reviews the restoration".into();
            }
            Ok(Output::Restored(value)) => {
                self.modal = None;
                self.restore_return = None;
                self.nodes.select(None);
                self.view.zoom = None;
                self.status = format!(
                    "Restored checkpoint · revision {}",
                    value["revision"].as_str().unwrap_or("unknown")
                );
                self.append(&format!("\nCheckpoint restored: {value}\n"));
                self.request(worker, false, false);
            }
            Err(error) => {
                if let Some(mut page) = self.restore_return.take() {
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
    pub(super) fn modal_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        worker: &super::preview::Worker,
    ) -> Result<bool> {
        use crossterm::event::KeyCode;
        let Some(mut modal) = self.modal.take() else {
            return Ok(false);
        };
        match &mut modal {
            Modal::Commands { selected } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(COMMANDS.len() - 1)
                }
                KeyCode::Enter => {
                    match *selected {
                        0 => self.open_history(vec![None])?,
                        1 => self.request(worker, false, true),
                        2 => self.open_preview()?,
                        3 => self.show_help(),
                        4 => return Ok(true),
                        _ => unreachable!(),
                    }
                    return Ok(false);
                }
                _ => {}
            },
            Modal::Loading { .. } => {
                if key.code == KeyCode::Esc {
                    if let Some(task) = &self.document_task {
                        task.cancel.cancel();
                    }
                    self.status = "Cancelling document operation…".into();
                    return Ok(false);
                }
            }
            Modal::History(page) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
                KeyCode::Up | KeyCode::Char('k') => page.selected = page.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    page.selected = (page.selected + 1).min(page.entries.len().saturating_sub(1))
                }
                KeyCode::Enter if !page.entries.is_empty() => {
                    self.modal = Some(Modal::Confirm(page.clone()));
                    return Ok(false);
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
                KeyCode::Enter => {
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
                        page,
                    )?);
                    self.restore_return = Some(page.clone());
                    self.modal = Some(Modal::Loading {
                        message: "Restoring document… · Esc requests cancellation before save",
                    });
                    self.status = "Restoring checkpoint…".into();
                    return Ok(false);
                }
                _ => {}
            },
        }
        self.modal = Some(modal);
        Ok(false)
    }
    pub(super) fn draw_checkpoint_modal(&self, frame: &mut ratatui::Frame) {
        use ratatui::{
            layout::{Constraint, Layout},
            style::{Color, Style},
            text::Line,
            widgets::{Clear, List, ListItem, ListState, Paragraph, Wrap},
        };
        let Some(modal) = &self.modal else {
            return;
        };
        let popup = super::centered(frame.area(), 100, 26);
        frame.render_widget(Clear, popup);
        let highlight = Style::default().fg(Color::Black).bg(Color::Cyan);
        match modal {
            Modal::Commands { selected } => {
                let items = COMMANDS.iter().map(|text| ListItem::new(*text));
                let mut selection = ListState::default().with_selected(Some(*selected));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(super::panel(
                            "Commands · Up/Down · Enter · Esc closes",
                            true,
                        ))
                        .highlight_style(highlight)
                        .highlight_symbol("› "),
                    popup,
                    &mut selection,
                );
            }
            Modal::Loading { message } => frame.render_widget(
                Paragraph::new(*message)
                    .block(super::panel("Document operation", true))
                    .wrap(Wrap { trim: false }),
                popup,
            ),
            Modal::History(page) => {
                let block =
                    super::panel("Checkpoints · Enter reviews · r reloads · Esc closes", true);
                let inner = block.inner(popup);
                frame.render_widget(block, popup);
                let sections = Layout::vertical([
                    Constraint::Length(if page.notice.is_some() { 5 } else { 2 }),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .split(inner);
                let mut lines = vec![Line::from(format!(
                    "Current document: {} · dependencies: {}",
                    short(&page.revision),
                    short(&page.dependency_revision)
                ))];
                if let Some(notice) = &page.notice {
                    lines.push(Line::styled(
                        notice.clone(),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if page.entries.is_empty() {
                    lines.push(Line::from("No committed checkpoints yet"));
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
                let mut selection = ListState::default()
                    .with_selected((!page.entries.is_empty()).then_some(page.selected));
                frame.render_stateful_widget(
                    List::new(items)
                        .highlight_style(highlight)
                        .highlight_symbol("› "),
                    sections[1],
                    &mut selection,
                );
                frame.render_widget(Paragraph::new(format!("Page {} · PgDn/n older{} · PgUp/p newer · Home latest\nRestoration creates a new history entry and preserves the draft.", page.cursors.len(), if page.next_before.is_some() { " available" } else { ": none" })).wrap(Wrap { trim: false }), sections[2]);
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
                        .block(super::panel("Review restoration", true))
                        .wrap(Wrap { trim: false }),
                    popup,
                );
            }
        }
    }
}
fn short(text: &str) -> String {
    text.chars().take(12).collect()
}
