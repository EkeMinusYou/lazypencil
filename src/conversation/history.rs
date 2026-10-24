//! Read-only, bounded access to saved local conversations and complete transcripts.

use super::storage::SavedConversation;
use crate::{Error, Result, document::hash, store::Store};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_TRANSCRIPT: u64 = 64 * 1024 * 1024;
const MAX_METADATA: u64 = 8 * 1024 * 1024;
pub const MAX_PAGE_BYTES: usize = 64 * 1024;

mod display;
pub use display::view;

struct Budget<'a> {
    started: Instant,
    cancel: Option<&'a CancellationToken>,
}
impl Budget<'_> {
    fn check(&self) -> Result<()> {
        crate::cancel::check(self.cancel)?;
        if self.started.elapsed() > Duration::from_secs(30) {
            return Err(Error::new(
                "AGENT_ERROR",
                "Saved history exceeded its 30 second read deadline",
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    scope: String,
    after: String,
    #[serde(default)]
    before: bool,
}

fn uuid(value: &str) -> Result<()> {
    if uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err(Error::new(
            "INVALID_ARGUMENT",
            "History IDs must be canonical local UUIDs",
        ))
    }
}

fn directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(Error::new(
            "AGENT_ERROR",
            "Saved history directory must not be a symlink",
        ));
    }
    Ok(())
}

fn open(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    if std::fs::symlink_metadata(path)
        .map_err(|e| Error::io(path, e))?
        .is_symlink()
    {
        return Err(Error::new(
            "AGENT_ERROR",
            "Saved history file must not be a symlink",
        ));
    }
    let file = options.open(path).map_err(|e| Error::io(path, e))?;
    if !file.metadata().map_err(|e| Error::io(path, e))?.is_file() {
        return Err(Error::new(
            "AGENT_ERROR",
            "Saved history requires regular files",
        ));
    }
    Ok(file)
}

fn root(store: &Store) -> PathBuf {
    store.document_data_dir().join("conversations")
}

fn session(store: &Store, id: &str) -> Result<(PathBuf, SavedConversation)> {
    uuid(id)?;
    directory(&root(store))?;
    let path = root(store).join(id);
    directory(&path)?;
    let mut bytes = Vec::new();
    open(&path.join("session.json"))?
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(&path, e))?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(Error::new(
            "AGENT_ERROR",
            "Saved conversation metadata exceeds 8 MiB",
        ));
    }
    let state: SavedConversation = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("AGENT_ERROR", "Cannot read saved conversation metadata"))?;
    if !matches!(state.version, 1..=3) || state.id != id || state.document != store.path() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Saved conversation belongs to a different document or format",
        ));
    }
    Ok((path, state))
}

/// UUID order is stable across metadata updates. New entries require a refresh
/// to be observed before the current cursor; this is explicitly a live listing.
pub fn list(
    store: &Store,
    conversation: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    if !(1..=200).contains(&limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "History page limit must be 1..200",
        ));
    }
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    budget.check()?;
    let path = if let Some(id) = conversation {
        session(store, id)?.0
    } else {
        root(store)
    };
    let scope =
        hash(format!("{}\n{}", store.path().display(), conversation.unwrap_or("")).as_bytes());
    let boundary = cursor
        .map(|cursor| {
            if cursor.len() > 1024 {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Invalid saved history cursor",
                ));
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid saved history cursor"))?;
            let value: Cursor = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid saved history cursor"))?;
            uuid(&value.after)?;
            if value.version != 1 || value.scope != scope {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Saved history cursor belongs to another listing",
                ));
            }
            Ok(value)
        })
        .transpose()?;
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(
                json!({"entries":[],"next_cursor":null,"previous_cursor":null,"snapshot":false,"order":"uuid_ascending"}),
            );
        }
        Err(error) => return Err(Error::io(&path, error)),
        Ok(_) => {}
    }
    directory(&path)?;
    let backwards = boundary.as_ref().is_some_and(|value| value.before);
    let mut other_side = false;
    // Keep at most one page and one lookahead, regardless of directory size.
    let mut candidates = BTreeSet::new();
    for entry in std::fs::read_dir(&path).map_err(|e| Error::io(&path, e))? {
        budget.check()?;
        let entry = entry.map_err(|e| Error::io(&path, e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let id = if conversation.is_some() {
            let Some(id) = name
                .strip_prefix("turn-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };
            id
        } else {
            name
        };
        if uuid(id).is_err() {
            continue;
        }
        if let Some(boundary) = &boundary
            && if backwards {
                id >= boundary.after.as_str()
            } else {
                id <= boundary.after.as_str()
            }
        {
            other_side = true;
            continue;
        }
        candidates.insert(id.to_string());
        if candidates.len() > limit + 1 {
            if backwards {
                candidates.pop_first();
            } else {
                candidates.pop_last();
            }
        }
    }
    let more = candidates.len() > limit;
    if more {
        if backwards {
            candidates.pop_first();
        } else {
            candidates.pop_last();
        }
    }
    let encode = |anchor: Option<&String>, before: bool| {
        let anchor = anchor
            .cloned()
            .or_else(|| boundary.as_ref().map(|value| value.after.clone()))?;
        let value = Cursor {
            version: 1,
            scope: scope.clone(),
            after: anchor,
            before,
        };
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).expect("cursor serialization")))
    };
    let next = if if backwards { other_side } else { more } {
        encode(candidates.last(), false)
    } else {
        None
    };
    let previous = if if backwards { more } else { other_side } {
        encode(candidates.first(), true)
    } else {
        None
    };
    let mut entries = Vec::new();
    for id in candidates {
        budget.check()?;
        let result = if let Some(conversation) = conversation {
            transcript(store, conversation, &id).and_then(|mut file| {
                header(&mut file, store, conversation, &id)?;
                Ok(json!({"id":id,"bytes":file.metadata().map_err(|e| Error::io(&path, e))?.len()}))
            })
        } else {
            session(store, &id).map(|(_, state)| json!({"id":id,"agent":state.agent,"requested_auth":state.requested_auth,
                "draft_bytes":state.draft.len(),"pending_images":state.pending_attachments.len(),"continuity_pending":state.continuity_pending}))
        };
        // Invalid entries remain visible without exposing arbitrary file bytes.
        entries
            .push(result.unwrap_or_else(|error| json!({"id":id,"unavailable":true,"error":error})));
    }
    budget.check()?;
    Ok(
        json!({"entries":entries,"next_cursor":next,"previous_cursor":previous,"snapshot":false,"order":"uuid_ascending"}),
    )
}

fn transcript(store: &Store, conversation: &str, turn: &str) -> Result<File> {
    uuid(turn)?;
    let path = session(store, conversation)?
        .0
        .join(format!("turn-{turn}.jsonl"));
    let file = open(&path)?;
    if file.metadata().map_err(|e| Error::io(&path, e))?.len() > MAX_TRANSCRIPT {
        return Err(Error::new("AGENT_ERROR", "Saved transcript exceeds 64 MiB"));
    }
    Ok(file)
}

fn header(file: &mut File, store: &Store, session: &str, turn: &str) -> Result<()> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let mut bytes = Vec::new();
    BufReader::new(file.take(MAX_METADATA + 1))
        .read_until(b'\n', &mut bytes)
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let value: Value = if bytes.len() as u64 <= MAX_METADATA && bytes.last() == Some(&b'\n') {
        serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("AGENT_ERROR", "Invalid transcript header"))?
    } else {
        return Err(Error::new("AGENT_ERROR", "Invalid transcript header"));
    };
    if value["type"] != "start"
        || value["session"] != session
        || value["turn"] != turn
        || value["document"] != json!(store.path())
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Transcript belongs to a different document, conversation or turn",
        ));
    }
    Ok(())
}

fn revision(file: &mut File, budget: &Budget<'_>) -> Result<String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0u64;
    loop {
        budget.check()?;
        let count = match file.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::new("IO_ERROR", error.to_string())),
        };
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_TRANSCRIPT {
            return Err(Error::new("AGENT_ERROR", "Saved transcript exceeds 64 MiB"));
        }
        digest.update(&buffer[..count]);
    }
    use std::fmt::Write;
    let mut text = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut text, "{byte:02x}").expect("string write");
    }
    Ok(text)
}

/// Pages preserve the complete original JSONL bytes, including a crash's partial
/// final record. Offsets are UTF-8 byte boundaries; no records are silently lost.
pub fn read(
    store: &Store,
    conversation: &str,
    turn: &str,
    offset: u64,
    limit: usize,
    expected_revision: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    if !(4..=MAX_PAGE_BYTES).contains(&limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Transcript byte limit must be 4..65536",
        ));
    }
    if offset > 0 && expected_revision.is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Continuation pages require the transcript revision",
        ));
    }
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    budget.check()?;
    let mut file = transcript(store, conversation, turn)?;
    let before = revision(&mut file, &budget)?;
    if expected_revision.is_some_and(|expected| expected != before) {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Saved transcript changed; refresh before reading more",
        ));
    }
    header(&mut file, store, conversation, turn)?;
    let length = file
        .metadata()
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?
        .len();
    if offset > length {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Transcript offset exceeds its length",
        ));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let (text, valid) = match std::str::from_utf8(&bytes) {
        Ok(text) => (text.to_string(), true),
        Err(error)
            if error.error_len().is_none()
                && error.valid_up_to() > 0
                && offset + (bytes.len() as u64) < length =>
        {
            (
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .expect("valid UTF-8 prefix")
                    .to_string(),
                true,
            )
        }
        // Crashes can cut a UTF-8 sequence. Preserve exact bytes as Base64 as
        // well as a displayable replacement; never silently discard a suffix.
        Err(_) => (String::from_utf8_lossy(&bytes).into_owned(), false),
    };
    let end = offset
        + if valid {
            text.len() as u64
        } else {
            bytes.len() as u64
        };
    if revision(&mut file, &budget)? != before {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Saved transcript changed while reading",
        ));
    }
    Ok(
        json!({"session":conversation,"turn":turn,"revision":before,"encoding":if valid {"utf-8"} else {"base64"},"format":"jsonl",
        "offset":offset,"end_offset":end,"total_bytes":length,"text":text,"utf8_valid":valid,
        "bytes_base64":(!valid).then(|| base64::engine::general_purpose::STANDARD.encode(&bytes)),
        "next_offset":(end < length).then_some(end)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{Conversation, storage::Transcript};
    use clap::Parser;
    use std::io::Write;
    use tempfile::TempDir;

    fn saved(store: &Store) -> Conversation {
        let arguments = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            store.path().as_os_str().to_owned(),
        ])
        .unwrap();
        let crate::cli::Command::Prompt(arguments) = arguments.command else {
            unreachable!()
        };
        Conversation::open(arguments, Some(store.data_dir().into())).unwrap()
    }
    fn turn(conversation: &Conversation, text: &str) -> (String, PathBuf) {
        let mut transcript = Transcript::create(conversation.directory()).unwrap();
        transcript
            .record(
                &json!({"type":"start","session":conversation.id(),"turn":transcript.id,
            "document":conversation.store().path()}),
            )
            .unwrap();
        transcript
            .record(&json!({"type":"user_prompt","text":text}))
            .unwrap();
        (transcript.id, transcript.path)
    }
    fn test_store(temp: &TempDir, name: &str) -> Store {
        Store::init(temp.path().join(name), Some(temp.path().join("data"))).unwrap()
    }

    #[test]
    fn saved_history_lists_all_sessions_and_turns_without_changing_metadata_or_launching_agents() {
        let temp = TempDir::new().unwrap();
        let store = test_store(&temp, "design.pen");
        assert_eq!(
            list(&store, None, 2, None, None).unwrap()["entries"],
            json!([])
        );
        assert!(!store.document_data_dir().exists());
        let conversations = (0..5).map(|_| saved(&store)).collect::<Vec<_>>();
        let mut expected = conversations
            .iter()
            .map(|c| c.id().to_string())
            .collect::<Vec<_>>();
        expected.sort();
        let metadata = conversations
            .iter()
            .map(|c| std::fs::read(c.directory().join("session.json")).unwrap())
            .collect::<Vec<_>>();
        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let page = list(&store, None, 2, cursor.as_deref(), None).unwrap();
            assert_eq!(page["snapshot"], false);
            ids.extend(
                page["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["id"].as_str().unwrap().to_string()),
            );
            cursor = page["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(ids, expected);
        let first = list(&store, None, 2, None, None).unwrap();
        let second = list(&store, None, 2, first["next_cursor"].as_str(), None).unwrap();
        let back = list(&store, None, 2, second["previous_cursor"].as_str(), None).unwrap();
        assert_eq!(back["entries"], first["entries"]);
        assert!(back["previous_cursor"].is_null());
        let last = list(&store, None, 2, second["next_cursor"].as_str(), None).unwrap();
        assert!(last["next_cursor"].is_null());
        assert_eq!(
            list(&store, None, 2, last["previous_cursor"].as_str(), None).unwrap()["entries"],
            second["entries"]
        );
        for (c, original) in conversations.iter().zip(metadata) {
            assert_eq!(
                std::fs::read(c.directory().join("session.json")).unwrap(),
                original
            );
            assert!(std::fs::read_dir(c.directory()).unwrap().all(|e| {
                !e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("grant-")
            }));
        }
        let c = &conversations[0];
        let mut expected = (0..5)
            .map(|n| turn(c, &format!("request {n}")).0)
            .collect::<Vec<_>>();
        expected.sort();
        let page = list(&store, Some(c.id()), 2, None, None).unwrap();
        assert_eq!(page["entries"][0]["id"], expected[0]);
        assert_eq!(page["entries"][1]["id"], expected[1]);
        let cursor = page["next_cursor"].as_str().unwrap();
        assert_eq!(
            list(&store, Some(conversations[1].id()), 2, Some(cursor), None)
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
        assert_eq!(
            list(&store, None, 2, Some(cursor), None).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
        let other = test_store(&temp, "other.pen");
        assert_eq!(
            list(&other, None, 2, Some(cursor), None).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
    }

    #[test]
    fn transcript_pages_preserve_japanese_large_records_and_unfinished_bytes_and_bind_every_byte() {
        let temp = TempDir::new().unwrap();
        let store = test_store(&temp, "design.pen");
        let conversation = saved(&store);
        let (id, path) = turn(
            &conversation,
            &"日本語🙂ABCDEFGHIJKLMNOPQRSTUVWXYZ\n".repeat(4000),
        );
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"type\":\"unfinished\",\"text\":\"\xe3\x81")
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut offset = 0;
        let mut expected = None;
        let mut restored = Vec::new();
        loop {
            let page = read(
                &store,
                conversation.id(),
                &id,
                offset,
                4096,
                expected.as_deref(),
                None,
            )
            .unwrap();
            assert!(page["end_offset"].as_u64().unwrap() - offset <= 4096);
            assert_eq!(page["revision"], hash(&original));
            if page["utf8_valid"] == true {
                restored.extend_from_slice(page["text"].as_str().unwrap().as_bytes());
            } else {
                restored.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(page["bytes_base64"].as_str().unwrap())
                        .unwrap(),
                );
            }
            expected = page["revision"].as_str().map(str::to_owned);
            let Some(next) = page["next_offset"].as_u64() else {
                break;
            };
            assert!(next > offset);
            offset = next;
        }
        assert_eq!(restored, original);
        assert_eq!(
            read(&store, conversation.id(), &id, 4, 4096, None, None)
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut changed = original.clone();
        let index = changed.iter().position(|b| *b == b'Z').unwrap();
        changed[index] = b'X';
        std::fs::write(&path, changed).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(
            read(
                &store,
                conversation.id(),
                &id,
                4096,
                4096,
                expected.as_deref(),
                None
            )
            .unwrap_err()
            .code,
            "REVISION_CONFLICT"
        );
        assert_eq!(
            read(
                &store,
                conversation.id(),
                "../../outside",
                0,
                4096,
                None,
                None
            )
            .unwrap_err()
            .code,
            "INVALID_ARGUMENT"
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            read(&store, conversation.id(), &id, 0, 4096, None, Some(&cancel))
                .unwrap_err()
                .code,
            "CANCELLED"
        );
    }

    #[test]
    fn foreign_headers_oversized_files_and_invalid_metadata_are_diagnosed_without_revealing_contents()
     {
        let temp = TempDir::new().unwrap();
        let store = test_store(&temp, "design.pen");
        let conversation = saved(&store);
        let (id, path) = turn(&conversation, "private text");
        let foreign =
            json!({"type":"start","session":conversation.id(),"turn":id,"document":"other.pen"});
        std::fs::write(&path, format!("{foreign}\nprivate text")).unwrap();
        assert_eq!(
            read(&store, conversation.id(), &id, 0, 4096, None, None)
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
        let page = list(&store, Some(conversation.id()), 50, None, None).unwrap();
        assert_eq!(page["entries"][0]["unavailable"], true);
        assert!(!page.to_string().contains("private text"));
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_TRANSCRIPT + 1)
            .unwrap();
        assert_eq!(
            read(&store, conversation.id(), &id, 0, 4096, None, None)
                .unwrap_err()
                .code,
            "AGENT_ERROR"
        );
        std::fs::write(
            conversation.directory().join("session.json"),
            b"unreadable secret metadata",
        )
        .unwrap();
        let page = list(&store, None, 50, None, None).unwrap();
        assert_eq!(page["entries"][0]["unavailable"], true);
        assert!(!page.to_string().contains("secret metadata"));
        for limit in [0, 201] {
            assert!(list(&store, None, limit, None, None).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_history_rejects_symlinks_and_fifos_without_waiting_for_a_writer() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let store = test_store(&temp, "design.pen");
        let conversation = saved(&store);
        let (id, path) = turn(&conversation, "test");
        let target = temp.path().join("outside.jsonl");
        std::fs::rename(&path, &target).unwrap();
        symlink(&target, &path).unwrap();
        assert!(read(&store, conversation.id(), &id, 0, 4096, None, None).is_err());
        std::fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let started = Instant::now();
        assert!(read(&store, conversation.id(), &id, 0, 4096, None, None).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        let destination = temp.path().join("saved-session");
        std::fs::rename(conversation.directory(), &destination).unwrap();
        symlink(&destination, conversation.directory()).unwrap();
        assert!(read(&store, conversation.id(), &id, 0, 4096, None, None).is_err());
    }
}
