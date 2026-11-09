//! Terminal viewing and prompting, sharing the same document and ACP services as CLI.

mod agent_status;
mod camera;
mod checkpoints;
mod commands;
mod composer;
mod controls;
mod diagnostics;
pub(crate) mod doctor;
pub mod documents;
mod events;
mod footer;
mod gestures;
mod graphics;
mod history;
mod images;
mod input;
mod markdown;
mod mouse;
mod nodes;
mod outbox;
mod permissions;
mod playback;
mod preview;
mod preview_search;
mod references;
mod saved;
mod search;
mod selection;
mod terminal;
#[cfg(test)]
mod test_artifacts;
#[cfg(test)]
mod tests;
mod theme;
mod viewport;
mod workspace;

use crate::config::keys::NavigationAction as Nav;
use crate::{
    Error, Result,
    clipboard::{self, InputContent},
    conversation::{Conversation, ConversationEvent},
    prompt::PromptArgs,
};
use agent_client_protocol::schema::v1::RequestPermissionRequest;
use clap::{Args, ValueEnum};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
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
#[command(
    mut_arg("file", |arg| arg.index(1).required(false).default_value(".").hide_default_value(true)),
    mut_arg("prompt", |arg| arg.index(2))
)]
pub struct TuiArgs {
    #[command(flatten)]
    pub prompt: PromptArgs,
    /// Start an empty conversation instead of restoring the previous one.
    #[arg(long, conflicts_with_all = ["session", "restart_session"])]
    pub new_conversation: bool,
    /// Select the image protocol; auto probes the terminal before reading input.
    #[arg(long, value_enum, default_value = "auto")]
    pub image_protocol: ImageProtocol,
    /// Override the user UI theme; auto follows the terminal background.
    #[arg(long, value_enum)]
    pub theme: Option<crate::config::keys::Theme>,
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
    fn next(self, backwards: bool, nodes_collapsed: bool, agent_collapsed: bool) -> Self {
        let current = if self.in_conversation() {
            Self::Prompt
        } else {
            self
        };
        let mut order = Vec::with_capacity(3);
        if !nodes_collapsed {
            order.push(Self::Nodes);
        }
        order.push(Self::Preview);
        if !agent_collapsed {
            order.push(Self::Prompt);
        }
        current.cycle(&order, backwards)
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
    run_with_selection(args, data, json_output, None)
}

pub fn run_with_selection(
    args: &TuiArgs,
    data: Option<PathBuf>,
    json_output: bool,
    selected: Option<documents::Selection>,
) -> Result<()> {
    let _pool = crate::native_memory::AutoreleasePool::new();
    let (prepared, startup) = match selected {
        Some(selected) => (selected.agent, Some(selected.terminal)),
        None => (None, None),
    };
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
    let (mut conversation, notice) =
        Conversation::open_tui(args.prompt.clone(), data, args.new_conversation)?;
    if let Some(host) = prepared {
        conversation.set_prepared_agent(host);
    }
    let mut app = App::new(conversation);
    app.apply_startup_prompt(&args.prompt.prompt)?;
    if let Some(notice) = notice {
        app.append(&format!("\n{notice}\n"));
    }
    app.attach_startup_images(&args.prompt.images)?;
    // Reserve two slots each for exact and temporary camera frames. The low
    // bit selects the slot; the high bit identifies temporary feedback.
    let id = (uuid::Uuid::new_v4().as_u128() as u32 & 0x7fff_fffe).max(2);
    let mut screen = match startup {
        Some(startup) => startup.activate(id)?,
        None => terminal::Screen::enter(
            args.image_protocol,
            id,
            settings.mouse.enabled,
            args.theme.unwrap_or(settings.theme),
        )?,
    };
    let worker = Worker::start(
        app.conversation.store().clone(),
        args.prompt.fonts.clone(),
        screen.picker.clone(),
        id,
    )?;
    worker.set_canvas_background(theme::canvas_background());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(terminal::io)?;
    let (result, session_id) = runtime.block_on(async {
        app.theme = args.theme.unwrap_or(settings.theme);
        app.terminal_background = screen.picker.terminal_background();
        app.keys = settings.keys;
        app.navigation = settings.navigation;
        app.panel_layout = settings.layout;
        app.preview_settings = settings.preview;
        app.mouse = settings.mouse;
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
    drop(app);
    drop(worker);
    drop(screen);
    runtime.shutdown_timeout(Duration::from_secs(1));
    eprintln!("Conversation: {session_id}");
    result
}

struct App {
    conversation: Conversation,
    theme: crate::config::keys::Theme,
    terminal_background: Option<[u8; 3]>,
    keys: crate::config::keys::PromptKeys,
    navigation: crate::config::keys::NavigationKeys,
    panel_layout: crate::config::keys::PanelLayout,
    preview_settings: crate::config::keys::PreviewSettings,
    mouse: crate::config::keys::MouseSettings,
    pointer: mouse::Pointer,
    selection: selection::Selection,
    native_input: gestures::Input,
    editor: TextArea<'static>,
    focus: Focus,
    nodes_collapsed: bool,
    agent_collapsed: bool,
    view: View,
    playback: playback::Playback,
    preview: Option<Preview>,
    preview_generation: Option<u64>,
    presentation: graphics::Presentation,
    input_profile: camera::InputProfile,
    preview_error: Option<Error>,
    // A retained frame stays stale during retries until a new frame succeeds.
    preview_stale: bool,
    nodes_loading: bool,
    pending_preview: Option<controls::Pending>,
    camera: Option<camera::Camera>,
    camera_deadline: Option<Instant>,
    camera_settle_deadline: Option<Instant>,
    camera_navigation: bool,
    camera_was_visible: bool,
    camera_id: u32,
    generation: u64,
    cells: Size,
    nodes: ListState,
    node_tree: nodes::Tree,
    diagnostics: diagnostics::View,
    attachments: ListState,
    reference_rows: ListState,
    filter: String,
    search_editor: search::Editor,
    searching: bool,
    preview_search: preview_search::Search,
    log: String,
    saved_log: String,
    saved_history: Option<crate::conversation::history::timeline::Reader>,
    saved_history_task: Option<history::Task>,
    history_requested: bool,
    tool_calls: std::collections::HashMap<String, composer::ToolDisplay>,
    turn_sequence: u64,
    history: Option<Paragraph<'static>>,
    history_metrics: (u16, u16, u16),
    scroll: u16,
    status: String,
    status_deadline: Option<Instant>,
    status_kind: footer::StatusKind,
    agent: agent_status::Status,
    connect_pending: bool,
    permissions: VecDeque<Permission>,
    permission_ui: Option<permissions::Dialog>,
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
    fn apply_startup_prompt(&mut self, prompt: &str) -> Result<()> {
        if !prompt.is_empty() {
            self.editor = editor(prompt);
            self.draft_changed = true;
            self.save_draft()?;
        }
        Ok(())
    }

    fn new(conversation: Conversation) -> Self {
        let editor = editor(conversation.draft());
        let agent = agent_status::Status::new(conversation.settings().fast == Some(true));
        let focus = if conversation.settings().create {
            Focus::Prompt
        } else {
            Focus::Preview
        };
        let mut app = Self {
            conversation,
            theme: Default::default(),
            terminal_background: None,
            keys: Default::default(),
            navigation: Default::default(),
            panel_layout: Default::default(),
            preview_settings: Default::default(),
            mouse: Default::default(),
            pointer: Default::default(),
            selection: Default::default(),
            native_input: Default::default(),
            editor,
            focus,
            nodes_collapsed: false,
            agent_collapsed: false,
            view: View::default(),
            playback: playback::Playback::default(),
            preview: None,
            preview_generation: None,
            presentation: Default::default(),
            input_profile: Default::default(),
            preview_error: None,
            preview_stale: false,
            nodes_loading: true,
            pending_preview: None,
            camera: None,
            camera_deadline: None,
            camera_settle_deadline: None,
            camera_navigation: false,
            camera_was_visible: false,
            camera_id: 0x8000_0001,
            generation: 0,
            cells: Size::new(0, 0),
            nodes: ListState::default(),
            node_tree: nodes::Tree::default(),
            diagnostics: diagnostics::View::default(),
            attachments: ListState::default(),
            reference_rows: ListState::default(),
            filter: String::new(),
            search_editor: Default::default(),
            searching: false,
            preview_search: Default::default(),
            log: String::new(),
            saved_log: String::new(),
            saved_history: None,
            saved_history_task: None,
            history_requested: false,
            tool_calls: Default::default(),
            turn_sequence: 0,
            history: None,
            history_metrics: (0, 0, 0),
            scroll: 0,
            status: String::new(),
            status_deadline: None,
            status_kind: footer::StatusKind::Info,
            agent,
            connect_pending: true,
            permissions: VecDeque::new(),
            permission_ui: None,
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
        app.restore_history();
        app
    }
    async fn event_loop(&mut self, screen: &mut terminal::Screen, worker: &Worker) -> Result<()> {
        let mut preview_control = crate::preview_control::Host::start()?;
        self.conversation.set_preview_control(
            preview_control.client(self.conversation.store().path(), self.conversation.id()),
        );
        let mut events = events::Events::new()?;
        if self.mouse.enabled && (self.mouse.native_pinch || self.mouse.native_scroll) {
            self.native_input.start(false);
        }
        screen.pointer_motion(self.native_input.active())?;
        self.camera_id = screen.camera_image_id();
        self.presentation.rectangular(
            screen.picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty
                && !screen.picker.tmux_detected(),
        );
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut termination = Box::pin(termination());
        loop {
            let _pool = crate::native_memory::AutoreleasePool::new();
            self.sync_turn(worker);
            self.permission_progress();
            self.load_older_history();
            if let Err(error) = self.send_queued().await {
                self.outbox.paused = true;
                self.error(error);
                self.dirty = true;
            }
            self.expire_preview_control();
            if self.connect_pending {
                self.connect_pending = false;
                if !self.conversation.active()
                    && let Err(error) = self.conversation.connect()
                {
                    self.agent.authentication = "unavailable".into();
                    self.agent.unavailable();
                    self.error(error);
                }
                self.dirty = true;
            }
            // Consume queued presses and releases before drawing. A slow image
            // transfer must not turn an input burst into one redraw per old key,
            // or let old key events repaint after an already queued release.
            for _ in 0..64 {
                let Some(event) = events.try_next() else {
                    break;
                };
                // A key can invalidate hit geometry while the next queued
                // click already belongs to the new view. Rebuild it first.
                if self.dirty
                    && (self.pointer.invalid() || self.pointer.scope != self.pointer_scope())
                    && event
                        .as_ref()
                        .is_ok_and(|batch| matches!(batch.event, Event::Mouse(_)))
                {
                    events.defer(event);
                    break;
                }
                if self
                    .terminal_event(event.map_err(terminal::io)?, screen, worker)
                    .await?
                {
                    self.save_draft()?;
                    return Ok(());
                }
            }
            let navigating = self.focus == Focus::Preview && !self.has_modal();
            screen.preview_keys(navigating)?;
            if self.dirty {
                let mut cells = self.cells;
                screen.draw(|frame| {
                    cells = self.draw(frame);
                })?;
                self.input_profile
                    .presented(self.presentation.serial(), self.camera.is_some());
                self.dirty = false;
                if cells != self.cells {
                    self.cells = cells;
                    self.request(worker, false, false);
                }
            }
            let playback_deadline = self.playback.deadline();
            let camera_deadline = self.camera_deadline;
            let camera_settle_deadline = self.camera_settle_deadline;
            let activity_deadline = (self.conversation.active() && !self.has_modal())
                .then(|| Instant::now() + Duration::from_millis(200));
            let gestures = self.native_input.shared.clone();
            tokio::select! {
                biased;
                output = worker.next_presentation() => {
                    match self.selection.accept_overlay(output) {
                        Ok(changed) => self.dirty |= changed,
                        Err(error) => self.set_preview_error_status(&error),
                    }
                },
                // Publish ready frames even while native gestures keep arriving.
                output = worker.next() => {
                    self.accept_interactive_preview(output, worker, &screen.picker);
                },
                event = gestures.next() => {
                    if let Err(error) = self.gesture_event(event, &screen.picker, worker) { self.error(error); }
                },
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
                event = events.next() => {
                    let Some(event) = event else { self.save_draft()?; return Ok(()); };
                    if self.terminal_event(event.map_err(terminal::io)?, screen, worker).await? {
                        self.save_draft()?;
                        return Ok(());
                    }
                },
                incoming = preview_control.next() => {
                    let incoming = incoming.ok_or_else(|| Error::new("PREVIEW_UNAVAILABLE", "Preview control listener stopped"))?;
                    self.receive_preview_control(incoming, worker, &screen.picker);
                    self.dirty = true;
                },
                _ = async {
                    if let Some(deadline) = camera_settle_deadline {
                        tokio::time::sleep_until(deadline.into()).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    self.finish_camera_gesture(worker);
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
                            self.set_preview_error_status(&error);
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
                        self.set_preview_error_status(&error);
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
                    if self.accept_prepared(content) { self.request(worker, false, false); }
                    self.dirty = true;
                },
                output = async { (&mut self.saved_history_task.as_mut().unwrap().receiver).await.unwrap_or_else(|error| Err(Error::new("IO_ERROR", format!("Conversation history worker stopped: {error}")))) }, if self.saved_history_task.is_some() => {
                    self.accept_history(output);
                },
                output = async { (&mut self.document_task.as_mut().unwrap().receiver).await.unwrap_or_else(|error| Err(Error::new("IO_ERROR", format!("Document worker stopped: {error}")))) }, if self.document_task.is_some() => {
                    let modal = self.has_modal();
                    self.finish_document_result(output, worker);
                    if modal && !self.has_modal() { screen.redraw(); }
                    self.dirty = true;
                },
                _ = timer.tick() => {
                    self.expire_status(Instant::now());
                    if self.draft_changed && self.last_draft_save.elapsed() >= Duration::from_secs(1) { self.save_draft()?; }
                    self.request(worker, true, false);
                },
            }
        }
    }
    async fn terminal_event(
        &mut self,
        batch: events::Batch,
        screen: &mut terminal::Screen,
        worker: &Worker,
    ) -> Result<bool> {
        match batch.event {
            Event::Key(key)
                if self.mouse.enabled
                    && key.code == KeyCode::Char(' ')
                    && self.focus == Focus::Preview
                    && !self.has_modal()
                    && key.modifiers.is_empty() =>
            {
                self.pointer.space = key.kind != KeyEventKind::Release;
            }
            Event::Key(key) if key.kind == KeyEventKind::Release => {
                if [
                    Nav::PanLeft,
                    Nav::PanRight,
                    Nav::PanUp,
                    Nav::PanDown,
                    Nav::ZoomIn,
                    Nav::ZoomOut,
                ]
                .into_iter()
                .any(|action| self.navigation.matches(action, key))
                {
                    self.finish_camera_gesture(worker);
                }
            }
            Event::Key(key) => {
                self.pointer.keyboard();
                self.pointer.space = false;
                if self.selection.dragging() {
                    self.selection.abort();
                }
                let modal = self.has_modal();
                match self.key(key, &screen.picker, worker).await {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    Err(error) if error.code == "RENDER_ERROR" => {
                        self.set_preview_error_status(&error)
                    }
                    Err(error) => self.error(error),
                }
                if modal && !self.has_modal() {
                    screen.redraw();
                }
                if self.focus != Focus::Preview || self.has_modal() {
                    self.finish_camera_gesture(worker);
                }
                screen.preview_keys(self.focus == Focus::Preview && !self.has_modal())?;
                screen.pointer_motion(self.native_input.active())?;
                self.pointer.clear();
                self.dirty = true;
            }
            Event::Mouse(event) => {
                let modal = self.has_modal();
                match self
                    .mouse_events(event, batch.count, &screen.picker, worker)
                    .await
                {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    Err(error) => self.error(error),
                }
                if modal && !self.has_modal() {
                    screen.redraw();
                }
                screen.preview_keys(self.focus == Focus::Preview && !self.has_modal())?;
                screen.pointer_motion(self.native_input.active())?;
            }
            Event::Paste(text)
                if self.permissions.is_empty()
                    && matches!(self.modal, Some(checkpoints::Modal::CommandInput(_))) =>
            {
                if let Some(checkpoints::Modal::CommandInput(input)) = &mut self.modal {
                    input.insert(&text);
                }
                self.dirty = true;
            }
            Event::Paste(text) if self.focus == Focus::Prompt && !self.has_modal() => {
                self.insert_text(&text);
                self.dirty = true;
            }
            Event::Paste(text)
                if (self.preview_search.editing || self.searching) && !self.has_overlay() =>
            {
                if self.preview_search.editing {
                    self.paste_preview_search(&text, &screen.picker, worker)?;
                } else {
                    let selected = self.selected_node().map(|node| node.id.clone());
                    self.search_editor.paste(&mut self.filter, &text);
                    self.search_nodes(selected.as_deref());
                }
                self.dirty = true;
            }
            Event::FocusLost => {
                self.native_input.cancel();
                self.selection.abort();
                self.pointer.cancel();
                self.refresh_selection_presentation(&screen.picker, worker)?;
                self.finish_camera_gesture(worker);
            }
            Event::Resize(_, _) => {
                self.native_input.cancel();
                self.selection.abort();
                self.pointer.clear();
                self.finish_camera_gesture(worker);
                self.presentation.invalidate();
                self.dirty = true;
            }
            _ => {}
        }
        Ok(false)
    }
    fn request(&mut self, worker: &Worker, check_only: bool, refresh: bool) {
        worker.set_show_node_names(self.preview_settings.show_node_names);
        if (check_only && self.camera_navigation) || self.cells.width == 0 || self.cells.height == 0
        {
            return;
        }
        if !check_only {
            self.camera_navigation = false;
            self.camera_settle_deadline = None;
            self.preview_error = None;
            self.camera_was_visible = self.camera.take().is_some();
            self.generation += 1;
        }
        let attachment = if self.focus == Focus::Attachments {
            self.attachments
                .selected()
                .and_then(|index| self.pending_images().get(index))
                .map(|image| (image.clone(), self.conversation.directory().to_path_buf()))
        } else {
            None
        };
        if refresh && attachment.is_none() {
            self.nodes_loading = true;
        }
        worker.set_selection(self.selection.render_state());
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
        self.camera_navigation = false;
        self.camera_settle_deadline = None;
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
                let selected = self.selected_node().map(|node| node.id.clone());
                let diagnostic = self.selected_diagnostic().cloned();
                if self.status_kind == footer::StatusKind::PreviewError {
                    self.clear_status();
                }
                self.preview_error = None;
                self.preview_stale = false;
                if !preview.attachment {
                    self.view = preview.view;
                    if !preview.time_dependent && !preview.live_pages {
                        self.playback.stop();
                    }
                }
                if preview.attachment && self.status.is_empty() {
                    self.set_status("Attachment preview".into());
                }
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
                self.preview_search
                    .refresh(&self.preview.as_ref().unwrap().nodes);
                self.selection.retain(&self.preview.as_ref().unwrap().nodes);
                let selected = selected
                    .filter(|id| {
                        self.preview
                            .as_ref()
                            .unwrap()
                            .nodes
                            .iter()
                            .any(|node| node.id == *id)
                    })
                    .or_else(|| self.selection.ids.first().cloned());
                if document {
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
                self.preview_stale = self.preview.is_some();
                if self.preview_stale {
                    self.selection.abort();
                } else {
                    self.selection.clear();
                    self.node_tree.rebuild(&[], &self.filter);
                    self.nodes.select(None);
                    self.refresh_diagnostics(None);
                }
                self.preview_generation = None;
                self.preview_error = Some(error.clone());
                self.nodes_loading = false;
                self.set_preview_error_status(&error);
            }
        }
        self.dirty = true;
        true
    }
    fn playback_unavailable(&self) -> Option<&'static str> {
        (!self.preview.as_ref().is_some_and(|preview| {
            !preview.attachment && (preview.time_dependent || preview.live_pages)
        })).then_some("The complete preview must contain a browser page or a shader with an @time uniform before playback can start")
    }
    fn start_playback(&mut self, fps: u32, now: Instant) -> Result<()> {
        if let Some(reason) = self.playback_unavailable() {
            return Err(Error::new("INVALID_ARGUMENT", reason));
        }
        self.playback.start(self.view.time, fps, now)?;
        self.clear_status();
        Ok(())
    }
    fn pause_playback(&mut self) {
        self.playback.stop();
        // Keep the viewport geometry stable while the requested frame finishes.
        // A new footer row would resize and cancel the retained page capture.
        self.dirty = true;
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
            self.outbox.draft = Some(text.clone());
            self.outbox.save(self.conversation.directory())?;
            if !self.conversation.active() {
                self.conversation
                    .set_pending_input(text, self.pending_images().to_vec())?;
            }
            self.draft_changed = false;
            self.last_draft_save = std::time::Instant::now();
        }
        Ok(())
    }
    fn sync_turn(&mut self, worker: &Worker) {
        if self.finish_turn() {
            self.request(worker, false, false);
        }
    }
    fn finish_turn(&mut self) -> bool {
        // Completion is recorded before the final frontend message is dequeued.
        // Synchronize once on the actual state transition, preserving any next draft.
        if self.turn_pending && !self.conversation.active() {
            self.permissions.clear();
            self.turn_pending = false;
            self.turn_started = None;
            let failed = self.conversation.result().is_some_and(Result::is_err);
            self.outbox.paused |= failed;
            let recovered_failure = failed && self.outbox.in_flight.is_some();
            self.finish_prompt(failed);
            if failed && !recovered_failure && self.editor.lines().iter().all(String::is_empty) {
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
            return true;
        }
        false
    }
    fn insert_text(&mut self, text: &str) -> bool {
        let text = clean(text);
        if self.editor.lines().iter().map(String::len).sum::<usize>()
            + self.editor.lines().len()
            + text.len()
            > 1024 * 1024
        {
            self.set_error_status("Prompt exceeds 1 MiB".into());
            return false;
        }
        self.editor.insert_str(text);
        self.draft_changed = true;
        true
    }
    fn error(&mut self, error: Error) {
        self.set_error_status(clean(&error.to_string()));
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
            Some("initialized") => {
                self.agent.initialized();
            }
            Some("update") => {
                let update = &value["notification"]["update"];
                match update["sessionUpdate"].as_str() {
                    Some("config_option_update") => {
                        self.update_agent_configuration(&update["configOptions"]);
                    }
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
                            tool.update(update);
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
                self.agent.authentication = self
                    .conversation
                    .authentication()
                    .map(|report| report.display())
                    .unwrap_or_else(|| {
                        value["authentication"]["source"]
                            .as_str()
                            .unwrap_or("not reported")
                            .into()
                    });
            }
            Some("mode_changed") => {
                self.update_agent_configuration(&value["response"]["configOptions"])
            }
            Some("permissions_changed") => {
                if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
                    self.set_error_status(clean(
                        error["message"]
                            .as_str()
                            .unwrap_or("Saving permission defaults failed"),
                    ));
                } else {
                    self.set_status(format!(
                        "Permissions changed to {}{}",
                        clean(value["mode"].as_str().unwrap_or("not confirmed")),
                        value["saved"]
                            .as_str()
                            .map(|saved| format!(" · {}", clean(saved)))
                            .unwrap_or_default()
                    ));
                }
            }
            Some("mode_error") => {
                self.outbox.paused = true;
                if let Err(error) = self.outbox.save(self.conversation.directory()) {
                    self.error(error);
                }
                self.set_error_status(clean(
                    value["error"]["message"]
                        .as_str()
                        .unwrap_or("Permission mode change failed"),
                ));
            }
            Some("ready") => {
                self.update_agent_configuration(&value["response"]["configOptions"]);
                if let Some(report) = self.conversation.authentication() {
                    self.agent.authentication = report.display();
                }
                if self.status_kind == footer::StatusKind::Info {
                    self.clear_status();
                }
            }
            Some("session_restarted") => {
                self.append("\nStarted a new native session with current document context and bounded local history. Previous model memory was not restored.\n");
            }
            Some("local_context") => {
                self.append("\nAdded current document context and bounded local history after an interrupted restart.\n");
            }
            Some("connection_error") => {
                self.agent.unavailable();
                self.error(Error::new(
                    "AGENT_ERROR",
                    value["error"]["message"]
                        .as_str()
                        .unwrap_or("Agent connection failed"),
                ));
            }
            Some("failed") => {
                self.append(&format!(
                    "\nRequest failed: {}\n",
                    value["error"]["message"]
                        .as_str()
                        .unwrap_or("Unknown error")
                ));
                self.set_error_status("Request failed; draft and attachments retained".into());
            }
            Some("complete") => {
                self.append("\nRequest complete\n");
            }
            Some("steering_result") => {
                if let Err(error) = self.steering_result(value) {
                    self.error(error);
                }
            }
            _ => {}
        }
    }
    fn toggle_panel(&mut self, action: Nav, worker: &Worker) {
        let was_attachment = self.focus == Focus::Attachments;
        match action {
            Nav::ToggleNodesPanel => {
                self.nodes_collapsed = !self.nodes_collapsed;
                if !self.nodes_collapsed {
                    self.focus = Focus::Nodes;
                } else if self.focus == Focus::Nodes {
                    self.focus = Focus::Preview;
                }
            }
            Nav::ToggleAgentPanel => {
                self.agent_collapsed = !self.agent_collapsed;
                if !self.agent_collapsed {
                    self.focus = Focus::Prompt;
                } else if self.focus.in_conversation() {
                    self.focus = Focus::Preview;
                }
            }
            _ => unreachable!(),
        }
        if self.focus != Focus::Nodes {
            self.searching = false;
        }
        if self.focus != Focus::Preview {
            self.preview_search.editing = false;
        }
        if was_attachment && self.focus != Focus::Attachments {
            self.request(worker, false, false);
        }
        self.dirty = true;
    }

    fn help_text(&self) -> String {
        let label = |action| self.navigation.label(action);
        let mut text = format!(
            "{} / {}: switch Nodes, Preview, Prompt (right / left)\n{}: command palette; /checkpoints: committed edits\nPrompt: {} sends; Alt+Enter / Shift+Enter inserts a newline\n{}: paste image, copied image files, or text\n",
            label(Nav::NextPanel),
            label(Nav::PreviousPanel),
            label(Nav::Commands),
            self.keys.submit_label(),
            self.keys.paste_label()
        );
        text.push_str(&format!(
            "{}: collapse/expand Nodes; {}: collapse/expand Agent\nTab skips collapsed panels; expanding focuses the panel\n",
            label(Nav::ToggleNodesPanel),
            label(Nav::ToggleAgentPanel)
        ));
        text.push_str(&format!(
            "{} / {}: switch Agent sections; Esc returns to Prompt\n",
            label(Nav::PreviousSection),
            label(Nav::NextSection)
        ));
        if self.mouse.enabled {
            text.push_str("Mouse/trackpad: click panels or rows to focus/select; scroll under the pointer without moving keyboard focus\nPreview: Space+left drag, middle drag or right drag pans; two-finger scrolling pans in both directions\n");
            text.push_str(if self.mouse.wheel_zoom { "Preview: wheel zooms; Ctrl+wheel pans; Shift+wheel pans horizontally\n" } else { "Preview: wheel pans; Ctrl+wheel zooms around pointer; Shift+wheel pans horizontally\n" });
            text.push_str("Menu: [N]/[A] collapse or restore panels; [Menu]/[...] exposes view actions\nNodes: triangle expands/collapses; Show fits; +Ref explicitly attaches a reference\nPrompt clicks focus input. Preview clicks select shapes; left drag selects a range; Shift adds/toggles.\n\n");
        }
        text.push_str("While working, submit text to steer a supported agent; unsupported adapters and node references queue for the next turn.\n");
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
                Nav::SelectChild,
                "Preview: select and fit the first enabled child",
            ),
            (Nav::SelectParent, "Preview: select and fit the parent"),
            (
                Nav::TimeBackward,
                "Preview: pause and step time back by 0.1s",
            ),
            (
                Nav::TimeForward,
                "Preview: pause and step time forward by 0.1s",
            ),
            (Nav::PlayPause, "Preview: toggle bounded playback"),
            (Nav::SearchPreview, "Preview: search text, names and IDs"),
            (Nav::NextSearchMatch, "Preview: next search result"),
            (Nav::PreviousSearchMatch, "Preview: previous search result"),
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
            (
                Nav::ActivateNode,
                "Nodes: expand or enter a branch; fit a node without children",
            ),
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
        text.push_str(&commands::help());
        text.push_str("\nSaved logs: arrows select/scroll; Enter opens; n/p pages\nu: select local session; N: new local conversation\nv: raw/formatted; Backspace returns; r refreshes; Esc closes\nCtrl+C: cancel active work; exit when idle\nDialogs keep their displayed keys; Esc closes help\nHelp: PgUp/PgDn or arrows scroll; Home/End jump\n\nClipboard access happens only on explicit paste. SSH reads the remote host's clipboard. Image protocols do not provide clipboard access.");
        text.push_str(&format!("\nDiagnostics: {} / {} select; {} reveals and fits the node\n{} / {} scroll details; {} returns to Nodes\n", label(Nav::PreviousItem), label(Nav::NextItem), label(Nav::ActivateNode), label(Nav::NodePageUp), label(Nav::NodePageDown), label(Nav::ClearSelection)));
        if self.mouse.enabled {
            text.push_str("Preview: click selects a shape; ordinary left drag selects fully enclosed shapes; Shift adds/toggles; Esc clears selection\nmacOS: Enable native gestures in the command palette (Accessibility access required once)\n");
        }
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
        let remember_error = self.conversation.remember_for_tui().err();
        self.modal = None;
        self.permission_ui = None;
        self.saved_return = None;
        self.restore_return = None;
        if !changed {
            self.set_status("This local conversation is already selected".into());
            if let Some(error) = remember_error {
                self.error(error);
            }
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
        self.agent = agent_status::Status::new(self.conversation.settings().fast == Some(true));
        self.connect_pending = true;
        self.log.clear();
        self.restore_history();
        self.tool_calls.clear();
        self.scroll = 0;
        self.set_status(format!(
            "Selected conversation {} · sends only on submit",
            self.conversation.id()
        ));
        self.append("\nSelected conversation. Saved history is loading; the session connects automatically. Your draft is sent only on submit.\n");
        if let Some(error) = remember_error {
            self.append(&format!(
                "\nCould not remember this conversation: {error}\n"
            ));
            self.error(error);
        }
        self.request(worker, false, false);
        Ok(())
    }

    async fn submit(&mut self) -> Result<()> {
        self.finish_turn();
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
        if self.queue_command()? {
            return Ok(());
        }
        if self.outbox.steering.is_some()
            && self.outbox.references.is_empty()
            && self.pending_images().is_empty()
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the previous mid-turn input's delivery receipt; your draft is retained",
            ));
        }
        if self.conversation.can_steer()
            && self.outbox.references.is_empty()
            && self.pending_images().is_empty()
        {
            return self.steer_prompt(self.editor.lines().join("\n"));
        }
        if self.conversation.active() || !self.outbox.queued.is_empty() {
            let text = self.editor.lines().join("\n");
            if !text.trim().is_empty() || !self.pending_images().is_empty() {
                self.queue_prompt(text)?;
            } else if self.conversation.active() {
                return Err(Error::new("INVALID_ARGUMENT", "Enter a follow-up prompt"));
            }
            if !self.conversation.active() {
                self.outbox.paused = false;
            }
            self.outbox.draft = Some(self.editor.lines().join("\n"));
            self.outbox.save(self.conversation.directory())?;
            self.draft_changed = false;
            self.clear_status();
            return self.send_queued().await;
        }
        self.queue_prompt(self.editor.lines().join("\n"))?;
        self.outbox.paused = false;
        self.outbox.save(self.conversation.directory())?;
        self.send_queued().await
    }
    async fn command(&mut self, worker: &Worker) -> Result<bool> {
        if self.queue_command()? {
            return Ok(false);
        }
        let line = self.editor.lines().join("\n");
        let (command, args) = commands::parse(&line)?;
        let exit = self.execute_command(command.action, &args, worker)?;
        // A new conversation must save the existing draft before switching.
        if self.pending_session.is_some() {
            self.finish_pending_session(worker).await?;
        } else {
            self.editor = editor("");
            self.draft_changed = true;
            self.save_draft()?;
        }
        Ok(exit)
    }
    fn set_theme(&mut self, theme: crate::config::keys::Theme, worker: &Worker) {
        self.theme = theme;
        theme::select(theme, self.terminal_background);
        self.history = None;
        worker.set_canvas_background(theme::canvas_background());
        self.selection.presentation = None;
        self.selection.invalidate();
        self.request(worker, false, false);
        self.dirty = true;
        self.set_status(format!(
            "UI theme: {theme:?} · session only; set [tui] theme in user config to keep it"
        ));
    }
    fn filtered_nodes(&self) -> &[nodes::Row] {
        self.node_tree.rows()
    }
    fn selected_node(&self) -> Option<&preview::Node> {
        let index = self.filtered_nodes().get(self.nodes.selected()?)?.index;
        self.preview.as_ref()?.nodes.get(index)
    }
    fn fit_selected(&mut self, picker: &graphics::Backend) -> Result<()> {
        let ids = if self.focus == Focus::Nodes || self.selection.ids.is_empty() {
            self.selected_node()
                .map(|node| node.id.clone())
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            self.selection.ids.clone()
        };
        let bounds = self
            .preview
            .as_ref()
            .and_then(|preview| {
                preview
                    .nodes
                    .iter()
                    .filter(|node| ids.contains(&node.id))
                    .map(|node| node.bounds)
                    .reduce(|bounds, next| bounds.union(next))
            })
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Select a node first"))?;
        self.view
            .fit(bounds, preview::pixel_size(picker, self.cells)?);
        self.selection.cancel();
        self.selection.ids = ids;
        Ok(())
    }
    fn export_unavailable(&self) -> Option<(&'static str, &'static str)> {
        if self.preview_stale {
            Some((
                "PREVIEW_UNAVAILABLE",
                "The previous preview is out of date; refresh successfully before exporting",
            ))
        } else if self.camera.is_some()
            || self.camera_deadline.is_some()
            || self.preview_generation != Some(self.generation)
        {
            Some((
                "INVALID_ARGUMENT",
                "The camera preview is updating; wait for the complete image before exporting",
            ))
        } else if let Some(preview) = &self.preview {
            (preview.summary["provisional"] == true).then_some((
                "ASSET_NOT_FOUND",
                "Resolve missing characters before exporting this provisional preview",
            ))
        } else {
            Some(("INVALID_ARGUMENT", "No complete preview is available"))
        }
    }
    fn open_preview(&mut self) -> Result<()> {
        if let Some((code, reason)) = self.export_unavailable() {
            return Err(Error::new(code, reason));
        }
        let preview = self.preview.as_ref().expect("exportable preview");
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
        self.set_status(format!("Opened {}", path.display()));
        Ok(())
    }
    fn agent_activity(&self) -> Span<'static> {
        let (label, color) = if !self.permissions.is_empty() {
            ("! Blocked", theme::colors().running)
        } else if self.conversation.active() {
            ("● Working", theme::colors().working)
        } else {
            ("○ Idle", theme::colors().secondary)
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
        } else if preview.view.time != self.view.time {
            " · finishing requested frame"
        } else {
            ""
        };
        Some(format!("Playback {:.3}s{playing}", preview.view.time))
    }

    fn draw(&mut self, frame: &mut Frame) -> Size {
        self.selection.bind(self.conversation.store().path());
        self.pointer.controls = self.mouse.enabled;
        self.pointer.begin();
        let scope = self.pointer_scope();
        if self.pointer.scope != scope {
            self.selection.abort();
            self.native_input.cancel();
            self.pointer.cancel();
            self.pointer.reset_modal_list();
            self.pointer.scope = scope;
        }
        self.expire_status(Instant::now());
        self.presentation.begin();
        let area = frame.area();
        frame.render_widget(Block::default().style(theme::base()), area);
        if area.width < 50 || area.height < 16 {
            self.native_input.cancel();
            self.pointer.clear();
            frame.render_widget(
                Paragraph::new(
                    "Resize the terminal to at least 50 columns × 16 rows. Ctrl+C exits when idle.",
                ),
                area,
            );
            self.presentation.finish(frame.buffer_mut(), true);
            return Size::new(0, 0);
        }
        let workspace = workspace::areas(
            area,
            &self.panel_layout,
            self.nodes_collapsed,
            self.agent_collapsed,
            self.footer_height(),
        );
        let mut header = workspace.header;
        if self.mouse.enabled
            && self.mouse.show_controls
            && self.header_line().width() + 17 <= usize::from(header.width)
        {
            header.width = header.width.saturating_sub(17);
        }
        frame.render_widget(Paragraph::new(self.header_line()), header);
        let panes = workspace.panes;
        self.pointer
            .add(panes[0], mouse::Target::Focus(Focus::Nodes));
        self.pointer
            .add(panes[1], mouse::Target::Focus(Focus::Preview));
        self.pointer
            .add(panes[2], mouse::Target::Focus(Focus::Prompt));
        if !self.nodes_collapsed {
            self.draw_nodes(frame, panes[0]);
        }
        let title = theme::title(
            if self.camera.is_some() { "↻" } else { "▣" },
            search::title(
                "Preview",
                &self.preview_search.query,
                self.preview_search.editing,
            ),
            if self.camera.is_some() {
                theme::colors().running
            } else {
                theme::colors().preview
            },
            false,
        );
        let title_width = title.width();
        let mut block = panel(title, self.focus == Focus::Preview);
        let zoom = self.preview_zoom_label();
        let preview_notice = if self.preview_stale {
            Some(" Showing previous preview ")
        } else if self
            .preview
            .as_ref()
            .is_some_and(|preview| preview.summary["provisional"] == true)
        {
            Some(" Missing glyphs · provisional layout ")
        } else {
            None
        };
        if let Some(notice) = preview_notice {
            block = block.title_top(
                Line::styled(notice, Style::default().fg(theme::colors().running)).right_aligned(),
            );
        }
        let zoom_width = zoom.as_ref().map_or(0, |zoom| zoom.len() + 3);
        if let Some(timeline) = self
            .preview_search
            .summary()
            .or_else(|| self.preview_timeline())
        {
            let width = usize::from(panes[1].width.saturating_sub(2)).saturating_sub(zoom_width);
            block = block.title_bottom(Line::styled(
                format!(" {timeline} ")
                    .chars()
                    .take(width)
                    .collect::<String>(),
                Style::default().fg(theme::colors().secondary),
            ));
        }
        if let Some(zoom) = zoom {
            block = block.title_bottom(
                Line::styled(
                    format!(" {zoom} "),
                    Style::default().fg(theme::colors().secondary),
                )
                .right_aligned(),
            );
        }
        let inner = block.inner(panes[1]);
        self.pointer.add(inner, mouse::Target::Canvas);
        frame.render_widget(block, panes[1]);
        if self.preview_search.editing && self.focus == Focus::Preview {
            let cursor = self
                .preview_search
                .editor
                .title_cursor(&self.preview_search.query, title_width);
            self.search_cursor(frame, panes[1], cursor);
        }
        if let Some(camera) = self
            .camera
            .as_ref()
            .filter(|camera| camera.generation == self.generation)
        {
            if let Some(image) = self.selection.presentation_for(self.view, self.cells) {
                self.presentation.render(frame.buffer_mut(), inner, image);
            } else {
                frame.render_widget(graphics::Image(&camera.image), inner);
                self.presentation.prepare(frame.buffer_mut(), inner, true);
            }
        } else if let Some(preview) = &self.preview {
            self.presentation.render(
                frame.buffer_mut(),
                inner,
                self.selection
                    .presentation_for(preview.view, preview.protocol.size())
                    .unwrap_or(&preview.protocol),
            );
        } else {
            let message = match &self.preview_error {
                Some(error) => Text::from(vec![
                    Line::styled(
                        "Preview unavailable",
                        Style::default()
                            .fg(theme::colors().error)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Line::styled(
                        clean(&error.to_string()),
                        Style::default().fg(theme::colors().secondary),
                    ),
                ]),
                None => Text::styled(
                    "Loading preview…",
                    Style::default().fg(theme::colors().secondary),
                ),
            };
            frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), inner);
        }
        if !self.agent_collapsed {
            self.draw_conversation(frame, panes[2]);
        }
        self.draw_footer(frame, workspace.footer);
        if self.mouse.enabled {
            self.draw_pointer_controls(frame, &workspace);
        }
        if self.help {
            self.pointer.overlay();
            let popup = centered(area, 100, 26);
            frame.render_widget(Clear, popup);
            let paragraph = Paragraph::new(self.help_text()).wrap(Wrap { trim: false });
            let block = dialog("Help");
            let inner = block.inner(popup);
            let sections =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            self.pointer.add(sections[0], mouse::Target::Help);
            let maximum = paragraph
                .line_count(sections[0].width)
                .saturating_sub(sections[0].height as usize)
                .min(u16::MAX as usize) as u16;
            self.help_scroll = self.help_scroll.min(maximum);
            frame.render_widget(block, popup);
            frame.render_widget(paragraph.scroll((self.help_scroll, 0)), sections[0]);
            frame.render_widget(
                Paragraph::new("PgUp/PgDn scroll · Esc closes")
                    .style(Style::default().fg(theme::colors().secondary)),
                sections[1],
            );
            self.pointer.button(
                frame,
                Rect::new(popup.right().saturating_sub(9), popup.y, 8, 1),
                "[Close]",
                mouse::Action::Close,
            );
        }
        self.draw_checkpoint_modal(frame);
        if let Some(permission) = self.permissions.front() {
            self.pointer.overlay();
            let popup = centered(area, 92, 26);
            frame.render_widget(Clear, popup);
            let block = dialog("Permission");
            let inner = block.inner(popup);
            frame.render_widget(block, popup);
            let details = serde_json::to_value(&permission.request.tool_call).unwrap_or_default();
            let details = crate::conversation::sanitize_event(&details);
            let details = clean(&serde_json::to_string_pretty(&details).unwrap_or_default());
            let paragraph = Paragraph::new(details)
                .style(Style::default().fg(theme::colors().secondary))
                .wrap(Wrap { trim: false });
            let title = clean(
                permission
                    .request
                    .tool_call
                    .fields
                    .title
                    .as_deref()
                    .unwrap_or("Agent requests permission"),
            );
            let title_rows = Paragraph::new(title.as_str())
                .wrap(Wrap { trim: false })
                .line_count(inner.width);
            let option_rows = permission
                .request
                .options
                .iter()
                .fold(title_rows, |rows, option| {
                    rows.saturating_add(
                        Paragraph::new(format!("  {}", clean(&option.name)))
                            .wrap(Wrap { trim: false })
                            .line_count(inner.width),
                    )
                })
                .min(u16::MAX as usize) as u16;
            let detail_rows = paragraph.line_count(inner.width).min(usize::from(
                inner.height.saturating_sub(option_rows.saturating_add(2)),
            )) as u16;
            let sections = Layout::vertical([
                Constraint::Length(detail_rows),
                Constraint::Length(1),
                Constraint::Length(option_rows),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);
            self.pointer
                .add(sections[0], mouse::Target::PermissionDetails);
            let max_scroll = paragraph
                .line_count(sections[0].width)
                .saturating_sub(usize::from(sections[0].height))
                .min(u16::MAX as usize) as u16;
            frame.render_widget(
                paragraph.scroll((permission.scroll.min(max_scroll), 0)),
                sections[0],
            );
            let mut y = sections[2]
                .y
                .saturating_add(title_rows.min(u16::MAX as usize) as u16);
            let mut lines = vec![Line::styled(
                title,
                Style::default().add_modifier(Modifier::BOLD),
            )];
            for (index, option) in permission.request.options.iter().enumerate() {
                let style = if index == permission.selected {
                    theme::permission_selected()
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
                sections[2],
            );
            frame.render_widget(
                Paragraph::new(
                    "↑/↓ choose · Enter respond · Esc denies · p permissions · PgUp/PgDn details",
                )
                .style(Style::default().fg(theme::colors().secondary)),
                sections[4],
            );
            // Match wrapped option labels rather than assuming one row each.
            for (index, option) in permission.request.options.iter().enumerate() {
                let height = Paragraph::new(format!("  {}", clean(&option.name)))
                    .wrap(Wrap { trim: false })
                    .line_count(sections[2].width) as u16;
                let height = height.min(sections[2].bottom().saturating_sub(y));
                let option_area = Rect::new(sections[2].x, y, sections[2].width, height);
                if index == permission.selected {
                    frame
                        .buffer_mut()
                        .set_style(option_area, theme::permission_selected());
                }
                self.pointer
                    .add(option_area, mouse::Target::PermissionChoice(index));
                y = y.saturating_add(height);
            }
            let permissions_button = Rect::new(popup.right().saturating_sub(27), popup.y, 16, 1);
            frame.render_widget(
                Paragraph::new("[Permissions…]")
                    .style(Style::default().fg(theme::colors().preview)),
                permissions_button,
            );
            self.pointer.add(
                permissions_button,
                mouse::Target::AgentPermissions(permissions::Hit::Continue),
            );
            self.pointer.button(
                frame,
                Rect::new(popup.right().saturating_sub(9), popup.y, 8, 1),
                "[Close]",
                mouse::Action::Close,
            );
            self.pointer.button(
                frame,
                Rect::new(
                    popup.right().saturating_sub(11),
                    popup.bottom().saturating_sub(1),
                    10,
                    1,
                ),
                "[Respond]",
                mouse::Action::Accept,
            );
        }
        self.draw_agent_permissions(frame);
        self.pointer.finish();
        self.native_input.retain_canvas(self.gesture_canvas());
        // Keep native preview pixels outside the command palette's popup visible.
        let command_palette = matches!(self.modal, Some(checkpoints::Modal::Commands { .. }));
        let modal = self.help
            || (self.modal.is_some() && !command_palette)
            || self.permission_ui.is_some()
            || !self.permissions.is_empty();
        let overlay = if command_palette {
            Some(commands::palette::area(area))
        } else {
            self.pointer
                .more
                .then(|| Self::pointer_menu_area(workspace.header))
        };
        self.presentation
            .finish_with_overlay(frame.buffer_mut(), modal, overlay);
        Size::new(inner.width, inner.height)
    }
}

fn editor(text: &str) -> TextArea<'static> {
    let mut editor = TextArea::new(text.split('\n').map(str::to_string).collect());
    editor.set_wrap_mode(ratatui_textarea::WrapMode::WordOrGlyph);
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
        .fg(if focused {
            theme::colors().focus
        } else {
            theme::colors().muted
        })
        .add_modifier(Modifier::BOLD)
        .patch(title.style);
    Block::default()
        .style(theme::base())
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .title(title.style(style))
        .border_style(Style::default().fg(if focused {
            theme::colors().focus
        } else {
            theme::colors().frame
        }))
}
fn dialog(title: impl Into<Line<'static>>) -> Block<'static> {
    panel(title, true).padding(ratatui::widgets::Padding::horizontal(1))
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
