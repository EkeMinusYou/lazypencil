//! Compact, focus-aware hints and independently timed notifications.
use super::*;

const NOTICE_DURATION: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusKind {
    Info,
    Attention,
    Error,
    PreviewError,
}

impl StatusKind {
    pub(super) fn is_error(self) -> bool {
        matches!(self, Self::Error | Self::PreviewError)
    }
}

struct Hint {
    key: String,
    description: &'static str,
}

impl Hint {
    fn new(key: impl Into<String>, description: &'static str) -> Self {
        Self {
            key: key.into(),
            description,
        }
    }

    fn line(self) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                self.key,
                Style::default()
                    .fg(theme::colors().key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", self.description),
                Style::default().fg(theme::colors().secondary),
            ),
        ])
    }
}

impl App {
    pub(super) fn set_status(&mut self, message: String) {
        self.status = clean(&message);
        self.status_kind = StatusKind::Info;
        self.status_deadline = (!self.status.is_empty()).then(|| Instant::now() + NOTICE_DURATION);
        self.dirty = true;
    }

    pub(super) fn set_error_status(&mut self, message: String) {
        self.set_status(message);
        self.status_kind = StatusKind::Error;
        self.status_deadline = None;
    }

    pub(super) fn set_preview_error_status(&mut self, error: &Error) {
        let message = if self.preview_stale {
            format!("Preview update failed; showing previous preview. {error}")
        } else {
            error.to_string()
        };
        self.set_error_status(clean(&message));
        self.status_kind = StatusKind::PreviewError;
    }

    pub(super) fn set_attention_status(&mut self, message: String) {
        self.set_status(message);
        self.status_kind = StatusKind::Attention;
        self.status_deadline = None;
    }

    pub(super) fn clear_status(&mut self) {
        self.status.clear();
        self.status_kind = StatusKind::Info;
        self.status_deadline = None;
    }

    pub(super) fn expire_status(&mut self, now: Instant) {
        if self.status_deadline.is_some_and(|deadline| now >= deadline) {
            self.clear_status();
            self.dirty = true;
        }
    }

    pub(super) fn footer_height(&self) -> u16 {
        super::workspace::FOOTER_HEIGHT
    }

    fn hint(&self, action: Nav, description: &'static str) -> Option<Hint> {
        self.navigation
            .preferred_label(
                action,
                self.focus == Focus::Prompt || self.searching || self.preview_search.editing,
            )
            .map(|key| Hint::new(key, description))
    }

    fn grouped_hint(&self, actions: &[Nav], description: &'static str) -> Option<Hint> {
        let keys = actions
            .iter()
            .map(|action| self.navigation.preferred_label(*action, false))
            .collect::<Option<Vec<_>>>()?;
        let keys = keys
            .iter()
            .map(|key| match key.as_str() {
                "Left" => "←",
                "Down" => "↓",
                "Up" => "↑",
                "Right" => "→",
                "PageUp" => "PgUp",
                "PageDown" => "PgDn",
                key => key,
            })
            .collect::<Vec<_>>()
            .join("/");
        Some(Hint::new(keys, description))
    }

    fn newline_hint(&self) -> Option<Hint> {
        for (modifiers, label) in [
            (KeyModifiers::ALT, "Alt+Enter"),
            (KeyModifiers::SHIFT, "Shift+Enter"),
            (KeyModifiers::NONE, "Enter"),
        ] {
            let event = KeyEvent::new(KeyCode::Enter, modifiers);
            let intercepted = [
                Nav::NextPanel,
                Nav::PreviousPanel,
                Nav::NextSection,
                Nav::PreviousSection,
                Nav::ToggleNodesPanel,
                Nav::ToggleAgentPanel,
                Nav::Help,
                Nav::Commands,
            ]
            .into_iter()
            .any(|action| self.navigation.matches_while_typing(action, event));
            if !self.keys.submits(event) && !self.keys.pastes(event) && !intercepted {
                return Some(Hint::new(label, "newline"));
            }
        }
        None
    }

    fn context_hints(&self) -> Vec<Hint> {
        if !self.permissions.is_empty() {
            return vec![Hint::new("Enter", "respond"), Hint::new("Esc", "deny")];
        }
        if self.help || self.modal.is_some() {
            return vec![Hint::new("Esc", "close")];
        }
        if self.input.is_some() || self.document_task.is_some() {
            return vec![Hint::new("Ctrl+C", "cancel")];
        }
        if self.searching || self.preview_search.editing {
            return vec![Hint::new("Enter", "keep filter"), Hint::new("Esc", "clear")];
        }
        let hints = match self.focus {
            Focus::Prompt => {
                let submit =
                    crate::config::keys::preferred_label(&self.keys.submit, true).map(|key| {
                        Hint::new(
                            key,
                            if self.outbox.steering.is_some()
                                && self.outbox.references.is_empty()
                                && self.pending_images().is_empty()
                            {
                                "wait"
                            } else if self.conversation.can_steer()
                                && self.outbox.references.is_empty()
                                && self.pending_images().is_empty()
                            {
                                "steer"
                            } else if self.conversation.active() || !self.outbox.queued.is_empty() {
                                "queue"
                            } else {
                                "send"
                            },
                        )
                    });
                if self.conversation.active() {
                    vec![Some(Hint::new("Ctrl+C", "cancel")), submit]
                } else {
                    vec![submit, self.newline_hint()]
                }
            }
            Focus::Nodes if self.diagnostics.active => {
                if self.selected_diagnostic().is_some() {
                    vec![
                        self.hint(Nav::ReferenceNode, "reference"),
                        self.hint(Nav::FitSelected, "fit"),
                    ]
                } else {
                    vec![
                        self.hint(Nav::ToggleDiagnostics, "nodes"),
                        self.hint(Nav::DiagnosticScope, "scope"),
                    ]
                }
            }
            Focus::Nodes if self.selected_node().is_some() => vec![
                self.hint(Nav::ReferenceNode, "reference"),
                self.hint(Nav::FitSelected, "fit"),
            ],
            Focus::Nodes => vec![
                self.hint(Nav::SearchNodes, "search"),
                self.hint(Nav::ToggleDiagnostics, "diagnostics"),
            ],
            Focus::Preview if !self.preview_search.query.is_empty() => vec![
                self.grouped_hint(&[Nav::NextSearchMatch, Nav::PreviousSearchMatch], "result"),
                Some(Hint::new("Esc", "clear search")),
            ],
            Focus::Preview => vec![
                self.grouped_hint(
                    &[Nav::PanLeft, Nav::PanDown, Nav::PanUp, Nav::PanRight],
                    "pan",
                ),
                self.hint(Nav::SearchPreview, "search"),
            ],
            Focus::Attachments | Focus::References => vec![
                self.hint(Nav::RemoveAttachment, "remove"),
                Some(Hint::new("Esc", "input")),
            ],
            Focus::Conversation => vec![
                self.grouped_hint(&[Nav::ScrollUp, Nav::ScrollDown], "scroll"),
                Some(Hint::new("Esc", "input")),
            ],
        };
        let mut hints: Vec<_> = hints.into_iter().flatten().collect();
        if self.focus != Focus::Prompt && self.conversation.active() {
            hints.insert(0, Hint::new("Ctrl+C", "cancel"));
        }
        // One recovery hint can replace a secondary action, never cancellation.
        let recovery = if self.agent_collapsed {
            self.hint(Nav::ToggleAgentPanel, "show Agent")
        } else if self.nodes_collapsed {
            self.hint(Nav::ToggleNodesPanel, "show Nodes")
        } else {
            None
        };
        if let Some(recovery) = recovery
            && !(self.focus == Focus::Prompt && self.conversation.active())
        {
            hints.insert(hints.len().min(1), recovery);
        }
        hints.truncate(2);
        hints
    }

    pub(super) fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        if area.height > 1
            && (!self.status.is_empty() || self.agent_collapsed && self.conversation.active())
        {
            let (text, color) = if !self.status.is_empty() {
                (
                    if self.status_kind.is_error() {
                        format!("! {}", self.status)
                    } else {
                        self.status.clone()
                    },
                    if self.status_kind.is_error() {
                        theme::colors().error
                    } else if self.status_kind == StatusKind::Attention {
                        theme::colors().running
                    } else {
                        theme::colors().secondary
                    },
                )
            } else {
                let activity = self.activity_line(Instant::now()).unwrap();
                (
                    activity.to_string(),
                    if self.permissions.is_empty() {
                        theme::colors().working
                    } else {
                        theme::colors().running
                    },
                )
            };
            draw_notice(frame, Rect::new(area.x, area.y, area.width, 1), text, color);
        }
        let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        let help = (!self.has_overlay())
            .then(|| self.hint(Nav::Help, "help"))
            .flatten()
            .map(Hint::line);
        let help_width = help.as_ref().map_or(0, |line| line.width() as u16);
        let available = usize::from(
            row.width
                .saturating_sub(help_width + u16::from(help.is_some()) * 2),
        );
        let mut left = Line::default();
        for hint in self.context_hints() {
            let hint = hint.line();
            let separator = usize::from(!left.spans.is_empty()) * 3;
            if left.width() + separator + hint.width() > available {
                continue;
            }
            if separator > 0 {
                left.spans.push(Span::styled(
                    " · ",
                    Style::default().fg(theme::colors().muted),
                ));
            }
            left.spans.extend(hint.spans);
        }
        frame.render_widget(left, row);
        if let Some(help) = help.filter(|_| help_width <= row.width) {
            frame.render_widget(
                help,
                Rect::new(row.right() - help_width, row.y, help_width, 1),
            );
        }
    }
}

fn draw_notice(frame: &mut Frame, area: Rect, text: String, color: Color) {
    let line = Line::styled(
        text.replace(['\n', '\r', '\t'], " "),
        Style::default().fg(color),
    );
    if line.width() <= usize::from(area.width) {
        frame.render_widget(line, area);
    } else if area.width > 0 {
        frame.render_widget(line, Rect::new(area.x, area.y, area.width - 1, 1));
        frame.render_widget(
            Span::styled("…", Style::default().fg(color)),
            Rect::new(area.right() - 1, area.y, 1, 1),
        );
    }
}
