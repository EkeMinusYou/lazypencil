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
        let preview = self.preview.as_ref().expect("selected preview node");
        let reference = Reference {
            target: node.target.clone(),
            name: references::single_line(&node.name),
            kind: node.kind.clone(),
            text_excerpt: clean(&node.text_excerpt),
            revision: preview.summary["revision"].as_str().unwrap_or("").into(),
            dependency_revision: preview.summary["dependency_revision"]
                .as_str()
                .unwrap_or("")
                .into(),
        };
        let mut pending = self.outbox.references.clone();
        if let Some(previous) = pending
            .iter_mut()
            .find(|item| item.target == reference.target)
        {
            *previous = reference;
        } else {
            pending.push(reference);
        }
        references::decode(references::selection(&pending).as_ref())?;
        self.outbox.references = pending;
        self.focus = Focus::Prompt;
        self.draft_changed = true;
        self.save_draft()?;
        self.status = "Node reference added · write a message to send".into();
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
                    Span::styled("↗ ", Style::default().fg(theme::NODES)),
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
                .block(Block::default().title(theme::title(
                    "↗",
                    format!("References · {}", self.outbox.references.len()),
                    theme::NODES,
                    focused,
                )))
                .highlight_symbol("› ")
                .highlight_style(theme::selected(focused)),
            area,
            &mut self.reference_rows,
        );
    }
}
