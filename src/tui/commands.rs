//! Shared workspace actions for the palette, slash commands and help.
use super::*;

pub(super) mod completion;
pub(super) mod palette;

#[derive(Clone, Copy, Debug)]
enum Requirement {
    Always,
    Idle,
    Checkpoints,
    LocalRead,
    ImageInput,
    ClipboardInput,
    PendingImages,
    ClearImages,
    CompletePreview,
    Playable,
    Pointer,
    NativeInput,
}

#[derive(Debug)]
pub(super) struct Command {
    pub action: Action,
    pub category: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub arguments: &'static str,
    min_args: usize,
    max_args: usize,
    requirement: Requirement,
}

macro_rules! commands {
    ($( $action:ident, $category:literal, $label:literal, $description:literal,
        $name:literal, [$($alias:literal),*], $arguments:literal, $min:literal, $max:literal, $requirement:ident; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(super) enum Action { $($action),* }

        pub(super) const COMMANDS: &[Command] = &[$(Command {
            action: Action::$action, category: $category, label: $label, description: $description,
            name: $name, aliases: &[$($alias),*], arguments: $arguments,
            min_args: $min, max_args: $max, requirement: Requirement::$requirement,
        }),*];
    };
}

commands! {
    Permissions, "Conversation", "Agent permissions…", "Choose the connected agent's permission mode and optionally save its default.",
        "permissions", ["mode"], "", 0, 0, Always;
    NewConversation, "Conversation", "New empty conversation…", "Save the current draft and start an empty local conversation.",
        "new-conversation", [], "", 0, 0, Idle;
    Logs, "Conversation", "Current conversation logs…", "Read saved turns of the current conversation without sending a request.",
        "logs", [], "", 0, 0, LocalRead;
    Conversations, "Conversation", "Saved conversations…", "Browse saved local conversations and choose one to continue.",
        "conversations", [], "", 0, 0, LocalRead;
    Queue, "Send queue", "Queue a follow-up…", "Defer this text until the current turn finishes instead of steering it.",
        "queue", [], "<text>", 1, 1, Always;
    ResumeQueue, "Send queue", "Resume queued prompts", "Resume saved or paused follow-ups.",
        "resume-queue", [], "", 0, 0, Always;
    ClearQueue, "Send queue", "Discard queued prompts", "Discard all queued prompts; keep the current draft and attachments.",
        "clear-queue", [], "", 0, 0, Always;
    Attach, "Images", "Attach image file…", "Attach a PNG, JPEG or WebP; quote paths containing spaces.",
        "attach", [], "<quoted path> [WIDTHxHEIGHT|original]", 1, 2, ImageInput;
    PasteImage, "Images", "Paste from clipboard…", "Read this host's clipboard; optionally choose the send size.",
        "paste-image", [], "[WIDTHxHEIGHT|original]", 0, 1, ClipboardInput;
    ResizeImage, "Images", "Resize pending image…", "Choose an image number from 1 and its send size; retain the original.",
        "resize-image", [], "<number> <WIDTHxHEIGHT|original>", 2, 2, PendingImages;
    ClearImages, "Images", "Remove pending images", "Remove all pending attachments without changing their source files.",
        "clear-images", [], "", 0, 0, ClearImages;
    Refresh, "Preview", "Refresh preview", "Refresh the document preview, including browser captures.",
        "refresh", [], "", 0, 0, Always;
    OpenPreview, "Preview", "Open preview externally", "Open the complete PNG in the external image viewer.",
        "open-preview", [], "", 0, 0, CompletePreview;
    Play, "Preview", "Play…", "Start page/shader playback at 1–10 fps (default: 5).",
        "play", [], "[fps]", 0, 1, Playable;
    Pause, "Preview", "Pause", "Stop requesting playback frames.",
        "pause", [], "", 0, 0, Always;
    Time, "Preview", "Seek shader time…", "Seek to a nonnegative shader time in seconds and pause playback.",
        "time", [], "<nonnegative seconds>", 1, 1, Always;
    ToggleNodes, "View", "Toggle Nodes panel", "Collapse or expand the Nodes panel.",
        "toggle-nodes", [], "", 0, 0, Always;
    ToggleAgent, "View", "Toggle Agent panel", "Collapse or expand the Agent panel.",
        "toggle-agent", [], "", 0, 0, Always;
    ToggleNodeNames, "View", "Toggle node names", "Show or hide root node names in Preview for this session.",
        "toggle-node-names", [], "", 0, 0, Always;
    Theme, "View", "Theme…", "Select auto, dark, light or monochrome UI colors for this session.",
        "theme", [], "[auto|dark|light|monochrome]", 0, 1, Always;
    ViewControls, "View", "View controls…", "Open view controls, including the temporary wheel pan/zoom switch.",
        "view-controls", [], "", 0, 0, Pointer;
    EnablePinch, "View", "Enable native gestures…", "Enable macOS pinch zoom and precise scrolling; grant Accessibility access if requested.",
        "enable-pinch", ["enable-native-input"], "", 0, 0, NativeInput;
    Checkpoints, "Checkpoints", "Browse and restore checkpoints…", "Browse committed edits; review a checkpoint before restoring it.",
        "checkpoints", ["history"], "", 0, 0, Checkpoints;
    Commands, "General", "Command palette", "Browse workspace actions by category.",
        "commands", [], "", 0, 0, Always;
    Help, "General", "Help", "Show help and available commands.",
        "help", [], "", 0, 0, Always;
    Quit, "General", "Quit", "Stop the conversation and exit.",
        "quit", [], "", 0, 0, Always;
}

impl Action {
    pub fn command(self) -> &'static Command {
        COMMANDS
            .iter()
            .find(|command| command.action == self)
            .unwrap()
    }
}

impl Command {
    pub fn slash_enabled(&self) -> bool {
        matches!(
            self.action,
            Action::Permissions
                | Action::NewConversation
                | Action::Logs
                | Action::Conversations
                | Action::Queue
                | Action::ResumeQueue
                | Action::ClearQueue
                | Action::Attach
                | Action::PasteImage
                | Action::ResizeImage
                | Action::ClearImages
                | Action::Help
        )
    }

    pub fn usage(&self) -> String {
        format!(
            "{}{}{}",
            if self.slash_enabled() {
                format!("/{}", self.name)
            } else {
                self.label.trim_end_matches('…').to_owned()
            },
            if self.arguments.is_empty() { "" } else { " " },
            self.arguments
        )
    }

    pub fn parse_arguments(&self, text: &str) -> Result<Vec<String>> {
        let args = if self.action == Action::Queue {
            if text.trim().is_empty() {
                vec![]
            } else {
                vec![text.trim_start().to_owned()]
            }
        } else {
            shell_words::split(text)
                .map_err(|error| Error::new("INVALID_ARGUMENT", error.to_string()))?
        };
        if !(self.min_args..=self.max_args).contains(&args.len()) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                format!("Usage: {}", self.usage()),
            ));
        }
        Ok(args)
    }
}

pub(super) fn parse(line: &str) -> Result<(&'static Command, Vec<String>)> {
    let (name, text) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let name = name.strip_prefix('/').unwrap_or(name);
    let command = COMMANDS
        .iter()
        .find(|command| command.name == name || command.aliases.contains(&name))
        .ok_or_else(|| {
            Error::new(
                "INVALID_ARGUMENT",
                "Unknown conversation command; use /help or the command palette",
            )
        })?;
    if !command.slash_enabled() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            format!(
                "Use the command palette: {} → {}",
                command.category, command.label
            ),
        ));
    }
    Ok((command, command.parse_arguments(text)?))
}

pub(super) fn help() -> String {
    let mut text = String::from("\nConversation commands\n");
    let mut category = "";
    for command in COMMANDS.iter().filter(|command| command.slash_enabled()) {
        if category != command.category {
            category = command.category;
            text.push_str(&format!("\n{category}\n"));
        }
        text.push_str(&format!("{}: {}\n", command.usage(), command.description));
        for alias in command.aliases {
            text.push_str(&format!("/{alias}: alias for /{}\n", command.name));
        }
    }
    text
}

pub(super) struct Input {
    pub action: Action,
    pub editor: TextArea<'static>,
    pub error: Option<String>,
}

impl Input {
    pub fn new(action: Action) -> Self {
        Self {
            action,
            editor: editor(""),
            error: None,
        }
    }

    pub fn insert(&mut self, text: &str) {
        let text = clean(text);
        if self.editor.lines().iter().map(String::len).sum::<usize>()
            + self.editor.lines().len()
            + text.len()
            > 1024 * 1024
        {
            self.error = Some("Command input exceeds 1 MiB".into());
            return;
        }
        self.editor.insert_str(text);
        self.error = None;
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        editing: bool,
        pointer: &mut mouse::Pointer,
    ) {
        let command = self.action.command();
        let shell = modal::Shell::draw(frame, area, "Commands", command.label, 2);
        shell.close(frame, pointer);
        shell.heading(frame, "Commands", command.label, false);
        shell.context(frame, &command.usage(), "Arguments");
        let colors = theme::colors();
        let body = shell.body;
        let description = Paragraph::new(command.description)
            .style(Style::default().fg(colors.secondary))
            .wrap(Wrap { trim: false });
        let description_rows = description
            .line_count(body.width)
            .min(usize::from(body.height.saturating_sub(4))) as u16;
        let sections = Layout::vertical([
            Constraint::Length(description_rows),
            Constraint::Length(u16::from(body.height >= 8)),
            Constraint::Min(3),
            Constraint::Length(u16::from(body.height >= 10)),
        ])
        .split(body);
        frame.render_widget(description, sections[0]);
        let hint = if self.action == Action::Theme {
            "Accepted values: auto, dark, light, monochrome.".to_string()
        } else {
            format!("Arguments: {}", command.arguments)
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(colors.secondary)),
            sections[3],
        );
        let input = sections[2];
        let regions = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(if self.error.is_some() {
                2.min(input.height.saturating_sub(3))
            } else {
                0
            }),
        ])
        .spacing(u16::from(self.error.is_some() && input.height >= 8))
        .split(input);
        let editor_area = regions[0];
        let error_area = regions[1];
        self.editor.set_style(
            Style::default()
                .fg(colors.text)
                .bg(colors.prompt_background),
        );
        self.editor.set_block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .border_style(Style::default().fg(colors.frame))
                .padding(ratatui::widgets::Padding::horizontal(1))
                .style(Style::default().bg(colors.prompt_background)),
        );
        self.editor.set_cursor_style(if editing {
            Style::default().add_modifier(Modifier::HIDDEN)
        } else {
            Style::default()
        });
        frame.render_widget(&self.editor, editor_area);
        let mut cursor = None;
        for y in editor_area.y..editor_area.bottom() {
            for x in editor_area.x..editor_area.right() {
                let cell = &mut frame.buffer_mut()[(x, y)];
                if cell.modifier.contains(Modifier::HIDDEN) {
                    cursor.get_or_insert((x, y));
                    cell.modifier.remove(Modifier::HIDDEN);
                }
            }
        }
        if let Some(cursor) = cursor {
            frame.set_cursor_position(cursor);
        }
        if let Some(error) = &self.error {
            frame.render_widget(
                Paragraph::new(error.as_str())
                    .style(Style::default().fg(colors.error))
                    .wrap(Wrap { trim: false }),
                error_area,
            );
        }
        let hints = Rect::new(
            shell.footer.x,
            shell.footer.y,
            shell.footer.width.saturating_sub(13),
            shell.footer.height,
        );
        frame.render_widget(
            Paragraph::new(vec![
                modal::hints(&[("Esc", "Back to commands")]),
                modal::hints(&[("Alt/Shift+Enter", "Newline")]),
            ]),
            hints,
        );
        modal::action(frame, shell.footer, "[Enter Run]", pointer);
    }
}

impl App {
    pub(super) fn command_unavailable(&self, command: &Command) -> Option<&'static str> {
        use Requirement::*;
        let busy = self.conversation.active()
            || self.conversation.modes().pending().is_some()
            || self.document_task.is_some();
        match command.requirement {
            Idle if busy || self.input.is_some() => {
                Some("Finish or cancel the active operation before selecting a conversation")
            }
            Checkpoints if busy => {
                Some("Wait for the active operation or cancel it before browsing checkpoints")
            }
            LocalRead if self.document_task.is_some() => {
                Some("Wait for the active local operation before reading saved logs")
            }
            ImageInput | ClipboardInput | PendingImages | ClearImages if self.input.is_some() => {
                Some("Finish or cancel the active image operation first")
            }
            PendingImages if self.pending_images().is_empty() => {
                Some("Attach an image before resizing it")
            }
            CompletePreview => self.export_unavailable().map(|(_, reason)| reason),
            Playable => self.playback_unavailable(),
            Pointer | NativeInput if !self.mouse.enabled => {
                Some("Pointer input is disabled in user settings")
            }
            NativeInput if !self.mouse.native_pinch && !self.mouse.native_scroll => {
                Some("Native gestures are disabled in user settings")
            }
            NativeInput
                if !cfg!(target_os = "macos") || std::env::var_os("SSH_CONNECTION").is_some() =>
            {
                Some("Native gestures require a local macOS session")
            }
            _ => None,
        }
    }

    pub(super) fn execute_command(
        &mut self,
        action: Action,
        args: &[String],
        worker: &Worker,
    ) -> Result<bool> {
        if let Some(reason) = self.command_unavailable(action.command()) {
            let code = if action == Action::OpenPreview {
                self.export_unavailable().expect("unavailable preview").0
            } else {
                "INVALID_ARGUMENT"
            };
            return Err(Error::new(code, reason));
        }
        match action {
            Action::Permissions => self.open_agent_permissions(),
            Action::NewConversation => self.pending_session = Some(saved::SessionChange::New),
            Action::Logs => {
                self.open_saved(saved::Page::current(self.conversation.id().into()), None)?
            }
            Action::Conversations => {
                self.open_saved(saved::Page::sessions(self.conversation.id().into()), None)?
            }
            Action::Queue => {
                self.enqueue_prompt(args[0].clone())?;
                self.set_status("Follow-up queued".into());
            }
            Action::ResumeQueue => self.outbox.paused = false,
            Action::ClearQueue => {
                self.clear_queued_prompts()?;
                self.set_status("Prompt queue cleared".into());
            }
            Action::Attach => self.start_input_with_bounds(
                Some(PathBuf::from(&args[0])),
                images::input_bounds(
                    args.get(1).map(String::as_str),
                    self.conversation.settings().image_bounds,
                )?,
            )?,
            Action::PasteImage => self.start_input_with_bounds(
                None,
                images::input_bounds(
                    args.first().map(String::as_str),
                    self.conversation.settings().image_bounds,
                )?,
            )?,
            Action::ResizeImage => {
                let index = args[0]
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .ok_or_else(|| {
                        Error::new(
                            "INVALID_ARGUMENT",
                            format!("Usage: {} (number starts at 1)", action.command().usage()),
                        )
                    })?;
                self.start_resize(index, images::input_bounds(Some(&args[1]), None)?)?;
            }
            Action::ClearImages => {
                self.clear_pending_images()?;
                self.request(worker, false, false);
            }
            Action::Refresh => self.request(worker, false, true),
            Action::OpenPreview => self.open_preview()?,
            Action::Play => {
                let fps = args
                    .first()
                    .map_or(Ok(playback::DEFAULT_FPS), |value| value.parse::<u32>())
                    .map_err(|_| {
                        Error::new("INVALID_ARGUMENT", "Use an integer frame rate from 1 to 10")
                    })?;
                self.start_playback(fps, Instant::now())?;
            }
            Action::Pause => self.pause_playback(),
            Action::Time => {
                let time = args[0].parse::<f64>().map_err(|_| {
                    Error::new(
                        "INVALID_ARGUMENT",
                        format!("Usage: {}", action.command().usage()),
                    )
                })?;
                self.view.set_time(time)?;
                self.playback.stop();
                self.request(worker, false, false);
            }
            Action::ToggleNodes => self.toggle_panel(Nav::ToggleNodesPanel, worker),
            Action::ToggleAgent => self.toggle_panel(Nav::ToggleAgentPanel, worker),
            Action::ToggleNodeNames => {
                self.preview_settings.show_node_names = !self.preview_settings.show_node_names;
                self.request(worker, false, false);
                self.set_status(format!(
                    "Node names: {}",
                    if self.preview_settings.show_node_names {
                        "on"
                    } else {
                        "off"
                    }
                ));
            }
            Action::Theme => {
                if let Some(name) = args.first() {
                    let theme =
                        <crate::config::keys::Theme as clap::ValueEnum>::from_str(name, false)
                            .map_err(|_| {
                                Error::new(
                                    "INVALID_ARGUMENT",
                                    format!("Usage: {}", action.command().usage()),
                                )
                            })?;
                    self.set_theme(theme, worker);
                } else {
                    self.set_status(format!(
                        "UI theme: {:?} · choose View → Theme in the command palette",
                        self.theme
                    ));
                }
            }
            Action::ViewControls => self.pointer.more = true,
            Action::EnablePinch => {
                let status = if self.native_input.start(true) {
                    "Native gestures enabled"
                } else if !self.native_input.authorized() {
                    "Native gestures need macOS Accessibility access; allow the requested app in System Settings and retry"
                } else {
                    "Cannot start native gesture input; restart the TUI after allowing Accessibility access"
                };
                self.set_status(status.into());
            }
            Action::Checkpoints => self.open_history(vec![None])?,
            Action::Commands => {
                self.command_filter.clear();
                self.modal = Some(checkpoints::Modal::commands());
            }
            Action::Help => self.show_help(),
            Action::Quit => return Ok(true),
        }
        if matches!(
            action,
            Action::Queue | Action::ResumeQueue | Action::ClearQueue
        ) {
            self.outbox.draft = Some(self.editor.lines().join("\n"));
            self.outbox.save(self.conversation.directory())?;
        }
        Ok(false)
    }
}
