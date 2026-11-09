//! Cell-based pointer routing shared by mouse wheels and trackpad scroll events.
use super::*;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListTarget {
    Nodes,
    Diagnostics,
    Attachments,
    References,
    Commands,
    Checkpoints,
    Saved,
    Documents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    ToggleNodes,
    ToggleAgent,
    ToggleNodeNames,
    ZoomIn,
    ZoomOut,
    FitAll,
    FitSelected,
    Reference,
    PlayPause,
    More,
    Commands,
    Help,
    Accept,
    Close,
    DocumentPath,
    WheelMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Focus(Focus),
    Canvas,
    List(ListTarget),
    Row(ListTarget, usize),
    Branch(usize),
    History,
    CommandCompletion(Option<usize>),
    Help,
    DiagnosticDetails,
    SavedText,
    CheckpointReview,
    PermissionDetails,
    AgentPermissions(super::permissions::Hit),
    AgentPermissionDetails,
    PermissionChoice(usize),
    Button(Action),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hit {
    area: Rect,
    target: Target,
}

#[derive(Clone, Copy)]
struct Press {
    button: MouseButton,
    origin: Position,
    last: Position,
    target: Target,
    moved: bool,
    pan: bool,
}

#[derive(Default)]
pub(super) struct Pointer {
    hits: Vec<Hit>,
    previous: Vec<Hit>,
    press: Option<Press>,
    pub space: bool,
    detached: Vec<ListTarget>,
    pub more: bool,
    pub controls: bool,
    modal_state: ListState,
    modal_kind: Option<ListTarget>,
    modal_count: usize,
    pub scope: String,
}

impl Pointer {
    pub fn invalid(&self) -> bool {
        self.hits.is_empty()
    }
    pub fn begin(&mut self) {
        self.previous = std::mem::take(&mut self.hits);
    }
    pub fn finish(&mut self) {
        // Keep an unchanged target clickable while unrelated panels redraw.
        // Scope changes cancel separately; a moved/replaced target cancels here.
        if let Some(press) = self.press {
            let previous = self
                .previous
                .iter()
                .rev()
                .find(|hit| hit.area.contains(press.origin));
            let current = self
                .hits
                .iter()
                .rev()
                .find(|hit| hit.area.contains(press.origin));
            if previous != current || current.is_none_or(|hit| hit.target != press.target) {
                self.press = None;
            }
        }
    }
    pub fn clear(&mut self) {
        self.hits.clear();
        self.cancel();
    }
    pub fn overlay(&mut self) {
        self.hits.clear();
    }
    pub fn cancel(&mut self) {
        self.press = None;
        self.space = false;
    }
    pub fn keyboard(&mut self) {
        self.press = None;
    }
    pub fn reattach(&mut self, target: ListTarget) {
        self.detached.retain(|list| *list != target);
    }
    pub fn reset_modal_list(&mut self) {
        self.modal_state = ListState::default();
        self.modal_kind = None;
        self.detached.retain(|list| {
            !matches!(
                list,
                ListTarget::Commands | ListTarget::Checkpoints | ListTarget::Saved
            )
        });
    }
    pub fn click(&mut self, event: MouseEvent) -> Option<Target> {
        let position = Position::new(event.column, event.row);
        let target = self.at(position);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.press = target.map(|target| Press {
                    button: MouseButton::Left,
                    origin: position,
                    last: position,
                    target,
                    moved: false,
                    pan: false,
                })
            }
            MouseEventKind::Drag(_) => {
                if let Some(press) = &mut self.press {
                    press.moved |= position != press.origin;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(press) = self.press.take()
                    && !press.moved
                    && target == Some(press.target)
                {
                    return target;
                }
            }
            _ => {}
        }
        None
    }
    pub fn add(&mut self, area: Rect, target: Target) {
        if !area.is_empty() {
            self.hits.push(Hit { area, target });
        }
    }
    pub fn at(&self, position: Position) -> Option<Target> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.area.contains(position))
            .map(|hit| hit.target)
    }
    pub fn area(&self, target: Target) -> Option<Rect> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.target == target)
            .map(|hit| hit.area)
    }
    pub fn button(&mut self, frame: &mut Frame, area: Rect, label: &str, action: Action) {
        let width = Line::raw(label).width() as u16;
        if self.controls && area.height > 0 && area.width >= width {
            let area = Rect::new(area.x, area.y, width, 1);
            frame.render_widget(
                Paragraph::new(label).style(
                    Style::default()
                        .fg(theme::colors().key)
                        .add_modifier(Modifier::BOLD),
                ),
                area,
            );
            self.add(area, Target::Button(action));
        }
    }
    pub fn row_hits(
        &mut self,
        area: Rect,
        state: &ListState,
        count: usize,
        height: u16,
        target: ListTarget,
    ) {
        self.add(area, Target::List(target));
        for row in 0..usize::from(area.height / height) {
            let index = state.offset() + row;
            if index >= count {
                break;
            }
            self.add(
                Rect::new(area.x, area.y + row as u16 * height, area.width, height),
                Target::Row(target, index),
            );
        }
    }
    pub fn list(
        &mut self,
        frame: &mut Frame,
        widget: List<'_>,
        area: Rect,
        state: &mut ListState,
        rows: (u16, ListTarget),
    ) {
        let (row_height, target) = rows;
        let count = widget.len();
        let selected = state.selected();
        if self.detached.contains(&target) {
            *state.selected_mut() = selected.filter(|index| {
                *index >= state.offset()
                    && *index < state.offset() + usize::from(area.height / row_height)
            });
        }
        frame.render_stateful_widget(widget, area, state);
        *state.selected_mut() = selected;
        self.row_hits(area, state, count, row_height, target);
    }
    pub fn scroll_list(
        &mut self,
        target: ListTarget,
        state: &mut ListState,
        count: usize,
        rows: usize,
        backwards: bool,
    ) {
        let visible = self.area(Target::List(target)).map_or(1, |area| {
            usize::from(area.height)
                / if target == ListTarget::Diagnostics {
                    2
                } else {
                    1
                }
        });
        let maximum = count.saturating_sub(visible);
        *state.offset_mut() = if backwards {
            state.offset().saturating_sub(rows)
        } else {
            state.offset().saturating_add(rows).min(maximum)
        };
        if !self.detached.contains(&target) {
            self.detached.push(target);
        }
    }
    pub fn grouped_command_rows(&mut self, area: Rect, selected: usize) -> Vec<(usize, u16)> {
        let target = ListTarget::Commands;
        if self.modal_kind != Some(target) {
            self.modal_state = ListState::default();
        }
        self.modal_kind = Some(target);
        self.modal_count = commands::COMMANDS.len();
        let maximum = commands::palette::maximum_start(area.height);
        let mut first = self.modal_state.offset().min(maximum);
        if !self.detached.contains(&target) {
            first = first.min(selected);
            while first < selected
                && commands::palette::rows(first, area.height)
                    .last()
                    .is_none_or(|row| row.0 < selected)
            {
                first += 1;
            }
        }
        *self.modal_state.offset_mut() = first;
        *self.modal_state.selected_mut() = Some(selected);
        let rows = commands::palette::rows(first, area.height)
            .into_iter()
            .map(|(index, y)| (index, area.y + y))
            .collect::<Vec<_>>();
        self.add(area, Target::List(target));
        for &(index, y) in &rows {
            self.add(
                Rect::new(area.x, y, area.width, 1),
                Target::Row(target, index),
            );
        }
        rows
    }
    pub fn modal_list(
        &mut self,
        frame: &mut Frame,
        widget: List<'_>,
        area: Rect,
        selected: usize,
        target: ListTarget,
    ) {
        let mut state = std::mem::take(&mut self.modal_state);
        if self.modal_kind != Some(target) {
            state = ListState::default();
        }
        self.modal_kind = Some(target);
        self.modal_count = widget.len();
        if target == ListTarget::Commands {
            let maximum = self.modal_count.saturating_sub(usize::from(area.height));
            *state.offset_mut() = state.offset().min(maximum);
        }
        state.select((self.modal_count > 0).then_some(selected));
        self.list(frame, widget, area, &mut state, (1, target));
        self.modal_state = state;
    }
}

impl App {
    pub(super) fn pointer_keyboard_navigation(&mut self, key: KeyEvent) {
        self.pointer.keyboard();
        if let Some(modal) = &self.modal {
            if matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::PageUp
                    | KeyCode::PageDown
                    | KeyCode::Char('j' | 'k')
            ) {
                match modal {
                    checkpoints::Modal::Commands { .. } => {
                        self.pointer.reattach(ListTarget::Commands)
                    }
                    checkpoints::Modal::History(_) => {
                        self.pointer.reattach(ListTarget::Checkpoints)
                    }
                    checkpoints::Modal::Saved(_) => self.pointer.reattach(ListTarget::Saved),
                    _ => {}
                }
            }
        } else if self.searching && self.focus == Focus::Nodes {
            self.pointer.reattach(ListTarget::Nodes);
        } else {
            let actions: &[Nav] = match self.focus {
                Focus::Nodes if !self.diagnostics.active => &[
                    Nav::PreviousItem,
                    Nav::NextItem,
                    Nav::NodePageUp,
                    Nav::NodePageDown,
                    Nav::ExpandNode,
                    Nav::CollapseNode,
                    Nav::ToggleNode,
                ],
                Focus::Nodes | Focus::Attachments | Focus::References => {
                    &[Nav::PreviousItem, Nav::NextItem]
                }
                _ => &[],
            };
            if actions
                .iter()
                .any(|action| self.navigation.matches(*action, key))
            {
                match self.focus {
                    Focus::Nodes => self.pointer.reattach(if self.diagnostics.active {
                        ListTarget::Diagnostics
                    } else {
                        ListTarget::Nodes
                    }),
                    Focus::Attachments => self.pointer.reattach(ListTarget::Attachments),
                    Focus::References => self.pointer.reattach(ListTarget::References),
                    _ => {}
                }
            }
        }
    }
    pub(super) fn pointer_scope(&self) -> String {
        if let Some(ui) = &self.permission_ui {
            return format!(
                "{}:{:?}",
                ui.identity(),
                self.conversation
                    .modes()
                    .available
                    .iter()
                    .map(|mode| &mode.id)
                    .collect::<Vec<_>>()
            );
        }
        if let Some(permission) = self.permissions.front() {
            return format!("permission:{}", permission.id);
        }
        if let Some(modal) = &self.modal {
            return match modal {
                checkpoints::Modal::Commands { .. } => "commands".into(),
                checkpoints::Modal::CommandInput(input) => {
                    format!("command-input:{:?}", input.action)
                }
                checkpoints::Modal::History(page) => {
                    format!("checkpoints:{}:{:?}", page.revision, page.cursors)
                }
                checkpoints::Modal::Confirm(page) => {
                    format!("restore:{}:{}", page.revision, page.selected)
                }
                checkpoints::Modal::Saved(page) => page.pointer_identity(),
                checkpoints::Modal::Loading { .. } => "loading".into(),
            };
        }
        if self.help {
            return "help".into();
        }
        if self.pointer.more {
            return "menu".into();
        }
        format!(
            "workspace:{}:{}:{}:{}:{}:{:?}",
            self.conversation.id(),
            self.preview
                .as_ref()
                .and_then(|preview| preview.summary["revision"].as_str())
                .unwrap_or(""),
            self.filter,
            self.pending_images().len(),
            self.outbox.references.len(),
            (self.focus == Focus::Prompt)
                .then(|| commands::completion::query(&self.editor))
                .flatten()
                .filter(|prefix| !commands::completion::candidates(prefix).is_empty())
        )
    }
    pub(super) fn scroll_modal_list(&mut self, list: ListTarget, backwards: bool, step: u16) {
        if self.pointer.modal_kind == Some(list) {
            if list == ListTarget::Commands {
                let height = self
                    .pointer
                    .area(Target::List(list))
                    .map_or(1, |area| area.height);
                let maximum = commands::palette::maximum_start(height);
                let first = self.pointer.modal_state.offset();
                *self.pointer.modal_state.offset_mut() = if backwards {
                    first.saturating_sub(usize::from(step))
                } else {
                    first.saturating_add(usize::from(step)).min(maximum)
                };
                if !self.pointer.detached.contains(&list) {
                    self.pointer.detached.push(list);
                }
                return;
            }
            let mut state = std::mem::take(&mut self.pointer.modal_state);
            self.pointer.scroll_list(
                list,
                &mut state,
                self.pointer.modal_count,
                usize::from(step),
                backwards,
            );
            self.pointer.modal_state = state;
        }
    }
    fn select_modal_row(&mut self, list: ListTarget, index: usize) {
        match (&mut self.modal, list) {
            (Some(checkpoints::Modal::Commands { selected }), ListTarget::Commands) => {
                *selected = index
            }
            (Some(checkpoints::Modal::History(page)), ListTarget::Checkpoints) => {
                page.selected = index
            }
            (Some(checkpoints::Modal::Saved(page)), ListTarget::Saved) => page.selected = index,
            _ => {}
        }
    }
    pub(super) async fn finish_pending_session(&mut self, worker: &Worker) -> Result<()> {
        if let Some(change) = self.pending_session.take()
            && let Err(error) = self.select_conversation(change, worker).await
        {
            if let Some(checkpoints::Modal::Saved(page)) = &mut self.modal {
                page.notice = Some(clean(&error.to_string()));
            }
            return Err(error);
        }
        Ok(())
    }
    async fn accept_modal(&mut self, worker: &Worker) -> Result<bool> {
        let exit = self.activate_modal(worker)?;
        self.finish_pending_session(worker).await?;
        Ok(exit)
    }
    fn close_pointer_modal(&mut self) {
        self.pointer.more = false;
        if let Some(permission) = self.permissions.pop_front() {
            let choice = crate::conversation::reject_once(&permission.request);
            if let Err(error) = self.conversation.respond(&permission.id, choice, true) {
                self.error(error);
            }
        } else if self.help {
            self.help = false;
        } else if let Some(modal) = self.modal.take() {
            match modal {
                checkpoints::Modal::Confirm(page) => {
                    self.modal = Some(checkpoints::Modal::History(page))
                }
                checkpoints::Modal::Loading { .. } => {
                    self.saved_return = None;
                    if let Some(task) = &self.document_task {
                        task.cancel.cancel();
                    }
                }
                _ => {}
            }
        }
        self.pointer.clear();
    }
    pub(super) fn pointer_focus(&mut self, focus: Focus, worker: &Worker) {
        let attachment = self.focus == Focus::Attachments;
        self.focus = focus;
        if focus != Focus::Nodes {
            self.searching = false;
        }
        if focus != Focus::Preview {
            self.preview_search.editing = false;
        }
        if attachment != (focus == Focus::Attachments) {
            self.request(worker, false, false);
        }
    }

    pub(super) fn effective_camera(&mut self) {
        if self.view.zoom.is_none()
            && let Some(preview) = &self.preview
            && !preview.attachment
        {
            self.view.center = preview.view.center;
            self.view.zoom = preview.view.zoom;
        }
    }

    pub(super) fn gesture_canvas(&self) -> Option<Rect> {
        (self.mouse.enabled
            && !self.has_modal()
            && self.focus != Focus::Attachments
            && self
                .preview
                .as_ref()
                .is_some_and(|preview| !preview.attachment))
        .then(|| self.pointer.area(Target::Canvas))
        .flatten()
    }

    fn zoom_pointer(
        &mut self,
        factor: f32,
        position: Option<Position>,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        if self
            .preview
            .as_ref()
            .is_none_or(|preview| preview.attachment)
        {
            return Ok(());
        }
        self.effective_camera();
        let old = self.view.zoom.unwrap_or(1.0);
        let zoom = (old * factor).clamp(0.01, 16.0);
        if let (Some(position), Some(area)) = (position, self.pointer.area(Target::Canvas)) {
            let font = picker.font_size();
            let offset = [
                (f64::from(position.x) + 0.5 - f64::from(area.x) - f64::from(area.width) / 2.0)
                    * f64::from(font.width),
                (f64::from(position.y) + 0.5 - f64::from(area.y) - f64::from(area.height) / 2.0)
                    * f64::from(font.height),
            ];
            for (axis, offset) in offset.into_iter().enumerate() {
                self.view.center[axis] += offset * (1.0 / f64::from(old) - 1.0 / f64::from(zoom));
            }
        }
        self.view.zoom = Some(zoom);
        self.request_camera_navigation(worker, picker)
    }

    pub(super) fn gesture_event(
        &mut self,
        event: gestures::GestureEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        match event {
            gestures::GestureEvent::Pinch(event) => self.pinch_event(event, picker, worker),
            gestures::GestureEvent::Scroll(event) => {
                self.precise_scroll_event(event, picker, worker)
            }
        }
    }

    fn precise_scroll_event(
        &mut self,
        event: gestures::ScrollEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        if !self.mouse.enabled
            || !self.mouse.native_scroll
            || self.has_modal()
            || self.focus == Focus::Attachments
            || self.pointer.at(event.position) != Some(Target::Canvas)
            || self
                .preview
                .as_ref()
                .is_none_or(|preview| preview.attachment)
            || event.pixels.iter().any(|value| !value.is_finite())
        {
            return Ok(());
        }
        let [mut x, mut y] = event.pixels;
        if x != 0.0 || y != 0.0 {
            self.input_profile.input("scroll", KeyEventKind::Press);
            self.pointer.press = None;
            self.effective_camera();
            if y != 0.0 && (event.modifiers.contains(KeyModifiers::CONTROL) ^ self.mouse.wheel_zoom)
            {
                let step = f64::from(picker.font_size().height) * f64::from(self.mouse.scroll_step);
                let factor = f64::from(self.mouse.zoom_step).powf(-y / step);
                self.zoom_pointer(
                    factor.clamp(0.000001, 1_000_000.0) as f32,
                    Some(event.position),
                    picker,
                    worker,
                )?;
                y = 0.0;
            } else if event.modifiers.contains(KeyModifiers::SHIFT) {
                x += y;
                y = 0.0;
            }
            if x != 0.0 || y != 0.0 {
                self.view.pan([x, y]);
                self.request_camera_navigation(worker, picker)?;
            }
            self.dirty = true;
        }
        if event.finished {
            self.finish_camera_gesture(worker);
        }
        Ok(())
    }

    pub(super) fn pinch_event(
        &mut self,
        event: gestures::PinchEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<()> {
        if !self.mouse.enabled
            || !self.mouse.native_pinch
            || self.has_modal()
            || self.focus == Focus::Attachments
            || self.pointer.at(event.position) != Some(Target::Canvas)
        {
            return Ok(());
        }
        if event.factor != 1.0 && event.factor.is_finite() && event.factor > 0.0 {
            self.input_profile.input("pinch", KeyEventKind::Press);
            self.zoom_pointer(
                event.factor.clamp(0.000001, 1_000_000.0) as f32,
                Some(event.position),
                picker,
                worker,
            )?;
            self.dirty = true;
        }
        if event.finished {
            self.finish_camera_gesture(worker);
        }
        Ok(())
    }

    pub(super) fn toggle_playback(&mut self) -> Result<()> {
        if self.playback.active() {
            self.pause_playback();
        } else {
            self.start_playback(playback::DEFAULT_FPS, Instant::now())?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) async fn mouse_event(
        &mut self,
        event: MouseEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        self.mouse_events(event, 1, picker, worker).await
    }

    pub(super) async fn mouse_events(
        &mut self,
        event: MouseEvent,
        count: u16,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        if !self.mouse.enabled {
            return Ok(false);
        }
        let position = Position::new(event.column, event.row);
        let target = self.pointer.at(position);
        self.native_input.observe_pointer(
            position,
            (target == Some(Target::Canvas))
                .then(|| self.gesture_canvas())
                .flatten(),
            picker.font_size(),
            &self.mouse,
        );
        let scope_valid = self.pointer.scope == self.pointer_scope();
        self.input_profile
            .pointer(event, count, target, scope_valid);
        if !scope_valid {
            self.pointer.clear();
            self.dirty = true;
            return Ok(false);
        }
        match event.kind {
            MouseEventKind::Moved => return Ok(false),
            MouseEventKind::Down(button) => {
                self.pointer.press = target.map(|target| Press {
                    button,
                    origin: position,
                    last: position,
                    target,
                    moved: false,
                    pan: target == Target::Canvas
                        && (button == MouseButton::Middle
                            || button == MouseButton::Right
                            || button == MouseButton::Left && self.pointer.space),
                });
                if target == Some(Target::Canvas)
                    && button == MouseButton::Left
                    && self.pointer.press.is_some_and(|press| !press.pan)
                {
                    self.begin_canvas_selection(position, event.modifiers, picker);
                }
            }
            MouseEventKind::Drag(_) => {
                if let Some(mut press) = self.pointer.press {
                    press.moved |= position != press.origin;
                    if press.pan && position != press.last {
                        self.effective_camera();
                        let font = picker.font_size();
                        self.view.pan([
                            (f64::from(press.last.x) - f64::from(position.x))
                                * f64::from(font.width),
                            (f64::from(press.last.y) - f64::from(position.y))
                                * f64::from(font.height),
                        ]);
                        self.request_camera_navigation(worker, picker)?;
                    }
                    if !press.pan
                        && press.target == Target::Canvas
                        && press.button == MouseButton::Left
                        && press.moved
                    {
                        self.drag_canvas_selection(position, picker, worker)?;
                    }
                    press.last = position;
                    self.pointer.press = Some(press);
                }
            }
            MouseEventKind::Up(button) => {
                let press = self.pointer.press.take();
                // Older terminals may omit Space release. Never leave a pan
                // modifier latched after completing a pointer gesture.
                self.pointer.space = false;
                self.finish_camera_gesture(worker);
                if press
                    .is_some_and(|press| press.target == Target::Canvas && press.button == button)
                {
                    self.pointer_focus(Focus::Preview, worker);
                }
                if let Some(press) = press
                    && press.target == Target::Canvas
                    && !press.pan
                    && button == MouseButton::Left
                    && press.button == button
                {
                    self.finish_canvas_selection(position, press.moved, picker, worker)?;
                    return Ok(false);
                }
                if let Some(press) = press
                    && button == press.button
                    && !press.moved
                    && !press.pan
                    && target == Some(press.target)
                    && button == MouseButton::Left
                {
                    return self.pointer_click(press.target, picker, worker).await;
                }
            }
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {
                self.pointer.press = None;
                let backwards = matches!(
                    event.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft
                );
                let horizontal = matches!(
                    event.kind,
                    MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                );
                if target == Some(Target::Canvas) {
                    if self.focus == Focus::Attachments || self.preview.is_none() {
                        return Ok(false);
                    }
                    if !horizontal
                        && (event.modifiers.contains(KeyModifiers::CONTROL) ^ self.mouse.wheel_zoom)
                    {
                        let factor = if backwards {
                            self.mouse.zoom_step
                        } else {
                            1.0 / self.mouse.zoom_step
                        };
                        self.zoom_pointer(
                            factor.powi(i32::from(count)),
                            Some(position),
                            picker,
                            worker,
                        )?;
                    } else {
                        self.effective_camera();
                        let font = picker.font_size();
                        let horizontal =
                            horizontal || event.modifiers.contains(KeyModifiers::SHIFT);
                        let step = f64::from(if horizontal { font.width } else { font.height })
                            * f64::from(self.mouse.scroll_step)
                            * f64::from(count)
                            * if backwards { -1.0 } else { 1.0 };
                        self.view
                            .pan(if horizontal { [step, 0.0] } else { [0.0, step] });
                        self.request_camera_navigation(worker, picker)?;
                    }
                } else if !horizontal && let Some(target) = target {
                    self.pointer_scroll(target, backwards, count);
                }
            }
        }
        self.dirty = true;
        Ok(false)
    }

    fn pointer_scroll(&mut self, target: Target, backwards: bool, count: u16) {
        let step = self.mouse.scroll_step.saturating_mul(count);
        let scroll = |value: &mut u16| {
            *value = if backwards {
                value.saturating_sub(step)
            } else {
                value.saturating_add(step)
            }
        };
        let list = match target {
            Target::Row(list, _) | Target::List(list) => Some(list),
            Target::Branch(_) => Some(ListTarget::Nodes),
            _ => None,
        };
        if let Some(list) = list {
            let attachment_count = self.pending_images().len();
            let (state, count, row_height) = match list {
                ListTarget::Nodes => (&mut self.nodes, self.node_tree.rows().len(), 1),
                ListTarget::Attachments => (&mut self.attachments, attachment_count, 1),
                ListTarget::References => {
                    (&mut self.reference_rows, self.outbox.references.len(), 1)
                }
                ListTarget::Diagnostics => {
                    self.scroll_diagnostic_list(backwards, step);
                    return;
                }
                ListTarget::Commands
                | ListTarget::Checkpoints
                | ListTarget::Saved
                | ListTarget::Documents => {
                    self.scroll_modal_list(list, backwards, step);
                    return;
                }
            };
            self.pointer.scroll_list(
                list,
                state,
                count,
                usize::from(step * row_height),
                backwards,
            );
        } else {
            match target {
                Target::History => {
                    self.scroll_history(backwards, step);
                    self.scroll = self.scroll.min(self.history_metrics.2);
                }
                Target::Help => scroll(&mut self.help_scroll),
                Target::DiagnosticDetails => self.scroll_diagnostic_details(backwards, step),
                Target::AgentPermissionDetails => {
                    if let Some(ui) = self.permission_ui.as_mut() {
                        scroll(&mut ui.scroll);
                    }
                }
                Target::AgentPermissions(permissions::Hit::Row(_)) => {
                    let key = KeyEvent::new(
                        if backwards {
                            KeyCode::Up
                        } else {
                            KeyCode::Down
                        },
                        KeyModifiers::NONE,
                    );
                    let _ = self.permission_key(key);
                }
                Target::PermissionDetails => {
                    if let Some(permission) = self.permissions.front_mut() {
                        scroll(&mut permission.scroll);
                    }
                }
                Target::SavedText => {
                    if let Some(checkpoints::Modal::Saved(page)) = &mut self.modal {
                        page.pointer_scroll(backwards, step);
                    }
                }
                Target::CheckpointReview => {
                    if let Some(checkpoints::Modal::Confirm(page)) = &mut self.modal {
                        scroll(&mut page.review_scroll);
                        page.review_scroll = page.review_scroll.min(page.review_maximum_scroll);
                    }
                }
                _ => {}
            }
        }
    }

    async fn pointer_click(
        &mut self,
        target: Target,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        match target {
            Target::Canvas => self.pointer_focus(Focus::Preview, worker),
            Target::Focus(focus) => self.pointer_focus(focus, worker),
            Target::List(list) => match list {
                ListTarget::Nodes | ListTarget::Diagnostics => {
                    self.pointer_focus(Focus::Nodes, worker)
                }
                ListTarget::Attachments => self.pointer_focus(Focus::Attachments, worker),
                ListTarget::References => self.pointer_focus(Focus::References, worker),
                _ => {}
            },
            Target::DiagnosticDetails => self.pointer_focus(Focus::Nodes, worker),
            Target::History => self.pointer_focus(Focus::Conversation, worker),
            Target::CommandCompletion(Some(index)) => self.complete_command(index),
            Target::Row(list, index) => match list {
                ListTarget::Nodes => {
                    self.nodes.select(Some(index));
                    self.pointer_focus(Focus::Nodes, worker);
                }
                ListTarget::Attachments => {
                    self.attachments.select(Some(index));
                    let already_focused = self.focus == Focus::Attachments;
                    self.pointer_focus(Focus::Attachments, worker);
                    if already_focused {
                        self.request(worker, false, false);
                    }
                }
                ListTarget::References => {
                    self.reference_rows.select(Some(index));
                    self.pointer_focus(Focus::References, worker);
                }
                ListTarget::Diagnostics => {
                    self.select_diagnostic_row(index);
                    self.pointer_focus(Focus::Nodes, worker);
                }
                _ => self.select_modal_row(list, index),
            },
            Target::Branch(index) => {
                self.nodes.select(Some(index));
                self.pointer_focus(Focus::Nodes, worker);
                self.toggle_pointer_branch(index);
            }
            Target::AgentPermissions(hit) => {
                if self.permission_ui.is_none() {
                    self.open_agent_permissions();
                } else {
                    self.permission_click(hit)?;
                }
            }
            Target::PermissionChoice(index) => {
                if let Some(permission) = self.permissions.front_mut() {
                    permission.selected = index;
                }
            }
            Target::Button(action) => return self.pointer_action(action, picker, worker).await,
            _ => {}
        }
        self.dirty = true;
        Ok(false)
    }

    async fn pointer_action(
        &mut self,
        action: Action,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        if action != Action::More {
            self.pointer.more = false;
        }
        match action {
            Action::ToggleNodes => self.toggle_panel(Nav::ToggleNodesPanel, worker),
            Action::ToggleAgent => self.toggle_panel(Nav::ToggleAgentPanel, worker),
            Action::ToggleNodeNames => {
                self.execute_command(commands::Action::ToggleNodeNames, &[], worker)?;
            }
            Action::ZoomIn => self.zoom_pointer(1.25, None, picker, worker)?,
            Action::ZoomOut => self.zoom_pointer(1.0 / 1.25, None, picker, worker)?,
            Action::FitAll => {
                self.view.zoom = None;
                self.request(worker, false, false);
            }
            Action::FitSelected if self.diagnostics.active => {
                self.fit_diagnostic(false, picker, worker)?
            }
            Action::FitSelected => {
                self.fit_selected(picker)?;
                self.request(worker, false, false);
                self.camera_feedback(picker)?;
            }
            Action::Reference => {
                if self.diagnostics.active {
                    self.reference_selected_diagnostic()?;
                } else {
                    self.reference_selected_node()?;
                }
            }
            Action::PlayPause => self.toggle_playback()?,
            Action::More => self.pointer.more = !self.pointer.more,
            Action::Commands => {
                self.pointer.more = false;
                self.modal = Some(checkpoints::Modal::commands());
            }
            Action::Help => {
                self.pointer.more = false;
                self.show_help();
            }
            Action::Accept => {
                if !self.permissions.is_empty() {
                    self.respond_permission()?;
                } else {
                    return self.accept_modal(worker).await;
                }
            }
            Action::Close => {
                self.close_pointer_modal();
            }
            Action::DocumentPath => {}
            Action::WheelMode => self.mouse.wheel_zoom = !self.mouse.wheel_zoom,
        }
        self.pointer.clear();
        self.dirty = true;
        Ok(false)
    }

    pub(super) fn pointer_menu_area(area: Rect) -> Rect {
        super::modal::area(area)
    }

    pub(super) fn draw_pointer_controls(
        &mut self,
        frame: &mut Frame,
        workspace: &workspace::Areas,
    ) {
        let header = workspace.header;
        let mut x = header.right().saturating_sub(16);
        if self.mouse.show_controls && self.header_line().width() + 17 <= usize::from(header.width)
        {
            for (label, action) in [
                ("[N]", Action::ToggleNodes),
                ("[A]", Action::ToggleAgent),
                ("[Menu]", Action::More),
            ] {
                self.pointer.button(
                    frame,
                    Rect::new(x, header.y, header.right().saturating_sub(x), 1),
                    label,
                    action,
                );
                x += label.len() as u16 + 1;
            }
        }
        let preview = workspace.panes[1];
        let play = if self.playback.active() {
            "[Pause]"
        } else {
            "[Play]"
        };
        let labels = [
            ("[-]", Action::ZoomOut),
            ("[+]", Action::ZoomIn),
            ("[Fit]", Action::FitAll),
            (play, Action::PlayPause),
            ("[Menu]", Action::More),
        ];
        let width = labels
            .iter()
            .map(|(label, _)| label.len() as u16 + 1)
            .sum::<u16>();
        if self.mouse.show_controls && preview.width >= width + 13 {
            let mut x = preview.right() - width;
            for (label, action) in labels {
                self.pointer.button(
                    frame,
                    Rect::new(x, preview.y, label.len() as u16, 1),
                    label,
                    action,
                );
                x += label.len() as u16 + 1;
            }
        } else if self.mouse.show_controls {
            self.pointer.button(
                frame,
                Rect::new(
                    preview.right().saturating_sub(6),
                    preview.y,
                    5.min(preview.width),
                    1,
                ),
                "[...]",
                Action::More,
            );
        }
        if self.pointer.more {
            let popup = Self::pointer_menu_area(frame.area());
            let popup = popup.intersection(frame.area());
            let shell = super::modal::Shell::draw(frame, popup, "View controls", 1);
            let inner = shell.body;
            // A menu captures all pointer input, including empty space.
            self.pointer.hits.clear();
            shell.close(frame, &mut self.pointer);
            frame.render_widget(
                Paragraph::new(super::modal::hints(&[("Esc", "Close")])),
                shell.footer,
            );
            let wheel = if self.mouse.wheel_zoom {
                "[Wheel: zoom]"
            } else {
                "[Wheel: pan]"
            };
            let names = if self.preview_settings.show_node_names {
                "[Node names: on]"
            } else {
                "[Node names: off]"
            };
            let columns = if inner.height < 12 { 2 } else { 1 };
            let rows = 12u16.div_ceil(columns);
            let width = inner.width / columns;
            for (row, (label, action)) in [
                ("[-] Zoom out", Action::ZoomOut),
                ("[+] Zoom in", Action::ZoomIn),
                ("[Fit] Whole document", Action::FitAll),
                ("[Show] Selected node", Action::FitSelected),
                ("[+Ref] Add reference", Action::Reference),
                (play, Action::PlayPause),
                (wheel, Action::WheelMode),
                (names, Action::ToggleNodeNames),
                ("[Commands]", Action::Commands),
                ("[Help]", Action::Help),
                ("[N] Toggle Nodes", Action::ToggleNodes),
                ("[A] Toggle Agent", Action::ToggleAgent),
            ]
            .into_iter()
            .enumerate()
            {
                let x = inner.x + (row as u16 / rows) * width;
                let y = inner.y + row as u16 % rows;
                if y < inner.bottom() {
                    self.pointer
                        .button(frame, Rect::new(x, y, width, 1), label, action);
                }
            }
        }
    }
}
