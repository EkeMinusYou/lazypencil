//! Terminal viewing and prompting, sharing the same document and ACP services as CLI.

mod account;
mod camera;
mod checkpoints;
mod composer;
mod controls;
mod diagnostics;
mod graphics;
mod images;
mod markdown;
mod nodes;
mod outbox;
mod playback;
mod preview;
mod references;
mod saved;
mod terminal;
#[cfg(test)]
mod tests;
mod theme;

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
use images::InputJob;
use preview::{Preview, Request, View, Worker};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
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
    References,
    Conversation,
}
impl Focus {
    fn next(self, backwards: bool) -> Self {
        let current = if self.in_conversation() {
            Self::Prompt
        } else {
            self
        };
        current.cycle(&[Self::Nodes, Self::Preview, Self::Prompt], backwards)
    }
    fn in_conversation(self) -> bool {
        matches!(
            self,
            Self::Prompt | Self::Attachments | Self::References | Self::Conversation
        )
    }
    fn next_section(self, backwards: bool, attachments: bool, references: bool) -> Self {
        let mut order = vec![Self::Conversation];
        if attachments {
            order.push(Self::Attachments);
        }
        if references {
            order.push(Self::References);
        }
        order.push(Self::Prompt);
        let current = if !order.contains(&self) {
            Self::Prompt
        } else {
            self
        };
        current.cycle(&order, backwards)
    }
    fn cycle(self, order: &[Self], backwards: bool) -> Self {
        let index = order.iter().position(|item| *item == self).unwrap();
        order[(index + if backwards { order.len() - 1 } else { 1 }) % order.len()]
    }
}
struct Permission {
    id: String,
    request: Box<RequestPermissionRequest>,
    selected: usize,
    scroll: u16,
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
    conversation.attachment_limits()?.check_count(
        conversation
            .images()
            .len()
            .saturating_add(args.prompt.images.len()),
    )?;
    for image in &args.prompt.images {
        conversation.attach_file(image)?;
    }
    // Reserve two slots each for exact and temporary camera frames. The low
    // bit selects the slot; the high bit identifies temporary feedback.
    let id = (uuid::Uuid::new_v4().as_u128() as u32 & 0x7fff_fffe).max(2);
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
        if let Some(check) = app.account_check.take() {
            check.close().await;
        }
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
    preview_generation: Option<u64>,
    presentation: graphics::Presentation,
    preview_error: Option<Error>,
    nodes_loading: bool,
    pending_preview: Option<controls::Pending>,
    camera: Option<camera::Camera>,
    camera_deadline: Option<Instant>,
    camera_was_visible: bool,
    camera_id: u32,
    generation: u64,
    cells: Size,
    nodes: ListState,
    node_tree: nodes::Tree,
    pending_node_selection: Option<String>,
    diagnostics: diagnostics::View,
    attachments: ListState,
    reference_rows: ListState,
    filter: String,
    searching: bool,
    log: String,
    tool_calls: std::collections::HashMap<String, composer::ToolDisplay>,
    turn_sequence: u64,
    history: Option<Paragraph<'static>>,
    scroll: u16,
    status: String,
    authentication: String,
    account_check: Option<account::Check>,
    account_pending: bool,
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
    turn_started: Option<Instant>,
    outbox: outbox::Outbox,
    last_draft_save: std::time::Instant,
    input_reader: fn(
        &Path,
        Option<&Path>,
        crate::attachments::AttachmentOptions,
        &CancellationToken,
    ) -> Result<InputContent>,
}
impl App {
    fn new(conversation: Conversation) -> Self {
        let editor = editor(conversation.draft());
        let status = format!("Session {}", conversation.id());
        let mut app = Self {
            conversation,
            keys: Default::default(),
            navigation: Default::default(),
            editor,
            focus: Focus::Prompt,
            view: View::default(),
            playback: playback::Playback::default(),
            preview: None,
            preview_generation: None,
            presentation: Default::default(),
            preview_error: None,
            nodes_loading: true,
            pending_preview: None,
            camera: None,
            camera_deadline: None,
            camera_was_visible: false,
            camera_id: 0x8000_0001,
            generation: 0,
            cells: Size::new(0, 0),
            nodes: ListState::default(),
            node_tree: nodes::Tree::default(),
            pending_node_selection: None,
            diagnostics: diagnostics::View::default(),
            attachments: ListState::default(),
            reference_rows: ListState::default(),
            filter: String::new(),
            searching: false,
            log: String::new(),
            tool_calls: Default::default(),
            turn_sequence: 0,
            history: None,
            scroll: 0,
            status,
            authentication: "checking".into(),
            account_check: None,
            account_pending: true,
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
            turn_started: None,
            outbox: Default::default(),
            last_draft_save: std::time::Instant::now(),
            input_reader: read_input,
        };
        app.restore_outbox();
        app
    }
    async fn event_loop(&mut self, screen: &mut terminal::Screen, worker: &Worker) -> Result<()> {
        let mut preview_control = crate::preview_control::Host::start()?;
        self.conversation.set_preview_control(
            preview_control.client(self.conversation.store().path(), self.conversation.id()),
        );
        let mut events = EventStream::new();
        self.camera_id = screen.camera_image_id();
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut termination = Box::pin(termination());
        loop {
            self.sync_turn(worker);
            if let Err(error) = self.send_queued().await {
                self.outbox.paused = true;
                self.error(error);
                self.dirty = true;
            }
            self.expire_preview_control();
            if self.conversation.active() {
                self.account_pending = false;
                if let Some(check) = &self.account_check {
                    check.cancel.cancel();
                }
            }
            if self.account_pending && self.account_check.is_none() {
                self.account_pending = false;
                match account::Check::start(&self.conversation) {
                    Ok(check) => self.account_check = Some(check),
                    Err(error) => {
                        self.authentication = "unavailable".into();
                        self.error(error);
                    }
                }
                self.dirty = true;
            }
            if self.dirty {
                let mut cells = self.cells;
                screen.draw(|frame| {
                    cells = self.draw(frame);
                })?;
                self.dirty = false;
                if cells != self.cells {
                    self.cells = cells;
                    self.request(worker, false, false);
                }
            }
            let playback_deadline = self.playback.deadline();
            let camera_deadline = self.camera_deadline;
            let activity_deadline = (self.conversation.active() && !self.has_modal())
                .then(|| Instant::now() + Duration::from_millis(200));
            tokio::select! {
                _ = async {
                    if let Some(deadline) = activity_deadline {
                        tokio::time::sleep_until(deadline.into()).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => { self.dirty = true; },
                signal = tokio::signal::ctrl_c() => {
                    signal.map_err(terminal::io)?;
                    if self.conversation.active() {
                        self.outbox.paused = true;
                        self.outbox.save(self.conversation.directory())?;
                    }
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
                        match self.key(key, &screen.picker, worker).await {
                            Ok(true) => { self.save_draft()?; return Ok(()); },
                            Ok(false) => {}, Err(error) => self.error(error),
                        }
                        if modal && !self.has_modal() { screen.redraw(); }
                        self.dirty = true;
                    },
                    Some(Ok(Event::Paste(text))) if self.focus == Focus::Prompt && !self.has_modal() => {
                        self.insert_text(&text); self.dirty = true;
                    },
                    Some(Ok(Event::Resize(_, _))) => { self.dirty = true; },
                    Some(Ok(_)) => {},
                    Some(Err(error)) => return Err(terminal::io(error)),
                    None => { self.save_draft()?; return Ok(()); },
                },
                incoming = preview_control.next() => {
                    let incoming = incoming.ok_or_else(|| Error::new("PREVIEW_UNAVAILABLE", "Preview control listener stopped"))?;
                    self.receive_preview_control(incoming, worker, &screen.picker);
                    self.dirty = true;
                },
                output = worker.next() => {
                    self.accept_controlled_preview(output, worker, &screen.picker);
                },
                _ = async {
                    if let Some(deadline) = camera_deadline {
                        tokio::time::sleep_until(deadline.into()).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    self.camera_deadline = None;
                    // An already completed exact frame always wins over interim work.
                    let accepted = worker.try_next().is_some_and(|output| {
                        self.accept_controlled_preview(output, worker, &screen.picker)
                    });
                    if !accepted {
                        if let Err(error) = self.render_camera_feedback(&screen.picker) {
                            self.error(error);
                        }
                        self.dirty = true;
                    }
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
                result = async { self.account_check.as_mut().unwrap().result().await }, if self.account_check.is_some() => {
                    let check = self.account_check.take().expect("account check");
                    if !check.cancel.is_cancelled() && !self.account_pending && !self.conversation.active() {
                        match result {
                            Ok(Some(report)) => self.authentication = report.display(),
                            Ok(None) => self.authentication = "not reported".into(),
                            Err(error) => {
                                self.authentication = "unavailable".into();
                                self.error(error);
                            }
                        }
                        self.dirty = true;
                    }
                },
                content = async { (&mut self.input.as_mut().unwrap().receiver).await.unwrap_or_else(|error| Err(Error::new("CLIPBOARD_UNAVAILABLE", error.to_string()))) }, if self.input.is_some() => {
                    self.input.take();
                    if self.accept_prepared(content) { self.request(worker, false, false); }
                    self.dirty = true;
                },
                output = async { (&mut self.document_task.as_mut().unwrap().receiver).await.unwrap_or_else(|error| Err(Error::new("IO_ERROR", format!("Document worker stopped: {error}")))) }, if self.document_task.is_some() => {
                    let modal = self.has_modal();
                    self.finish_document_result(output, worker);
                    if modal && !self.has_modal() { screen.redraw(); }
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
            self.preview_error = None;
            self.camera_was_visible = self.camera.take().is_some();
            if self.camera_was_visible {
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
        if refresh && attachment.is_none() {
            self.nodes_loading = true;
        }
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
    fn accept_preview(&mut self, output: preview::Output) -> bool {
        if output.generation != self.generation {
            return false;
        }
        self.camera_deadline = None;
        // Background polls still retry failed renders, but identical failures
        // must not flood the conversation, overwrite status, or repaint Kitty.
        if let Err(error) = &output.result
            && self.preview_error.as_ref().is_some_and(|previous| {
                previous.code == error.code
                    && previous.message == error.message
                    && previous.details == error.details
            })
        {
            return false;
        }
        self.camera = None;
        match output.result {
            Ok(mut preview) => {
                let selected = self
                    .selected_node()
                    .map(|node| node.id.clone())
                    .or_else(|| self.pending_node_selection.clone());
                let diagnostic = self.selected_diagnostic().cloned();
                self.preview_error = None;
                if !preview.attachment {
                    self.view = preview.view;
                    if !preview.time_dependent && !preview.live_pages {
                        self.playback.stop();
                    }
                }
                self.status = if preview.attachment {
                    "Attachment preview".into()
                } else {
                    String::new()
                };
                // Preserve the document node list while browsing an attachment.
                if preview.attachment
                    && let Some(previous) = &mut self.preview
                {
                    preview.nodes = std::mem::take(&mut previous.nodes);
                    preview.summary["diagnostics"] = previous.summary["diagnostics"].clone();
                    for key in ["revision", "dependency_revision"] {
                        preview.summary[key] = previous.summary[key].clone();
                    }
                }
                let document = !preview.attachment;
                self.preview = Some(preview);
                if document {
                    self.pending_node_selection = None;
                    self.nodes_loading = false;
                    self.refresh_nodes(selected.as_deref());
                    self.refresh_diagnostics(diagnostic.as_ref());
                } else if self
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.summary["revision"].is_string())
                {
                    self.nodes_loading = false;
                }
                self.preview_generation = Some(output.generation);
            }
            Err(error) => {
                self.playback.stop();
                if let Some(node) = self.selected_node() {
                    self.pending_node_selection = Some(node.id.clone());
                }
                self.preview = None;
                self.node_tree.rebuild(&[], &self.filter);
                self.nodes.select(None);
                self.refresh_diagnostics(None);
                self.preview_generation = None;
                self.preview_error = Some(error.clone());
                self.nodes_loading = false;
                self.error(error);
            }
        }
        self.dirty = true;
        true
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
        self.status.clear();
        Ok(())
    }
    fn pause_playback(&mut self) {
        self.playback.stop();
        self.status = if self
            .preview
            .as_ref()
            .is_some_and(|preview| !preview.attachment && preview.view.time != self.view.time)
        {
            "finishing requested frame".into()
        } else {
            String::new()
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
        if self.draft_changed {
            let text = self.editor.lines().join("\n");
            if !self.conversation.active() {
                self.conversation.set_draft(text.clone())?;
            }
            self.outbox.draft = if self.conversation.active()
                || self.turn_pending
                || !self.outbox.queued.is_empty()
            {
                Some(text)
            } else {
                None
            };
            self.outbox.save(self.conversation.directory())?;
            self.draft_changed = false;
            self.last_draft_save = std::time::Instant::now();
        }
        Ok(())
    }
    fn sync_turn(&mut self, worker: &Worker) {
        // Completion is recorded before the final frontend message is dequeued.
        // Synchronize once on the actual state transition, preserving any next draft.
        if self.turn_pending && !self.conversation.active() {
            self.permissions.clear();
            self.turn_pending = false;
            self.turn_started = None;
            let failed = self.conversation.result().is_some_and(Result::is_err);
            self.outbox.paused |= failed;
            let reference_failure = failed && self.outbox.in_flight.is_some();
            self.finish_reference_prompt(failed);
            if failed && !reference_failure && self.editor.lines().iter().all(String::is_empty) {
                self.editor = editor(self.conversation.draft());
            }
            if self.focus.in_conversation() {
                self.focus = Focus::Prompt;
            }
            self.draft_changed = true;
            if let Err(error) = self.save_draft() {
                self.error(error);
            }
            self.dirty = true;
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
    fn error(&mut self, error: Error) {
        self.status = clean(&error.to_string());
        self.append(&format!("\n{}\n", error));
    }
    fn append(&mut self, text: &str) {
        self.history = None;
        self.log.push_str(&clean(text));
        if self.log.len() > 256 * 1024 {
            let mut index = self.log.len() - 192 * 1024;
            while !self.log.is_char_boundary(index) {
                index += 1;
            }
            self.log.drain(..index);
            let visible: std::collections::HashSet<String> = self
                .log
                .lines()
                .filter_map(|line| line.strip_prefix("[tool_call] "))
                .filter_map(|id| serde_json::from_str(id).ok())
                .collect();
            self.tool_calls.retain(|id, _| visible.contains(id));
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
                        if let Some(id) = update["toolCallId"].as_str() {
                            let id = format!("{}:{id}", self.turn_sequence);
                            let fresh = !self.tool_calls.contains_key(&id);
                            let tool = self.tool_calls.entry(id.clone()).or_default();
                            if let Some(title) =
                                update["title"].as_str().filter(|title| !title.is_empty())
                            {
                                tool.title = clean(title).replace(['\n', '\t'], " ");
                            }
                            if let Some(status) = update["status"].as_str() {
                                tool.status = status.into();
                            }
                            if fresh {
                                self.append(&format!(
                                    "\n[tool_call] {}\n",
                                    serde_json::to_string(&id).expect("tool id")
                                ));
                            } else {
                                self.history = None;
                            }
                        } else {
                            let title = update["title"].as_str().unwrap_or("Tool");
                            let status = update["status"].as_str().unwrap_or("running");
                            self.append(&format!("\n[{status}] {title}\n"));
                        }
                    }
                    _ => {}
                }
            }
            Some("auth_status") => {
                self.authentication = self
                    .conversation
                    .authentication()
                    .map(|report| report.display())
                    .unwrap_or_else(|| {
                        value["authentication"]["source"]
                            .as_str()
                            .unwrap_or("not reported")
                            .into()
                    });
                self.status = format!(
                    "Authentication: {}",
                    value["authentication"]["source"]
                        .as_str()
                        .unwrap_or("unknown")
                );
            }
            Some("ready") => {
                if let Some(report) = self.conversation.authentication() {
                    self.authentication = report.display();
                }
                self.status.clear();
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
            if self.conversation.active() {
                self.outbox.paused = true;
                self.outbox.save(self.conversation.directory())?;
            }
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
            let selected = self.selected_node().map(|node| node.id.clone());
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
            self.search_nodes(selected.as_deref());
            return Ok(false);
        }
        let section = self.focus.in_conversation()
            && (global(Nav::PreviousSection) || global(Nav::NextSection));
        let return_to_prompt = matches!(
            self.focus,
            Focus::Attachments | Focus::References | Focus::Conversation
        ) && key.code == KeyCode::Esc;
        if section || return_to_prompt || global(Nav::NextPanel) || global(Nav::PreviousPanel) {
            let was_attachment = self.focus == Focus::Attachments;
            self.focus = if return_to_prompt {
                Focus::Prompt
            } else if section {
                self.focus.next_section(
                    global(Nav::PreviousSection),
                    !self.conversation.active() && !self.conversation.images().is_empty(),
                    !self.outbox.references.is_empty(),
                )
            } else {
                self.focus.next(global(Nav::PreviousPanel))
            };
            if self.focus == Focus::Attachments
                && !self.conversation.images().is_empty()
                && self.attachments.selected().is_none()
            {
                self.attachments.select(Some(0));
            }
            if self.focus == Focus::References && self.reference_rows.selected().is_none() {
                self.reference_rows.select(Some(0));
            }
            if was_attachment || self.focus == Focus::Attachments {
                self.request(worker, false, false);
            }
            return Ok(false);
        }
        if self.focus == Focus::Prompt {
            if key.code == KeyCode::Enter
                && key.modifiers.is_empty()
                && self.editor.lines().len() == 1
                && self.editor.lines()[0].starts_with('/')
            {
                return self.command(worker).await;
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
                && key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            {
                self.insert_text("\n");
                return Ok(false);
            }
            if key.modifiers == KeyModifiers::CONTROL {
                match key.code {
                    KeyCode::Char('u') => {
                        self.draft_changed |= self.editor.delete_line_by_head();
                        return Ok(false);
                    }
                    KeyCode::Char('_' | '/') => {
                        self.draft_changed |= self.editor.undo();
                        return Ok(false);
                    }
                    KeyCode::Char('y') => {
                        self.insert_text(&self.editor.yank_text());
                        return Ok(false);
                    }
                    _ => {}
                }
            }
            // Bound interactive input as well as bracketed and clipboard pastes.
            if self.editor.lines().iter().map(String::len).sum::<usize>() >= 1024 * 1024
                && (key.code == KeyCode::Enter
                    || matches!(key.code, KeyCode::Char(_)) && !ctrl && !alt)
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
                if action(Nav::ToggleDiagnostics) {
                    let selected = self.selected_diagnostic().cloned();
                    if !self.diagnostics.active {
                        self.diagnostics.selected_only = self.selected_node().is_some();
                        self.refresh_diagnostics(selected.as_ref());
                    }
                    self.diagnostics.active = !self.diagnostics.active;
                    return Ok(false);
                }
                if self.diagnostics.active {
                    self.diagnostic_key(key, picker, worker)?;
                    return Ok(false);
                }
                if action(Nav::ReferenceNode) {
                    self.reference_selected_node()?;
                    return Ok(false);
                }
                let count = self.filtered_nodes().len();
                if action(Nav::PreviousItem) {
                    self.nodes.select(
                        (count > 0).then(|| self.nodes.selected().unwrap_or(0).saturating_sub(1)),
                    );
                } else if action(Nav::NextItem) {
                    self.nodes.select((count > 0).then(|| {
                        (self.nodes.selected().map_or(0, |n| n + 1)).min(count.saturating_sub(1))
                    }));
                } else if action(Nav::ExpandNode)
                    || action(Nav::CollapseNode)
                    || action(Nav::ToggleNode)
                {
                    self.navigate_node_tree(if action(Nav::ExpandNode) {
                        Nav::ExpandNode
                    } else if action(Nav::CollapseNode) {
                        Nav::CollapseNode
                    } else {
                        Nav::ToggleNode
                    });
                } else if action(Nav::SearchNodes) {
                    self.searching = true;
                } else if action(Nav::NodePageUp) || action(Nav::NodePageDown) {
                    self.page_nodes(action(Nav::NodePageUp));
                } else if action(Nav::ClearSelection) {
                    self.nodes.select(None);
                    self.pending_node_selection = None;
                } else if action(Nav::ActivateNode) || action(Nav::FitSelected) {
                    self.fit_selected(picker)?;
                    self.request(worker, false, false);
                    self.camera_feedback(picker)?;
                }
            }
            Focus::References => self.reference_key(key)?,
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
                    if self.input.is_some() {
                        return Err(Error::new(
                            "INVALID_ARGUMENT",
                            "Finish or cancel image conversion before removing an attachment",
                        ));
                    }
                    if let Some(index) = self.attachments.selected() {
                        self.conversation.remove_image(index)?;
                        self.attachments.select(if count > 1 {
                            Some(index.min(count - 2))
                        } else {
                            None
                        });
                        if count == 1 {
                            self.focus = Focus::Prompt;
                        }
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
            "{} / {}: switch Nodes, Preview, Prompt (right / left)\n{}: command palette; /history: checkpoints\nPrompt: {} sends; Alt+Enter / Shift+Enter inserts a newline\n{}: paste image, copied image files, or text\n",
            label(Nav::NextPanel),
            label(Nav::PreviousPanel),
            label(Nav::Commands),
            self.keys.submit_label(),
            self.keys.paste_label()
        );
        text.push_str(&format!(
            "{} / {}: switch Agent sections; Esc returns to Prompt\n",
            label(Nav::PreviousSection),
            label(Nav::NextSection)
        ));
        text.push_str("/attach <quoted path> [WIDTHxHEIGHT|original]: attach PNG, JPEG or WebP\n/paste-image [WIDTHxHEIGHT|original]: read this host's clipboard\n/resize-image <number> <WIDTHxHEIGHT|original>: choose send size\n/clear-images: remove pending images\n/resume-queue: resume saved or paused follow-ups; /clear-queue: discard queued prompts\n");
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
            (
                Nav::PreviousItem,
                "Nodes/Attachments/References: previous item",
            ),
            (Nav::NextItem, "Nodes/Attachments/References: next item"),
            (Nav::SearchNodes, "Nodes: search names and IDs"),
            (
                Nav::ReferenceNode,
                "Nodes/Diagnostics: add a prompt reference and focus input",
            ),
            (Nav::ToggleDiagnostics, "Nodes: switch Nodes / Diagnostics"),
            (
                Nav::DiagnosticScope,
                "Diagnostics: switch selected node and descendants / all nodes",
            ),
            (Nav::ClearSelection, "Nodes: clear the editing target"),
            (Nav::ActivateNode, "Nodes: select and fit the target"),
            (Nav::ExpandNode, "Nodes: expand or enter a branch"),
            (
                Nav::CollapseNode,
                "Nodes: close the parent branch and select it",
            ),
            (Nav::ToggleNode, "Nodes: toggle a branch"),
            (Nav::NodePageUp, "Nodes: move up half a page"),
            (Nav::NodePageDown, "Nodes: move down half a page"),
            (
                Nav::RemoveAttachment,
                "Attachments/References: remove pending item",
            ),
            (Nav::ScrollUp, "Agent history: scroll back"),
            (Nav::ScrollDown, "Agent history: scroll forward"),
            (Nav::FollowOutput, "Agent history: follow new output"),
            (Nav::Refresh, "refresh preview, including browser captures"),
            (Nav::OpenPreview, "open complete PNG in external viewer"),
            (Nav::Quit, "outside Prompt: close conversation and exit"),
        ] {
            text.push_str(&format!("{}: {description}\n", label(action)));
        }
        text.push_str("\n/open-preview: open the complete PNG externally\n/time <seconds>: seek; /play [1-10 fps], /pause\n/logs: read current saved turns; /conversations: saved conversations\nSaved logs: arrows select/scroll; Enter opens; n/p pages\nu: select local session; N: new local conversation\nv: raw/formatted; Backspace returns; r refreshes; Esc closes\n/commands, /history, /help, /quit\nCtrl+C: cancel active work; exit when idle\nDialogs keep their displayed keys; Esc closes help\nHelp: PgUp/PgDn or arrows scroll; Home/End jump\n\nClipboard access happens only on explicit paste. SSH reads the remote host's clipboard. Image protocols do not provide clipboard access.");
        text.push_str(&format!("\nDiagnostics: {} / {} select; {} reveals and fits the node\n{} / {} scroll details; {} returns to Nodes\n", label(Nav::PreviousItem), label(Nav::NextItem), label(Nav::ActivateNode), label(Nav::NodePageUp), label(Nav::NodePageDown), label(Nav::ClearSelection)));
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
        self.outbox = Default::default();
        self.restore_outbox();
        self.draft_changed = false;
        self.turn_pending = false;
        self.turn_started = None;
        self.last_draft_save = Instant::now();
        self.attachments.select(None);
        self.reference_rows.select(None);
        self.permissions.clear();
        self.authentication = "checking".into();
        self.account_pending = true;
        if let Some(check) = &self.account_check {
            check.cancel.cancel();
        }
        self.log.clear();
        self.tool_calls.clear();
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
        if self.conversation.active() || !self.outbox.queued.is_empty() {
            let text = self.editor.lines().join("\n");
            if !text.trim().is_empty() {
                self.outbox.push(outbox::Prompt {
                    text,
                    selection: None,
                    references: self.outbox.references.clone(),
                    image_ids: Vec::new(),
                })?;
                self.outbox.references.clear();
                self.reference_rows.select(None);
                self.editor = editor("");
            } else if self.conversation.active() {
                return Err(Error::new("INVALID_ARGUMENT", "Enter a follow-up prompt"));
            }
            if !self.conversation.active() {
                self.outbox.paused = false;
            }
            self.outbox.draft = Some(self.editor.lines().join("\n"));
            self.outbox.save(self.conversation.directory())?;
            self.draft_changed = false;
            self.status.clear();
            return self.send_queued().await;
        }
        self.save_draft()?;
        let target = crate::conversation::references::selection(&self.outbox.references);
        let text = self.conversation.draft().to_string();
        self.conversation.start_send(target).await?;
        let references = std::mem::take(&mut self.outbox.references);
        self.reference_rows.select(None);
        self.outbox.in_flight = (!references.is_empty()).then(|| outbox::Prompt {
            text: text.clone(),
            selection: None,
            references: references.clone(),
            image_ids: self
                .conversation
                .images()
                .iter()
                .map(|image| image.id.clone())
                .collect(),
        });
        self.outbox.paused = false;
        self.turn_pending = true;
        self.turn_sequence = self.turn_sequence.wrapping_add(1);
        self.turn_started = Some(Instant::now());
        self.append(&format!(
            "\nYou: {text}{}\n[{} images]\n\n{:?}:\n",
            references::history(&references),
            self.conversation.images().len(),
            self.conversation.settings().agent,
        ));
        self.scroll = 0;
        self.status.clear();
        self.editor = editor("");
        self.outbox.draft = Some(String::new());
        self.outbox.save(self.conversation.directory())?;
        Ok(())
    }
    async fn command(&mut self, worker: &Worker) -> Result<bool> {
        let line = self.editor.lines().join("\n");
        let words = shell_words::split(&line)
            .map_err(|error| Error::new("INVALID_ARGUMENT", error.to_string()))?;
        match words.first().map(String::as_str) {
            Some("/clear-queue") if words.len() == 1 => {
                self.outbox.queued.clear();
                self.outbox.paused = false;
                self.status = "Prompt queue cleared".into();
            }
            Some("/resume-queue") if words.len() == 1 => {
                self.outbox.paused = false;
            }
            Some("/attach") if (2..=3).contains(&words.len()) => {
                let bounds = images::input_bounds(
                    words.get(2).map(String::as_str),
                    self.conversation.settings().image_bounds,
                )?;
                self.start_input_with_bounds(Some(PathBuf::from(&words[1])), bounds)?
            }
            Some("/paste-image") if (1..=2).contains(&words.len()) => {
                let bounds = images::input_bounds(
                    words.get(1).map(String::as_str),
                    self.conversation.settings().image_bounds,
                )?;
                self.start_input_with_bounds(None, bounds)?
            }
            Some("/resize-image") if words.len() == 3 => {
                let index = words[1]
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .ok_or_else(|| {
                        Error::new(
                            "INVALID_ARGUMENT",
                            "Use /resize-image <image number from 1> <WIDTHxHEIGHT|original>",
                        )
                    })?;
                let bounds = if words[2] == "original" {
                    None
                } else {
                    Some(words[2].parse()?)
                };
                self.start_resize(index, bounds)?;
            }
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
                if self.input.is_some() {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "Finish or cancel image input before clearing attachments",
                    ));
                }
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
                    "Commands: /resume-queue, /clear-queue, /attach <quoted path> [WIDTHxHEIGHT|original], /paste-image [WIDTHxHEIGHT|original], /resize-image <number> <WIDTHxHEIGHT|original>, /clear-images, /open-preview, /time <nonnegative seconds>, /play [fps], /pause, /commands, /history, /logs, /conversations, /help, /quit",
                ));
            }
        }
        self.editor = editor("");
        self.draft_changed = true;
        self.save_draft()?;
        Ok(false)
    }
    fn filtered_nodes(&self) -> &[nodes::Row] {
        self.node_tree.rows()
    }
    fn selected_node(&self) -> Option<&preview::Node> {
        let index = self.filtered_nodes().get(self.nodes.selected()?)?.index;
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
        if self.camera.is_some() || self.camera_deadline.is_some() {
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
    fn agent_activity(&self) -> Span<'static> {
        let (label, color) = if !self.permissions.is_empty() {
            ("! Blocked", theme::RUNNING)
        } else if self.conversation.active() {
            ("● Working", theme::WORKING)
        } else {
            ("○ Idle", theme::SECONDARY)
        };
        Span::styled(label, Style::default().fg(color))
    }

    fn preview_zoom_label(&self) -> Option<String> {
        self.preview
            .as_ref()
            .filter(|preview| !preview.attachment)
            .map(|preview| {
                // Cached camera feedback displays the requested zoom; otherwise the
                // terminal still shows the last completed frame while rendering.
                let view = if self.camera.is_some() {
                    self.view
                } else {
                    preview.view
                };
                format!("{:.0}%", view.zoom.unwrap_or(1.0) * 100.0)
            })
    }

    fn preview_timeline(&self) -> Option<String> {
        let preview = self.preview.as_ref().filter(|preview| {
            !preview.attachment && (preview.time_dependent || preview.live_pages)
        })?;
        let playing = if self.playback.fps().is_some() {
            " · playing"
        } else {
            ""
        };
        Some(format!("Playback {:.3}s{playing}", preview.view.time))
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
            Constraint::Length(2),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "✎ lazypencil",
                    Style::default()
                        .fg(theme::AGENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    clean(
                        &self
                            .conversation
                            .store()
                            .path()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    )
                    .replace(['\n', '\t'], " "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!("{:?}", self.conversation.settings().agent),
                    Style::default()
                        .fg(theme::PREVIEW)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · ", Style::default().fg(theme::MUTED)),
                Span::styled("account: ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    clean(&self.authentication),
                    Style::default().fg(theme::MUTED),
                ),
                Span::styled(
                    format!(" · session: {} · ", &self.conversation.id()[..8]),
                    Style::default().fg(theme::MUTED),
                ),
                Span::styled(
                    clean(&self.conversation.store().path().display().to_string()),
                    Style::default().fg(theme::MUTED),
                ),
            ])),
            sections[0],
        );
        let panes = Layout::horizontal([
            Constraint::Percentage(22),
            Constraint::Percentage(48),
            Constraint::Percentage(30),
        ])
        .split(sections[1]);
        self.draw_nodes(frame, panes[0]);
        let mut block = panel(
            theme::title(
                if self.camera.is_some() { "↻" } else { "▣" },
                "Preview",
                if self.camera.is_some() {
                    theme::RUNNING
                } else {
                    theme::PREVIEW
                },
                self.focus == Focus::Preview,
            ),
            self.focus == Focus::Preview,
        );
        let zoom = self.preview_zoom_label();
        let zoom_width = zoom.as_ref().map_or(0, |zoom| zoom.len() + 3);
        if let Some(timeline) = self.preview_timeline() {
            let width = usize::from(panes[1].width.saturating_sub(2)).saturating_sub(zoom_width);
            block = block.title_bottom(Line::styled(
                format!(" {timeline} ")
                    .chars()
                    .take(width)
                    .collect::<String>(),
                Style::default().fg(theme::SECONDARY),
            ));
        }
        if let Some(zoom) = zoom {
            block = block.title_bottom(
                Line::styled(format!(" {zoom} "), Style::default().fg(theme::SECONDARY))
                    .right_aligned(),
            );
        }
        let inner = block.inner(panes[1]);
        frame.render_widget(block, panes[1]);
        if let Some(camera) = self
            .camera
            .as_ref()
            .filter(|camera| camera.generation == self.generation)
        {
            frame.render_widget(graphics::Image(&camera.protocol), inner);
            self.presentation.prepare(frame.buffer_mut(), inner, true);
        } else if let Some(preview) = &self.preview {
            frame.render_widget(graphics::Image(&preview.protocol), inner);
            self.presentation.prepare(frame.buffer_mut(), inner, false);
        } else {
            frame.render_widget(
                Paragraph::new(match &self.preview_error {
                    Some(error) => format!("Preview unavailable\n{}", clean(&error.to_string())),
                    None => "Loading preview…".into(),
                })
                .wrap(Wrap { trim: false }),
                inner,
            );
        }
        self.draw_conversation(frame, panes[2]);
        let key = |label: String| {
            Span::styled(
                label,
                Style::default().fg(theme::KEY).add_modifier(Modifier::BOLD),
            )
        };
        let description = |label| Span::styled(label, Style::default().fg(theme::SECONDARY));
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(self.status.clone()),
                Line::from(vec![
                    key(if self.focus == Focus::Nodes && self.diagnostics.active {
                        self.navigation.label(Nav::DiagnosticScope)
                    } else {
                        String::new()
                    }),
                    description(if self.focus == Focus::Nodes && self.diagnostics.active {
                        if self.diagnostics.selected_only {
                            " all nodes · "
                        } else {
                            " selected branch · "
                        }
                    } else {
                        ""
                    }),
                    key(if self.focus == Focus::Nodes {
                        self.navigation.label(Nav::ReferenceNode)
                    } else {
                        String::new()
                    }),
                    description(if self.focus == Focus::Nodes {
                        " reference · "
                    } else {
                        ""
                    }),
                    key(if self.focus == Focus::Nodes {
                        self.navigation.label(Nav::ToggleDiagnostics)
                    } else {
                        String::new()
                    }),
                    description(if self.focus == Focus::Nodes {
                        " Nodes/Diagnostics · "
                    } else {
                        ""
                    }),
                    key(format!(
                        "{} / {}",
                        self.navigation.label(Nav::NextPanel),
                        self.navigation.label(Nav::PreviousPanel)
                    )),
                    description(" panels · "),
                    key(self.navigation.label(Nav::Commands)),
                    description(" commands · "),
                    key(self.navigation.label(Nav::Help)),
                    description(" help · "),
                    key(self.navigation.label(Nav::Quit)),
                    description(" exits outside prompt · "),
                    key("Ctrl+C".into()),
                    description(" cancels/exits"),
                ]),
            ]))
            .wrap(Wrap { trim: false }),
            sections[2],
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
    editor.set_cursor_style(Style::default());
    editor.set_cursor_line_style(Style::default());
    editor.move_cursor(CursorMove::Bottom);
    editor.move_cursor(CursorMove::End);
    editor
}
fn panel(title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    let title = title.into();
    let style = Style::default()
        .fg(if focused { theme::FOCUS } else { theme::MUTED })
        .add_modifier(Modifier::BOLD)
        .patch(title.style);
    Block::default()
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .title(title.style(style))
        .border_style(Style::default().fg(if focused { theme::FOCUS } else { theme::FRAME }))
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
    options: crate::attachments::AttachmentOptions,
    cancel: &CancellationToken,
) -> Result<InputContent> {
    if let Some(path) = path {
        clipboard::read_image_with_options(executable, path, options, cancel)
    } else {
        clipboard::read_clipboard_with_options(executable, options, cancel)
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
