//! Change the local conversation without authenticating or sending a request.

use super::{Conversation, Session};
use crate::{Error, Result, agent::permissions::DocumentPermissions};

impl Conversation {
    /// Select a compatible saved conversation, or create an empty one with
    /// `None`. A failed selection keeps the current lease, draft and images.
    /// Native context is loaded only when the user next sends a request.
    pub async fn select(&mut self, id: Option<&str>) -> Result<bool> {
        self.require_idle()?;
        let id = id
            .map(|id| {
                uuid::Uuid::parse_str(id)
                    .map(|id| id.to_string())
                    .map_err(|_| {
                        Error::new("INVALID_ARGUMENT", "Expected a local conversation UUID")
                    })
            })
            .transpose()?;
        if id.as_deref() == Some(self.id()) {
            return Ok(false);
        }
        let mut args = self.args.clone();
        args.session = id;
        args.restart_session = false;
        args.create = false;
        args.prompt.clear();
        args.images.clear();
        // Acquire and validate the target before releasing the original lease.
        // A busy, missing, foreign or incompatible target cannot clear a draft
        // or disconnect the current adapter.
        let session = Session::open(&self.store, &args, &self.command)?;
        self.close().await?;
        self.session = session;
        self.args = args;
        self.transcript.take();
        self.events.clear();
        self.result = None;
        self.permissions = DocumentPermissions::new();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        attachments::Attachment,
        cli::{Arguments, Command},
        store::Store,
    };
    use clap::Parser;
    use serde_json::json;
    use tempfile::TempDir;

    fn conversation(temp: &TempDir) -> Conversation {
        let path = temp.path().join("design.pen");
        let data = temp.path().join("data");
        Store::init(&path, Some(data.clone())).unwrap();
        let arguments = Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            path.into_os_string(),
            "--adapter=not-installed-offline-fixture".into(),
        ])
        .unwrap();
        let Command::Prompt(args) = arguments.command else {
            unreachable!()
        };
        Conversation::open_with_executable(args, Some(data), std::env::current_exe().unwrap())
            .unwrap()
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn selection_preserves_each_draft_image_native_id_and_lock_without_starting_an_agent() {
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut conversation = conversation(&temp);
            let store = conversation.store().clone();
            let document = std::fs::read(store.path()).unwrap();
            let first = conversation.id().to_string();
            conversation.set_draft("最初の下書き\n画像を保持".into()).unwrap();
            let image = Attachment::from_file(conversation.directory(), &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png")).unwrap();
            conversation.add_image(image.clone()).unwrap();
            conversation.session.state.native_session_id = Some("first-native".into());
            conversation.session.save().unwrap();
            assert!(!conversation.select(Some(&first.to_uppercase())).await.unwrap());
            assert!(conversation.select(None).await.unwrap());
            let second = conversation.id().to_string();
            assert_ne!(first, second);
            assert!(conversation.draft().is_empty() && conversation.images().is_empty());
            assert!(conversation.session.state.native_session_id.is_none());
            conversation.set_draft("Second draft".into()).unwrap();
            conversation.session.state.native_session_id = Some("second-native".into());
            conversation.events.push_back(super::super::ConversationEvent::Message(json!({"type":"old"})));
            conversation.result = Some(Ok(()));
            assert!(conversation.select(Some(&first)).await.unwrap());
            assert_eq!(conversation.draft(), "最初の下書き\n画像を保持");
            assert_eq!(conversation.images().len(), 1);
            assert_eq!(conversation.images()[0].id, image.id);
            image.read(conversation.directory()).unwrap();
            assert_eq!(conversation.session.state.native_session_id.as_deref(), Some("first-native"));
            assert!(conversation.events.is_empty() && conversation.result().is_none() && conversation.host.is_none());
            let mut args = conversation.settings().clone();
            args.session = Some(first.clone());
            assert_eq!(Session::open(&store, &args, &conversation.command).err().unwrap().code, "REVISION_CONFLICT");
            assert!(conversation.select(Some(&second)).await.unwrap());
            assert_eq!(conversation.draft(), "Second draft");
            assert!(conversation.images().is_empty());
            assert_eq!(conversation.session.state.native_session_id.as_deref(), Some("second-native"));
            assert_eq!(std::fs::read(store.path()).unwrap(), document);
            assert_eq!(crate::conversation::history::list(&store, Some(&first), 10, None, None).unwrap()["entries"], json!([]));
        });
    }

    #[test]
    fn missing_busy_and_incompatible_targets_cannot_replace_the_current_state() {
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut current = conversation(&temp);
            let first = current.id().to_string();
            current.set_draft("First".into()).unwrap();
            current.select(None).await.unwrap();
            current.set_draft("Current draft".into()).unwrap();
            let second = current.id().to_string();
            let bytes = std::fs::read(current.directory().join("session.json")).unwrap();
            let mut args = current.settings().clone();
            args.session = Some(first.clone());
            let busy = Session::open(current.store(), &args, &current.command).unwrap();
            assert_eq!(
                current.select(Some(&first)).await.unwrap_err().code,
                "REVISION_CONFLICT"
            );
            drop(busy);
            let metadata = current
                .directory()
                .parent()
                .unwrap()
                .join(&first)
                .join("session.json");
            let original = std::fs::read(&metadata).unwrap();
            let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            value["adapter_fingerprint"] = json!("different adapter");
            let foreign = serde_json::to_vec(&value).unwrap();
            std::fs::write(&metadata, &foreign).unwrap();
            assert!(current.select(Some(&first)).await.is_err());
            assert_eq!(std::fs::read(&metadata).unwrap(), foreign);
            for id in ["not-an-id".to_string(), uuid::Uuid::new_v4().to_string()] {
                assert!(current.select(Some(&id)).await.is_err());
                assert_eq!(current.id(), second);
                assert_eq!(current.draft(), "Current draft");
                assert_eq!(
                    std::fs::read(current.directory().join("session.json")).unwrap(),
                    bytes
                );
            }
            std::fs::write(&metadata, original).unwrap();
            current.select(Some(&first)).await.unwrap();
            assert_eq!(current.draft(), "First");
        });
    }

    #[cfg(unix)]
    #[test]
    fn selection_rejects_symlink_directories_metadata_and_lock_leaves_without_touching_the_target()
    {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut current = conversation(&temp);
            let first = current.id().to_string();
            let directory = current.directory().to_path_buf();
            current.select(None).await.unwrap();
            current.set_draft("Keep this".into()).unwrap();
            let outside = temp.path().join("outside");
            std::fs::rename(&directory, &outside).unwrap();
            let bytes = std::fs::read(outside.join("session.json")).unwrap();
            symlink(&outside, &directory).unwrap();
            assert!(current.select(Some(&first)).await.is_err());
            std::fs::remove_file(&directory).unwrap();
            std::fs::rename(&outside, &directory).unwrap();
            let outside = temp.path().join("outside-file");
            std::fs::write(&outside, &bytes).unwrap();
            for name in ["lock", "session.json"] {
                let path = directory.join(name);
                let old = std::fs::read(&path).unwrap();
                std::fs::remove_file(&path).unwrap();
                symlink(&outside, &path).unwrap();
                assert!(current.select(Some(&first)).await.is_err());
                assert_eq!(std::fs::read(&outside).unwrap(), bytes);
                assert_eq!(current.draft(), "Keep this");
                std::fs::remove_file(&path).unwrap();
                std::fs::write(&path, old).unwrap();
            }
            current.select(Some(&first)).await.unwrap();
        });
    }
}
