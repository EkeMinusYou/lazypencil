//! Search the rendered document's text and navigate matching canvas elements.
use super::*;

#[derive(Default)]
pub(super) struct Search {
    pub query: String,
    pub editing: bool,
    matches: Vec<String>,
    current: Option<usize>,
}

impl Search {
    pub fn refresh(&mut self, nodes: &[preview::Node]) {
        let selected = self
            .current
            .and_then(|index| self.matches.get(index))
            .cloned();
        self.matches.clear();
        if !self.query.is_empty() {
            let query = self.query.to_lowercase();
            self.matches.extend(
                nodes
                    .iter()
                    .enumerate()
                    .filter(|(index, node)| {
                        selection::visible_in(nodes, *index, node.bounds)
                            && [&node.text, &node.name, &node.id]
                                .iter()
                                .any(|text| text.to_lowercase().contains(&query))
                    })
                    .map(|(_, node)| node.id.clone()),
            );
        }
        self.current = selected.and_then(|id| self.matches.iter().position(|item| *item == id));
    }

    pub(super) fn summary(&self) -> Option<String> {
        if self.query.is_empty() {
            None
        } else if self.matches.is_empty() {
            Some("No matches".into())
        } else {
            Some(format!(
                "{}/{}",
                self.current.map_or(0, |index| index + 1),
                self.matches.len()
            ))
        }
    }

    fn step(&mut self, backwards: bool) -> Option<String> {
        let count = self.matches.len();
        if count == 0 {
            self.current = None;
            return None;
        }
        let index = match self.current {
            Some(index) if backwards => (index + count - 1) % count,
            Some(index) => (index + 1) % count,
            None if backwards => count - 1,
            None => 0,
        };
        self.current = Some(index);
        Some(self.matches[index].clone())
    }
}

impl App {
    pub(super) fn start_preview_search(&mut self) {
        self.pause_playback();
        self.pointer.cancel();
        self.preview_search.editing = true;
    }

    pub(super) fn preview_search_input(
        &mut self,
        key: KeyEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        match search::edit(&mut self.preview_search.query, key) {
            search::Edit::Clear => self.preview_search = Default::default(),
            search::Edit::Keep => self.preview_search.editing = false,
            search::Edit::Changed => self.update_preview_search(picker, worker)?,
            search::Edit::Ignored => {}
        }
        Ok(())
    }

    pub(super) fn paste_preview_search(
        &mut self,
        text: &str,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        search::paste(&mut self.preview_search.query, text);
        self.update_preview_search(picker, worker)
    }

    fn update_preview_search(&mut self, picker: &graphics::Backend, worker: &Worker) -> Result<()> {
        self.preview_search.current = None;
        if let Some(preview) = &self.preview {
            self.preview_search.refresh(&preview.nodes);
        }
        self.step_preview_search(false, picker, worker)
    }

    pub(super) fn step_preview_search(
        &mut self,
        backwards: bool,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        let Some(id) = self.preview_search.step(backwards) else {
            return Ok(());
        };
        self.pause_playback();
        self.selection.cancel();
        self.selection.ids = vec![id.clone()];
        self.filter.clear();
        self.searching = false;
        self.diagnostics.active = false;
        self.refresh_nodes(Some(&id));
        self.pointer.reattach(mouse::ListTarget::Nodes);
        self.selection.presentation = None;
        self.selection.invalidate();
        self.fit_selected(picker)?;
        self.request(worker, false, false);
        self.camera_feedback(picker)?;
        Ok(())
    }
}
