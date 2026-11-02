//! Browse measured diagnostics without changing the editing target until activation.
use super::*;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct View {
    pub active: bool,
    pub selected_only: bool,
    rows: Vec<usize>,
    selection: ListState,
    scroll: u16,
    detail_height: u16,
}

impl App {
    pub(super) fn visible_diagnostic_count(&self) -> usize {
        self.diagnostics.rows.len()
    }

    fn layout_diagnostics(&self) -> &[Value] {
        self.preview
            .as_ref()
            .and_then(|preview| preview.summary["diagnostics"].as_array())
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn selected_diagnostic(&self) -> Option<&Value> {
        self.layout_diagnostics().get(
            *self
                .diagnostics
                .rows
                .get(self.diagnostics.selection.selected()?)?,
        )
    }

    pub(super) fn refresh_diagnostics(&mut self, selected: Option<&Value>) {
        let items = self.layout_diagnostics();
        let mut subtree = HashSet::new();
        if self.diagnostics.selected_only
            && let (Some(preview), Some(node)) = (&self.preview, self.selected_node())
        {
            let mut pending = vec![node];
            while let Some(node) = pending.pop() {
                subtree.insert(node.id.as_str());
                pending.extend(node.children.iter().map(|index| &preview.nodes[*index]));
            }
        }
        let rows = items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                !self.diagnostics.selected_only
                    || item["node_id"]
                        .as_str()
                        .is_some_and(|id| subtree.contains(id))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let index = selected.and_then(|selected| {
            rows.iter().position(|index| {
                let item = &items[*index];
                item["node_id"] == selected["node_id"] && item["code"] == selected["code"]
            })
        });
        let index = index.or_else(|| (!rows.is_empty()).then_some(0));
        if selected != index.and_then(|index| items.get(rows[index])) {
            self.diagnostics.scroll = 0;
        }
        self.diagnostics.rows = rows;
        self.diagnostics.selection.select(index);
    }

    pub(super) fn diagnostic_key(
        &mut self,
        key: KeyEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        let action = |action| self.navigation.matches(action, key);
        if action(Nav::DiagnosticScope) {
            let selected = self.selected_diagnostic().cloned();
            self.diagnostics.selected_only = !self.diagnostics.selected_only;
            self.refresh_diagnostics(selected.as_ref());
        } else if action(Nav::ReferenceNode) {
            self.reference_selected_diagnostic()?;
        } else if action(Nav::ClearSelection) {
            self.diagnostics.active = false;
        } else if action(Nav::PreviousItem) || action(Nav::NextItem) {
            let count = self.diagnostics.rows.len();
            let current = self.diagnostics.selection.selected().unwrap_or(0);
            let index = if action(Nav::PreviousItem) {
                current.saturating_sub(1)
            } else {
                current.saturating_add(1).min(count.saturating_sub(1))
            };
            self.diagnostics
                .selection
                .select((count > 0).then_some(index));
            self.diagnostics.scroll = 0;
        } else if action(Nav::NodePageUp) || action(Nav::NodePageDown) {
            let step = (self.diagnostics.detail_height / 2).max(1);
            self.diagnostics.scroll = if action(Nav::NodePageUp) {
                self.diagnostics.scroll.saturating_sub(step)
            } else {
                self.diagnostics.scroll.saturating_add(step)
            };
        } else if action(Nav::ActivateNode) || action(Nav::FitSelected) {
            let activate = action(Nav::ActivateNode);
            let Some(id) = self
                .selected_diagnostic()
                .and_then(|item| item["node_id"].as_str())
                .map(str::to_owned)
            else {
                return Ok(());
            };
            if !self
                .preview
                .as_ref()
                .is_some_and(|preview| preview.nodes.iter().any(|node| node.id == id))
            {
                return Ok(());
            }
            self.filter.clear();
            self.refresh_nodes(Some(&id));
            if activate {
                self.diagnostics.active = false;
            }
            self.fit_selected(picker)?;
            self.request(worker, false, false);
            self.camera_feedback(picker)?;
        }
        Ok(())
    }

    pub(super) fn draw_diagnostics(&mut self, frame: &mut Frame, area: Rect) {
        let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
        let scope = if self.diagnostics.selected_only {
            self.selected_node()
                .map(|node| {
                    format!(
                        "{}: {}",
                        if node.children.is_empty() {
                            "Node"
                        } else {
                            "Subtree"
                        },
                        nodes::node_label(&node.name)
                    )
                })
                .unwrap_or_else(|| "Selected node".into())
        } else {
            "All nodes".into()
        };
        frame.render_widget(
            Paragraph::new(scope).style(Style::default().fg(theme::SECONDARY)),
            sections[0],
        );
        let area = sections[1];
        if self.diagnostics.rows.is_empty() {
            let message = if self.preview_error.is_some() {
                "Diagnostics unavailable: preview failed"
            } else if self.preview.is_none() {
                "Loading diagnostics…"
            } else if self.diagnostics.selected_only && self.selected_node().is_none() {
                "Select a node in Nodes"
            } else if self.diagnostics.selected_only {
                if self
                    .selected_node()
                    .is_some_and(|node| !node.children.is_empty())
                {
                    "No diagnostics in subtree"
                } else {
                    "No diagnostics for node"
                }
            } else {
                "No layout diagnostics"
            };
            frame.render_widget(
                Paragraph::new(message)
                    .style(Style::default().fg(theme::SECONDARY))
                    .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        let sections = Layout::vertical([
            Constraint::Length((area.height / 3).max(2)),
            Constraint::Min(1),
        ])
        .split(area);
        let items = self
            .diagnostics
            .rows
            .iter()
            .filter_map(|index| self.layout_diagnostics().get(*index))
            .map(|item| {
                let severity = item["severity"].as_str().unwrap_or("warning");
                let color = if severity == "error" {
                    theme::ERROR
                } else {
                    theme::RUNNING
                };
                ListItem::new(Text::from(vec![
                    Line::styled(
                        format!(
                            "{} {}",
                            severity,
                            nodes::node_label(item["code"].as_str().unwrap_or(""))
                        ),
                        Style::default().fg(color),
                    ),
                    Line::raw(nodes::node_label(item["node_id"].as_str().unwrap_or(""))),
                ]))
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme::node_selected(self.focus == Focus::Nodes)),
            sections[0],
            &mut self.diagnostics.selection,
        );
        let Some(item) = self.selected_diagnostic() else {
            return;
        };
        let node_id = item["node_id"].as_str().unwrap_or("");
        let name = self
            .preview
            .as_ref()
            .and_then(|preview| preview.nodes.iter().find(|node| node.id == node_id))
            .map(|node| node.name.as_str())
            .unwrap_or(node_id);
        let mut text = format!(
            "{}: {}\n{}\n{}\n\n{}",
            nodes::node_label(item["severity"].as_str().unwrap_or("warning")),
            nodes::node_label(item["code"].as_str().unwrap_or("")),
            nodes::node_label(name),
            nodes::node_label(node_id),
            clean(item["message"].as_str().unwrap_or(""))
        );
        if let Some(details) = item.get("details").filter(|details| !details.is_null()) {
            text.push_str("\n\n");
            text.push_str(&clean(
                &serde_json::to_string_pretty(details).unwrap_or_default(),
            ));
        }
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::FRAME));
        let inner = block.inner(sections[1]);
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
        let maximum = paragraph
            .line_count(inner.width)
            .saturating_sub(usize::from(inner.height))
            .min(u16::MAX as usize) as u16;
        self.diagnostics.scroll = self.diagnostics.scroll.min(maximum);
        self.diagnostics.detail_height = inner.height;
        let block = block.title(Line::styled(
            format!(
                " Details {}/{} ",
                self.diagnostics.scroll.saturating_add(1),
                maximum.saturating_add(1)
            ),
            Style::default().fg(theme::SECONDARY),
        ));
        frame.render_widget(
            paragraph.scroll((self.diagnostics.scroll, 0)).block(block),
            sections[1],
        );
    }
}
