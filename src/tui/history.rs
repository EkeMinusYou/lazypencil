//! Saved text is loaded independently of live agent events and permissions.

use super::*;
use crate::conversation::history::timeline::{Page, Reader};
use tokio::sync::oneshot;

pub(super) struct Task {
    pub receiver: oneshot::Receiver<Result<Page>>,
    cancel: CancellationToken,
}
impl Task {
    fn spawn(
        action: impl FnOnce(&CancellationToken) -> Result<Page> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = oneshot::channel();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        std::thread::Builder::new()
            .name("lazypencil-conversation-history".into())
            .spawn(move || {
                let _ = sender.send(action(&token));
            })
            .map_err(terminal::io)?;
        Ok(Self { receiver, cancel })
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl App {
    pub(super) fn restore_history(&mut self) {
        self.saved_history_task.take();
        self.saved_history = None;
        self.saved_log.clear();
        self.history = None;
        self.history_metrics = (0, 0, 0);
        self.history_requested = false;
        self.history_initial = true;
        self.scroll = 0;
        let store = self.conversation.store().clone();
        let session = self.conversation.id().to_owned();
        let cutoff = std::time::SystemTime::now();
        match Task::spawn(move |cancel| Reader::start(store, session, cutoff, cancel)) {
            Ok(task) => self.saved_history_task = Some(task),
            Err(error) => self.error(error),
        }
    }

    pub(super) fn scroll_history(&mut self, backwards: bool, step: u16) {
        if backwards {
            self.scroll = self.scroll.saturating_add(step);
            if self.scroll >= self.history_metrics.2 {
                self.history_requested = true;
            }
        } else {
            self.scroll = self.scroll.saturating_sub(step);
        }
    }

    pub(super) fn load_older_history(&mut self) {
        if !self.history_requested || self.saved_history_task.is_some() {
            return;
        }
        self.history_requested = false;
        let Some(reader) = self.saved_history.take() else {
            return;
        };
        match Task::spawn(move |cancel| reader.previous(cancel)) {
            Ok(task) => self.saved_history_task = Some(task),
            Err(error) => self.error(error),
        }
    }

    pub(super) fn accept_history(&mut self, result: Result<Page>) {
        self.saved_history_task.take();
        match result {
            Ok(page) => {
                if self.history_initial {
                    self.scroll = 0;
                    self.history_requested = false;
                    self.history_initial = false;
                }
                self.saved_log.insert_str(0, &clean(&page.text));
                self.saved_history = page.reader;
                self.history = None;
                // Prepending does not move the viewport to the top of the new page.
                self.history_metrics.0 = 0;
            }
            Err(error) => {
                self.append(&format!(
                    "\nSaved history could not be loaded: {error} · /logs\n"
                ));
                self.error(error);
            }
        }
        self.dirty = true;
    }
}
