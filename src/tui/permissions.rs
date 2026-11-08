//! Permission-mode picker, reviewed persistence and turn-boundary changes.
use super::*;
use crate::{config::mode::Prepared, conversation::modes::Scope};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    Row(usize),
    Scope(usize),
    Continue,
    Back,
    Close,
    CancelPending,
    Decision(bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Modes,
    Scope,
    Review,
    Full,
    Busy,
    Waiting,
    Error,
}
pub(super) struct Dialog {
    step: Step,
    selected: usize,
    chosen: Option<crate::conversation::modes::Mode>,
    scope: usize,
    state: ListState,
    pub(super) scroll: u16,
    focus: usize,
    reviewed: Option<Prepared>,
    full_confirmed: bool,
    error: Option<String>,
}
impl Dialog {
    pub(super) fn identity(&self) -> String {
        format!("agent-permissions:{:?}:{}", self.step, self.scope)
    }
}
impl Default for Dialog {
    fn default() -> Self {
        Self {
            step: Step::Modes,
            selected: 0,
            chosen: None,
            scope: 0,
            state: ListState::default(),
            scroll: 0,
            focus: 0,
            reviewed: None,
            full_confirmed: false,
            error: None,
        }
    }
}
const SCOPES: [Scope; 3] = [Scope::Conversation, Scope::Project, Scope::Global];

fn facts(agent: crate::prompt::AgentName, id: &str, description: Option<&str>) -> String {
    use crate::prompt::AgentName;
    match (agent,id) {
        (AgentName::Codex,"read-only") => "File changes and network access require approval.\nDesign tools remain available through lazypencil MCP.",
        (AgentName::Codex,"workspace-write") => "Edit workspace files and run local commands.\nAdditional approval requests go to you.",
        (AgentName::Codex,"agent") => "Edit workspace files and run local commands.\nAdditional approval requests go to an automatic reviewer.\nSome actions may still stop for your decision.",
        (AgentName::Codex,"agent-full-access") => "Unrestricted computer and network access.\nDisables the usual sandbox and approval prompts.",
        (AgentName::Claude,"default") => "Manual approval for changes and command execution.\nDesign tools retain their separate MCP permissions.",
        (AgentName::Claude,"acceptEdits") => "Automatically accept file edits.\nShell and Git commands can still require approval.",
        (AgentName::Claude,"auto") => "Claude reviews actions with its classifier.\nSome operations can still require your decision.",
        (AgentName::Claude,"plan") => "Explore and plan before implementing changes.",
        (AgentName::Claude,"bypassPermissions") => "Bypass the usual permission checks.\nSome protected operations and interactive requests remain.",
        _ => description.unwrap_or("Permissions are controlled by the connected adapter."),
    }.into()
}
fn meaningful(value: &Value) -> Value {
    match value {
        Value::Object(items) => Value::Object(
            items
                .iter()
                .filter(|(_, v)| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
                .map(|(k, v)| (k.clone(), meaningful(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}
impl App {
    pub(super) fn open_agent_permissions(&mut self) {
        self.help = false;
        self.pointer.more = false;
        self.modal = None;
        let modes = self.conversation.modes();
        let selected = modes
            .available
            .iter()
            .position(|mode| Some(mode.id.as_str()) == modes.current.as_deref())
            .unwrap_or(0);
        self.permission_ui = Some(Dialog {
            selected,
            ..Dialog::default()
        });
        self.dirty = true;
    }
    pub(super) fn permission_progress(&mut self) {
        let modes = self.conversation.modes();
        let waiting = self
            .permission_ui
            .as_ref()
            .is_some_and(|ui| ui.step == Step::Waiting);
        if modes.error.is_some() {
            if waiting {
                self.permission_ui.as_mut().expect("dialog").step = Step::Error;
            }
            if !self.outbox.paused {
                self.outbox.paused = true;
                if let Err(error) = self.outbox.save(self.conversation.directory()) {
                    self.error(error);
                }
            }
        } else if waiting && modes.pending().is_none() {
            let status = format!(
                "Permissions changed to {}{}",
                modes.current.as_deref().unwrap_or("not reported"),
                modes
                    .saved
                    .as_ref()
                    .map(|saved| format!(" · {saved}"))
                    .unwrap_or_default()
            );
            self.permission_ui = None;
            self.set_status(status);
            self.dirty = true;
        }
    }
    fn begin_permission_change(&mut self, mut ui: Dialog) -> Result<()> {
        let mode = ui
            .chosen
            .as_ref()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Choose an available permission mode"))?
            .id
            .clone();
        if matches!(mode.as_str(), "agent-full-access" | "bypassPermissions") && !ui.full_confirmed
        {
            ui.step = Step::Full;
            ui.focus = 0;
            ui.scroll = 0;
            self.permission_ui = Some(ui);
            return Ok(());
        }
        if self.conversation.active() && ui.step != Step::Busy {
            ui.step = Step::Busy;
            ui.focus = 0;
            ui.scroll = 0;
            self.permission_ui = Some(ui);
            return Ok(());
        }
        match self.conversation.change_mode(mode, ui.reviewed.take()) {
            Ok(()) => {
                ui.step = Step::Waiting;
                ui.scroll = 0;
                ui.error = None;
            }
            Err(error) => {
                ui.error = Some(clean(&error.to_string()));
                ui.step = Step::Error;
            }
        }
        self.permission_ui = Some(ui);
        self.set_attention_status(
            if self.conversation.active() {
                "Permission change scheduled after this turn"
            } else {
                "Applying permission change…"
            }
            .into(),
        );
        self.dirty = true;
        Ok(())
    }
    pub(super) fn permission_click(&mut self, mut hit: Hit) -> Result<()> {
        let Some(mut ui) = self.permission_ui.take() else {
            return Ok(());
        };
        ui.error = None;
        if let Hit::Decision(allow) = hit {
            ui.focus = usize::from(allow);
            hit = Hit::Continue;
        }
        match hit {
            Hit::Close => {
                self.dirty = true;
                return Ok(());
            }
            Hit::CancelPending => {
                let result = self.conversation.cancel_mode_change();
                if let Err(error) = result {
                    ui.error = Some(clean(&error.to_string()));
                } else {
                    ui.step = Step::Modes;
                }
            }
            Hit::Back => {
                ui.step = match ui.step {
                    Step::Scope => Step::Modes,
                    Step::Review | Step::Full | Step::Busy => Step::Scope,
                    _ => Step::Modes,
                };
                ui.focus = 0;
                ui.scroll = 0;
                ui.full_confirmed = false;
            }
            Hit::Row(index) => {
                ui.selected = index;
                ui.scroll = 0;
                ui.full_confirmed = false;
            }
            Hit::Scope(index) => {
                ui.scope = index;
                ui.reviewed = None;
                ui.scroll = 0;
            }
            Hit::Decision(_) => unreachable!(),
            Hit::Continue => match ui.step {
                Step::Modes => {
                    if self.conversation.modes().available.is_empty() {
                        self.connect_pending = true;
                        ui.error =
                            Some("Connecting to the adapter; no prompt will be sent.".into());
                    } else if self.conversation.modes().pending().is_some() {
                        ui.error = Some("Cancel the pending permission change first.".into());
                    } else {
                        ui.chosen = self
                            .conversation
                            .modes()
                            .available
                            .get(ui.selected)
                            .cloned();
                        ui.step = Step::Scope;
                        ui.scope = 0;
                        ui.focus = 0;
                        ui.scroll = 0;
                    }
                }
                Step::Scope => {
                    let mode = ui
                        .chosen
                        .as_ref()
                        .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Choose a mode again"))?;
                    match self.conversation.review_mode(&mode.id, SCOPES[ui.scope]) {
                        Ok(Some(review)) => {
                            ui.reviewed = Some(review);
                            ui.step = Step::Review;
                            ui.focus = 0;
                            ui.scroll = 0;
                        }
                        Ok(None) => return self.begin_permission_change(ui),
                        Err(error) => ui.error = Some(clean(&error.to_string())),
                    }
                }
                Step::Review => return self.begin_permission_change(ui),
                Step::Full => {
                    if ui.focus == 0 {
                        ui.step = Step::Scope;
                    } else {
                        ui.full_confirmed = true;
                        return self.begin_permission_change(ui);
                    }
                }
                Step::Busy => {
                    if ui.focus == 0 {
                        ui.step = Step::Scope;
                    } else {
                        return self.begin_permission_change(ui);
                    }
                }
                Step::Waiting => {
                    if !self.conversation.modes().changing() {
                        self.conversation.cancel_mode_change()?;
                        ui.step = Step::Modes;
                    }
                }
                Step::Error => {
                    if ui.focus == 0 {
                        ui.step = Step::Modes;
                        ui.scroll = 0;
                    } else {
                        ui.full_confirmed = false;
                        let mode = ui
                            .chosen
                            .as_ref()
                            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Choose a mode again"))?;
                        match self.conversation.review_mode(&mode.id, SCOPES[ui.scope]) {
                            Ok(Some(review)) => {
                                ui.reviewed = Some(review);
                                ui.step = Step::Review;
                                ui.focus = 0;
                                ui.scroll = 0;
                            }
                            Ok(None) => return self.begin_permission_change(ui),
                            Err(error) => ui.error = Some(clean(&error.to_string())),
                        }
                    }
                }
            },
        }
        self.permission_ui = Some(ui);
        self.dirty = true;
        Ok(())
    }
    pub(super) fn permission_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(ui) = self.permission_ui.as_mut() else {
            return Ok(());
        };
        if key.code == KeyCode::Esc {
            return self.permission_click(Hit::Close);
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if ui.step == Step::Modes => {
                ui.selected = ui.selected.saturating_sub(1);
                ui.scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') if ui.step == Step::Modes => {
                ui.selected = (ui.selected + 1)
                    .min(self.conversation.modes().available.len().saturating_sub(1));
                ui.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') if ui.step == Step::Scope => {
                ui.scope = ui.scope.saturating_sub(1);
                ui.scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') if ui.step == Step::Scope => {
                ui.scope = (ui.scope + 1).min(2);
                ui.scroll = 0;
            }
            KeyCode::PageDown => ui.scroll = ui.scroll.saturating_add(5),
            KeyCode::PageUp => ui.scroll = ui.scroll.saturating_sub(5),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                ui.focus = (ui.focus + 1) % 2
            }
            KeyCode::Enter => return self.permission_click(Hit::Continue),
            KeyCode::Backspace => return self.permission_click(Hit::Back),
            KeyCode::Char('c') if self.conversation.modes().pending().is_some() => {
                return self.permission_click(Hit::CancelPending);
            }
            _ => {}
        }
        self.dirty = true;
        Ok(())
    }
    pub(super) fn draw_agent_permissions(&mut self, frame: &mut Frame) {
        let Some(ui) = self.permission_ui.as_mut() else {
            return;
        };
        self.pointer.overlay();
        let colors = theme::colors();
        // Match the palette's dimmed workspace while retaining native TUI controls.
        for cell in frame.buffer_mut().content.iter_mut() {
            cell.set_fg(colors.muted);
        }
        let popup = centered(frame.area(), 126, 32);
        frame.render_widget(Clear, popup);
        let title = match ui.step {
            Step::Modes => "Agent permissions",
            Step::Scope => "Apply permissions",
            Step::Review => "Review permission defaults",
            Step::Full => "Allow full access?",
            Step::Busy => "Change permissions after this turn",
            Step::Waiting => "Permission change pending",
            Step::Error => "Permissions could not be changed",
        };
        let block = dialog(title);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let parts = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(inner);
        let modes = self.conversation.modes();
        let current = modes.current.as_deref().unwrap_or("not confirmed");
        frame.render_widget(
            Paragraph::new(format!(
                "{:?} · Current: {} · Source: {}",
                self.conversation.settings().agent,
                clean(current),
                self.conversation.mode_source()
            ))
            .style(Style::default().fg(colors.secondary))
            .wrap(Wrap { trim: false }),
            parts[0],
        );
        let selected = if ui.step == Step::Modes {
            modes.available.get(ui.selected)
        } else {
            ui.chosen.as_ref()
        };
        let mode = selected.map(|m| m.id.as_str()).unwrap_or("not available");
        let body = parts[1];
        let mut details = String::new();
        let mut details_area = body;
        if matches!(ui.step, Step::Modes | Step::Scope) {
            let count = if ui.step == Step::Modes {
                modes.available.len()
            } else {
                3
            };
            let (list, detail) = if body.width >= 100 {
                let split =
                    Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
                        .spacing(2)
                        .split(body);
                (split[0], split[1])
            } else {
                let split = Layout::vertical([
                    Constraint::Length(
                        (count * 2).min(usize::from(body.height.saturating_sub(4))) as u16
                    ),
                    Constraint::Min(3),
                ])
                .spacing(1)
                .split(body);
                (split[0], split[1])
            };
            details_area = detail;
            let choices: Vec<ListItem> = if ui.step == Step::Modes {
                modes
                    .available
                    .iter()
                    .map(|mode| {
                        ListItem::new(vec![
                            Line::from(clean(&mode.name)),
                            Line::styled(
                                format!(
                                    "  {}{}",
                                    clean(&mode.id),
                                    if Some(mode.id.as_str()) == modes.current.as_deref() {
                                        " · Current"
                                    } else {
                                        ""
                                    }
                                ),
                                Style::default().fg(colors.secondary),
                            ),
                        ])
                    })
                    .collect()
            } else {
                SCOPES
                    .iter()
                    .map(|scope| ListItem::new(vec![Line::from(scope.label()), Line::default()]))
                    .collect()
            };
            let chosen = if ui.step == Step::Modes {
                ui.selected
            } else {
                ui.scope
            };
            ui.state.select(Some(chosen));
            frame.render_stateful_widget(
                List::new(choices)
                    .highlight_style(theme::modal_selected())
                    .highlight_symbol("› "),
                list,
                &mut ui.state,
            );
            for index in ui.state.offset()..count {
                let y = list.y + ((index - ui.state.offset()) * 2) as u16;
                if y >= list.bottom() {
                    break;
                }
                self.pointer.add(
                    Rect::new(list.x, y, list.width, 2.min(list.bottom() - y)),
                    mouse::Target::AgentPermissions(if ui.step == Step::Modes {
                        Hit::Row(index)
                    } else {
                        Hit::Scope(index)
                    }),
                );
            }
            if ui.step == Step::Modes {
                if let Some(mode) = selected {
                    details = format!(
                        "{}\n{}\n\n{}\n\nWorking directory\n{}",
                        clean(&mode.name),
                        clean(&mode.id),
                        facts(
                            self.conversation.settings().agent,
                            &mode.id,
                            mode.description.as_deref()
                        ),
                        clean(
                            &self
                                .conversation
                                .store()
                                .path()
                                .parent()
                                .unwrap_or(Path::new("."))
                                .display()
                                .to_string()
                        )
                    );
                } else {
                    details="No modes are available from this adapter.\nEnter reconnects without sending a prompt.".into();
                }
                if body.width < 100
                    && let Some(mode) = selected
                {
                    details = format!(
                        "{}\n\nWorking directory\n{}",
                        facts(
                            self.conversation.settings().agent,
                            &mode.id,
                            mode.description.as_deref()
                        ),
                        clean(
                            &self
                                .conversation
                                .store()
                                .path()
                                .parent()
                                .unwrap_or(Path::new("."))
                                .display()
                                .to_string()
                        )
                    );
                }
                if let Some(error) = &modes.error {
                    details.push_str(&format!("\n\n{}", clean(&error.message)));
                }
                if modes.current.is_none() && self.conversation.settings().mode.is_some() {
                    details.push_str(&format!(
                        "\n\nConfigured: {}",
                        clean(
                            self.conversation
                                .settings()
                                .mode
                                .as_deref()
                                .unwrap_or_default()
                        )
                    ));
                }
                if let Some(pending) = modes.pending() {
                    details.push_str(&format!(
                        "\n\nPending: {} · c cancels before applying",
                        clean(pending)
                    ));
                }
            } else {
                details = format!(
                    "Selected mode: {}\n\nApply to this conversation.\n{}\n\nSaved defaults follow:\n--mode > project > global > built-in.\nThis explicit TUI selection overrides the current launch.",
                    clean(mode),
                    match SCOPES[ui.scope] {
                        Scope::Conversation =>
                            "Saved defaults stay unchanged. Closing this TUI clears the temporary selection.",
                        Scope::Project =>
                            "Save for future launches in this project.\nReview and trust all project agent settings next.",
                        Scope::Global =>
                            "Save for future launches across projects.\nProject settings and --mode still take precedence.",
                    }
                );
            }
        } else {
            match ui.step {
                Step::Review => {
                    if let Some(review) = &ui.reviewed {
                        details = format!(
                            "FILE\n{}\n\nApply {} to this conversation and save the default.\n{}\n\nAGENT SETTINGS AFTER THIS CHANGE\n{}",
                            clean(&review.path().display().to_string()),
                            clean(mode),
                            if review.scope() == Scope::Project {
                                "Trust applies to all agent settings shown below, for this project only.\nChanged settings require a new review."
                            } else {
                                "Global defaults can be overridden by project settings or --mode."
                            },
                            serde_json::to_string_pretty(&meaningful(review.agent_settings()))
                                .unwrap_or_default()
                        );
                    }
                }
                Step::Full => {
                    details = format!(
                        "Full access to this computer\n\nSelected: {}\n\n{}\n\nApplies to: {}\n\nKeep the current mode to retain its existing restrictions.",
                        clean(mode),
                        facts(
                            self.conversation.settings().agent,
                            mode,
                            selected.and_then(|m| m.description.as_deref())
                        ),
                        SCOPES[ui.scope].label()
                    )
                }
                Step::Busy => {
                    details = format!(
                        "Current turn: {}\nAfter this turn: {}\n\nThe current task keeps its permissions.\nChanging the mode does not approve pending requests.\n\nQueued prompts wait for the adapter to confirm the change.",
                        clean(current),
                        clean(mode)
                    )
                }
                Step::Waiting => {
                    details = format!(
                        "Current: {}\nPending: {}\n\n{}\n\nYour draft stays unsent. Queued prompts wait.\nChanging this mode does not approve pending tool requests.",
                        clean(current),
                        clean(modes.pending().unwrap_or(mode)),
                        if modes.saving() {
                            "Saving the reviewed permission default."
                        } else if modes.changing() {
                            "Waiting for the adapter to confirm the permission mode."
                        } else {
                            "Waiting for this turn to end. Press c to cancel the change."
                        }
                    )
                }
                Step::Error => {
                    details = format!(
                        "Requested: {}\nStill reported: {}\n\n{}\n\nYour draft is preserved. Queued prompts are paused.\nEnter returns to the available modes.",
                        clean(mode),
                        clean(current),
                        clean(
                            ui.error
                                .as_deref()
                                .or_else(|| modes.error.as_ref().map(|e| e.message.as_str()))
                                .unwrap_or("The permission change was not confirmed.")
                        )
                    )
                }
                _ => {}
            }
        }
        let paragraph = Paragraph::new(clean(&details))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(if ui.step == Step::Full {
                colors.running
            } else {
                colors.text
            }));
        let max = paragraph
            .line_count(details_area.width)
            .saturating_sub(usize::from(details_area.height))
            .min(u16::MAX as usize) as u16;
        ui.scroll = ui.scroll.min(max);
        frame.render_widget(paragraph.scroll((ui.scroll, 0)), details_area);
        self.pointer
            .add(details_area, mouse::Target::AgentPermissionDetails);
        if let Some(error) = &ui.error {
            frame.render_widget(
                Paragraph::new(error.as_str())
                    .style(Style::default().fg(colors.running))
                    .wrap(Wrap { trim: false }),
                parts[2],
            );
        } else {
            frame.render_widget(
                Paragraph::new(
                    if matches!(ui.step, Step::Full | Step::Busy | Step::Error) {
                        "Tab Choose action · Enter Confirm · Esc Close"
                    } else {
                        "↑ ↓ Choose · PgUp/PgDn Details · Backspace Back · Esc Close"
                    },
                )
                .style(Style::default().fg(colors.secondary))
                .wrap(Wrap { trim: false }),
                parts[2],
            );
        }
        let footer = parts[3];
        let label = match ui.step {
            Step::Full | Step::Busy if ui.focus == 0 => "[Keep current]",
            Step::Full => "[Enable full access]",
            Step::Busy => "[Apply after turn]",
            Step::Review => {
                if ui
                    .reviewed
                    .as_ref()
                    .is_some_and(|r| r.scope() == Scope::Project)
                {
                    "[Trust & apply]"
                } else {
                    "[Apply & save]"
                }
            }
            Step::Waiting if modes.changing() => "[Applying…]",
            Step::Waiting => "[Cancel pending]",
            Step::Error => "[Choose mode]",
            _ => "[Continue]",
        };
        let button = Rect::new(
            footer.right().saturating_sub(21),
            footer.y,
            21.min(footer.width),
            1,
        );
        if !matches!(ui.step, Step::Full | Step::Busy | Step::Error) {
            frame.render_widget(Paragraph::new(label).style(theme::modal_selected()), button);
        }
        if !matches!(ui.step, Step::Full | Step::Busy | Step::Error)
            && (ui.step != Step::Waiting || !modes.changing())
        {
            self.pointer
                .add(button, mouse::Target::AgentPermissions(Hit::Continue));
        }
        if matches!(ui.step, Step::Scope | Step::Review) {
            let back = Rect::new(footer.x, footer.y, 8, 1);
            frame.render_widget(
                Paragraph::new("[Back]").style(Style::default().fg(colors.preview)),
                back,
            );
            self.pointer
                .add(back, mouse::Target::AgentPermissions(Hit::Back));
        }
        if matches!(ui.step, Step::Full | Step::Busy | Step::Error) {
            let alternate = Rect::new(footer.x, footer.y, 24.min(footer.width), 1);
            frame.render_widget(
                Paragraph::new(if ui.step == Step::Error {
                    "[Choose mode]"
                } else {
                    "[Keep current]"
                })
                .style(if ui.focus == 0 {
                    theme::modal_selected()
                } else {
                    Style::default().fg(colors.preview)
                }),
                alternate,
            );
            self.pointer.add(
                alternate,
                mouse::Target::AgentPermissions(Hit::Decision(false)),
            );
            let apply = match ui.step {
                Step::Full => "[Enable full access]",
                Step::Error => "[Retry]",
                _ => "[Apply after turn]",
            };
            frame.render_widget(
                Paragraph::new(apply).style(if ui.focus == 1 {
                    theme::modal_selected()
                } else {
                    Style::default().fg(colors.running)
                }),
                button,
            );
            self.pointer
                .add(button, mouse::Target::AgentPermissions(Hit::Decision(true)));
        }
        let close = Rect::new(popup.right().saturating_sub(9), popup.y, 8, 1);
        frame.render_widget(
            Paragraph::new("[Close]").style(Style::default().fg(colors.preview)),
            close,
        );
        self.pointer
            .add(close, mouse::Target::AgentPermissions(Hit::Close));
    }
}
