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
    io::Write,
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
        let directory = store.document_data_dir().join("conversations").join(&id);
        if args.session.is_some() && !directory.is_dir() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "No local conversation exists for this document and ID",
            ));
        }
        create_private_directory(&directory)?;
        let lock_path = directory.join("lock");
        let lock = private_options()
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| Error::io(&lock_path, e))?;
        lock.try_lock_exclusive().map_err(|e| {
            Error::new(
                "REVISION_CONFLICT",
                format!("The conversation is already in use: {e}"),
            )
        })?;
        let fingerprint = hash(
            &serde_json::to_vec(&(command, &args.adapter_args))
                .map_err(|e| Error::new("INVALID_ARGUMENT", e.to_string()))?,
        );
        let state = if args.session.is_some() {
            let bytes =
                crate::attachments::read_bounded(&directory.join("session.json"), 8 * 1024 * 1024)?;
            let state: SavedConversation = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new("AGENT_ERROR", "Cannot read the conversation metadata"))?;
            if !matches!(state.version, 1 | 2)
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
                version: 2,
                id,
                document: store.path().into(),
                agent: args.agent,
                adapter_fingerprint: fingerprint,
                requested_auth: args.auth,
                native_session_id: None,
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
        // A report from an earlier process is not evidence for this connection.
        session.state.authentication = None;
        session.state.version = 2;
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
