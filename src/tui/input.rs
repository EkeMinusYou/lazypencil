//! Route terminal keys in priority order before dispatching to focused handlers.
use super::*;

impl App {
    pub(super) async fn key(
        &mut self,
        key: KeyEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        if key.kind == KeyEventKind::Release {
            return Ok(false);
        }
        self.pointer_keyboard_navigation(key);
        if self.mouse.enabled
            && self.focus == Focus::Preview
            && !self.has_modal()
            && key.code == KeyCode::Char(' ')
            && key.modifiers.is_empty()
        {
            self.pointer.space = true;
            return Ok(false);
        }
        if self.pointer.more {
            self.pointer.more = false;
            if key.code == KeyCode::Esc {
                return Ok(false);
            }
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Char('c') && ctrl {
            return self.cancel_input();
        }
        if !self.permissions.is_empty() {
            self.permission_input(key)?;
            return Ok(false);
        }
        let typing = self.focus == Focus::Prompt
            || self.searching
            || matches!(self.modal, Some(checkpoints::Modal::CommandInput(_)));
        let global = |action| {
            if typing {
                self.navigation.matches_while_typing(action, key)
            } else {
                self.navigation.matches(action, key)
            }
        };
        if global(Nav::Commands) {
            self.help = false;
            if self.modal.take().is_some() {
                self.saved_return = None;
                if let Some(task) = &self.document_task {
                    task.cancel.cancel();
                }
            } else {
                self.modal = Some(checkpoints::Modal::commands());
            }
            return Ok(false);
        }
        if self.modal.is_some() {
            let exit = self.modal_key(key, worker)?;
            self.finish_pending_session(worker).await?;
            return Ok(exit);
        }
        if global(Nav::Help) {
            self.help = !self.help;
            self.help_scroll = 0;
            return Ok(false);
        }
        if self.help {
            self.help_input(key);
            return Ok(false);
        }
        if global(Nav::ToggleNodesPanel) || global(Nav::ToggleAgentPanel) {
            self.toggle_panel(
                if global(Nav::ToggleNodesPanel) {
                    Nav::ToggleNodesPanel
                } else {
                    Nav::ToggleAgentPanel
                },
                worker,
            );
            return Ok(false);
        }
        if self.searching {
            self.search_input(key);
            return Ok(false);
        }
        let section = self.focus.in_conversation()
            && (global(Nav::PreviousSection) || global(Nav::NextSection));
        let return_to_prompt = matches!(
            self.focus,
            Focus::Attachments | Focus::References | Focus::Conversation
        ) && key.code == KeyCode::Esc;
        if section || return_to_prompt || global(Nav::NextPanel) || global(Nav::PreviousPanel) {
            let was_attachment = self.focus == Focus::Attachments;
            self.focus = if return_to_prompt {
                Focus::Prompt
            } else if section {
                self.focus.next_section(
                    global(Nav::PreviousSection),
                    !self.conversation.active() && !self.conversation.images().is_empty(),
                    !self.outbox.references.is_empty(),
                )
            } else {
                self.focus.next(
                    global(Nav::PreviousPanel),
                    self.nodes_collapsed,
                    self.agent_collapsed,
                )
            };
            if self.focus == Focus::Attachments
                && !self.conversation.images().is_empty()
                && self.attachments.selected().is_none()
            {
                self.attachments.select(Some(0));
            }
            if self.focus == Focus::References && self.reference_rows.selected().is_none() {
                self.reference_rows.select(Some(0));
            }
            if was_attachment || self.focus == Focus::Attachments {
                self.request(worker, false, false);
            }
            return Ok(false);
        }
        if self.focus == Focus::Prompt {
            return self.prompt_input(key, worker).await;
        }
        self.panel_input(key, picker, worker)
    }
    fn cancel_input(&mut self) -> Result<bool> {
        if self.conversation.active() {
            self.outbox.paused = true;
            self.outbox.save(self.conversation.directory())?;
        }
        if let Some(task) = &self.document_task {
            task.cancel.cancel();
            if self.conversation.active() {
                self.conversation.cancel();
                self.permissions.clear();
                self.set_attention_status("Cancelling saved-log read and active request…".into());
            } else {
                self.set_attention_status("Cancelling document operation…".into());
            }
        } else if let Some(input) = self.input.take() {
            input.cancel.cancel();
            self.set_status("Image input cancelled".into());
        } else if self.conversation.active() {
            self.conversation.cancel();
            self.permissions.clear();
            self.set_attention_status("Cancelling request; committed edits are retained".into());
        } else if self.playback.active() {
            self.pause_playback();
        } else {
            return Ok(true);
        }
        Ok(false)
    }
    fn permission_input(&mut self, key: KeyEvent) -> Result<()> {
        if key.code == KeyCode::Enter {
            return self.respond_permission();
        }
        let permission = self.permissions.front_mut().expect("pending permission");

        let count = permission.request.options.len();
        let choice = match key.code {
            KeyCode::PageUp => {
                permission.scroll = permission.scroll.saturating_sub(5);
                None
            }
            KeyCode::PageDown => {
                permission.scroll = permission.scroll.saturating_add(5);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                permission.selected = permission.selected.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                permission.selected = (permission.selected + 1).min(count.saturating_sub(1));
                None
            }
            KeyCode::Esc => Some(crate::conversation::reject_once(&permission.request)),
            _ => None,
        };
        if let Some(choice) = choice {
            let permission = self.permissions.pop_front().unwrap();
            self.conversation.respond(&permission.id, choice, true)?;
        }
        Ok(())
    }
    pub(super) fn respond_permission(&mut self) -> Result<()> {
        if let Some(permission) = self.permissions.pop_front() {
            let choice = permission
                .request
                .options
                .get(permission.selected)
                .map(|option| option.option_id.to_string());
            self.conversation.respond(&permission.id, choice, true)?;
        }
        Ok(())
    }
    fn help_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.help = false,
            KeyCode::PageUp | KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(5),
            KeyCode::PageDown | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(5)
            }
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = u16::MAX,
            _ => (),
        }
    }
    fn search_input(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        let selected = self.selected_node().map(|node| node.id.clone());
        match key.code {
            KeyCode::Esc => {
                self.searching = false;
                self.filter.clear();
            }
            KeyCode::Enter => self.searching = false,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) if !ctrl && !alt => self.filter.push(c),
            _ => {}
        }
        self.search_nodes(selected.as_deref());
    }
    async fn prompt_input(&mut self, key: KeyEvent, worker: &Worker) -> Result<bool> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        if key.code == KeyCode::Enter
            && key.modifiers.is_empty()
            && self.editor.lines().len() == 1
            && self.editor.lines()[0].starts_with('/')
        {
            return self.command(worker).await;
        }
        if self.keys.pastes(key) {
            self.start_input(None)?;
            return Ok(false);
        }
        if self.keys.submits(key) {
            self.submit().await?;
            return Ok(false);
        }
        if key.code == KeyCode::Enter
            && key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
        {
            self.insert_text("\n");
            return Ok(false);
        }
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('u') => {
                    self.draft_changed |= self.editor.delete_line_by_head();
                    return Ok(false);
                }
                KeyCode::Char('_' | '/') => {
                    self.draft_changed |= self.editor.undo();
                    return Ok(false);
                }
                KeyCode::Char('y') => {
                    self.insert_text(&self.editor.yank_text());
                    return Ok(false);
                }
                _ => {}
            }
        }
        // Bound interactive input as well as bracketed and clipboard pastes.
        if self.editor.lines().iter().map(String::len).sum::<usize>() >= 1024 * 1024
            && (key.code == KeyCode::Enter || matches!(key.code, KeyCode::Char(_)) && !ctrl && !alt)
        {
            return Err(Error::new("INVALID_ARGUMENT", "Prompt exceeds 1 MiB"));
        }
        self.draft_changed |= self.editor.input(key);
        Ok(false)
    }
    fn panel_input(
        &mut self,
        key: KeyEvent,
        picker: &graphics::Backend,
        worker: &Worker,
    ) -> Result<bool> {
        let action = |action| self.navigation.matches(action, key);
        if action(Nav::Quit) {
            return Ok(true);
        }
        if action(Nav::OpenPreview) {
            self.open_preview()?;
            return Ok(false);
        }
        if action(Nav::Refresh) {
            self.request(worker, false, true);
            return Ok(false);
        }
        match self.focus {
            Focus::Preview => {
                if key.code == KeyCode::Esc && !self.selection.ids.is_empty() {
                    self.selection.clear();
                    self.nodes.select(None);
                    return Ok(false);
                }
                if action(Nav::SelectChild) || action(Nav::SelectParent) {
                    self.navigate_canvas_hierarchy(action(Nav::SelectChild), picker, worker)?;
                    return Ok(false);
                }
                if action(Nav::PlayPause) {
                    self.toggle_playback()?;
                    return Ok(false);
                }
                let zoom = self.view.zoom.unwrap_or(1.0);
                if action(Nav::PanLeft) {
                    self.input_profile.input("pan_left", key.kind);
                    self.view.pan([-64.0, 0.0]);
                } else if action(Nav::PanRight) {
                    self.input_profile.input("pan_right", key.kind);
                    self.view.pan([64.0, 0.0]);
                } else if action(Nav::PanUp) {
                    self.input_profile.input("pan_up", key.kind);
                    self.view.pan([0.0, -64.0]);
                } else if action(Nav::PanDown) {
                    self.input_profile.input("pan_down", key.kind);
                    self.view.pan([0.0, 64.0]);
                } else if action(Nav::ZoomIn) {
                    self.input_profile.input("zoom_in", key.kind);
                    self.view.zoom = Some((zoom * 1.25).min(16.0));
                } else if action(Nav::ZoomOut) {
                    self.input_profile.input("zoom_out", key.kind);
                    self.view.zoom = Some((zoom / 1.25).max(0.01));
                } else if action(Nav::FitDocument) {
                    self.view.zoom = None;
                } else if action(Nav::TimeBackward) {
                    self.view.set_time((self.view.time - 0.1).max(0.0))?;
                    self.playback.stop();
                } else if action(Nav::TimeForward) {
                    self.view.set_time(self.view.time + 0.1)?;
                    self.playback.stop();
                } else if action(Nav::FitSelected) {
                    self.fit_selected(picker)?;
                } else {
                    return Ok(false);
                }
                if [
                    Nav::PanLeft,
                    Nav::PanRight,
                    Nav::PanUp,
                    Nav::PanDown,
                    Nav::ZoomIn,
                    Nav::ZoomOut,
                ]
                .into_iter()
                .any(|navigation| self.navigation.matches(navigation, key))
                {
                    self.request_camera_navigation(worker, picker)?;
                } else {
                    self.request(worker, false, false);
                    self.camera_feedback(picker)?;
                }
            }
            Focus::Nodes => {
                if action(Nav::ToggleDiagnostics) {
                    let selected = self.selected_diagnostic().cloned();
                    if !self.diagnostics.active {
                        self.diagnostics.selected_only = self.selected_node().is_some();
                        self.refresh_diagnostics(selected.as_ref());
                    }
                    self.diagnostics.active = !self.diagnostics.active;
                    return Ok(false);
                }
                if self.diagnostics.active {
                    self.diagnostic_key(key, picker, worker)?;
                    return Ok(false);
                }
                if action(Nav::ReferenceNode) {
                    self.reference_selected_node()?;
                    return Ok(false);
                }
                let count = self.filtered_nodes().len();
                if action(Nav::PreviousItem) {
                    self.nodes.select(
                        (count > 0).then(|| self.nodes.selected().unwrap_or(0).saturating_sub(1)),
                    );
                } else if action(Nav::NextItem) {
                    self.nodes.select((count > 0).then(|| {
                        (self.nodes.selected().map_or(0, |n| n + 1)).min(count.saturating_sub(1))
                    }));
                } else if action(Nav::ExpandNode)
                    || action(Nav::CollapseNode)
                    || action(Nav::ToggleNode)
                {
                    self.navigate_node_tree(if action(Nav::ExpandNode) {
                        Nav::ExpandNode
                    } else if action(Nav::CollapseNode) {
                        Nav::CollapseNode
                    } else {
                        Nav::ToggleNode
                    });
                } else if action(Nav::SearchNodes) {
                    self.searching = true;
                } else if action(Nav::NodePageUp) || action(Nav::NodePageDown) {
                    self.page_nodes(action(Nav::NodePageUp));
                } else if action(Nav::ClearSelection) {
                    self.nodes.select(None);
                } else if action(Nav::ActivateNode)
                    && self
                        .selected_node()
                        .is_some_and(|node| !node.children.is_empty())
                {
                    self.navigate_node_tree(Nav::ExpandNode);
                } else if action(Nav::ActivateNode) || action(Nav::FitSelected) {
                    self.fit_selected(picker)?;
                    self.request(worker, false, false);
                    self.camera_feedback(picker)?;
                }
            }
            Focus::References => self.reference_key(key)?,
            Focus::Attachments => {
                let count = self.conversation.images().len();
                if action(Nav::PreviousItem) {
                    self.attachments.select(Some(
                        self.attachments.selected().unwrap_or(0).saturating_sub(1),
                    ));
                } else if action(Nav::NextItem) {
                    self.attachments.select(Some(
                        (self.attachments.selected().map_or(0, |n| n + 1))
                            .min(count.saturating_sub(1)),
                    ));
                } else if action(Nav::RemoveAttachment) {
                    if self.input.is_some() {
                        return Err(Error::new(
                            "INVALID_ARGUMENT",
                            "Finish or cancel image conversion before removing an attachment",
                        ));
                    }
                    if let Some(index) = self.attachments.selected() {
                        self.conversation.remove_image(index)?;
                        self.attachments.select(if count > 1 {
                            Some(index.min(count - 2))
                        } else {
                            None
                        });
                        if count == 1 {
                            self.focus = Focus::Prompt;
                        }
                    }
                } else {
                    return Ok(false);
                }
                self.request(worker, false, false);
            }
            Focus::Conversation => {
                if action(Nav::ScrollUp) {
                    self.scroll = self.scroll.saturating_add(5);
                } else if action(Nav::ScrollDown) {
                    self.scroll = self.scroll.saturating_sub(5);
                } else if action(Nav::FollowOutput) {
                    self.scroll = 0;
                }
            }
            Focus::Prompt => {}
        }
        Ok(false)
    }
}
