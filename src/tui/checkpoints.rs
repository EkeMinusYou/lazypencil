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
    pub review_scroll: u16,
    pub review_maximum_scroll: u16,
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
            let page = store.history_page_with_cancellation(
                PAGE_SIZE,
                cursors.last().copied().flatten(),
                Some(cancel),
            )?;
            crate::store::require_revision(&store.read()?, &revision)?;
            project.verify_unchanged()?;
            crate::cancel::check(Some(cancel))?;
            let next_before = page.next_before_sequence;
            let entries = page
                .entries
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
                review_scroll: 0,
                review_maximum_scroll: 0,
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
    CommandInput(Box<super::commands::Input>),
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

impl super::App {
    pub(super) fn has_modal(&self) -> bool {
        self.has_overlay() || self.preview_search.editing
    }
    pub(super) fn has_overlay(&self) -> bool {
        self.help
            || self.modal.is_some()
            || self.permission_ui.is_some()
            || !self.permissions.is_empty()
            || self.pointer.more
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
            Modal::Commands { selected } => {
                let command = &super::commands::COMMANDS[selected];
                if let Some(reason) = self.command_unavailable(command) {
                    self.modal = Some(Modal::Commands { selected });
                    self.set_attention_status(reason.into());
                } else if !command.arguments.is_empty() {
                    self.modal = Some(Modal::CommandInput(Box::new(super::commands::Input::new(
                        command.action,
                    ))));
                } else {
                    match self.execute_command(command.action, &[], worker) {
                        Ok(exit) => return Ok(exit),
                        Err(error) => {
                            self.modal = Some(Modal::Commands { selected });
                            return Err(error);
                        }
                    }
                }
            }
            Modal::CommandInput(mut input) => {
                let args = input
                    .action
                    .command()
                    .parse_arguments(&input.editor.lines().join("\n"));
                match args.and_then(|args| self.execute_command(input.action, &args, worker)) {
                    Ok(exit) => return Ok(exit),
                    Err(error) => {
                        input.error = Some(super::clean(&error.to_string()));
                        self.modal = Some(Modal::CommandInput(input));
                    }
                }
            }
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
        if key.code == KeyCode::Enter
            && key.modifiers.is_empty()
            && !matches!(self.modal, Some(Modal::Saved(_)))
        {
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
            Modal::CommandInput(input) => {
                if key.code == KeyCode::Esc {
                    let selected = super::commands::COMMANDS
                        .iter()
                        .position(|command| command.action == input.action)
                        .unwrap();
                    self.modal = Some(Modal::Commands { selected });
                    return Ok(false);
                }
                if key.code == KeyCode::Enter {
                    if key.modifiers.intersects(
                        crossterm::event::KeyModifiers::ALT | crossterm::event::KeyModifiers::SHIFT,
                    ) {
                        input.insert("\n");
                    }
                } else {
                    input.editor.input(key);
                    input.error = None;
                }
            }
            Modal::Commands { selected } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Ok(false),
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    *selected = (*selected + 1).min(super::commands::COMMANDS.len() - 1)
                }
                KeyCode::Home => *selected = 0,
                KeyCode::End => *selected = super::commands::COMMANDS.len() - 1,
                KeyCode::PageDown => {
                    *selected = (*selected + 8).min(super::commands::COMMANDS.len() - 1)
                }
                KeyCode::PageUp => *selected = selected.saturating_sub(8),
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
                KeyCode::Up | KeyCode::Char('k') => {
                    page.review_scroll = page.review_scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    page.review_scroll = page
                        .review_scroll
                        .saturating_add(1)
                        .min(page.review_maximum_scroll)
                }
                KeyCode::PageUp => page.review_scroll = page.review_scroll.saturating_sub(5),
                KeyCode::PageDown => {
                    page.review_scroll = page
                        .review_scroll
                        .saturating_add(5)
                        .min(page.review_maximum_scroll)
                }
                KeyCode::Home => page.review_scroll = 0,
                KeyCode::End => page.review_scroll = page.review_maximum_scroll,
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
        use super::{modal, mouse, theme};
        use ratatui::{
            layout::{Constraint, Layout},
            style::{Modifier, Style},
            text::Line,
            widgets::{List, ListItem, Paragraph, Wrap},
        };
        if matches!(self.modal, Some(Modal::Commands { .. })) {
            self.draw_command_palette(frame);
            return;
        }
        let Some(modal) = &mut self.modal else {
            return;
        };
        self.pointer.overlay();
        let popup = modal::area(frame.area());
        match modal {
            Modal::Saved(page) => {
                super::saved::draw(page, frame, popup, &mut self.pointer);
                return;
            }
            Modal::CommandInput(input) => {
                input.draw(frame, popup, self.permissions.is_empty(), &mut self.pointer);
                return;
            }
            _ => {}
        }
        let title = match modal {
            Modal::Loading { .. } => "Document operation",
            Modal::History(_) => "Checkpoints",
            Modal::Confirm(_) => "Review restoration",
            _ => unreachable!(),
        };
        let shell = modal::Shell::draw(frame, popup, title, 2);
        shell.close(frame, &mut self.pointer);
        let colors = theme::colors();
        match modal {
            Modal::Loading { message } => {
                frame.render_widget(
                    Paragraph::new(*message)
                        .style(Style::default().fg(colors.secondary))
                        .wrap(Wrap { trim: false }),
                    shell.body,
                );
                frame.render_widget(
                    Paragraph::new(modal::hints(&[("Esc", "Cancel")])),
                    shell.footer,
                );
            }
            Modal::History(page) => {
                let mut lines = vec![Line::styled(
                    format!(
                        "Current document: {} · dependencies: {}",
                        short(&page.revision),
                        short(&page.dependency_revision)
                    ),
                    Style::default().fg(colors.secondary),
                )];
                lines.push(Line::styled(
                    format!(
                        "Page {} · older checkpoints {}",
                        page.cursors.len(),
                        if page.next_before.is_some() {
                            "available"
                        } else {
                            "unavailable"
                        }
                    ),
                    Style::default().fg(colors.secondary),
                ));
                if let Some(notice) = &page.notice {
                    lines.push(Line::styled(
                        notice.clone(),
                        Style::default().fg(colors.running),
                    ));
                }
                let header = Paragraph::new(lines).wrap(Wrap { trim: false });
                let header_height = header
                    .line_count(shell.body.width)
                    .min(usize::from(shell.body.height.saturating_sub(3)))
                    as u16;
                let sections = Layout::vertical([
                    Constraint::Length(header_height),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ])
                .split(shell.body);
                frame.render_widget(header, sections[0]);
                if page.entries.is_empty() {
                    frame.render_widget(
                        Paragraph::new("No committed checkpoints yet")
                            .style(Style::default().fg(colors.secondary)),
                        sections[2],
                    );
                } else {
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
                            .highlight_style(theme::modal_selected())
                            .highlight_symbol("▏› "),
                        sections[2],
                        page.selected,
                        mouse::ListTarget::Checkpoints,
                    );
                    modal::action(frame, shell.footer, "[Enter Review]", &mut self.pointer);
                }
                let hints = super::Rect::new(
                    shell.footer.x,
                    shell.footer.y,
                    shell.footer.width.saturating_sub(16),
                    shell.footer.height,
                );
                frame.render_widget(
                    Paragraph::new(vec![
                        modal::hints(&[("↑ ↓", "Select"), ("r", "Reload"), ("Esc", "Close")]),
                        modal::hints(&[("PgDn/PgUp", "Pages"), ("Home", "Latest")]),
                    ]),
                    hints,
                );
            }
            Modal::Confirm(page) => {
                let entry = &page.entries[page.selected];
                let body = modal::detail_surface(frame, shell.body);
                let text = vec![
                    Line::styled(
                        format!("Restore checkpoint #{}?", entry.sequence),
                        Style::default()
                            .fg(colors.running)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Line::default(),
                    Line::raw(super::clean(&format!("Checkpoint: {}", entry.id))),
                    Line::raw(super::clean(&format!("Created: {}", entry.created_at))),
                    Line::raw(super::clean(&format!("Request: {}", entry.request_id))),
                    Line::raw(super::clean(&format!("Current: {}", page.revision))),
                    Line::raw(super::clean(&format!("Target:  {}", entry.revision))),
                    Line::default(),
                    Line::raw("Uses current referenced assets and creates a new history entry."),
                    Line::raw("Your draft and attachments are preserved."),
                    Line::styled(
                        "Concurrent document or dependency changes stop restoration.",
                        Style::default().fg(colors.secondary),
                    ),
                ];
                let text = Paragraph::new(text).wrap(Wrap { trim: false });
                page.review_maximum_scroll = text
                    .line_count(body.width)
                    .saturating_sub(usize::from(body.height))
                    .min(u16::MAX as usize) as u16;
                page.review_scroll = page.review_scroll.min(page.review_maximum_scroll);
                frame.render_widget(text.scroll((page.review_scroll, 0)), body);
                self.pointer.add(body, mouse::Target::CheckpointReview);
                frame.render_widget(
                    Paragraph::new(vec![
                        modal::hints(&[("Esc", "Back to checkpoints")]),
                        modal::hints(&[("PgUp/PgDn", "Scroll review")]),
                    ]),
                    shell.footer,
                );
                modal::action(frame, shell.footer, "[Enter Restore]", &mut self.pointer);
            }
            _ => unreachable!(),
        }
    }
}
fn short(text: &str) -> String {
    text.chars().take(12).collect()
}
