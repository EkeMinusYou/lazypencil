//! Selected agent settings in the global header, refreshed by ACP snapshots.
use super::*;

impl App {
    pub(super) fn update_agent_configuration(&mut self, options: &Value) {
        let options = options.as_array().map(Vec::as_slice).unwrap_or_default();
        let models: Vec<_> = options
            .iter()
            .filter(|option| option["category"] == "model" || option["id"] == "model")
            .collect();
        self.agent_model = if models.len() == 1 {
            models[0]["currentValue"].as_str().map(str::to_owned)
        } else {
            None
        };
        // ACP updates contain the full configuration, so missing controls clear old state.
        let fast: Vec<_> = options
            .iter()
            .filter(|option| option["id"] == "fast-mode")
            .collect();
        self.agent_fast =
            fast.len() == 1 && (fast[0]["currentValue"] == "on" || fast[0]["currentValue"] == true);
        self.dirty = true;
    }

    pub(super) fn header_line(&self) -> Line<'static> {
        let separator = || Span::styled(" · ", Style::default().fg(theme::MUTED));
        let mut spans = vec![
            Span::styled(
                "✎ lazypencil",
                Style::default()
                    .fg(theme::AGENT)
                    .add_modifier(Modifier::BOLD),
            ),
            separator(),
            Span::styled(
                format!("{:?}", self.conversation.settings().agent),
                Style::default()
                    .fg(theme::PREVIEW)
                    .add_modifier(Modifier::BOLD),
            ),
            separator(),
            Span::styled(
                format!(
                    "model: {}",
                    clean(self.agent_model.as_deref().unwrap_or("default"))
                        .replace(['\n', '\t'], " ")
                ),
                Style::default().fg(theme::SECONDARY),
            ),
        ];
        if self.agent_fast {
            spans.extend([
                separator(),
                Span::styled(
                    "Fast",
                    Style::default()
                        .fg(theme::SUCCESS)
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
        ]);
        Line::from(spans)
    }
}
