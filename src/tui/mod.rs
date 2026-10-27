//! Terminal viewing and prompting, sharing the same document and ACP services as CLI.

mod camera;
mod checkpoints;
mod graphics;
mod playback;
mod preview;
mod saved;
mod terminal;
#[cfg(test)]
mod tests;

use crate::config::keys::NavigationAction as Nav;
use crate::{
    Error, Result,
    clipboard::{self, InputContent},
    conversation::{Conversation, ConversationEvent},
    prompt::PromptArgs,
};
use agent_client_protocol::schema::v1::RequestPermissionRequest;
use clap::{Args, ValueEnum};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use preview::{Preview, Request, View, Worker};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_textarea::{CursorMove, TextArea};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImageProtocol {
    Auto,
    Kitty,
    Sixel,
    Halfblocks,
}

#[derive(Debug, Args)]
pub struct TuiArgs {
    #[command(flatten)]
    pub prompt: PromptArgs,
    /// Select the image protocol; auto probes the terminal before reading input.
    #[arg(long, value_enum, default_value = "auto")]
    pub image_protocol: ImageProtocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Prompt,
    Preview,
    Nodes,
    Attachments,
    Conversation,
}
impl Focus {
    fn next(self, backwards: bool) -> Self {
        let order = [
            Self::Prompt,
            Self::Preview,
            Self::Nodes,
            Self::Attachments,
            Self::Conversation,
        ];
        let index = order.iter().position(|item| *item == self).unwrap();
        order[(index + if backwards { 4 } else { 1 }) % 5]
    }
}
struct Permission {
    id: String,
    request: Box<RequestPermissionRequest>,
    selected: usize,
    scroll: u16,
}
struct InputJob {
    cancel: CancellationToken,
    receiver: tokio::sync::oneshot::Receiver<Result<InputContent>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for InputJob {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn run(args: &TuiArgs, data: Option<PathBuf>, json_output: bool) -> Result<()> {
    if json_output {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "TUI does not use --json; use prompt for NDJSON events",
        ));
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "TUI requires a terminal on stdin and stdout",
        ));
    }
    let settings = crate::config::keys::load_tui()?;
    let mut conversation = Conversation::open(args.prompt.clone(), data)?;
    for image in &args.prompt.images {
        conversation.attach_file(image)?;
    }
    // Own a pair of image IDs: exact frames use the low half, temporary camera
    // feedback uses the corresponding high bit. Both are released on exit.
    let id = (uuid::Uuid::new_v4().as_u128() as u32 & 0x7fff_ffff).max(1);
    let mut screen = terminal::Screen::enter(args.image_protocol, id)?;
    let worker = Worker::start(
        conversation.store().clone(),
        args.prompt.fonts.clone(),
        screen.picker.clone(),
        id,
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(terminal::io)?;
    let (result, session_id) = runtime.block_on(async {
        let mut app = App::new(conversation);
        app.keys = settings.keys;
        app.navigation = settings.navigation;
        app.status = format!(
            "Session {} · {} shows shortcuts",
            app.conversation.id(),
            app.navigation.label(Nav::Help)
        );
        if !screen.startup_text.is_empty() {
            app.insert_text(&screen.startup_text);
        }
        let result = app.event_loop(&mut screen, &worker).await;
        // Request cancellation before awaiting an adapter's potentially slow shutdown.
        if let Some(task) = &app.document_task {
            task.cancel.cancel();
        }
        if let Some(input) = &app.input {
            input.cancel.cancel();
        }
        worker.cancel();
        let session_id = app.conversation.id().to_string();
        let closed = app.conversation.close().await;
        drop(app.document_task.take());
        drop(app.input.take());
        (result.and(closed), session_id)
    });
    drop(worker);
    drop(screen);
    runtime.shutdown_timeout(Duration::from_secs(1));
    eprintln!("Conversation: {session_id}");
    result
}

struct App {
    conversation: Conversation,
    keys: crate::config::keys::PromptKeys,
    navigation: crate::config::keys::NavigationKeys,
    editor: TextArea<'static>,
    focus: Focus,
    view: View,
    playback: playback::Playback,
    preview: Option<Preview>,
    camera: Option<camera::Camera>,
    camera_id: u32,
    generation: u64,
    cells: Size,
    nodes: ListState,
    attachments: ListState,
    filter: String,
    searching: bool,
    log: String,
    scroll: u16,
    status: String,
    protocol: String,
    authentication: String,
    permissions: VecDeque<Permission>,
    input: Option<InputJob>,
    document_task: Option<checkpoints::Task>,
    modal: Option<checkpoints::Modal>,
    restore_return: Option<checkpoints::Page>,
    saved_return: Option<Box<saved::Page>>,
    pending_session: Option<saved::SessionChange>,
    help: bool,
    help_scroll: u16,
    dirty: bool,
    draft_changed: bool,
    turn_pending: bool,
    last_draft_save: std::time::Instant,
    input_reader: fn(&Path, Option<&Path>, &CancellationToken) -> Result<InputContent>,
}
impl App {
    fn new(conversation: Conversation) -> Self {
        let editor = editor(conversation.draft());
        let status = format!("Session {} · F1 shows shortcuts", conversation.id());
        Self {
            conversation,
            keys: Default::default(),
            navigation: Default::default(),
            editor,
            focus: Focus::Prompt,
            view: View::default(),
            playback: playback::Playback::default(),
            preview: None,
            camera: None,
            camera_id: 0x8000_0001,
            generation: 0,
            cells: Size::new(0, 0),
            nodes: ListState::default(),
            attachments: ListState::default(),
            filter: String::new(),
            searching: false,
            log: String::new(),
            scroll: 0,
            status,
            protocol: "Halfblocks".into(),
            authentication: "unknown".into(),
            permissions: VecDeque::new(),
            input: None,
            document_task: None,
            modal: None,
            restore_return: None,
            saved_return: None,
            pending_session: None,
            help: false,
            help_scroll: 0,
            dirty: true,
            draft_changed: false,
            turn_pending: false,
            last_draft_save: std::time::Instant::now(),
            input_reader: read_input,
        }
    }
    async fn event_loop(&mut self, screen: &mut terminal::Screen, worker: &Worker) -> Result<()> {
        let mut events = EventStream::new();
        self.protocol = format!("{:?}", screen.picker.protocol_type());
        self.camera_id = screen.camera_image_id();
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut termination = Box::pin(termination());
        loop {
            if self.dirty {
                let mut cells = self.cells;
                screen
                    .terminal
                    .draw(|frame| {
                        cells = self.draw(frame);
                    })
                    .map_err(terminal::io)?;
                self.dirty = false;
                if cells != self.cells {
                    self.cells = cells;
                    self.request(worker, false, false);
                }
            }
            let playback_deadline = self.playback.deadline();
            tokio::select! {
                signal = tokio::signal::ctrl_c() => {
                    signal.map_err(terminal::io)?;
                    if let Some(task) = &self.document_task {
                        task.cancel.cancel();
                        if self.conversation.active() { self.conversation.cancel(); self.permissions.clear(); }
                    }
                    else if self.conversation.active() { self.conversation.cancel(); self.permissions.clear(); }
                    else if self.playback.active() { self.pause_playback(); self.dirty = true; }
                    else { self.save_draft()?; return Ok(()); }
                },
                signal = &mut termination => { signal.map_err(terminal::io)?; self.save_draft()?; return Ok(()); },
                event = events.next() => match event {
                    Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                        let modal = self.has_modal();
                        let camera = self.camera.as_ref().map(|camera| camera.generation);
                        match self.key(key, &screen.picker, worker).await {
                            Ok(true) => { self.save_draft()?; return Ok(()); },
                            Ok(false) => {}, Err(error) => self.error(error),
                        }
                        if modal && !self.has_modal() || camera != self.camera.as_ref().map(|camera| camera.generation) { screen.redraw()?; }
                        self.dirty = true;
                    },
                    Some(Ok(Event::Paste(text))) if self.focus == Focus::Prompt && !self.conversation.active() && !self.has_modal() => {
                        self.insert_text(&text); self.dirty = true;
                    },
                    Some(Ok(Event::Resize(_, _))) => { self.dirty = true; },
                    Some(Ok(_)) => {},
                    Some(Err(error)) => return Err(terminal::io(error)),
                    None => { self.save_draft()?; return Ok(()); },
                },
                output = worker.next() => {
                    let current = output.generation == self.generation;
                    self.accept_preview(output);
                    // Replacing a Kitty image ID removes its placements. Resend
                    // every placeholder instead of diffing unchanged image cells.
                    if current { screen.redraw()?; }
                },
                _ = async {
                    if let Some(deadline) = playback_deadline {
                        tokio::time::sleep_until(deadline.into()).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    if let Err(error) = self.playback_tick(worker, Instant::now()) {
                        self.playback.stop();
                        self.error(error);
                        self.dirty = true;
                    }
                },
                event = self.conversation.next_event() => {
                    match event? {
                        ConversationEvent::Message(value) => {
                            self.message(&value);
                        },
                        ConversationEvent::Permission { id, request } => {
                            let selected = request.options.iter().position(|option| matches!(option.kind, agent_client_protocol::schema::v1::PermissionOptionKind::RejectOnce)).unwrap_or(0);
                            self.permissions.push_back(Permission { id, request, selected, scroll: 0 });
                        },
                    }
                    self.sync_turn(worker);
                    self.dirty = true;
                },
                content = async { (&mut self.input.as_mut().unwrap().receiver).await.unwrap_or_else(|error| Err(Error::new("CLIPBOARD_UNAVAILABLE", error.to_string()))) }, if self.input.is_some() => {
                    self.input.take();
                    self.accept_input(content);
                    self.dirty = true;
                },
                output = async { (&mut self.document_task.as_mut().unwrap().receiver).await.unwrap_or_else(|error| Err(Error::new("IO_ERROR", format!("Document worker stopped: {error}")))) }, if self.document_task.is_some() => {
                    let modal = self.has_modal();
                    self.finish_document_result(output, worker);
                    if modal && !self.has_modal() { screen.redraw()?; }
                    self.dirty = true;
                },
                _ = timer.tick() => {
                    if self.draft_changed && self.last_draft_save.elapsed() >= Duration::from_secs(1) { self.save_draft()?; }
                    self.request(worker, true, false);
                },
            }
        }
    }
    fn request(&mut self, worker: &Worker, check_only: bool, refresh: bool) {
        if self.cells.width == 0 || self.cells.height == 0 {
            return;
        }
        if !check_only {
            if self.camera.take().is_some() {
                self.status = "Rendering requested preview".into();
            }
            self.generation += 1;
        }
        let attachment = if self.focus == Focus::Attachments {
            self.attachments
                .selected()
                .and_then(|index| self.conversation.images().get(index))
                .map(|image| (image.clone(), self.conversation.directory().to_path_buf()))
        } else {
            None
        };
        worker.request(Request {
            generation: self.generation,
            cells: self.cells,
            view: self.view,
            refresh,
            refresh_pages: false,
            check_only,
            attachment,
        });
    }
    fn accept_preview(&mut self, output: preview::Output) {
        if output.generation != self.generation {
            return;
        }
        self.camera = None;
        match output.result {
            Ok(mut preview) => {
                if !preview.attachment {
                    self.view = preview.view;
                    if !preview.time_dependent && !preview.live_pages {
                        self.playback.stop();
                    }
                }
                self.status = if preview.attachment {
                    "Attachment preview".into()
                } else {
                    let state = self
                        .playback
                        .fps()
                        .map_or_else(String::new, |fps| format!(" · playing ≤{fps}fps"));
                    format!(
                        "{} nodes · {} diagnostics · {:.0}% · t={:.3}s{state}",
                        preview.nodes.len(),
                        preview.summary["diagnostics"]
                            .as_array()
                            .map_or(0, Vec::len),
                        self.view.zoom.unwrap_or(1.0) * 100.0,
                        self.view.time
                    )
                };
                // Preserve the document node list while browsing an attachment.
                if preview.attachment
                    && let Some(previous) = &mut self.preview
                {
                    preview.nodes = std::mem::take(&mut previous.nodes);
                }
                self.preview = Some(preview);
            }
            Err(error) => {
                self.playback.stop();
                self.preview = None;
                self.error(error);
            }
        }
        self.dirty = true;
    }
    fn start_playback(&mut self, fps: u32, now: Instant) -> Result<()> {
        if !self.preview.as_ref().is_some_and(|preview| {
            !preview.attachment && (preview.time_dependent || preview.live_pages)
        }) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The complete preview must contain a browser page or a shader with an @time uniform before playback can start",
            ));
        }
        self.playback.start(self.view.time, fps, now)?;
        self.status = format!("Playing dynamic preview · ≤{fps}fps · Space in Preview pauses");
        Ok(())
    }
    fn pause_playback(&mut self) {
        self.playback.stop();
        self.status = match self.preview.as_ref().filter(|preview| !preview.attachment) {
            Some(preview) => {
                let pending = if preview.view.time != self.view.time {
                    " · finishing requested frame"
                } else {
                    ""
                };
                format!("Playback paused · t={:.3}s{pending}", preview.view.time)
            }
            None => "Playback paused".into(),
        };
    }
    fn playback_tick(&mut self, worker: &Worker, now: Instant) -> Result<()> {
        let Some(time) = self.playback.frame(now) else {
            return Ok(());
        };
        if self.has_modal()
            || self.focus == Focus::Attachments
            || self.cells.width == 0
            || self.cells.height == 0
        {
            return Ok(());
        }
        if !self.preview.as_ref().is_some_and(|preview| {
            !preview.attachment && (preview.time_dependent || preview.live_pages)
        }) {
            self.playback.stop();
            return Ok(());
        }
        let refresh_pages = self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.live_pages);
        let mut view = self.view;
        view.set_time(time)?;
        let generation = self.generation + 1;
        if worker.request_if_idle(Request {
            generation,
            cells: self.cells,
            view,
            refresh: false,
            refresh_pages,
            check_only: false,
            attachment: None,
        }) {
            self.generation = generation;
            self.view = view;
        }
        Ok(())
    }
    fn save_draft(&mut self) -> Result<()> {
        if self.draft_changed && !self.conversation.active() {
            self.conversation
                .set_draft(self.editor.lines().join("\n"))?;
            self.draft_changed = false;
            self.last_draft_save = std::time::Instant::now();
        }
        Ok(())
    }
    fn sync_turn(&mut self, worker: &Worker) {
        // Completion is recorded before the final frontend message is dequeued.
        // Clear once on the actual state transition, preserving any next draft.
        if self.turn_pending && !self.conversation.active() {
            self.permissions.clear();
            self.editor = editor(self.conversation.draft());
            self.draft_changed = false;
            self.turn_pending = false;
            self.request(worker, false, false);
        }
    }
    fn insert_text(&mut self, text: &str) {
        let text = clean(text);
        if self.editor.lines().iter().map(String::len).sum::<usize>()
            + self.editor.lines().len()
            + text.len()
            > 1024 * 1024
        {
            self.status = "Prompt exceeds 1 MiB".into();
            return;
        }
        self.editor.insert_str(text);
        self.draft_changed = true;
    }
    fn start_input(&mut self, path: Option<PathBuf>) -> Result<()> {
        if self.conversation.active() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the active turn or cancel it before attaching images",
            ));
        }
        if self.input.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Image input is already running",
            ));
        }
        let executable = std::env::current_exe().map_err(terminal::io)?;
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let reader = self.input_reader;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("lazypencil-input".into())
            .spawn(move || {
                let result = reader(&executable, path.as_deref(), &token);
                let _ = sender.send(result);
            })
            .map_err(terminal::io)?;
        self.input = Some(InputJob {
            cancel,
            receiver,
            thread: Some(thread),
        });
        self.status = "Reading image input…".into();
        Ok(())
    }
    fn accept_input(&mut self, content: Result<InputContent>) {
        match content {
            Ok(InputContent::Text { text }) => {
                self.insert_text(&text);
                self.status = "Pasted text".into();
            }
            Ok(InputContent::Images { images }) => {
                let images = images
                    .into_iter()
                    .map(|image| image.store(self.conversation.directory()))
                    .collect::<Result<Vec<_>>>();
                if let Err(error) = images.and_then(|images| self.conversation.add_images(images)) {
                    self.error(error);
                    return;
                }
                self.status = format!(
                    "{} images attached · Tab to Attachments to review or remove",
                    self.conversation.images().len()
                );
            }
            Err(error) => self.error(error),
        }
    }
    fn error(&mut self, error: Error) {
        self.status = clean(&error.to_string());
        self.append(&format!("\n{}\n", error));
    }
    fn append(&mut self, text: &str) {
        self.log.push_str(&clean(text));
        if self.log.len() > 256 * 1024 {
            let mut index = self.log.len() - 192 * 1024;
            while !self.log.is_char_boundary(index) {
                index += 1;
            }
            self.log.drain(..index);
        }
    }
    fn message(&mut self, value: &Value) {
        match value["type"].as_str() {
            Some("update") => {
                let update = &value["notification"]["update"];
                match update["sessionUpdate"].as_str() {
                    Some("agent_message_chunk") => {
                        if let Some(text) = update["content"]["text"].as_str() {
                            self.append(text);
                        }
                    }
                    Some("tool_call" | "tool_call_update") => {
                        let title = update["title"].as_str().unwrap_or("Tool");
                        let status = update["status"].as_str().unwrap_or("running");
                        self.append(&format!("\n[{status}] {title}\n"));
                    }
                    _ => {}
                }
            }
            Some("auth_status") => {
                self.authentication = value["authentication"]["source"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                self.status = format!(
                    "Authentication: {}",
                    value["authentication"]["source"]
                        .as_str()
                        .unwrap_or("unknown")
                );
            }
            Some("ready") => {
                self.status = "Agent ready".into();
            }
            Some("session_restarted") => {
                self.append("\nStarted a new native session with current document context and bounded local history. Previous model memory was not restored.\n");
            }
            Some("local_context") => {
                self.append("\nAdded current document context and bounded local history after an interrupted restart.\n");
            }
            Some("failed") => {
                self.append(&format!(
                    "\nRequest failed: {}\n",
                    value["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown error")
                ));
                self.status = "Request failed; draft and attachments retained".into();
            }
            Some("complete") => {
                self.append("\nRequest complete\n");
            }
            _ => {}
        }
    }
    async fn key(
        &mut self,
        key: KeyEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        if key.kind == KeyEventKind::Release {
            return Ok(false);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if key.code == KeyCode::Char('c') && ctrl {
            if let Some(task) = &self.document_task {
                task.cancel.cancel();
                if self.conversation.active() {
                    self.conversation.cancel();
                    self.permissions.clear();
                    self.status = "Cancelling saved-log read and active request…".into();
                } else {
                    self.status = "Cancelling document operation…".into();
                }
            } else if let Some(input) = self.input.take() {
                input.cancel.cancel();
                self.status = "Image input cancelled".into();
            } else if self.conversation.active() {
                self.conversation.cancel();
                self.permissions.clear();
                self.status = "Cancelling request; committed edits are retained".into();
            } else if self.playback.active() {
                self.pause_playback();
            } else {
                return Ok(true);
            }
            return Ok(false);
        }
        if let Some(permission) = self.permissions.front_mut() {
            let count = permission.request.options.len();
            let choice = match key.code {
                KeyCode::PageUp => {
                    permission.scroll = permission.scroll.saturating_sub(5);
                    None
                }
                KeyCode::PageDown => {
                    permission.scroll = permission.scroll.saturating_add(5);
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    permission.selected = permission.selected.saturating_sub(1);
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    permission.selected = (permission.selected + 1).min(count.saturating_sub(1));
                    None
                }
                KeyCode::Enter => Some(
                    permission
                        .request
                        .options
                        .get(permission.selected)
                        .map(|option| option.option_id.to_string()),
                ),
                KeyCode::Esc => Some(crate::conversation::reject_once(&permission.request)),
                _ => None,
            };
            if let Some(choice) = choice {
                let permission = self.permissions.pop_front().unwrap();
                self.conversation.respond(&permission.id, choice, true)?;
            }
            return Ok(false);
        }
        let typing = self.focus == Focus::Prompt || self.searching;
        let global = |action| {
            if typing {
                self.navigation.matches_while_typing(action, key)
            } else {
                self.navigation.matches(action, key)
            }
        };
        if global(Nav::Commands) {
            self.help = false;
            if self.modal.take().is_some() {
                self.saved_return = None;
                if let Some(task) = &self.document_task {
                    task.cancel.cancel();
                }
            } else {
                self.modal = Some(checkpoints::Modal::commands());
            }
            return Ok(false);
        }
        if self.modal.is_some() {
            let exit = self.modal_key(key, worker)?;
            if let Some(change) = self.pending_session.take()
                && let Err(error) = self.select_conversation(change, worker).await
            {
                if let Some(checkpoints::Modal::Saved(page)) = &mut self.modal {
                    page.notice = Some(clean(&error.to_string()));
                }
                return Err(error);
            }
            return Ok(exit);
        }
        if global(Nav::Help) {
            self.help = !self.help;
            self.help_scroll = 0;
            return Ok(false);
        }
        if self.help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.help = false,
                KeyCode::PageUp | KeyCode::Up => {
                    self.help_scroll = self.help_scroll.saturating_sub(5)
                }
                KeyCode::PageDown | KeyCode::Down => {
                    self.help_scroll = self.help_scroll.saturating_add(5)
                }
                KeyCode::Home => self.help_scroll = 0,
                KeyCode::End => self.help_scroll = u16::MAX,
                _ => (),
            }
            return Ok(false);
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) if !ctrl && !alt => self.filter.push(c),
                _ => {}
            }
            self.nodes.select(Some(0));
            return Ok(false);
        }
        if global(Nav::NextPanel) || global(Nav::PreviousPanel) {
            let backwards = global(Nav::PreviousPanel);
            let was_attachment = self.focus == Focus::Attachments;
            self.focus = self.focus.next(backwards);
            if self.focus == Focus::Attachments
                && !self.conversation.images().is_empty()
                && self.attachments.selected().is_none()
            {
                self.attachments.select(Some(0));
            }
            if was_attachment || self.focus == Focus::Attachments {
                self.request(worker, false, false);
            }
            return Ok(false);
        }
        if self.focus == Focus::Prompt {
            if self.conversation.active() {
                return Ok(false);
            }
            if self.keys.pastes(key) {
                self.start_input(None)?;
                return Ok(false);
            }
            if self.keys.submits(key) {
                self.submit().await?;
                return Ok(false);
            }
            if key.code == KeyCode::Enter
                && self.editor.lines().len() == 1
                && self.editor.lines()[0].starts_with('/')
            {
                return self.command(worker).await;
            }
            // Bound interactive input as well as bracketed and clipboard pastes.
            if self.editor.lines().iter().map(String::len).sum::<usize>() >= 1024 * 1024
                && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter)
            {
                return Err(Error::new("INVALID_ARGUMENT", "Prompt exceeds 1 MiB"));
            }
            self.draft_changed |= self.editor.input(key);
            return Ok(false);
        }
        let action = |action| self.navigation.matches(action, key);
        if action(Nav::Quit) {
            return Ok(true);
        }
        if action(Nav::OpenPreview) {
            self.open_preview()?;
            return Ok(false);
        }
        if action(Nav::Refresh) {
            self.request(worker, false, true);
            return Ok(false);
        }
        match self.focus {
            Focus::Preview => {
                if action(Nav::PlayPause) {
                    if self.playback.active() {
                        self.pause_playback();
                    } else {
                        self.start_playback(playback::DEFAULT_FPS, Instant::now())?;
                    }
                    return Ok(false);
                }
                let zoom = self.view.zoom.unwrap_or(1.0);
                if action(Nav::PanLeft) {
                    self.view.pan([-64.0, 0.0]);
                } else if action(Nav::PanRight) {
                    self.view.pan([64.0, 0.0]);
                } else if action(Nav::PanUp) {
                    self.view.pan([0.0, -64.0]);
                } else if action(Nav::PanDown) {
                    self.view.pan([0.0, 64.0]);
                } else if action(Nav::ZoomIn) {
                    self.view.zoom = Some((zoom * 1.25).min(16.0));
                } else if action(Nav::ZoomOut) {
                    self.view.zoom = Some((zoom / 1.25).max(0.01));
                } else if action(Nav::FitDocument) {
                    self.view.zoom = None;
                } else if action(Nav::TimeBackward) {
                    self.view.set_time((self.view.time - 0.1).max(0.0))?;
                    self.playback.stop();
                } else if action(Nav::TimeForward) {
                    self.view.set_time(self.view.time + 0.1)?;
                    self.playback.stop();
                } else if action(Nav::FitSelected) {
                    self.fit_selected(picker)?;
                } else {
                    return Ok(false);
                }
                self.request(worker, false, false);
                self.camera_feedback(picker)?;
            }
            Focus::Nodes => {
                let count = self.filtered_nodes().len();
                if action(Nav::PreviousItem) {
                    self.nodes
                        .select(Some(self.nodes.selected().unwrap_or(0).saturating_sub(1)));
                } else if action(Nav::NextItem) {
                    self.nodes.select(Some(
                        (self.nodes.selected().map_or(0, |n| n + 1)).min(count.saturating_sub(1)),
                    ));
                } else if action(Nav::SearchNodes) {
                    self.searching = true;
                } else if action(Nav::ClearSelection) {
                    self.nodes.select(None);
                } else if action(Nav::ActivateNode) || action(Nav::FitSelected) {
                    self.fit_selected(picker)?;
                    self.focus = Focus::Preview;
                    self.request(worker, false, false);
                    self.camera_feedback(picker)?;
                }
            }
            Focus::Attachments => {
                let count = self.conversation.images().len();
                if action(Nav::PreviousItem) {
                    self.attachments.select(Some(
                        self.attachments.selected().unwrap_or(0).saturating_sub(1),
                    ));
                } else if action(Nav::NextItem) {
                    self.attachments.select(Some(
                        (self.attachments.selected().map_or(0, |n| n + 1))
                            .min(count.saturating_sub(1)),
                    ));
                } else if action(Nav::RemoveAttachment) {
                    if let Some(index) = self.attachments.selected() {
                        self.conversation.remove_image(index)?;
                        self.attachments.select(if count > 1 {
                            Some(index.min(count - 2))
                        } else {
                            None
                        });
                    }
                } else {
                    return Ok(false);
                }
                self.request(worker, false, false);
            }
            Focus::Conversation => {
                if action(Nav::ScrollUp) {
                    self.scroll = self.scroll.saturating_add(5);
                } else if action(Nav::ScrollDown) {
                    self.scroll = self.scroll.saturating_sub(5);
                } else if action(Nav::FollowOutput) {
                    self.scroll = 0;
                }
            }
            Focus::Prompt => {}
        }
        Ok(false)
    }
    fn help_text(&self) -> String {
        let label = |action| self.navigation.label(action);
        let mut text = format!(
            "{} / {}: switch panels\n{}: command palette; /history: checkpoints\nPrompt: Enter inserts a newline; {} sends\n{}: paste image, copied image files, or text\n",
            label(Nav::NextPanel),
            label(Nav::PreviousPanel),
            label(Nav::Commands),
            self.keys.submit_label(),
            self.keys.paste_label()
        );
        text.push_str("/attach <quoted path>: attach PNG, JPEG or WebP\n/paste-image: explicitly read this host's clipboard\n/clear-images: remove pending images\n");
        for (action, description) in [
            (Nav::Help, "show or hide help outside dialogs"),
            (Nav::PanLeft, "Preview: pan left"),
            (Nav::PanRight, "Preview: pan right"),
            (Nav::PanUp, "Preview: pan up"),
            (Nav::PanDown, "Preview: pan down"),
            (Nav::ZoomIn, "Preview: zoom in"),
            (Nav::ZoomOut, "Preview: zoom out"),
            (Nav::FitDocument, "Preview: fit the document"),
            (Nav::FitSelected, "Preview/Nodes: fit selected node"),
            (
                Nav::TimeBackward,
                "Preview: pause and step time back by 0.1s",
            ),
            (
                Nav::TimeForward,
                "Preview: pause and step time forward by 0.1s",
            ),
            (Nav::PlayPause, "Preview: toggle bounded playback"),
            (Nav::PreviousItem, "Nodes/Attachments: previous item"),
            (Nav::NextItem, "Nodes/Attachments: next item"),
            (Nav::SearchNodes, "Nodes: search names and IDs"),
            (Nav::ClearSelection, "Nodes: clear the editing target"),
            (Nav::ActivateNode, "Nodes: select and fit the target"),
            (Nav::RemoveAttachment, "Attachments: remove pending image"),
            (Nav::ScrollUp, "Conversation: scroll back"),
            (Nav::ScrollDown, "Conversation: scroll forward"),
            (Nav::FollowOutput, "Conversation: follow new output"),
            (Nav::Refresh, "refresh preview, including browser captures"),
            (Nav::OpenPreview, "open complete PNG in external viewer"),
            (Nav::Quit, "outside Prompt: close conversation and exit"),
        ] {
            text.push_str(&format!("{}: {description}\n", label(action)));
        }
        text.push_str("\n/open-preview: open the complete PNG externally\n/time <seconds>: seek; /play [1-10 fps], /pause\n/logs: read current saved turns; /conversations: saved conversations\nSaved logs: arrows select/scroll; Enter opens; n/p pages\nu: select local session; N: new local conversation\nv: raw/formatted; Backspace returns; r refreshes; Esc closes\n/commands, /history, /help, /quit\nCtrl+C: cancel active work; exit when idle\nDialogs keep their displayed keys; Esc closes help\nHelp: PgUp/PgDn or arrows scroll; Home/End jump\n\nClipboard access happens only on explicit paste. SSH reads the remote host's clipboard. Image protocols do not provide clipboard access.");
        text
    }

    fn show_help(&mut self) {
        self.help = true;
        self.help_scroll = 0;
    }

    async fn select_conversation(
        &mut self,
        change: saved::SessionChange,
        worker: &Worker,
    ) -> Result<()> {
        if self.input.is_some() || self.document_task.is_some() || self.conversation.active() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Finish or cancel the active operation before selecting a conversation",
            ));
        }
        self.save_draft()?;
        let id = match &change {
            saved::SessionChange::Existing(id) => Some(id.as_str()),
            saved::SessionChange::New => None,
        };
        let changed = self.conversation.select(id).await?;
        self.modal = None;
        self.saved_return = None;
        self.restore_return = None;
        if !changed {
            self.status = "This local conversation is already selected".into();
            return Ok(());
        }
        self.editor = editor(self.conversation.draft());
        self.draft_changed = false;
        self.turn_pending = false;
        self.last_draft_save = Instant::now();
        self.attachments.select(None);
        self.permissions.clear();
        self.authentication = "unknown".into();
        self.log.clear();
        self.scroll = 0;
        self.status = format!(
            "Selected conversation {} · sends only on submit",
            self.conversation.id()
        );
        self.append("\nSelected saved local state. Open saved logs to read earlier turns. Native context resumes on the next submitted request.\n");
        self.request(worker, false, false);
        Ok(())
    }

    async fn submit(&mut self) -> Result<()> {
        if self.document_task.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the document operation before sending",
            ));
        }
        if self.input.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for image input before sending",
            ));
        }
        self.save_draft()?;
        let target = self.selected_node().map(|node| node.target.clone());
        let text = self.conversation.draft().to_string();
        self.conversation.send(target).await?;
        self.turn_pending = true;
        self.append(&format!(
            "\nYou: {text}\n[{} images]\n",
            self.conversation.images().len()
        ));
        self.scroll = 0;
        self.status = "Agent running · Ctrl+C cancels".into();
        Ok(())
    }
    async fn command(&mut self, worker: &Worker) -> Result<bool> {
        let line = self.editor.lines().join("\n");
        let words = shell_words::split(&line)
            .map_err(|error| Error::new("INVALID_ARGUMENT", error.to_string()))?;
        match words.first().map(String::as_str) {
            Some("/attach") if words.len() == 2 => {
                self.start_input(Some(PathBuf::from(&words[1])))?
            }
            Some("/paste-image") if words.len() == 1 => self.start_input(None)?,
            Some("/help") if words.len() == 1 => self.show_help(),
            Some("/commands") if words.len() == 1 => {
                self.modal = Some(checkpoints::Modal::commands())
            }
            Some("/history") if words.len() == 1 => self.open_history(vec![None])?,
            Some("/logs") if words.len() == 1 => {
                self.open_saved(saved::Page::current(self.conversation.id().into()), None)?
            }
            Some("/conversations") if words.len() == 1 => {
                self.open_saved(saved::Page::sessions(self.conversation.id().into()), None)?
            }
            Some("/clear-images") if words.len() == 1 => {
                self.conversation.clear_images()?;
                self.request(worker, false, false);
            }
            Some("/open-preview") if words.len() == 1 => self.open_preview()?,
            Some("/time") if words.len() == 2 => {
                let time = words[1].parse::<f64>().map_err(|_| {
                    Error::new("INVALID_ARGUMENT", "Usage: /time <nonnegative seconds>")
                })?;
                self.view.set_time(time)?;
                self.playback.stop();
                self.request(worker, false, false);
            }
            Some("/play") if words.len() <= 2 => {
                let fps = words
                    .get(1)
                    .map_or(Ok(playback::DEFAULT_FPS), |text| text.parse::<u32>())
                    .map_err(|_| {
                        Error::new(
                            "INVALID_ARGUMENT",
                            "Usage: /play [integer fps from 1 to 10]",
                        )
                    })?;
                self.start_playback(fps, Instant::now())?;
            }
            Some("/pause") if words.len() == 1 => self.pause_playback(),
            Some("/quit") if words.len() == 1 => return Ok(true),
            _ => {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Commands: /attach <quoted path>, /paste-image, /clear-images, /open-preview, /time <nonnegative seconds>, /play [fps], /pause, /commands, /history, /logs, /conversations, /help, /quit",
                ));
            }
        }
        self.editor = editor("");
        self.draft_changed = true;
        self.save_draft()?;
        Ok(false)
    }
    fn filtered_nodes(&self) -> Vec<usize> {
        self.preview
            .as_ref()
            .map(|preview| {
                preview
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| {
                        self.filter.is_empty()
                            || node
                                .name
                                .to_lowercase()
                                .contains(&self.filter.to_lowercase())
                            || node.id.to_lowercase().contains(&self.filter.to_lowercase())
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .unwrap_or_default()
    }
    fn selected_node(&self) -> Option<&preview::Node> {
        let index = *self.filtered_nodes().get(self.nodes.selected()?)?;
        self.preview.as_ref()?.nodes.get(index)
    }
    fn fit_selected(&mut self, picker: &graphics::Backend) -> Result<()> {
        let bounds = self
            .selected_node()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Select a node first"))?
            .bounds;
        self.view
            .fit(bounds, preview::pixel_size(picker, self.cells)?);
        Ok(())
    }
    fn open_preview(&mut self) -> Result<()> {
        if self.camera.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The camera preview is updating; wait for the complete image before exporting",
            ));
        }
        let preview = self
            .preview
            .as_ref()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "No complete preview is available"))?;
        let directory = self.conversation.directory().join("previews");
        crate::store::create_private_directory(&directory)?;
        let path = directory.join(format!("{}.png", uuid::Uuid::new_v4()));
        crate::store::atomic_replace(&path, &preview.image.png()?)?;
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = std::process::Command::new("explorer.exe");
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = std::process::Command::new("xdg-open");
        command
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().map_err(terminal::io)?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        self.status = format!("Opened {}", path.display());
        Ok(())
    }
    fn draw(&mut self, frame: &mut Frame) -> Size {
        let area = frame.area();
        if area.width < 50 || area.height < 16 {
            frame.render_widget(
                Paragraph::new(
                    "Resize the terminal to at least 50 columns × 16 rows. Ctrl+C exits when idle.",
                ),
                area,
            );
            return Size::new(0, 0);
        }
        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(2),
        ])
        .split(area);
        let state = if self.conversation.active() {
            "running"
        } else {
            "idle"
        };
        frame.render_widget(
            Paragraph::new(format!(
                "lazypencil · {:?} · {state} · session: {} · auth: {} · {}",
                self.conversation.settings().agent,
                &self.conversation.id()[..8],
                self.authentication,
                clean(&self.conversation.store().path().display().to_string())
            ))
            .style(Style::default().fg(Color::Cyan)),
            sections[0],
        );
        let panes = Layout::horizontal([
            Constraint::Percentage(22),
            Constraint::Percentage(48),
            Constraint::Percentage(30),
        ])
        .split(sections[1]);
        let nodes_title = if self.searching || !self.filter.is_empty() {
            format!("Nodes /{}", self.filter)
        } else {
            "Nodes".into()
        };
        let indices = self.filtered_nodes();
        let items: Vec<_> = indices
            .iter()
            .map(|index| ListItem::new(clean(&self.preview.as_ref().unwrap().nodes[*index].name)))
            .collect();
        if let Some(index) = self.nodes.selected()
            && index >= items.len()
        {
            self.nodes.select(if items.is_empty() {
                None
            } else {
                Some(items.len() - 1)
            });
        }
        frame.render_stateful_widget(
            List::new(items)
                .block(panel(nodes_title, self.focus == Focus::Nodes))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› "),
            panes[0],
            &mut self.nodes,
        );
        let block = panel(
            format!(
                "Preview{} · {} · {} / {} zoom",
                if self.camera.is_some() {
                    " · updating"
                } else {
                    ""
                },
                self.protocol,
                self.navigation.label(Nav::ZoomIn),
                self.navigation.label(Nav::ZoomOut)
            ),
            self.focus == Focus::Preview,
        );
        let inner = block.inner(panes[1]);
        frame.render_widget(block, panes[1]);
        if let Some(camera) = &self.camera {
            frame.render_widget(ratatui_image::Image::new(&camera.protocol), inner);
        } else if let Some(preview) = &self.preview {
            frame.render_widget(ratatui_image::Image::new(&preview.protocol), inner);
        } else {
            frame.render_widget(
                Paragraph::new("No complete preview\nRendering errors appear below.")
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
        let logblock = panel(
            format!(
                "Conversation · {} / {}",
                self.navigation.label(Nav::ScrollUp),
                self.navigation.label(Nav::ScrollDown)
            ),
            self.focus == Focus::Conversation,
        );
        let logarea = logblock.inner(panes[2]);
        let paragraph = Paragraph::new(self.log.as_str()).wrap(Wrap { trim: false });
        let lines = paragraph.line_count(logarea.width).min(u16::MAX as usize) as u16;
        let scroll = lines
            .saturating_sub(logarea.height)
            .saturating_sub(self.scroll);
        frame.render_widget(paragraph.scroll((scroll, 0)).block(logblock), panes[2]);
        let attachments = self
            .conversation
            .images()
            .iter()
            .enumerate()
            .map(|(index, image)| {
                ListItem::new(format!(
                    "[Image {}] {} · {}×{} · PNG",
                    index + 1,
                    clean(&image.display_name),
                    image.width,
                    image.height
                ))
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            List::new(attachments)
                .block(panel(
                    "Attachments · d removes",
                    self.focus == Focus::Attachments,
                ))
                .highlight_style(Style::default().bg(Color::DarkGray)),
            sections[2],
            &mut self.attachments,
        );
        let prompt_title = format!(
            "Prompt · {} sends · {} pastes",
            self.keys.submit_label(),
            self.keys.paste_label()
        );
        self.editor.set_block(panel(
            if self.conversation.active() {
                "Prompt · running".to_owned()
            } else {
                prompt_title
            },
            self.focus == Focus::Prompt,
        ));
        self.editor.set_cursor_style(
            if self.focus == Focus::Prompt && !self.conversation.active() {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            },
        );
        frame.render_widget(&self.editor, sections[3]);
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(self.status.clone()),
                Line::from(format!("{} / {} panels · {} commands · {} help · {} exits outside prompt · Ctrl+C cancels/exits", self.navigation.label(Nav::NextPanel), self.navigation.label(Nav::PreviousPanel), self.navigation.label(Nav::Commands), self.navigation.label(Nav::Help), self.navigation.label(Nav::Quit))),
            ]))
            .wrap(Wrap { trim: false }),
            sections[4],
        );
        if self.help {
            let popup = centered(area, 100, 26);
            frame.render_widget(Clear, popup);
            let paragraph = Paragraph::new(self.help_text()).wrap(Wrap { trim: false });
            let block = panel("Help · PgUp/PgDn scroll · Esc closes", true);
            let inner = block.inner(popup);
            let maximum = paragraph
                .line_count(inner.width)
                .saturating_sub(inner.height as usize)
                .min(u16::MAX as usize) as u16;
            self.help_scroll = self.help_scroll.min(maximum);
            frame.render_widget(paragraph.scroll((self.help_scroll, 0)).block(block), popup);
        }
        self.draw_checkpoint_modal(frame);
        if let Some(permission) = self.permissions.front() {
            let popup = centered(area, 92, 26);
            frame.render_widget(Clear, popup);
            let block = panel("Permission · PgUp/PgDn details · Esc denies", true);
            let inner = block.inner(popup);
            frame.render_widget(block, popup);
            let sections = Layout::vertical([
                Constraint::Min(2),
                Constraint::Length(permission.request.options.len() as u16 + 3),
            ])
            .split(inner);
            let details = serde_json::to_value(&permission.request.tool_call).unwrap_or_default();
            let details = crate::conversation::sanitize_event(&details);
            let details = clean(&serde_json::to_string_pretty(&details).unwrap_or_default());
            let paragraph = Paragraph::new(details).wrap(Wrap { trim: false });
            let max_scroll = paragraph
                .line_count(sections[0].width)
                .saturating_sub(usize::from(sections[0].height))
                .min(u16::MAX as usize) as u16;
            frame.render_widget(
                paragraph.scroll((permission.scroll.min(max_scroll), 0)),
                sections[0],
            );
            let mut lines = vec![
                Line::from(clean(
                    permission
                        .request
                        .tool_call
                        .fields
                        .title
                        .as_deref()
                        .unwrap_or("Agent requests permission"),
                )),
                Line::from("Up/Down chooses · Enter responds · Esc denies"),
            ];
            for (index, option) in permission.request.options.iter().enumerate() {
                let style = if index == permission.selected {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default()
                };
                lines.push(Line::styled(
                    format!(
                        "{} {}",
                        if index == permission.selected {
                            "›"
                        } else {
                            " "
                        },
                        clean(&option.name)
                    ),
                    style,
                ));
            }
            frame.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }),
                sections[1],
            );
        }
        Size::new(inner.width, inner.height)
    }
}

fn editor(text: &str) -> TextArea<'static> {
    let mut editor = TextArea::new(text.split('\n').map(str::to_string).collect());
    editor.set_max_histories(100);
    editor.set_cursor_line_style(Style::default());
    editor.move_cursor(CursorMove::Bottom);
    editor.move_cursor(CursorMove::End);
    editor
}
fn panel(title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
}
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
fn read_input(
    executable: &Path,
    path: Option<&Path>,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    if let Some(path) = path {
        clipboard::read_image(executable, path, cancel)
    } else {
        clipboard::read_clipboard(executable, cancel)
    }
}

async fn termination() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
        tokio::select! { _ = terminate.recv() => {}, _ = hangup.recv() => {} }
        Ok(())
    }
    #[cfg(not(unix))]
    std::future::pending().await
}
