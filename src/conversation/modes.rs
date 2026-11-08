//! Native permission modes and reviewed defaults, independent of document-tool grants.
use super::*;
use crate::config::mode::Prepared;
pub use crate::config::mode::Scope;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Mode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}
#[derive(Default)]
pub struct Modes {
    pub available: Vec<Mode>,
    pub current: Option<String>,
    pub error: Option<Error>,
    pub saved: Option<String>,
    pub(crate) pending: Option<Change>,
    pub(crate) saving: Option<Saving>,
}
pub(crate) struct Change {
    pub mode: String,
    pub save: Option<Prepared>,
    pub sent: bool,
    pub old_mode: Option<String>,
}
pub(crate) struct Saving {
    pub receiver:
        tokio::sync::oneshot::Receiver<Result<Option<crate::config::agent_execution::Selection>>>,
    prepared: Prepared,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Saving {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Modes {
    pub fn pending(&self) -> Option<&str> {
        self.pending.as_ref().map(|change| change.mode.as_str())
    }
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }
    pub fn changing(&self) -> bool {
        self.pending.as_ref().is_some_and(|change| change.sent)
    }
    pub(crate) fn observe(&mut self, value: &Value) {
        let mut normalized = value.clone();
        crate::agent::normalize_modes(&mut normalized);
        let value = &normalized;
        if let Some(items) = value["modes"]["availableModes"].as_array() {
            self.available = items
                .iter()
                .take(128)
                .filter_map(|item| serde_json::from_value::<Mode>(item.clone()).ok())
                .filter(|mode| {
                    !mode.id.is_empty()
                        && mode.id.len() <= 256
                        && !mode.id.chars().any(char::is_control)
                })
                .map(|mut mode| {
                    mode.name = mode.name.chars().take(256).collect();
                    mode.description = mode.description.map(|s| s.chars().take(4096).collect());
                    mode
                })
                .collect();
            self.current = value["modes"]["currentModeId"]
                .as_str()
                .filter(|id| id.len() <= 256 && !id.chars().any(char::is_control))
                .map(str::to_owned);
        }
    }
}
impl Conversation {
    pub fn modes(&self) -> &Modes {
        &self.modes
    }
    pub fn mode_source(&self) -> &str {
        if self.mode_override {
            "this conversation"
        } else if self.args.mode.is_none() && self.args.adapter.is_some() {
            "adapter default"
        } else {
            self.args
                .mode_origin
                .as_deref()
                .unwrap_or(if self.args.mode.is_some() {
                    "requested mode"
                } else {
                    "built-in default"
                })
        }
    }
    pub fn review_mode(&self, mode: &str, scope: Scope) -> Result<Option<Prepared>> {
        if !self
            .modes
            .available
            .iter()
            .any(|candidate| candidate.id == mode)
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The requested mode is not advertised by this adapter",
            ));
        }
        if scope == Scope::Conversation {
            Ok(None)
        } else {
            Prepared::new(self.store.path(), self.args.agent, mode, scope).map(Some)
        }
    }
    /// User-facing callers must obtain explicit confirmation for elevated native modes.
    pub fn change_mode(&mut self, mode: String, save: Option<Prepared>) -> Result<()> {
        self.require_project_agent()?;
        if self.modes.pending.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Cancel the pending permission change first",
            ));
        }
        if !self
            .modes
            .available
            .iter()
            .any(|candidate| candidate.id == mode)
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The requested mode is not advertised by this adapter",
            ));
        }
        if let Some(save) = &save {
            save.matches(self.store.path(), self.args.agent, &mode)?;
        }
        self.modes.error = None;
        self.modes.saved = None;
        self.modes.pending = Some(Change {
            mode,
            save,
            sent: false,
            old_mode: self.args.mode.clone(),
        });
        if let Err(error) = self.drive_mode_change() {
            if let Some(change) = self.modes.pending.take() {
                self.args.mode = change.old_mode;
            }
            self.modes.error = Some(error.clone());
            return Err(error);
        }
        Ok(())
    }
    pub fn cancel_mode_change(&mut self) -> Result<()> {
        if self.modes.changing() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the adapter to finish the permission change",
            ));
        }
        self.modes.pending = None;
        Ok(())
    }
    pub(crate) fn drive_mode_change(&mut self) -> Result<()> {
        if self.active() {
            return Ok(());
        }
        let Some(change) = self.modes.pending.as_ref().filter(|change| !change.sent) else {
            return Ok(());
        };
        self.require_project_agent()?;
        if let Some(save) = &change.save {
            save.verify()?;
        }
        let mode = change.mode.clone();
        if let Some(host) = &self.host {
            if !self.ready {
                return Ok(());
            }
            host.set_mode(mode)?;
        } else {
            self.args.mode = Some(mode);
            self.start_connection()?;
        }
        self.modes.pending.as_mut().expect("pending change").sent = true;
        Ok(())
    }
    pub(crate) fn finish_mode_change(&mut self, response: &Value) -> Result<()> {
        self.modes.observe(response);
        if !self.modes.changing() {
            return Ok(());
        }
        let change = self.modes.pending.take().expect("sent mode change");
        if self.modes.current.as_deref() != Some(change.mode.as_str()) {
            self.args.mode = change.old_mode;
            self.modes.error = Some(Error::new(
                "AGENT_ERROR",
                "The adapter did not confirm the requested permission mode",
            ));
            return Ok(());
        }
        self.args.mode = Some(change.mode.clone());
        self.mode_override = true;
        self.mode_overrides
            .insert(self.id().to_owned(), change.mode.clone());
        if let Some(prepared) = change.save.clone() {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let save = prepared.clone();
            let store = self.store.clone();
            match std::thread::Builder::new()
                .name("lazypencil-permission-save".into())
                .spawn(move || {
                    let _ = sender.send(save.save(&store));
                }) {
                Ok(thread) => {
                    self.modes.saving = Some(Saving {
                        receiver,
                        prepared,
                        thread: Some(thread),
                    });
                    self.modes.pending = Some(change);
                    return Ok(());
                }
                Err(error) => {
                    self.modes.error = Some(Error::new(
                        "IO_ERROR",
                        format!("Mode applied, but saving defaults failed: {error}"),
                    ))
                }
            }
        }
        self.publish_mode_change(&change.mode)
    }
    pub(crate) fn complete_mode_save(
        &mut self,
        job: Saving,
        result: Result<Option<crate::config::agent_execution::Selection>>,
    ) -> Result<()> {
        let mode = self.modes.pending.take().expect("saving mode change").mode;
        match result {
            Ok(selection) => {
                if let Some(selection) = selection {
                    self.args.project_execution = Some(selection);
                }
                self.modes.saved = Some(format!(
                    "{}: {}",
                    job.prepared.scope().label(),
                    job.prepared.path().display()
                ));
            }
            Err(error) => {
                self.modes.error = Some(Error::new(
                    error.code,
                    format!(
                        "Mode applied, but saving defaults failed: {}",
                        error.message
                    ),
                ))
            }
        }
        self.publish_mode_change(&mode)
    }
    fn publish_mode_change(&mut self, mode: &str) -> Result<()> {
        self.message(json!({"type":"permissions_changed","mode":mode,"source":"this conversation","saved":self.modes.saved,"error":self.modes.error}))
    }
}
