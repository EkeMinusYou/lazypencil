//! Session-scoped snapshots from ACP v1 available_commands_update.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use agent_client_protocol::schema::v1::AvailableCommand;
use serde_json::Value;
use tokio::sync::watch;

use crate::{Error, Result, cancel};

const MAX_EARLY_SESSIONS: usize = 16;

#[derive(Default)]
pub(super) struct Catalogue {
    session: Option<String>,
    early: HashMap<String, Arc<[AvailableCommand]>>,
    commands: Option<Arc<[AvailableCommand]>>,
}

impl Catalogue {
    pub fn bind(&mut self, session: &str) {
        self.session = Some(session.to_owned());
        self.commands = self.early.remove(session);
        self.early.clear();
    }

    pub fn observe(&mut self, params: &Value) {
        let Some(session) = params["sessionId"].as_str().filter(|id| id.len() <= 4096) else {
            return;
        };
        let Some(commands) = params["update"]["availableCommands"].as_array() else {
            return;
        };
        if self.session.as_deref().is_some_and(|id| id != session) {
            return;
        }
        if self.session.is_none()
            && self.early.len() >= MAX_EARLY_SESSIONS
            && !self.early.contains_key(session)
        {
            return;
        }
        let mut seen = HashSet::new();
        let entries = commands
            .iter()
            .filter_map(|entry| {
                let name = entry["name"].as_str()?;
                (name.len() <= 256
                    && !name.starts_with('/')
                    && !name.strip_prefix('$').unwrap_or(name).is_empty()
                    && !name.chars().any(|c| c.is_whitespace() || c.is_control()))
                .then(|| serde_json::from_value::<AvailableCommand>(entry.clone()).ok())
                .flatten()
                .filter(|entry| seen.insert(entry.name.clone()))
            })
            .collect::<Arc<[_]>>();
        if self.session.is_some() {
            self.commands = Some(entries);
        } else {
            self.early.insert(session.to_owned(), entries);
        }
    }

    #[cfg(feature = "tui")]
    pub fn snapshot(&self) -> Option<Arc<[AvailableCommand]>> {
        self.commands.clone()
    }
}

pub(crate) fn invocation(name: &str) -> String {
    if name.starts_with('$') {
        name.to_owned()
    } else {
        format!("/{name}")
    }
}

pub(crate) fn command_prompt(name: &str, arguments: &str) -> String {
    format!("{} {}", invocation(name), arguments.trim_start())
}

pub(super) async fn require_advertised(
    catalogue: &mut watch::Receiver<Catalogue>,
    name: &str,
    timeout: std::time::Duration,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<()> {
    let wait = catalogue.wait_for(|state| state.commands.is_some());
    let state = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return cancel::check(Some(cancellation)),
        result = tokio::time::timeout(timeout, wait) => result,
    }
    .ok()
    .and_then(std::result::Result::ok)
    .ok_or_else(|| {
        Error::new(
            "AGENT_COMMANDS_UNAVAILABLE",
            "The agent has not reported its commands. Reconnect or remove the selected command.",
        )
    })?;
    if state
        .commands
        .as_ref()
        .is_some_and(|entries| entries.iter().any(|entry| entry.name == name))
    {
        Ok(())
    } else {
        Err(Error::new(
            "AGENT_COMMAND_UNAVAILABLE",
            format!(
                "{} is no longer advertised. Choose another command or remove it.",
                invocation(name)
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn update(session: &str, names: &[&str]) -> Value {
        let entries: Vec<_> = names
            .iter()
            .map(|name| json!({"name":name,"description":""}))
            .collect();
        json!({"sessionId":session,"update":{"availableCommands":entries}})
    }

    #[tokio::test]
    async fn agent_command_selection_waits_for_the_bound_catalogue_and_observes_removal() {
        let (sender, mut receiver) = watch::channel(Catalogue::default());
        sender.send_modify(|state| state.observe(&update("other", &["$review"])));
        sender.send_modify(|state| state.bind("active"));
        let cancel = CancellationToken::new();
        assert_eq!(
            require_advertised(&mut receiver, "$review", Duration::ZERO, &cancel)
                .await
                .unwrap_err()
                .code,
            "AGENT_COMMANDS_UNAVAILABLE"
        );
        let wait = require_advertised(&mut receiver, "$review", Duration::from_secs(1), &cancel);
        let publish = async {
            tokio::task::yield_now().await;
            sender.send_modify(|state| state.observe(&update("active", &["$review"])));
        };
        let (result, ()) = tokio::join!(wait, publish);
        result.unwrap();
        sender.send_modify(|state| state.observe(&update("active", &[])));
        sender.send_modify(|state| state.observe(&update("other", &["$review"])));
        assert_eq!(
            require_advertised(&mut receiver, "$review", Duration::ZERO, &cancel)
                .await
                .unwrap_err()
                .code,
            "AGENT_COMMAND_UNAVAILABLE"
        );
        cancel.cancel();
        assert_eq!(
            require_advertised(&mut receiver, "$review", Duration::ZERO, &cancel)
                .await
                .unwrap_err()
                .code,
            "CANCELLED"
        );
    }
}
