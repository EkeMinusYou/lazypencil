//! TUI-only resume selection; ordinary CLI/library opens remain explicit.

use super::{Conversation, Session, history, storage};
use crate::{
    Error, Result,
    prompt::PromptArgs,
    store::{Store, atomic_replace},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LastConversation {
    version: u32,
    document: PathBuf,
    session: String,
}

fn path(store: &Store) -> PathBuf {
    store.document_data_dir().join("last-conversation.json")
}

fn last(store: &Store) -> Result<Option<String>> {
    let path = path(store);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(&path, error)),
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    history::open(&path)?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(&path, error))?;
    let saved: LastConversation = serde_json::from_slice(&bytes).map_err(|_| {
        Error::new(
            "INVALID_ARGUMENT",
            "Cannot read the previous conversation selection",
        )
    })?;
    if bytes.len() > 4096 || saved.version != 1 || saved.document != store.path() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Invalid previous conversation selection",
        ));
    }
    history::uuid(&saved.session)?;
    Ok(Some(saved.session))
}

pub(super) fn open(
    store: &Store,
    args: &PromptArgs,
    command: &Path,
) -> Result<(Session, Option<String>)> {
    let notice = match last(store) {
        Ok(Some(id)) => {
            let mut saved = args.clone();
            saved.session = Some(id.clone());
            match Session::open(store, &saved, command) {
                Ok(session) => {
                    return Ok((
                        session,
                        Some(format!(
                            "Resumed previous conversation {id} · New: /new-conversation · sends only on submit"
                        )),
                    ));
                }
                Err(error) if error.code == "REVISION_CONFLICT" => format!(
                    "Previous conversation {id} is in use by another TUI; opened a new conversation"
                ),
                Err(error) => format!(
                    "Previous conversation {id} could not be restored ({error}); opened a new conversation"
                ),
            }
        }
        Ok(None) => return Ok((Session::open(store, args, command)?, None)),
        Err(error) => format!(
            "Previous conversation selection is unavailable ({error}); opened a new conversation"
        ),
    };
    Ok((Session::open(store, args, command)?, Some(notice)))
}

impl Conversation {
    pub(crate) fn open_tui(
        args: PromptArgs,
        data: Option<PathBuf>,
        new: bool,
    ) -> Result<(Self, Option<String>)> {
        if new && (args.session.is_some() || args.restart_session) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "A new conversation cannot also resume a session",
            ));
        }
        let explicit = new || args.create || args.session.is_some();
        let executable =
            std::env::current_exe().map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        let (conversation, notice) = Self::open_local(args, data, executable, !new)?;
        if explicit {
            conversation.remember_for_tui()?;
        } else if notice.is_none() {
            // Initialize an absent selection once, so even the first unsent draft
            // can be restored. Never replace a concurrent explicit choice.
            let _registry = storage::registry_lock(&conversation.store, None)?;
            match std::fs::symlink_metadata(path(&conversation.store)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    conversation.write_selection()?
                }
                Err(error) => return Err(Error::io(&path(&conversation.store), error)),
                Ok(_) => {}
            }
        }
        Ok((conversation, notice))
    }

    /// Only explicit TUI choices and accepted sends change the next resume target.
    pub(crate) fn remember_for_tui(&self) -> Result<()> {
        let _registry = storage::registry_lock(&self.store, None)?;
        self.write_selection()
    }

    fn write_selection(&self) -> Result<()> {
        atomic_replace(
            &path(&self.store),
            &serde_json::to_vec_pretty(&LastConversation {
                version: 1,
                document: self.store.path().into(),
                session: self.id().into(),
            })
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Arguments, Command};
    use clap::Parser;
    use tempfile::TempDir;

    pub(crate) fn fixture() -> (TempDir, PromptArgs, PathBuf) {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("design.pen");
        let data = temp.path().join("data");
        Store::init(&file, Some(data.clone())).unwrap();
        let parsed = Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            file.into_os_string(),
            "--adapter=offline-resume-fixture".into(),
        ])
        .unwrap();
        let Command::Prompt(args) = parsed.command else {
            unreachable!()
        };
        (temp, args, data)
    }

    fn automatic(args: &PromptArgs, data: &Path) -> (Conversation, Option<String>) {
        Conversation::open_local(
            args.clone(),
            Some(data.into()),
            std::env::current_exe().unwrap(),
            true,
        )
        .unwrap()
    }

    #[test]
    fn first_tui_open_initializes_selection_and_restores_an_unsent_draft() {
        let (_temp, args, data) = fixture();
        let (mut first, _) =
            Conversation::open_tui(args.clone(), Some(data.clone()), false).unwrap();
        let id = first.id().to_owned();
        first.set_draft("First unsent draft".into()).unwrap();
        let pointer = path(first.store());
        let bytes = std::fs::read(&pointer).unwrap();
        drop(first);
        let (restored, _) = Conversation::open_tui(args, Some(data), false).unwrap();
        assert_eq!(restored.id(), id);
        assert_eq!(restored.draft(), "First unsent draft");
        assert_eq!(std::fs::read(pointer).unwrap(), bytes);
    }

    #[test]
    fn automatic_resume_restores_local_state_without_sending_or_retargeting() {
        let (_temp, args, data) = fixture();
        let (mut first, notice) = automatic(&args, &data);
        assert!(notice.is_none());
        assert!(!path(first.store()).exists());
        first.set_draft("未送信の下書き".into()).unwrap();
        first.session.state.native_session_id = Some("saved-native-session".into());
        first.session.save().unwrap();
        first.remember_for_tui().unwrap();
        let id = first.id().to_owned();
        let pointer = path(first.store());
        let bytes = std::fs::read(&pointer).unwrap();
        drop(first);
        let (restored, notice) = automatic(&args, &data);
        assert_eq!(restored.id(), id);
        assert_eq!(restored.draft(), "未送信の下書き");
        assert_eq!(
            restored.session.state.native_session_id.as_deref(),
            Some("saved-native-session")
        );
        assert!(!restored.active() && restored.host.is_none() && restored.events.is_empty());
        assert!(notice.unwrap().contains("Resumed previous conversation"));
        drop(restored);
        assert_eq!(std::fs::read(pointer).unwrap(), bytes);
    }

    #[test]
    fn busy_automatic_resume_creates_an_independent_conversation_but_explicit_resume_fails() {
        let (_temp, args, data) = fixture();
        let (first, _) = automatic(&args, &data);
        first.remember_for_tui().unwrap();
        let id = first.id().to_owned();
        let (second, notice) = automatic(&args, &data);
        assert_ne!(second.id(), id);
        assert!(notice.unwrap().contains("in use"));
        assert_eq!(last(first.store()).unwrap().as_deref(), Some(id.as_str()));
        let mut explicit = args.clone();
        explicit.session = Some(id.clone());
        assert_eq!(
            Conversation::open_tui(explicit, Some(data), false)
                .err()
                .unwrap()
                .code,
            "REVISION_CONFLICT"
        );
        drop(second);
        assert_eq!(last(first.store()).unwrap().as_deref(), Some(id.as_str()));
    }

    #[test]
    fn explicit_new_and_selection_survive_reverse_close_order_without_losing_drafts() {
        let (_temp, args, data) = fixture();
        let (mut first, _) = automatic(&args, &data);
        first.set_draft("First draft".into()).unwrap();
        first.remember_for_tui().unwrap();
        let first_id = first.id().to_owned();
        let (second, _) = Conversation::open_tui(args.clone(), Some(data.clone()), true).unwrap();
        let second_id = second.id().to_owned();
        let store = first.store().clone();
        drop(second);
        drop(first);
        assert_eq!(last(&store).unwrap().as_deref(), Some(second_id.as_str()));
        let (mut restored, _) = automatic(&args, &data);
        assert_eq!(restored.id(), second_id);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                restored.select(Some(&first_id)).await.unwrap();
                restored.remember_for_tui().unwrap();
            });
        assert_eq!(restored.draft(), "First draft");
        drop(restored);
        assert_eq!(automatic(&args, &data).0.id(), first_id);
    }

    #[test]
    fn unavailable_or_incompatible_automatic_targets_are_preserved_and_disclosed() {
        let (_temp, args, data) = fixture();
        let (first, _) = automatic(&args, &data);
        first.remember_for_tui().unwrap();
        let pointer = path(first.store());
        let bytes = std::fs::read(&pointer).unwrap();
        let directory = first.directory().to_owned();
        drop(first);
        let mut different = args.clone();
        different.auth = crate::prompt::AuthMode::Api;
        let (fresh, notice) = automatic(&different, &data);
        assert!(notice.unwrap().contains("could not be restored"));
        drop(fresh);
        std::fs::write(directory.join("session.json"), "broken").unwrap();
        let (fresh, notice) = automatic(&args, &data);
        assert!(notice.unwrap().contains("could not be restored"));
        drop(fresh);
        std::fs::remove_dir_all(directory).unwrap();
        let (fresh, notice) = automatic(&args, &data);
        assert!(notice.unwrap().contains("could not be restored"));
        drop(fresh);
        assert_eq!(std::fs::read(&pointer).unwrap(), bytes);
        std::fs::write(&pointer, "broken selection").unwrap();
        let (_, notice) = automatic(&args, &data);
        assert!(notice.unwrap().contains("selection is unavailable"));
        assert_eq!(std::fs::read(pointer).unwrap(), b"broken selection");
    }

    #[test]
    fn concurrent_startup_only_acquires_the_previous_conversation_once() {
        let (_temp, args, data) = fixture();
        let (first, _) = automatic(&args, &data);
        first.remember_for_tui().unwrap();
        let id = first.id().to_owned();
        drop(first);
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let acquired = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let (args, data, start, acquired) =
                    (args.clone(), data.clone(), start.clone(), acquired.clone());
                std::thread::spawn(move || {
                    start.wait();
                    let (conversation, _) = automatic(&args, &data);
                    acquired.wait();
                    conversation.id().to_owned()
                })
            })
            .collect();
        let ids: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids.iter().filter(|candidate| **candidate == id).count(), 1);
        assert_eq!(automatic(&args, &data).0.id(), id);
    }

    #[test]
    fn ordinary_cli_open_does_not_use_or_update_the_tui_selection() {
        let (_temp, args, data) = fixture();
        let (first, _) = automatic(&args, &data);
        first.remember_for_tui().unwrap();
        let id = first.id().to_owned();
        let store = first.store().clone();
        drop(first);
        let cli = Conversation::open(args.clone(), Some(data.clone())).unwrap();
        assert_ne!(cli.id(), id);
        drop(cli);
        assert_eq!(last(&store).unwrap().as_deref(), Some(id.as_str()));
    }

    #[test]
    fn new_conversation_cli_flag_is_tui_only_and_conflicts_with_explicit_resume() {
        assert!(
            Arguments::try_parse_from(["lazypencil", "tui", "design.pen", "--new-conversation"])
                .is_ok()
        );
        for argv in [
            vec![
                "lazypencil",
                "tui",
                "design.pen",
                "--new-conversation",
                "--session",
                "id",
            ],
            vec![
                "lazypencil",
                "tui",
                "design.pen",
                "--new-conversation",
                "--restart-session",
            ],
            vec!["lazypencil", "prompt", "design.pen", "--new-conversation"],
        ] {
            assert!(Arguments::try_parse_from(argv).is_err());
        }
    }
}
