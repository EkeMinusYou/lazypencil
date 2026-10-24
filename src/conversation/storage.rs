use crate::{
    Error, Result,
    attachments::Attachment,
    document::hash,
    prompt::{AgentName, AuthMode, PromptArgs},
    store::{Store, atomic_replace, create_private_directory},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedConversation {
    pub(super) version: u32,
    pub(super) id: String,
    pub(super) document: PathBuf,
    pub(super) agent: AgentName,
    pub(super) adapter_fingerprint: String,
    pub(super) requested_auth: AuthMode,
    pub(super) native_session_id: Option<String>,
    #[serde(default)]
    pub(super) continuity_pending: bool,
    pub(super) attachments: Vec<Attachment>,
    #[serde(default)]
    pub(super) authentication: Option<crate::agent::auth::AuthReport>,
    #[serde(default)]
    pub(super) draft: String,
    #[serde(default)]
    pub(super) pending_attachments: Vec<Attachment>,
}

pub(super) struct Session {
    pub(super) directory: PathBuf,
    pub(super) state: SavedConversation,
    pub(super) _lock: File,
}

struct RegistryGuard(File);

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Closing one descriptor may leave a lock held by a duplicate inherited at spawn.
        // The conversation owns the lease, so release it explicitly at the owner's end.
        let _ = FileExt::unlock(&self._lock);
    }
}

impl Session {
    pub(super) fn open(store: &Store, args: &PromptArgs, command: &Path) -> Result<Self> {
        let id = match &args.session {
            Some(id) => uuid::Uuid::parse_str(id)
                .map_err(|_| {
                    Error::new(
                        "INVALID_ARGUMENT",
                        "Session must be a local conversation UUID",
                    )
                })?
                .to_string(),
            None => uuid::Uuid::new_v4().to_string(),
        };
        let root = store.document_data_dir().join("conversations");
        match std::fs::symlink_metadata(&root) {
            Ok(_) => require_directory(&root)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_private_directory(&root)?;
            }
            Err(error) => return Err(Error::io(&root, error)),
        }
        require_directory(&root)?;
        // This stable parent lease also serializes later local deletion with
        // opening a conversation. Never unlink the registry while it is used.
        let _registry = RegistryGuard(lock_file(&root.join("registry.lock"), true)?);
        let directory = root.join(&id);
        if args.session.is_some() {
            require_directory(&directory).map_err(|_| {
                Error::new(
                    "INVALID_ARGUMENT",
                    "No regular local conversation exists for this document and ID",
                )
            })?;
        } else if directory.exists() {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "The new conversation directory already exists",
            ));
        } else {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(&directory)
                .map_err(|e| Error::io(&directory, e))?;
        }
        let lock_path = directory.join("lock");
        let lock = lock_file(&lock_path, false)?;
        let fingerprint = hash(
            &serde_json::to_vec(&(command, &args.adapter_args))
                .map_err(|e| Error::new("INVALID_ARGUMENT", e.to_string()))?,
        );
        let state = if args.session.is_some() {
            let path = directory.join("session.json");
            require_regular_file(&path)?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
            }
            let file = options.open(&path).map_err(|e| Error::io(&path, e))?;
            if !file.metadata().map_err(|e| Error::io(&path, e))?.is_file() {
                return Err(Error::new(
                    "AGENT_ERROR",
                    "Conversation metadata must be a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| Error::io(&path, e))?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(Error::new(
                    "AGENT_ERROR",
                    "Conversation metadata exceeds 8 MiB",
                ));
            }
            let state: SavedConversation = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new("AGENT_ERROR", "Cannot read the conversation metadata"))?;
            if !matches!(state.version, 1..=3)
                || state.id != id
                || state.document != store.path()
                || state.agent != args.agent
                || state.adapter_fingerprint != fingerprint
                || state.requested_auth != args.auth
            {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "The conversation belongs to a different document, adapter, or requested authentication mode",
                ));
            }
            state
        } else {
            SavedConversation {
                version: 3,
                id,
                document: store.path().into(),
                agent: args.agent,
                adapter_fingerprint: fingerprint,
                requested_auth: args.auth,
                native_session_id: None,
                continuity_pending: false,
                attachments: vec![],
                authentication: None,
                draft: String::new(),
                pending_attachments: vec![],
            }
        };
        let mut session = Self {
            directory,
            state,
            _lock: lock,
        };
        #[cfg(unix)]
        session.reclaim_stale_grants();
        // A report from an earlier process is not evidence for this connection.
        session.state.authentication = None;
        session.state.version = 3;
        session.save()?;
        Ok(session)
    }
    pub(super) fn save(&self) -> Result<()> {
        atomic_replace(
            &self.directory.join("session.json"),
            &serde_json::to_vec_pretty(&self.state)
                .map_err(|e| Error::new("IO_ERROR", e.to_string()))?,
        )
    }

    #[cfg(unix)]
    fn reclaim_stale_grants(&self) {
        // This conversation's exclusive lease is held. Sweep only recognized
        // connection files; malformed records, images and journals are retained.
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if kind.is_file()
                && name
                    .strip_prefix("grant-")
                    .and_then(|name| name.strip_suffix(".json"))
                    .is_some_and(|id| {
                        uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
                    })
            {
                let _ = crate::attachments::reclaim_stale_grant(
                    &entry.path(),
                    &self.state.document,
                    &self.state.id,
                );
            } else if kind.is_file()
                && name
                    .strip_prefix("grant-")
                    .and_then(|name| name.strip_suffix(".owner"))
                    .is_some_and(|id| {
                        uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
                    })
            {
                let _ = super::lease::remove_orphan(&entry.path().with_extension("json"));
            } else if kind.is_dir()
                && let Some(connection) = name.strip_prefix("design-usage-")
                && uuid::Uuid::parse_str(connection)
                    .is_ok_and(|value| value.to_string() == connection)
            {
                let _ = crate::guidance::usage::reclaim_stale_grant(
                    &entry.path().join("active.json"),
                    &self.state.document,
                    &self.state.id,
                    connection,
                );
                let _ = super::lease::remove_orphan(&entry.path().join("active.json"));
            }
        }
    }
}

fn require_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation directory must not be a symlink",
        ));
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation metadata must be a regular file",
        ));
    }
    Ok(())
}

fn lock_file(path: &Path, wait: bool) -> Result<File> {
    let mut options = private_options();
    options.create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.is_symlink()) {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation lock must be a regular file",
        ));
    }
    let file = options.open(path).map_err(|e| Error::io(path, e))?;
    if !file.metadata().map_err(|e| Error::io(path, e))?.is_file() {
        return Err(Error::new(
            "AGENT_ERROR",
            "Conversation lock must be a regular file",
        ));
    }
    let start = std::time::Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(error)
                if wait
                    && crate::store::lock_contended(&error)
                    && start.elapsed() < std::time::Duration::from_secs(5) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    format!("The conversation is already in use: {error}"),
                ));
            }
        }
    }
    Ok(file)
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

pub(super) struct Transcript {
    pub(super) id: String,
    pub(super) file: File,
    pub(super) path: PathBuf,
    pub(super) bytes: usize,
}
impl Transcript {
    pub(super) fn create(directory: &Path) -> Result<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("turn-{id}.jsonl"));
        let file = private_options()
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::io(&path, e))?;
        Ok(Self {
            id,
            file,
            path,
            bytes: 0,
        })
    }
    pub(super) fn record(&mut self, value: &Value) -> Result<()> {
        let mut value = value.clone();
        omit_binary(&mut value);
        let mut bytes =
            serde_json::to_vec(&value).map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
        bytes.push(b'\n');
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > 64 * 1024 * 1024 {
            return Err(Error::new(
                "AGENT_ERROR",
                "The conversation turn transcript exceeded 64 MiB",
            ));
        }
        self.file
            .write_all(&bytes)
            .and_then(|()| self.file.sync_data())
            .map_err(|e| Error::io(&self.path, e))
    }
}

/// Preserve textual events while keeping image payloads out of transcripts and NDJSON.
pub(super) fn omit_binary(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "image" | "audio"))
                && let Some(Value::String(data)) = object.remove("data")
            {
                object.insert(
                    "omitted_data".into(),
                    json!({"base64_bytes":data.len(),"hash":hash(data.as_bytes())}),
                );
            }
            if let Some(Value::String(blob)) = object.remove("blob") {
                object.insert(
                    "omitted_blob".into(),
                    json!({"base64_bytes":blob.len(),"hash":hash(blob.as_bytes())}),
                );
            }
            for child in object.values_mut() {
                omit_binary(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                omit_binary(child);
            }
        }
        _ => {}
    }
}

pub fn sanitize_event(value: &Value) -> Value {
    let mut value = value.clone();
    omit_binary(&mut value);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn legacy_sessions_upgrade_without_losing_drafts_and_pending_context_survives_reopen() {
        for version in [1, 2] {
            let temp = tempfile::TempDir::new().unwrap();
            let store = Store::init(
                temp.path().join("design.pen"),
                Some(temp.path().join("data")),
            )
            .unwrap();
            let arguments = crate::cli::Arguments::try_parse_from([
                "lazypencil".into(),
                "prompt".into(),
                store.path().as_os_str().to_owned(),
            ])
            .unwrap();
            let crate::cli::Command::Prompt(mut args) = arguments.command else {
                unreachable!()
            };
            let command = Path::new("unused-adapter");
            let mut session = Session::open(&store, &args, command).unwrap();
            session.state.draft = "Saved request".into();
            session.state.native_session_id = Some("saved-native-id".into());
            session.save().unwrap();
            args.session = Some(session.state.id.clone());
            let path = session.directory.join("session.json");
            drop(session);
            let mut legacy: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            legacy["version"] = json!(version);
            legacy.as_object_mut().unwrap().remove("continuity_pending");
            std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
            let mut upgraded = Session::open(&store, &args, command).unwrap();
            assert_eq!(upgraded.state.version, 3);
            assert_eq!(upgraded.state.draft, "Saved request");
            assert_eq!(
                upgraded.state.native_session_id.as_deref(),
                Some("saved-native-id")
            );
            assert!(!upgraded.state.continuity_pending);
            upgraded.state.continuity_pending = true;
            upgraded.save().unwrap();
            drop(upgraded);
            let resumed = Session::open(&store, &args, command).unwrap();
            assert!(resumed.state.continuity_pending);
            assert_eq!(resumed.state.draft, "Saved request");
            drop(resumed);
            let mut unknown: Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            unknown["version"] = json!(4);
            let bytes = serde_json::to_vec(&unknown).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(Session::open(&store, &args, command).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn registry_owner_releases_its_short_lease_even_if_a_descriptor_was_duplicated() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("registry.lock");
        let guard = RegistryGuard(lock_file(&path, true).unwrap());
        let duplicate = guard.0.try_clone().unwrap();
        assert!(lock_file(&path, false).is_err());
        drop(guard);
        let next = lock_file(&path, false).unwrap();
        assert!(duplicate.try_lock_exclusive().is_err());
        drop(next);
        drop(duplicate);
    }

    #[test]
    fn session_owner_releases_the_lock_even_if_a_descriptor_was_duplicated() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Store::init(
            temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        let arguments = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            store.path().as_os_str().to_owned(),
        ])
        .unwrap();
        let crate::cli::Command::Prompt(mut args) = arguments.command else {
            unreachable!()
        };
        let command = Path::new("unused-adapter");
        let session = Session::open(&store, &args, command).unwrap();
        args.session = Some(session.state.id.clone());
        // A duplicate models a descriptor inherited by a concurrently spawned child.
        let duplicate = session._lock.try_clone().unwrap();
        assert_eq!(
            Session::open(&store, &args, command).err().unwrap().code,
            "REVISION_CONFLICT"
        );
        drop(session);
        let resumed = Session::open(&store, &args, command).unwrap();
        assert!(duplicate.try_lock_exclusive().is_err());
        drop(duplicate);
        assert_eq!(
            Session::open(&store, &args, command).err().unwrap().code,
            "REVISION_CONFLICT"
        );
        drop(resumed);
        Session::open(&store, &args, command).unwrap();
    }
}
