//! Own blocking document capture until it is adopted, cancelled or closed.

use super::*;
use crate::service::{PreparedDesignStart, prepare_design_start};

pub(super) struct Preparation {
    pub selection: Option<Value>,
    pub cancel: CancellationToken,
    pub receiver: oneshot::Receiver<Result<PreparedDesignStart>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Preparation {
    pub fn start(
        store: Store,
        style: Option<String>,
        worker: PathBuf,
        selection: Option<Value>,
    ) -> Result<Self> {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (sender, receiver) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("lazypencil-prompt-context".into())
            .spawn(move || {
                let result =
                    prepare_design_start(&store, style.as_deref(), Some(&worker), Some(&token));
                let _ = sender.send(result);
            })
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        Ok(Self {
            selection,
            cancel,
            receiver,
            thread: Some(thread),
        })
    }
}

impl Drop for Preparation {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
