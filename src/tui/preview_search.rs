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
        match key.code {
            KeyCode::Esc => self.preview_search = Default::default(),
            KeyCode::Enter => self.preview_search.editing = false,
            KeyCode::Down | KeyCode::Tab => self.step_preview_search(false, picker, worker)?,
            KeyCode::Up | KeyCode::BackTab => self.step_preview_search(true, picker, worker)?,
            KeyCode::Backspace => {
                self.preview_search.query.pop();
                self.update_preview_search(picker, worker)?;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !c.is_control() =>
            {
                self.paste_preview_search(&c.to_string(), picker, worker)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn paste_preview_search(
        &mut self,
        text: &str,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        for c in text.chars().filter(|c| !c.is_control()) {
            if self.preview_search.query.len() + c.len_utf8() > 1024 * 1024 {
                break;
            }
            self.preview_search.query.push(c);
        }
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

    pub(super) fn draw_preview_search(&self, frame: &mut Frame, pane: Rect) {
        let search = &self.preview_search;
        if (!search.editing && search.query.is_empty()) || pane.width < 4 || pane.height < 2 {
            return;
        }
        let area = Rect::new(pane.x + 1, pane.bottom() - 1, pane.width - 2, 1);
        let count = if search.query.is_empty() {
            String::new()
        } else if search.matches.is_empty() {
            " No matches ".into()
        } else {
            format!(
                " {}/{} ",
                search.current.map_or(0, |n| n + 1),
                search.matches.len()
            )
        };
        let available = usize::from(area.width).saturating_sub(count.len() + 2);
        let query = clean(&search.query);
        let mut start = query.len();
        let mut width = 0;
        for (index, c) in query.char_indices().rev() {
            width += Span::raw(c.to_string()).width();
            if width > available {
                break;
            }
            start = index;
        }
        let input = Line::from(format!("/{}", &query[start..]));
        let cursor = input.width() as u16;
        frame.render_widget(Clear, area);
        frame.render_widget(input.style(Style::default().fg(theme::colors().key)), area);
        if count.len() <= usize::from(area.width) {
            frame.render_widget(
                Line::styled(&count, Style::default().fg(theme::colors().secondary))
                    .right_aligned(),
                area,
            );
        }
        if search.editing && !self.help && self.modal.is_none() && self.permissions.is_empty() {
            frame.set_cursor_position((area.x + cursor.min(area.width - 1), area.y));
        }
    }
}
