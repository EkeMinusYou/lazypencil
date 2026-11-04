//! Selected agent settings in the global header, refreshed by ACP snapshots.
use super::*;

/// A pending connection has no negotiated controls; each full snapshot replaces them.
enum Configuration {
    Pending {
        fast: bool,
    },
    Reported {
        model: Option<String>,
        effort: Option<String>,
        fast: bool,
    },
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
        self.configuration = Configuration::Pending { fast: self.fast() };
    }

    pub fn unavailable(&mut self) {
        self.configuration = Configuration::Reported {
            model: None,
            effort: None,
            fast: false,
        };
    }

    fn fast(&self) -> bool {
        match &self.configuration {
            Configuration::Pending { fast } | Configuration::Reported { fast, .. } => *fast,
        }
    }

    fn model(&self) -> &str {
        match &self.configuration {
            Configuration::Pending { .. } => "connecting",
            Configuration::Reported { model, .. } => model.as_deref().unwrap_or("not reported"),
        }
    }

    fn effort(&self) -> Option<&str> {
        match &self.configuration {
            Configuration::Pending { .. } => None,
            Configuration::Reported { effort, .. } => {
                Some(effort.as_deref().unwrap_or("not reported"))
            }
        }
    }

    fn update(&mut self, options: &Value) {
        let options = options.as_array().map(Vec::as_slice).unwrap_or_default();
        let models: Vec<_> = options
            .iter()
            .filter(|option| option["category"] == "model" || option["id"] == "model")
            .collect();
        let model = if models.len() == 1 {
            models[0]["currentValue"].as_str().map(str::to_owned)
        } else {
            None
        };
        let efforts: Vec<_> = options
            .iter()
            .filter(|option| {
                option["id"] == "reasoning_effort" || option["category"] == "thought_level"
            })
            .collect();
        let effort = if efforts.len() == 1 {
            efforts[0]["currentValue"].as_str().map(str::to_owned)
        } else {
            None
        };
        // ACP updates contain the full configuration, so missing controls clear old state.
        let fast: Vec<_> = options
            .iter()
            .filter(|option| option["id"] == "fast-mode")
            .collect();
        let fast =
            fast.len() == 1 && (fast[0]["currentValue"] == "on" || fast[0]["currentValue"] == true);
        self.configuration = Configuration::Reported {
            model,
            effort,
            fast,
        };
    }
}

impl App {
    pub(super) fn update_agent_configuration(&mut self, options: &Value) {
        self.agent.update(options);
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
        if self.agent.fast() {
            spans.extend([
                Span::raw(" "),
                Span::styled(
                    "Fast",
                    Style::default()
                        .fg(theme::colors().success)
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
        }
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
