//! Reviewed local deletion, independent of adapter installation and authentication.

use super::super::{history, storage};
use crate::{Error, Result, store::Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, Metadata},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const TRASH: &str = "conversation-trash-";
const MAX_ENTRIES: usize = 100_000;

struct Budget<'a> {
    started: Instant,
    cancel: Option<&'a CancellationToken>,
}
impl Budget<'_> {
    fn check(&self) -> Result<()> {
        crate::cancel::check(self.cancel)?;
        if self.started.elapsed() >= Duration::from_secs(30) {
            return Err(Error::new(
                "AGENT_ERROR",
                "Conversation review exceeded 30 seconds",
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
struct Inventory {
    entries: usize,
    bytes: u64,
    files: usize,
    symlinks: usize,
    hash: Sha256,
    observed: Vec<(PathBuf, Value)>,
}

fn stamp(metadata: &Metadata) -> Value {
    json!({"identity":identity(metadata),"bytes":metadata.len(),"modified":format!("{:?}",metadata.modified()),"directory":metadata.is_dir(),"symlink":metadata.is_symlink()})
}

impl Inventory {
    fn verify(&self, budget: &Budget<'_>) -> Result<()> {
        for (path, expected) in &self.observed {
            budget.check()?;
            if stamp(&fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?)
                != *expected
            {
                return Err(changed());
            }
        }
        Ok(())
    }
}

fn identity(metadata: &Metadata) -> Value {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        json!([metadata.dev(), metadata.ino()])
    }
    #[cfg(not(unix))]
    json!(format!("{:?}", metadata.created()))
}

fn scan(root: &Path, budget: &Budget<'_>) -> Result<Inventory> {
    let mut inventory = Inventory::default();
    visit(root, root, 0, budget, &mut inventory)?;
    inventory.verify(budget)?;
    Ok(inventory)
}

fn visit(
    root: &Path,
    path: &Path,
    depth: usize,
    budget: &Budget<'_>,
    inventory: &mut Inventory,
) -> Result<()> {
    budget.check()?;
    inventory.entries += 1;
    if inventory.entries > MAX_ENTRIES || depth > 64 {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation tree exceeds its review entry/depth limit",
        ));
    }
    let before = fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?;
    inventory.observed.push((path.into(), stamp(&before)));
    let name = path
        .strip_prefix(root)
        .unwrap()
        .as_os_str()
        .as_encoded_bytes();
    inventory.hash.update((name.len() as u64).to_le_bytes());
    inventory.hash.update(name);
    inventory
        .hash
        .update(serde_json::to_vec(&identity(&before)).unwrap());
    inventory.hash.update(format!("{:?}", before.modified()));
    if before.is_symlink() {
        inventory.hash.update(b"symlink");
        let target = fs::read_link(path).map_err(|error| Error::io(path, error))?;
        inventory
            .hash
            .update((target.as_os_str().as_encoded_bytes().len() as u64).to_le_bytes());
        inventory.hash.update(target.as_os_str().as_encoded_bytes());
        inventory.symlinks += 1;
    } else if before.is_dir() {
        inventory.hash.update(b"directory");
        let mut entries = Vec::new();
        for entry in fs::read_dir(path).map_err(|error| Error::io(path, error))? {
            budget.check()?;
            if entries.len() + inventory.entries >= MAX_ENTRIES {
                return Err(Error::new(
                    "AGENT_ERROR",
                    "Conversation tree exceeds its review entry limit",
                ));
            }
            entries.push(entry.map_err(|error| Error::io(path, error))?.path());
        }
        entries.sort();
        for entry in entries {
            visit(root, &entry, depth + 1, budget, inventory)?;
        }
    } else if before.is_file() {
        inventory.hash.update(b"file");
        inventory.hash.update(before.len().to_le_bytes());
        let mut input = crate::store::open_regular_file(path)?;
        if identity(&input.metadata().map_err(|error| Error::io(path, error))?) != identity(&before)
        {
            return Err(changed());
        }
        let mut buffer = [0; 64 * 1024];
        let mut bytes = 0u64;
        loop {
            budget.check()?;
            let count = input
                .read(&mut buffer)
                .map_err(|error| Error::io(path, error))?;
            if count == 0 {
                break;
            }
            bytes = bytes.checked_add(count as u64).ok_or_else(changed)?;
            if bytes > before.len() {
                return Err(changed());
            }
            inventory.hash.update(&buffer[..count]);
        }
        if bytes != before.len() {
            return Err(changed());
        }
        inventory.bytes = inventory.bytes.checked_add(bytes).ok_or_else(changed)?;
        inventory.files += 1;
    } else {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation contains a non-regular file",
        ));
    }
    let after = fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?;
    if identity(&before) != identity(&after)
        || before.modified().ok() != after.modified().ok()
        || before.len() != after.len()
    {
        return Err(changed());
    }
    budget.check()
}

fn changed() -> Error {
    Error::new(
        "REVISION_CONFLICT",
        "Conversation changed; review its deletion again",
    )
}

fn target(
    store: &Store,
    id: &str,
    budget: &Budget<'_>,
) -> Result<(storage::RegistryGuard, PathBuf, storage::SavedConversation)> {
    // Validate the namespace before opening a lease leaf. Re-read metadata under
    // the exclusive target lease; a concurrent frontend may have just closed.
    let (directory, _) = history::session(store, id)?;
    let registry = storage::registry_lock(store, budget.cancel)?;
    history::session(store, id)?;
    let lease = storage::exclusive_lease(&directory.join("lock"))?;
    let (_, state) = history::session(store, id)?;
    drop(registry);
    budget.check()?;
    Ok((lease, directory, state))
}

/// Inspect exactly one inactive local conversation. Nothing is sent to an agent.
pub fn review_delete(store: &Store, id: &str, cancel: Option<&CancellationToken>) -> Result<Value> {
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    let (_lease, directory, state) = target(store, id, &budget)?;
    let inventory = scan(&directory, &budget)?;
    Ok(
        json!({"document":store.path(),"session":id,"revision":format!("{:x}",inventory.hash.finalize()),"bytes":inventory.bytes,"files":inventory.files,"symlinks":inventory.symlinks,"draft_characters":state.draft.chars().count(),"pending_images":state.pending_attachments.len(),"retained_images":state.attachments.len(),"deleted":false,"scope":"local_conversation_only","provider_history_preserved":true,"project_assets_preserved":true}),
    )
}

/// Commit a reviewed local deletion. Renaming under the stable registry lease
/// prevents another frontend from reopening a partially removed conversation.
pub fn delete(
    store: &Store,
    id: &str,
    expected_revision: &str,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    if expected_revision.len() != 64
        || !expected_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Deletion requires the reviewed SHA-256 revision",
        ));
    }
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    let (_lease, directory, _) = target(store, id, &budget)?;
    let inventory = scan(&directory, &budget)?;
    let revision = format!("{:x}", inventory.hash.clone().finalize());
    if revision != expected_revision {
        return Err(changed());
    }
    let _registry = storage::registry_lock(store, cancel)?;
    budget.check()?;
    inventory.verify(&budget)?;
    let operation = uuid::Uuid::new_v4().to_string();
    let trash = store
        .document_data_dir()
        .join(format!("{TRASH}{id}-{operation}"));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&trash)
        .map_err(|error| Error::io(&trash, error))?;
    let cleanup_lease = storage::exclusive_lease(&trash.join("lock"))?;
    crate::store::atomic_replace(&trash.join("deletion.json"), &serde_json::to_vec(&json!({"version":1,"document":store.path(),"session":id,"operation":operation,"revision":revision})).unwrap())?;
    crate::store::sync_directory(&store.document_data_dir())?;
    budget.check()?;
    fs::rename(&directory, trash.join("payload")).map_err(|error| Error::io(&directory, error))?;
    let durability_errors = [directory.parent().unwrap(), trash.as_path()]
        .into_iter()
        .filter_map(|path| {
            crate::store::sync_directory(path)
                .err()
                .map(|error| error.to_string())
        })
        .collect::<Vec<_>>();
    drop(_registry);
    // Cancellation cannot undo the rename. Report the committed deletion even
    // if cleanup fails; the explicit cleanup command can finish after a crash.
    let cleanup = if cancel.is_some_and(CancellationToken::is_cancelled) {
        Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "Deletion committed; cleanup cancelled",
        ))
    } else {
        finish_trash(&trash)
    };
    drop(cleanup_lease);
    Ok(
        json!({"document":store.path(),"session":id,"revision":revision,"deleted":true,"bytes":inventory.bytes,"cleanup_complete":cleanup.is_ok(),"cleanup_error":cleanup.err().map(|error|error.to_string()),"durability_errors":durability_errors,"scope":"local_conversation_only","provider_history_preserved":true,"project_assets_preserved":true}),
    )
}

fn finish_trash(trash: &Path) -> std::io::Result<()> {
    let payload = trash.join("payload");
    match fs::symlink_metadata(&payload) {
        Ok(metadata) if metadata.is_symlink() => fs::remove_file(&payload)?,
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(&payload)?,
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Deletion payload must be a directory",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::remove_dir_all(trash)
}

/// Finish only recognized, document-bound tombstones left by local deletion.
pub fn cleanup_deleted(store: &Store, cancel: Option<&CancellationToken>) -> Result<Value> {
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    let parent = store.document_data_dir();
    let mut removed = Vec::new();
    let mut retained = Vec::new();
    if fs::symlink_metadata(&parent)
        .is_ok_and(|metadata| metadata.is_symlink() || !metadata.is_dir())
    {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation data directory must be a regular directory",
        ));
    }
    let entries = match fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"removed":removed,"retained":retained}));
        }
        Err(error) => return Err(Error::io(&parent, error)),
    };
    for (index, entry) in entries.enumerate() {
        budget.check()?;
        if index >= MAX_ENTRIES {
            return Err(Error::new(
                "AGENT_ERROR",
                "Conversation cleanup exceeds its entry limit",
            ));
        }
        let entry = entry.map_err(|error| Error::io(&parent, error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(ids) = name.strip_prefix(TRASH) else {
            continue;
        };
        let Some((session, operation)) = ids.get(..36).zip(ids.get(37..)) else {
            continue;
        };
        if ids.as_bytes().get(36) != Some(&b'-')
            || [session, operation]
                .iter()
                .any(|id| !uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == *id))
        {
            continue;
        }
        let path = entry.path();
        let result = (|| {
            let metadata = fs::symlink_metadata(&path).map_err(|error| Error::io(&path, error))?;
            if !metadata.is_dir() || metadata.is_symlink() {
                return Err(changed());
            }
            let _registry = storage::registry_lock(store, cancel)?;
            let _lease = storage::exclusive_lease(&path.join("lock"))?;
            // Keep the journal outside the payload so a partial recursive
            // cleanup remains recoverable after session.json has been removed.
            let mut bytes = Vec::new();
            crate::store::open_regular_file(&path.join("deletion.json"))?
                .take(16 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| Error::io(&path, error))?;
            if bytes.len() > 16 * 1024 {
                return Err(changed());
            }
            let state: Value = serde_json::from_slice(&bytes).map_err(|_| changed())?;
            if state["session"] != session
                || state["operation"] != operation
                || state["document"] != json!(store.path())
                || state["version"] != 1
                || !state["revision"].as_str().is_some_and(|revision| {
                    revision.len() == 64 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            {
                return Err(changed());
            }
            drop(_registry);
            budget.check()?;
            finish_trash(&path).map_err(|error| Error::io(&path, error))
        })();
        match result {
            Ok(()) => removed.push(name),
            Err(error) if error.code == "CANCELLED" => return Err(error),
            Err(error) => retained.push(json!({"name":name,"error":error})),
        }
    }
    Ok(
        json!({"document":store.path(),"removed":removed,"retained":retained,"scope":"local_deleted_conversations_only"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::{Arguments, Command},
        conversation::Conversation,
    };
    use clap::Parser;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Store, Conversation) {
        let temp = TempDir::new().unwrap();
        let store = Store::init(
            temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        let args = Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            store.path().as_os_str().to_owned(),
            "--adapter=not-installed-for-local-deletion".into(),
        ])
        .unwrap();
        let Command::Prompt(args) = args.command else {
            unreachable!()
        };
        let conversation = Conversation::open_with_executable(
            args,
            Some(store.data_dir().into()),
            std::env::current_exe().unwrap(),
        )
        .unwrap();
        (temp, store, conversation)
    }

    #[test]
    fn deletion_reviews_and_removes_only_inactive_local_data_and_preserves_assets_and_provider_history()
     {
        let (temp, store, mut conversation) = fixture();
        conversation.set_draft("未送信🙂".into()).unwrap();
        let id = conversation.id().to_string();
        let directory = conversation.directory().to_path_buf();
        assert_eq!(
            review_delete(&store, &id, None).unwrap_err().code,
            "REVISION_CONFLICT"
        );
        drop(conversation);
        let original = fs::read(store.path()).unwrap();
        let asset = temp.path().join("project-image.png");
        let provider = temp.path().join("provider-history.jsonl");
        fs::write(&asset, b"project asset").unwrap();
        fs::write(&provider, b"provider history").unwrap();
        fs::create_dir(directory.join("attachments")).unwrap();
        fs::write(
            directory.join("attachments/retained.png"),
            include_bytes!("../../../examples/assets/crop-source.png"),
        )
        .unwrap();
        fs::write(directory.join("turn-local.jsonl"), b"retained transcript").unwrap();
        let review = review_delete(&store, &id, None).unwrap();
        assert_eq!(review["draft_characters"], 4);
        assert_eq!(review["deleted"], false);
        assert!(review["bytes"].as_u64().unwrap() > 100);
        let deleted = delete(&store, &id, review["revision"].as_str().unwrap(), None).unwrap();
        assert_eq!(deleted["deleted"], true);
        assert_eq!(deleted["cleanup_complete"], true);
        assert!(!directory.exists());
        assert_eq!(fs::read(store.path()).unwrap(), original);
        assert_eq!(fs::read(asset).unwrap(), b"project asset");
        assert_eq!(fs::read(provider).unwrap(), b"provider history");
        assert!(
            store
                .document_data_dir()
                .join("conversations.lock")
                .is_file()
        );
        assert!(
            history::list(&store, None, 10, None, None).unwrap()["entries"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn deletion_rejects_same_size_same_mtime_content_changes_and_requires_a_fresh_review() {
        let (_temp, store, conversation) = fixture();
        let id = conversation.id().to_string();
        let directory = conversation.directory().to_path_buf();
        drop(conversation);
        let path = directory.join("turn-existing.jsonl");
        fs::write(&path, b"aaaa").unwrap();
        let review = review_delete(&store, &id, None).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"bbbb").unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(
            delete(&store, &id, review["revision"].as_str().unwrap(), None)
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        assert_eq!(fs::read(&path).unwrap(), b"bbbb");
        let next = review_delete(&store, &id, None).unwrap();
        assert_ne!(next["revision"], review["revision"]);
        assert_eq!(
            delete(&store, &id, next["revision"].as_str().unwrap(), None).unwrap()["deleted"],
            true
        );
    }

    #[test]
    fn deletion_cancellation_foreign_metadata_and_deadlines_preserve_the_original_tree() {
        let (_temp, store, conversation) = fixture();
        let id = conversation.id().to_string();
        let directory = conversation.directory().to_path_buf();
        drop(conversation);
        let review = review_delete(&store, &id, None).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            review_delete(&store, &id, Some(&cancel)).unwrap_err().code,
            "CANCELLED"
        );
        assert_eq!(
            delete(
                &store,
                &id,
                review["revision"].as_str().unwrap(),
                Some(&cancel)
            )
            .unwrap_err()
            .code,
            "CANCELLED"
        );
        assert!(directory.is_dir());
        assert!(delete(&store, &id, "bad revision", None).is_err());
        let budget = Budget {
            started: Instant::now() - Duration::from_secs(31),
            cancel: None,
        };
        assert!(scan(&directory, &budget).is_err());
        let path = directory.join("session.json");
        let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        state["document"] = json!(store.path().with_extension("foreign.pen"));
        fs::write(&path, state.to_string()).unwrap();
        assert_eq!(
            review_delete(&store, &id, None).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
        assert!(directory.is_dir());
    }

    #[test]
    fn deletion_cleanup_recovers_partial_payloads_and_protects_busy_or_foreign_tombstones() {
        let (_temp, store, conversation) = fixture();
        let id = conversation.id().to_string();
        let directory = conversation.directory().to_path_buf();
        drop(conversation);
        let revision = review_delete(&store, &id, None).unwrap()["revision"].clone();
        let operation = uuid::Uuid::new_v4().to_string();
        let trash = store
            .document_data_dir()
            .join(format!("{TRASH}{id}-{operation}"));
        fs::create_dir(&trash).unwrap();
        fs::rename(&directory, trash.join("payload")).unwrap();
        fs::remove_file(trash.join("payload/session.json")).unwrap();
        fs::write(trash.join("deletion.json"),json!({"version":1,"document":store.path(),"session":id,"operation":operation,"revision":revision}).to_string()).unwrap();
        let lease = storage::exclusive_lease(&trash.join("lock")).unwrap();
        assert_eq!(
            cleanup_deleted(&store, None).unwrap()["retained"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(trash.exists());
        drop(lease);
        let foreign = store.document_data_dir().join(format!(
            "{TRASH}{}-{}",
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join("important.txt"), b"unknown data").unwrap();
        let result = cleanup_deleted(&store, None).unwrap();
        assert_eq!(result["removed"].as_array().unwrap().len(), 1);
        assert!(!trash.exists());
        assert_eq!(
            fs::read(foreign.join("important.txt")).unwrap(),
            b"unknown data"
        );
    }

    #[cfg(unix)]
    #[test]
    fn deletion_never_follows_symlinks_and_rejects_non_regular_metadata_and_lock_leaves() {
        use std::os::unix::fs::symlink;
        let (temp, store, conversation) = fixture();
        let id = conversation.id().to_string();
        let directory = conversation.directory().to_path_buf();
        drop(conversation);
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"outside data").unwrap();
        for name in ["session.json", "lock"] {
            let path = directory.join(name);
            let bytes = fs::read(&path).unwrap();
            fs::remove_file(&path).unwrap();
            symlink(outside.join("keep"), &path).unwrap();
            assert!(review_delete(&store, &id, None).is_err());
            fs::remove_file(&path).unwrap();
            fs::write(&path, bytes).unwrap();
        }
        symlink(&outside, directory.join("external-link")).unwrap();
        let review = review_delete(&store, &id, None).unwrap();
        assert_eq!(review["symlinks"], 1);
        assert_eq!(
            delete(&store, &id, review["revision"].as_str().unwrap(), None).unwrap()["deleted"],
            true
        );
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside data");
    }
}
