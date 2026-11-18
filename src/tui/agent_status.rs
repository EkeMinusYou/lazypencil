//! Selected agent settings in the global header, refreshed by ACP snapshots.
use super::*;
use crate::agent::{SessionSelector, unique_option};

/// A pending connection has no negotiated controls; each full snapshot replaces them.
enum Configuration {
    Pending { fast: bool },
    Reported(Value),
}

pub(super) struct Status {
    pub authentication: String,
    configuration: Configuration,
}

impl Status {
    pub fn new(fast: bool) -> Self {
        Self {
            authentication: "checking".into(),
            configuration: Configuration::Pending { fast },
        }
    }

    pub fn initialized(&mut self) {
        self.configuration = Configuration::Pending {
            fast: self.speed() == Some("fast"),
        };
    }

    pub fn unavailable(&mut self) {
        self.configuration = Configuration::Reported(Value::Null);
    }

    fn speed(&self) -> Option<&'static str> {
        match &self.configuration {
            Configuration::Pending { fast } => fast.then_some("fast"),
            Configuration::Reported(options) => {
                let current = &SessionSelector::Speed.option(options)?["currentValue"];
                match current.as_str() {
                    Some("ultrafast") => Some("ultrafast"),
                    Some("on") => Some("fast"),
                    _ if current == true => Some("fast"),
                    _ => None,
                }
            }
        }
    }

    fn model(&self) -> &str {
        match &self.configuration {
            Configuration::Pending { .. } => "connecting",
            Configuration::Reported(options) => SessionSelector::Model
                .option(options)
                .and_then(|option| option["currentValue"].as_str())
                .unwrap_or("not reported"),
        }
    }

    fn effort(&self) -> Option<&str> {
        match &self.configuration {
            Configuration::Pending { .. } => None,
            Configuration::Reported(options) => Some(
                unique_option(options, |option| {
                    option["id"] == "reasoning_effort" || option["category"] == "thought_level"
                })
                .and_then(|option| option["currentValue"].as_str())
                .unwrap_or("not reported"),
            ),
        }
    }

    pub fn options(&self) -> &Value {
        match &self.configuration {
            Configuration::Pending { .. } => &Value::Null,
            Configuration::Reported(options) => options,
        }
    }
}

pub(super) struct Picker {
    selector: SessionSelector,
    choices: Vec<(String, String)>,
    current: Option<usize>,
    pub selected: usize,
}

impl Picker {
    pub fn new(selector: SessionSelector, options: &Value) -> Self {
        let choices: Vec<_> = selector
            .choices(options)
            .map(|choice| {
                let value = choice["value"].as_str().expect("select choice");
                let name = match (selector, value) {
                    (SessionSelector::Speed, "off") => "Standard",
                    (SessionSelector::Speed, "on") => "Fast",
                    (SessionSelector::Speed, "ultrafast") => "Ultrafast",
                    _ => choice["name"].as_str().unwrap_or(value),
                };
                (value.to_owned(), clean(name).replace(['\n', '\t'], " "))
            })
            .collect();
        let current = selector.option(options).and_then(|option| {
            choices
                .iter()
                .position(|(value, _)| option["currentValue"] == *value)
        });
        Self {
            selector,
            choices,
            current,
            selected: current.unwrap_or_default(),
        }
    }

    pub fn key(&mut self, key: KeyCode) {
        let last = self.choices.len().saturating_sub(1);
        self.selected = match key {
            KeyCode::Up => self.selected.saturating_sub(1),
            KeyCode::Down => (self.selected + 1).min(last),
            KeyCode::Home => 0,
            KeyCode::End => last,
            KeyCode::PageUp => self.selected.saturating_sub(8),
            KeyCode::PageDown => (self.selected + 8).min(last),
            _ => self.selected,
        };
    }

    pub fn apply(self, app: &mut App) -> Result<()> {
        if let Some((value, _)) = self.choices.into_iter().nth(self.selected) {
            let is_current = self
                .selector
                .option(app.agent.options())
                .is_some_and(|option| option["currentValue"] == value);
            if !is_current {
                app.conversation
                    .change_session_option(self.selector, value)?;
                app.set_status(format!("Updating agent {}…", self.selector.name()));
            }
        }
        Ok(())
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect, pointer: &mut mouse::Pointer) {
        let title = match self.selector {
            SessionSelector::Model => "Choose model",
            SessionSelector::Speed => "Choose speed",
        };
        let shell = modal::Shell::draw(frame, area, "Agent", title, 1);
        shell.close(frame, pointer);
        let choices = self.choices.iter().enumerate().map(|(index, (_, label))| {
            let current = if self.current == Some(index) {
                "Current"
            } else {
                ""
            };
            modal::result(
                label,
                current,
                shell.body.width.saturating_sub(2),
                index == self.selected,
            )
        });
        pointer.modal_list(
            frame,
            List::new(choices)
                .highlight_style(theme::modal_selected())
                .highlight_symbol("› "),
            shell.body,
            self.selected,
            mouse::ListTarget::AgentOptions,
        );
        frame.render_widget(
            Paragraph::new(modal::hints(&[("↑ ↓", "Select"), ("Esc", "Close")])),
            shell.footer,
        );
        modal::action(frame, shell.footer, "[Enter Apply]", pointer);
    }
}

impl App {
    pub(super) fn update_agent_configuration(&mut self, options: &Value) {
        self.agent.configuration = Configuration::Reported(options.clone());
        self.dirty = true;
    }

    pub(super) fn header_line(&self) -> Line<'static> {
        let separator = || Span::styled(" · ", Style::default().fg(theme::colors().muted));
        let mut spans = vec![
            Span::styled(
                "✎ lazypencil",
                Style::default()
                    .fg(theme::colors().agent)
                    .add_modifier(Modifier::BOLD),
            ),
            separator(),
            Span::styled(
                format!("{:?}", self.conversation.settings().agent),
                Style::default()
                    .fg(theme::colors().preview)
                    .add_modifier(Modifier::BOLD),
            ),
            separator(),
            Span::styled(
                clean(self.agent.model()).replace(['\n', '\t'], " "),
                Style::default().fg(theme::colors().success),
            ),
        ];
        if let Some(effort) = self.agent.effort() {
            spans.extend([
                Span::raw(" "),
                Span::styled(
                    clean(effort).replace(['\n', '\t'], " "),
                    Style::default().fg(theme::colors().success),
                ),
            ]);
        }
        if let Some(speed) = self.agent.speed() {
            spans.extend([
                Span::raw(" "),
                Span::styled(speed, Style::default().fg(theme::colors().success)),
            ]);
        }
        spans.extend([
            separator(),
            Span::styled(
                clean(
                    self.conversation
                        .modes()
                        .current
                        .as_deref()
                        .unwrap_or("mode not confirmed"),
                ),
                Style::default().fg(theme::colors().preview),
            ),
        ]);
        spans.extend([
            separator(),
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
            separator(),
            Span::styled(
                clean(&self.agent.authentication),
                Style::default().fg(theme::colors().secondary),
            ),
            separator(),
            Span::styled(
                self.conversation.id()[..8].to_owned(),
                Style::default().fg(theme::colors().secondary),
            ),
            separator(),
            Span::styled(
                clean(&self.conversation.store().path().display().to_string()),
                Style::default().fg(theme::colors().secondary),
            ),
        ]);
        Line::from(spans)
    }
}
