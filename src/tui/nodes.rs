//! Collapsible document hierarchy with stable editing targets and cached rows.
use super::*;
use std::collections::HashSet;

pub(super) struct Row {
    pub index: usize,
    depth: usize,
    expanded: bool,
    matched: bool,
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
                });
            }
        }
    }
}

impl App {
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
        let focused = self.focus == Focus::Nodes;
        let mut block = panel(theme::title("▤", title, theme::NODES, focused), focused);
        if let Some(preview) = &self.preview {
            let diagnostics = preview.summary["diagnostics"]
                .as_array()
                .map_or(0, Vec::len);
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
        if self.diagnostics.active {
            self.draw_diagnostics(frame, inner);
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
                let color = if focused && self.nodes.selected() == Some(row_index) {
                    theme::NODE_SELECTION
                } else {
                    theme::NODES
                };
                Some(ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{}{marker} ", "  ".repeat(indent)),
                        Style::default().fg(color),
                    ),
                    Span::raw(node.display_name.as_str()),
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
        frame.render_stateful_widget(
            List::new(items).highlight_style(theme::node_selected(focused)),
            inner,
            &mut self.nodes,
        );
        if self.node_tree.rows.is_empty() && !self.filter.is_empty() {
            frame.render_widget(
                Paragraph::new("No matching nodes").style(Style::default().fg(theme::SECONDARY)),
                inner,
            );
        }
    }
}

pub(super) fn node_label(value: &str) -> String {
    clean(value).replace(['\n', '\t'], " ")
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
        })
        .collect()
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
