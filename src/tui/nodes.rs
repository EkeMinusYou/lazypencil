//! Collapsible document hierarchy with stable editing targets and cached rows.
use super::*;
use std::{borrow::Cow, collections::HashSet};

pub(super) struct Row {
    pub index: usize,
    depth: usize,
    expanded: bool,
    matched: bool,
    diagnostic_count: usize,
    has_error_diagnostic: bool,
}

#[derive(Default)]
pub(super) struct Tree {
    expanded: HashSet<String>,
    search_closed: HashSet<String>,
    query: String,
    viewport_height: u16,
    rows: Vec<Row>,
}

impl Tree {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn position(&self, index: usize) -> Option<usize> {
        self.rows.iter().position(|row| row.index == index)
    }

    fn reveal(&mut self, index: usize, nodes: &[preview::Node]) {
        let mut parent = nodes[index].parent;
        while let Some(index) = parent {
            self.expanded.insert(nodes[index].id.clone());
            self.search_closed.remove(&nodes[index].id);
            parent = nodes[index].parent;
        }
    }

    pub fn rebuild(&mut self, nodes: &[preview::Node], filter: &str) {
        let query = filter.to_lowercase();
        if self.query != query {
            self.search_closed.clear();
            self.query.clone_from(&query);
        }
        let matched = nodes
            .iter()
            .map(|node| {
                query.is_empty()
                    || node.name.to_lowercase().contains(&query)
                    || node.id.to_lowercase().contains(&query)
            })
            .collect::<Vec<_>>();
        let mut included = matched.clone();
        if !query.is_empty() {
            // Matching branches include their descendants. Matching leaves
            // retain their ancestors without pulling in unrelated siblings.
            for (index, node) in nodes.iter().enumerate() {
                included[index] |= node.parent.is_some_and(|parent| included[parent]);
            }
            for (index, node) in nodes.iter().enumerate().rev() {
                if included[index]
                    && let Some(parent) = node.parent
                {
                    included[parent] = true;
                }
            }
        }
        let mut counts = nodes
            .iter()
            .map(|node| node.diagnostic_count)
            .collect::<Vec<_>>();
        let mut errors = nodes
            .iter()
            .map(|node| node.has_error_diagnostic)
            .collect::<Vec<_>>();
        for (index, node) in nodes.iter().enumerate().rev() {
            if let Some(parent) = node.parent {
                let count = counts[index];
                let error = errors[index];
                counts[parent] += count;
                errors[parent] |= error;
            }
        }
        let mut depth = vec![0; nodes.len()];
        let mut visible = vec![false; nodes.len()];
        self.rows.clear();
        // Layout items are in preorder, with instance-aware parent indices.
        for (index, node) in nodes.iter().enumerate() {
            depth[index] = node.parent.map_or(0, |parent| depth[parent] + 1);
            visible[index] = if query.is_empty() {
                node.parent.is_none_or(|parent| {
                    visible[parent] && self.expanded.contains(&nodes[parent].id)
                })
            } else {
                included[index]
                    && node.parent.is_none_or(|parent| {
                        visible[parent] && !self.search_closed.contains(&nodes[parent].id)
                    })
            };
            if visible[index] {
                self.rows.push(Row {
                    index,
                    depth: depth[index],
                    expanded: !node.children.is_empty()
                        && if query.is_empty() {
                            self.expanded.contains(&node.id)
                        } else {
                            !self.search_closed.contains(&node.id)
                                && node.children.iter().any(|child| included[*child])
                        },
                    matched: matched[index],
                    diagnostic_count: counts[index],
                    has_error_diagnostic: errors[index],
                });
            }
        }
    }
}

impl App {
    pub(super) fn toggle_pointer_branch(&mut self, row: usize) {
        let Some(row) = self.node_tree.rows.get(row) else {
            return;
        };
        let Some(node) = self
            .preview
            .as_ref()
            .and_then(|preview| preview.nodes.get(row.index))
        else {
            return;
        };
        if node.children.is_empty() {
            return;
        }
        let id = node.id.clone();
        if row.expanded {
            self.node_tree.expanded.remove(&id);
            self.node_tree.search_closed.insert(id.clone());
        } else {
            self.node_tree.expanded.insert(id.clone());
            self.node_tree.search_closed.remove(&id);
        }
        // Rebuild without revealing ancestors or reopening the clicked branch.
        self.node_tree
            .rebuild(&self.preview.as_ref().unwrap().nodes, &self.filter);
        self.nodes.select(
            self.node_tree
                .rows
                .iter()
                .position(|row| self.preview.as_ref().unwrap().nodes[row.index].id == id),
        );
    }
    pub(super) fn refresh_nodes(&mut self, selected: Option<&str>) {
        let Some(preview) = &self.preview else {
            return;
        };
        let nodes = &preview.nodes;
        let ids = nodes
            .iter()
            .filter(|node| !node.children.is_empty())
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        self.node_tree
            .expanded
            .retain(|id| ids.contains(id.as_str()));
        self.node_tree
            .search_closed
            .retain(|id| ids.contains(id.as_str()));
        let selected = selected.and_then(|id| nodes.iter().position(|node| node.id == id));
        if let Some(index) = selected {
            self.node_tree.reveal(index, nodes);
        }
        self.node_tree.rebuild(nodes, &self.filter);
        self.nodes
            .select(selected.and_then(|index| self.node_tree.position(index)));
    }

    pub(super) fn search_nodes(&mut self, selected: Option<&str>) {
        self.refresh_nodes(if self.filter.is_empty() {
            selected
        } else {
            None
        });
        if !self.filter.is_empty() {
            self.nodes
                .select(self.node_tree.rows.iter().position(|row| row.matched));
        }
    }

    pub(super) fn navigate_node_tree(&mut self, action: Nav) {
        let row_index = self.nodes.selected().unwrap_or(0);
        let Some(row) = self.node_tree.rows.get(row_index) else {
            return;
        };
        let Some(preview) = &self.preview else {
            return;
        };
        let index = row.index;
        let node = &preview.nodes[index];
        let id = node.id.clone();
        let branch = !node.children.is_empty();
        let expanded = row.expanded;
        match action {
            Nav::ExpandNode if branch && expanded => {
                if let Some(child) = self.node_tree.rows.get(row_index + 1)
                    && preview.nodes[child.index].parent == Some(index)
                {
                    self.nodes.select(Some(row_index + 1));
                }
            }
            Nav::CollapseNode => {
                let parent = node.parent.unwrap_or(index);
                let parent_id = preview.nodes[parent].id.clone();
                self.node_tree.expanded.remove(&id);
                self.node_tree.expanded.remove(&parent_id);
                if !self.filter.is_empty() {
                    self.node_tree.search_closed.insert(id);
                    self.node_tree.search_closed.insert(parent_id.clone());
                }
                self.refresh_nodes(Some(&parent_id));
            }
            Nav::ExpandNode if branch => {
                self.node_tree.expanded.insert(id.clone());
                self.node_tree.search_closed.remove(&id);
                self.refresh_nodes(Some(&id));
            }
            Nav::ToggleNode if branch && self.filter.is_empty() => {
                if expanded {
                    self.node_tree.expanded.remove(&id);
                } else {
                    self.node_tree.expanded.insert(id.clone());
                }
                self.refresh_nodes(Some(&id));
            }
            _ => {}
        }
    }

    pub(super) fn page_nodes(&mut self, backwards: bool) {
        let count = self.node_tree.rows.len();
        if count == 0 {
            self.nodes.select(None);
            return;
        }
        let step = usize::from(self.node_tree.viewport_height / 2).max(1);
        let current = self.nodes.selected().unwrap_or(0).min(count - 1);
        self.nodes.select(Some(if backwards {
            current.saturating_sub(step)
        } else {
            current.saturating_add(step).min(count - 1)
        }));
    }

    pub(super) fn draw_nodes(&mut self, frame: &mut Frame, area: Rect) {
        let title = if self.diagnostics.active {
            "Diagnostics".into()
        } else if self.searching || !self.filter.is_empty() {
            format!("Nodes /{}", self.filter)
        } else {
            "Nodes".into()
        };
        let title = if !self.diagnostics.active && self.nodes_loading && self.preview.is_some() {
            format!("{title} · Loading…")
        } else {
            title
        };
        let focused = self.focus == Focus::Nodes;
        let title = theme::title("▤", title, theme::NODES, focused);
        let title_width = title.width();
        let mut block = panel(title, focused);
        if let Some(preview) = &self.preview {
            let diagnostics = if self.diagnostics.active && self.diagnostics.selected_only {
                self.visible_diagnostic_count()
            } else {
                preview.summary["diagnostics"]
                    .as_array()
                    .map_or(0, Vec::len)
            };
            let count = preview.nodes.len();
            block = block.title_bottom(Line::from(vec![
                Span::styled(
                    format!(" {count} node{} · ", if count == 1 { "" } else { "s" }),
                    Style::default().fg(theme::SECONDARY),
                ),
                Span::styled(
                    format!(
                        "{diagnostics} diagnostic{} ",
                        if diagnostics == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(if diagnostics == 0 {
                        theme::SECONDARY
                    } else {
                        theme::RUNNING
                    }),
                ),
            ]));
        }
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if self.mouse.enabled
            && self.mouse.show_controls
            && usize::from(area.width) >= title_width + 17
        {
            let x = area.right() - 15;
            self.pointer.button(
                frame,
                Rect::new(x, area.y, 6, 1),
                "[Show]",
                mouse::Action::FitSelected,
            );
            self.pointer.button(
                frame,
                Rect::new(x + 7, area.y, 6, 1),
                "[+Ref]",
                mouse::Action::Reference,
            );
        }
        if self.diagnostics.active {
            self.draw_diagnostics(frame, inner);
            return;
        }
        if !self
            .preview
            .as_ref()
            .is_some_and(|preview| !preview.attachment || preview.summary["revision"].is_string())
        {
            let message = match &self.preview_error {
                Some(error) => format!("Nodes unavailable\n{}", clean(&error.to_string())),
                None => "Loading nodes…".into(),
            };
            frame.render_widget(
                Paragraph::new(message)
                    .style(Style::default().fg(theme::SECONDARY))
                    .wrap(Wrap { trim: false }),
                inner,
            );
            return;
        }
        self.node_tree.viewport_height = inner.height;
        let items = self
            .node_tree
            .rows
            .iter()
            .enumerate()
            .filter_map(|(row_index, row)| {
                let node = self.preview.as_ref()?.nodes.get(row.index)?;
                let indent = row
                    .depth
                    .min(usize::from(inner.width.saturating_sub(10)) / 2);
                let marker = if node.children.is_empty() {
                    " "
                } else if row.expanded {
                    "▾"
                } else {
                    "▸"
                };
                let selected = self.nodes.selected() == Some(row_index);
                let color = if selected {
                    theme::NODE_SELECTION
                } else {
                    theme::NODES
                };
                let badge = if row.diagnostic_count == 0 {
                    String::new()
                } else {
                    format!(
                        " [{} {}]",
                        if row.has_error_diagnostic {
                            "✕"
                        } else {
                            "⚠"
                        },
                        row.diagnostic_count
                    )
                };
                let badge_width = Span::raw(badge.as_str()).width();
                let (name, padding) = if badge.is_empty() {
                    (Cow::Borrowed(node.display_name.as_str()), 0)
                } else {
                    let available =
                        usize::from(inner.width).saturating_sub(indent * 2 + 2 + badge_width);
                    let (name, width) = fitted_label(&node.display_name, available);
                    (name, available.saturating_sub(width))
                };
                Some(ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{}{marker} ", "  ".repeat(indent)),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        name,
                        if selected {
                            Style::default().fg(theme::NODE_SELECTION)
                        } else {
                            Style::default()
                        },
                    ),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(
                        badge,
                        Style::default().fg(if row.has_error_diagnostic {
                            theme::ERROR
                        } else {
                            theme::RUNNING
                        }),
                    ),
                ])))
            })
            .collect::<Vec<_>>();
        if self
            .nodes
            .selected()
            .is_some_and(|index| index >= items.len())
        {
            self.nodes.select(None);
        }
        let mut highlight = theme::node_selected(focused);
        // Keep the severity color when the selected row is highlighted.
        highlight.fg = None;
        self.pointer.list(
            frame,
            List::new(items).highlight_style(highlight),
            inner,
            &mut self.nodes,
            (1, mouse::ListTarget::Nodes),
        );
        for (visible, row) in self
            .node_tree
            .rows
            .iter()
            .skip(self.nodes.offset())
            .take(usize::from(inner.height))
            .enumerate()
        {
            if self
                .preview
                .as_ref()
                .is_some_and(|preview| !preview.nodes[row.index].children.is_empty())
            {
                let indent = row
                    .depth
                    .min(usize::from(inner.width.saturating_sub(10)) / 2);
                self.pointer.add(
                    Rect::new(inner.x + indent as u16 * 2, inner.y + visible as u16, 1, 1),
                    mouse::Target::Branch(self.nodes.offset() + visible),
                );
            }
        }
        if self.node_tree.rows.is_empty() {
            frame.render_widget(
                Paragraph::new(if self.filter.is_empty() {
                    "No nodes"
                } else {
                    "No matching nodes"
                })
                .style(Style::default().fg(theme::SECONDARY)),
                inner,
            );
        }
    }
}

pub(super) fn node_label(value: &str) -> String {
    clean(value).replace(['\n', '\t'], " ")
}

fn fitted_label(value: &str, width: usize) -> (Cow<'_, str>, usize) {
    let span = Span::raw(value);
    if span.width() <= width {
        return (Cow::Borrowed(value), span.width());
    }
    if width == 0 {
        return (Cow::Borrowed(""), 0);
    }
    let mut label = String::new();
    let mut used = 0;
    for grapheme in span.styled_graphemes(Style::default()) {
        let cells = Span::raw(grapheme.symbol).width();
        if used + cells > width - 1 {
            break;
        }
        label.push_str(grapheme.symbol);
        used += cells;
    }
    label.push('…');
    (Cow::Owned(label), used + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Vec<preview::Node> {
        [
            ("sidebar", "Sidebar", None, vec![1, 3]),
            ("room-a", "Room", Some(0), vec![2]),
            ("icon/instance-a", "Icon", Some(1), vec![]),
            ("room-b", "Room", Some(0), vec![4]),
            ("icon/instance-b", "Icon", Some(3), vec![]),
            ("other", "Other", None, vec![]),
        ]
        .into_iter()
        .map(|(id, name, parent, children)| preview::Node {
            id: id.into(),
            name: name.into(),
            display_name: node_label(name),
            parent,
            children,
            bounds: crate::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
            target: json!({"id":id}),
            kind: "rectangle".into(),
            text_excerpt: String::new(),
            diagnostic_count: 0,
            has_error_diagnostic: false,
        })
        .collect()
    }

    #[test]
    fn diagnostic_badges_include_hidden_descendants_and_keep_error_severity() {
        let mut nodes = fixture();
        nodes[2].diagnostic_count = 2;
        nodes[4].diagnostic_count = 1;
        nodes[4].has_error_diagnostic = true;
        let mut tree = Tree::default();
        tree.rebuild(&nodes, "");
        assert_eq!(tree.rows[0].diagnostic_count, 3);
        assert!(tree.rows[0].has_error_diagnostic);
        assert_eq!(tree.rows[1].diagnostic_count, 0);
        tree.expanded.insert("sidebar".into());
        tree.rebuild(&nodes, "");
        assert_eq!(tree.rows[1].diagnostic_count, 2);
        assert!(!tree.rows[1].has_error_diagnostic);
        assert_eq!(tree.rows[2].diagnostic_count, 1);
        assert!(tree.rows[2].has_error_diagnostic);
        tree.rebuild(&nodes, "instance-a");
        assert_eq!(
            tree.rows[0].diagnostic_count, 3,
            "Searching must not hide other problems in a branch"
        );
        let (label, used) = fitted_label("👨‍👩‍👧‍👦日本語の長いノード名", 5);
        assert!(label.starts_with("👨‍👩‍👧‍👦"));
        assert!(label.ends_with('…'));
        assert!(used <= 5);
    }

    #[test]
    fn search_exposes_matching_paths_and_branch_children_without_losing_fold_state() {
        let nodes = fixture();
        let mut tree = Tree::default();
        let indices = |tree: &Tree| tree.rows.iter().map(|row| row.index).collect::<Vec<_>>();
        tree.rebuild(&nodes, "");
        assert_eq!(indices(&tree), [0, 5]);
        tree.expanded.insert("sidebar".into());
        tree.rebuild(&nodes, "");
        assert_eq!(indices(&tree), [0, 1, 3, 5]);
        tree.rebuild(&nodes, "ICON");
        assert_eq!(indices(&tree), [0, 1, 2, 3, 4]);
        assert_eq!(
            tree.rows.iter().map(|row| row.depth).collect::<Vec<_>>(),
            [0, 1, 2, 1, 2]
        );
        assert_eq!(
            tree.rows
                .iter()
                .filter(|row| row.matched)
                .map(|row| row.index)
                .collect::<Vec<_>>(),
            [2, 4]
        );
        tree.rebuild(&nodes, "room-a");
        assert_eq!(indices(&tree), [0, 1, 2]);
        tree.rebuild(&nodes, "instance-b");
        assert_eq!(indices(&tree), [0, 3, 4]);
        tree.rebuild(&nodes, "missing");
        assert!(tree.rows.is_empty());
        tree.rebuild(&nodes, "");
        assert_eq!(indices(&tree), [0, 1, 3, 5]);
    }
}
