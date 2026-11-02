//! Draft references are independent of the tree selection and the active request.
use super::*;
use crate::conversation::references::{self, Reference};

pub(super) fn history(references: &[Reference]) -> String {
    if references.is_empty() {
        String::new()
    } else {
        format!("\n\nReferences: {}", references::labels(references))
    }
}

impl App {
    pub(super) fn reference_selected_node(&mut self) -> Result<()> {
        let node = self
            .selected_node()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Select a node first"))?;
        let reference = self.captured_reference(node, None);
        self.add_prompt_reference(reference)
    }

    pub(super) fn reference_selected_diagnostic(&mut self) -> Result<()> {
        let diagnostic = self
            .selected_diagnostic()
            .cloned()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Select a diagnostic first"))?;
        let node = self
            .preview
            .as_ref()
            .and_then(|preview| {
                preview
                    .nodes
                    .iter()
                    .find(|node| diagnostic["node_id"].as_str() == Some(node.id.as_str()))
            })
            .ok_or_else(|| Error::new("NODE_NOT_FOUND", "The diagnostic target is unavailable"))?;
        let reference = self.captured_reference(node, Some(diagnostic));
        self.add_prompt_reference(reference)
    }

    fn captured_reference(&self, node: &preview::Node, diagnostic: Option<Value>) -> Reference {
        let preview = self.preview.as_ref().expect("captured preview node");
        Reference {
            target: node.target.clone(),
            name: references::single_line(&node.name),
            kind: node.kind.clone(),
            text_excerpt: clean(&node.text_excerpt),
            revision: preview.summary["revision"].as_str().unwrap_or("").into(),
            dependency_revision: preview.summary["dependency_revision"]
                .as_str()
                .unwrap_or("")
                .into(),
            diagnostic,
        }
    }

    fn add_prompt_reference(&mut self, reference: Reference) -> Result<()> {
        let diagnostic = reference.diagnostic.is_some();
        let mut pending = self.outbox.references.clone();
        if let Some(previous) = pending.iter_mut().find(|item| {
            item.target == reference.target && item.diagnostic_code() == reference.diagnostic_code()
        }) {
            *previous = reference;
        } else {
            pending.push(reference);
        }
        references::decode(references::selection(&pending).as_ref())?;
        self.outbox.references = pending;
        self.agent_collapsed = false;
        self.focus = Focus::Prompt;
        self.draft_changed = true;
        self.save_draft()?;
        self.status = format!(
            "{} reference added · write a message to send",
            if diagnostic { "Diagnostic" } else { "Node" }
        );
        Ok(())
    }

    pub(super) fn reference_available(&self, reference: &Reference) -> Option<bool> {
        self.preview.as_ref().map(|preview| {
            preview
                .nodes
                .iter()
                .any(|node| node.target == reference.target)
        })
    }

    pub(super) fn reference_key(&mut self, key: KeyEvent) -> Result<()> {
        let action = |action| self.navigation.matches(action, key);
        let count = self.outbox.references.len();
        if action(Nav::PreviousItem) || action(Nav::NextItem) {
            let current = self.reference_rows.selected().unwrap_or(0);
            let index = if action(Nav::PreviousItem) {
                current.saturating_sub(1)
            } else {
                current.saturating_add(1).min(count.saturating_sub(1))
            };
            self.reference_rows.select((count > 0).then_some(index));
        } else if action(Nav::RemoveAttachment)
            && let Some(index) = self
                .reference_rows
                .selected()
                .filter(|index| *index < count)
        {
            self.outbox.references.remove(index);
            self.reference_rows
                .select((count > 1).then(|| index.min(count - 2)));
            if count == 1 {
                self.focus = Focus::Prompt;
            }
            self.draft_changed = true;
            self.save_draft()?;
        }
        Ok(())
    }

    pub(super) fn finish_reference_prompt(&mut self, failed: bool) {
        let Some(prompt) = self.outbox.in_flight.take() else {
            return;
        };
        if !failed {
            return;
        }
        if self.editor.lines().iter().all(String::is_empty)
            && self.outbox.references.is_empty()
            && self.outbox.queued.is_empty()
        {
            self.editor = editor(&prompt.text);
            self.outbox.references = prompt.references;
            self.outbox.draft = Some(prompt.text);
        } else {
            // Keep the next draft intact. Recovery remains paused and explicit.
            self.outbox.queued.push_front(prompt);
            self.outbox.paused = true;
        }
    }

    pub(super) fn draw_references(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::References;
        let items = self
            .outbox
            .references
            .iter()
            .map(|reference| {
                let state = match self.reference_available(reference) {
                    Some(true) => "",
                    Some(false) => " · unavailable",
                    None => " · checking",
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        reference
                            .diagnostic_code()
                            .map(|code| format!("{} · ", nodes::node_label(code)))
                            .unwrap_or_default(),
                        Style::default().fg(theme::RUNNING),
                    ),
                    Span::raw(nodes::node_label(&reference.name)),
                    Span::styled(
                        format!(
                            " · {}{}",
                            nodes::node_label(reference.target["id"].as_str().unwrap_or("")),
                            state
                        ),
                        Style::default().fg(if state.is_empty() {
                            theme::SECONDARY
                        } else {
                            theme::RUNNING
                        }),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            List::new(items)
                .block(self.context_block(
                    "References",
                    "↗",
                    self.outbox.references.len(),
                    focused,
                    area.width,
                ))
                .highlight_symbol(if focused { "› " } else { "  " })
                .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
                .highlight_style(if focused {
                    theme::node_selected(true)
                } else {
                    Style::default()
                }),
            area,
            &mut self.reference_rows,
        );
    }
}
