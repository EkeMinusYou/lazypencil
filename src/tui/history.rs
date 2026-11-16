//! Saved text is loaded independently of live agent events and permissions.

use super::transcript::{Entry, Kind};
use super::*;
use crate::conversation::history::{
    RecordKind,
    timeline::{Page, Reader},
};
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
                if page.bytes > crate::conversation::history::MAX_PAGE_BYTES {
                    self.error(Error::new(
                        "AGENT_ERROR",
                        "Saved history page exceeds its display bound",
                    ));
                    return;
                }
                self.saved_log
                    .prepend(page.records.into_iter().map(|record| {
                        let kind = match record.kind {
                            RecordKind::User => Kind::User,
                            RecordKind::Agent => Kind::Agent,
                            RecordKind::Thought => Kind::Thought,
                            RecordKind::Tool => {
                                Kind::Activity(record.status.unwrap_or_else(|| "running".into()))
                            }
                            RecordKind::Error => Kind::Failure,
                            RecordKind::Complete => Kind::Complete,
                            RecordKind::Turn | RecordKind::Status | RecordKind::Record => {
                                Kind::Notice
                            }
                        };
                        let mut entry = Entry::new(kind, record.text);
                        entry.start = record.start;
                        entry
                    }));
                self.saved_history = page.reader;
                self.history = None;
                // Prepending does not move the viewport to the top of the new page.
                self.history_metrics.0 = 0;
            }
            Err(error) => {
                self.append_notice(&format!(
                    "\nSaved history could not be loaded: {error} · /logs\n"
                ));
                self.error(error);
            }
        }
        self.dirty = true;
    }
}
