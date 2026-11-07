//! Durable history lookup and recovery state. Existing checkpoints stay readable.
use super::*;
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

mod prune;
pub use prune::PruneRequest;
pub(super) use prune::run as prune_history;

type Cancel<'a> = Option<&'a CancellationToken>;
const INDEX_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Namespace {
    modified: Option<(u64, u32)>,
    identity: Option<(u64, u64)>,
}

fn namespace(store: &Store) -> Result<Option<Namespace>> {
    let path = store.history_directory();
    if !exists(&path)? {
        return Ok(None);
    }
    require_directory(&path)?;
    let metadata = fs::metadata(&path).map_err(|error| Error::io(&path, error))?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        Some((metadata.dev(), metadata.ino()))
    };
    #[cfg(not(unix))]
    let identity = None;
    Ok(Some(Namespace {
        identity,
        modified: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| (duration.as_secs(), duration.subsec_nanos())),
    }))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    sequence: u64,
    id: String,
}

impl Pointer {
    fn of(entry: &HistoryEntry) -> Self {
        Self {
            sequence: entry.sequence,
            id: entry.id.clone(),
        }
    }

    fn validate(&self) -> Result<()> {
        if !valid_checkpoint_id(&self.id)
            || self.sequence == 0
            || self.id[..20].parse::<u64>().ok() != Some(self.sequence)
        {
            return Err(corrupt("Invalid indexed checkpoint identity"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    checkpoint: Pointer,
    previous: Option<Pointer>,
    request_id: String,
    fingerprint: String,
    state: TransactionState,
    pruned: bool,
}

impl Record {
    fn validate(&self) -> Result<()> {
        self.checkpoint.validate()?;
        if let Some(previous) = &self.previous {
            previous.validate()?;
            if previous.sequence >= self.checkpoint.sequence {
                return Err(corrupt("History links must have decreasing sequences"));
            }
        }
        validate_request_id(&self.request_id)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Index {
    version: u32,
    document: PathBuf,
    last_sequence: u64,
    tail: Option<Pointer>,
    floor: u64,
    generation: u64,
    namespace: Option<Namespace>,
}

fn corrupt(message: &str) -> Error {
    Error::new("RECOVERY_REQUIRED", message)
}

fn directory(store: &Store) -> PathBuf {
    store.document_data_dir().join("history-index")
}

fn read_json<T: DeserializeOwned>(path: &Path, limit: usize, cancel: Cancel<'_>) -> Result<T> {
    crate::cancel::check(cancel)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| Error::io(path, error))?;
    if !file
        .metadata()
        .map_err(|error| Error::io(path, error))?
        .is_file()
        || fs::symlink_metadata(path)
            .map_err(|error| Error::io(path, error))?
            .file_type()
            .is_symlink()
    {
        return Err(corrupt("History records must be regular files"));
    }
    let bytes = content::read_bounded(
        file,
        path,
        limit,
        cancel,
        corrupt("History record exceeds its size limit"),
    )?;
    let value =
        serde_json::from_slice(&bytes).map_err(|_| corrupt("Cannot read history record"))?;
    crate::cancel::check(cancel)?;
    Ok(value)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic_replace(
        path,
        &serde_json::to_vec(value).map_err(|_| corrupt("Cannot encode history record"))?,
    )
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io(path, error)),
    }
}

fn require_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(corrupt("History directories must not be symbolic links"));
    }
    Ok(())
}

fn read_checkpoint(store: &Store, pointer: &Pointer, cancel: Cancel<'_>) -> Result<HistoryEntry> {
    pointer.validate()?;
    let root = store.history_directory();
    require_directory(&root)?;
    let path = root.join(&pointer.id);
    require_directory(&path)?;
    let entry: HistoryEntry = read_json(&path.join("metadata.json"), MAX_DOCUMENT_BYTES, cancel)?;
    if Pointer::of(&entry) != *pointer
        || entry
            .document_path
            .as_ref()
            .is_some_and(|path| path != store.path())
    {
        return Err(corrupt(
            "History identity does not match its document or directory",
        ));
    }
    Ok(entry)
}

pub(super) fn checkpoint(store: &Store, id: &str, cancel: Cancel<'_>) -> Result<HistoryEntry> {
    if !valid_checkpoint_id(id) {
        return Err(Error::new("INVALID_ARGUMENT", "Invalid checkpoint ID"));
    }
    let pointer = Pointer {
        sequence: id[..20]
            .parse()
            .map_err(|_| corrupt("Invalid checkpoint sequence"))?,
        id: id.into(),
    };
    if exists(&directory(store))? && exists(&Index::record_path(store, pointer.sequence))? {
        let record = Index::record(store, pointer.sequence, cancel)?;
        if record.checkpoint == pointer && record.pruned {
            return Err(Error::new(
                "SNAPSHOT_UNAVAILABLE",
                "This checkpoint was explicitly pruned and can no longer be restored",
            )
            .details(json!({"checkpoint_id":id})));
        }
    }
    read_checkpoint(store, &pointer, cancel)
}

impl Index {
    fn path(store: &Store) -> PathBuf {
        directory(store).join("state.json")
    }

    fn validate(&self, store: &Store) -> Result<()> {
        if self.version != INDEX_VERSION
            || self.document != store.path()
            || self.floor > self.last_sequence.saturating_add(1)
        {
            return Err(corrupt(
                "History index belongs to another document or format",
            ));
        }
        if let Some(tail) = &self.tail {
            tail.validate()?;
            if tail.sequence > self.last_sequence || tail.sequence < self.floor {
                return Err(corrupt("History index has an invalid tail"));
            }
        }
        Ok(())
    }

    pub(super) fn load(store: &Store, cancel: Cancel<'_>) -> Result<Self> {
        require_directory(&directory(store))?;
        let index: Self = read_json(&Self::path(store), 64 * 1024, cancel)?;
        index.validate(store)?;
        Ok(index)
    }

    fn save(&self, store: &Store) -> Result<()> {
        self.validate(store)?;
        write_json(&Self::path(store), self)
    }

    fn record_path(store: &Store, sequence: u64) -> PathBuf {
        directory(store)
            .join("sequence")
            .join(format!("{sequence:020}.json"))
    }

    fn request_path(store: &Store, request: &str) -> PathBuf {
        directory(store)
            .join("requests")
            .join(format!("{}.json", hash(request.as_bytes())))
    }

    fn record(store: &Store, sequence: u64, cancel: Cancel<'_>) -> Result<Record> {
        let record: Record = read_json(
            &Self::record_path(store, sequence),
            MAX_DOCUMENT_BYTES,
            cancel,
        )?;
        record.validate()?;
        if record.checkpoint.sequence != sequence {
            return Err(corrupt("History sequence index identity mismatch"));
        }
        Ok(record)
    }

    pub(super) fn next_sequence(&self) -> Result<u64> {
        self.last_sequence
            .checked_add(1)
            .ok_or_else(|| corrupt("History sequence is exhausted"))
    }

    pub(super) fn prepare(&self, store: &Store, entry: &HistoryEntry) -> Result<()> {
        let record = Record {
            checkpoint: Pointer::of(entry),
            previous: self.tail.clone(),
            request_id: entry.request_id.clone(),
            fingerprint: entry.fingerprint.clone(),
            state: TransactionState::Prepared,
            pruned: false,
        };
        record.validate()?;
        // Publish the recovery pointer before replacement of the authoritative document.
        write_json(&directory(store).join("pending.json"), &record)
    }

    pub(super) fn write_before(
        &self,
        store: &Store,
        destination: &Path,
        before: &Document,
        cancel: Cancel<'_>,
    ) -> Result<()> {
        if let Some(tail) = &self.tail {
            let source = store.history_directory().join(&tail.id).join("after.pen");
            if fs::symlink_metadata(&source).is_ok_and(|metadata| {
                #[cfg(unix)]
                let private = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o077 == 0
                };
                #[cfg(not(unix))]
                let private = true;
                metadata.is_file() && !metadata.file_type().is_symlink() && private
            }) && content::file_hash(&source, MAX_DOCUMENT_BYTES, cancel)
                .is_ok_and(|revision| revision == before.revision())
            {
                crate::cancel::check(cancel)?;
                // History payloads are immutable through the application. Link
                // consecutive snapshots, never the mutable user's document.
                if fs::hard_link(&source, destination).is_ok() {
                    return Ok(());
                }
            }
        }
        crate::cancel::check(cancel)?;
        write_new_synced(destination, before.text().as_bytes())
    }

    fn publish(&mut self, store: &Store, record: &Record) -> Result<()> {
        record.validate()?;
        write_json(
            &Self::record_path(store, record.checkpoint.sequence),
            record,
        )?;
        write_json(
            &Self::request_path(store, &record.request_id),
            &record.checkpoint,
        )?;
        self.last_sequence = self.last_sequence.max(record.checkpoint.sequence);
        if record.state == TransactionState::Committed {
            if self
                .tail
                .as_ref()
                .is_none_or(|tail| tail.sequence <= record.checkpoint.sequence)
            {
                self.tail = Some(record.checkpoint.clone());
            }
        } else if self.tail.as_ref() == Some(&record.checkpoint) {
            self.tail = record.previous.clone();
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| corrupt("History generation is exhausted"))?;
        self.namespace = namespace(store)?;
        self.save(store)
    }

    pub(super) fn finish(store: &Store, entry: &HistoryEntry) -> Result<()> {
        let mut index = Self::load(store, None)?;
        let path = directory(store).join("pending.json");
        let mut record: Record = read_json(&path, MAX_DOCUMENT_BYTES, None)?;
        if record.checkpoint != Pointer::of(entry)
            || record.request_id != entry.request_id
            || record.fingerprint != entry.fingerprint
            || entry.state == TransactionState::Prepared
        {
            return Err(corrupt("Pending history identity mismatch"));
        }
        record.state = entry.state.clone();
        index.publish(store, &record)?;
        fs::remove_file(&path).map_err(|error| Error::io(&path, error))?;
        sync_directory(&directory(store))
    }

    pub(super) fn replay(
        &self,
        store: &Store,
        request: &str,
        fingerprint: &str,
        cancel: Cancel<'_>,
    ) -> Result<Option<Value>> {
        let path = Self::request_path(store, request);
        if !exists(&path)? {
            return Ok(None);
        }
        let pointer: Pointer = read_json(&path, 256, cancel)?;
        pointer.validate()?;
        let record = Self::record(store, pointer.sequence, cancel)?;
        if record.checkpoint != pointer || record.request_id != request {
            return Err(corrupt("History request index identity mismatch"));
        }
        if record.fingerprint != fingerprint {
            return Err(Error::new(
                "REQUEST_ID_CONFLICT",
                "This request ID was used with different content",
            ));
        }
        if record.state != TransactionState::Committed {
            return Ok(None);
        }
        if record.pruned {
            return Err(Error::new("REQUEST_EXPIRED", "This request was committed, but its retained result was explicitly pruned; it will not be executed again")
                .details(json!({"request_id":request,"checkpoint_id":pointer.id})));
        }
        let entry = read_checkpoint(store, &pointer, cancel)?;
        if entry.state != record.state
            || entry.request_id != request
            || entry.fingerprint != fingerprint
        {
            return Err(corrupt("Indexed request differs from its checkpoint"));
        }
        let mut result = entry.result;
        result["replayed"] = json!(true);
        result["current_revision"] = json!(store.read()?.revision());
        Ok(Some(result))
    }

    pub(super) fn page(
        &self,
        store: &Store,
        limit: usize,
        before: Option<u64>,
        cancel: Cancel<'_>,
    ) -> Result<HistoryPage> {
        if before.is_some_and(|before| before <= self.floor) {
            return Ok(HistoryPage {
                entries: Vec::new(),
                next_before_sequence: None,
            });
        }
        let mut next = self.tail.clone();
        if let Some(before) = before.filter(|before| *before <= self.last_sequence) {
            let path = Self::record_path(store, before);
            if exists(&path)? {
                next = Self::record(store, before, cancel)?.previous;
            } else {
                // Arbitrary numeric cutoffs can fall in gaps in legacy history.
                // Normal page cursors address their sequence record directly.
                let root = directory(store).join("sequence");
                let mut nearest = None;
                for child in fs::read_dir(&root).map_err(|error| Error::io(&root, error))? {
                    crate::cancel::check(cancel)?;
                    let name = child.map_err(|error| Error::io(&root, error))?.file_name();
                    if let Some(sequence) = name
                        .to_str()
                        .and_then(|name| name.strip_suffix(".json"))
                        .and_then(|name| name.parse::<u64>().ok())
                        .filter(|sequence| *sequence < before)
                    {
                        nearest = Some(nearest.map_or(sequence, |old: u64| old.max(sequence)));
                    }
                }
                next = if let Some(sequence) = nearest {
                    let record = Self::record(store, sequence, cancel)?;
                    if record.state == TransactionState::Committed {
                        Some(record.checkpoint)
                    } else {
                        record.previous
                    }
                } else {
                    None
                };
            }
        }
        let mut entries = Vec::new();
        let mut remaining = crate::query::MAX_RESPONSE_BYTES - 1026;
        while let Some(pointer) = next.clone() {
            crate::cancel::check(cancel)?;
            if entries.len() >= limit.min(1000) || pointer.sequence < self.floor {
                break;
            }
            let record = Self::record(store, pointer.sequence, cancel)?;
            if record.checkpoint != pointer
                || record.state != TransactionState::Committed
                || record.pruned
            {
                return Err(corrupt("Invalid retained checkpoint link"));
            }
            let entry = read_checkpoint(store, &pointer, cancel)?;
            if entry.state != TransactionState::Committed {
                return Err(corrupt("Checkpoint state changed outside recovery"));
            }
            if let Err(error) = crate::query::check_response_size(
                &entry,
                remaining.saturating_sub(1),
                cancel,
                "A checkpoint entry exceeds the history response limit",
            ) {
                if entries.is_empty() || error.code == "CANCELLED" {
                    return Err(error);
                }
                break;
            }
            remaining -= crate::query::serialized_size(&entry) + 1;
            entries.push(entry);
            next = record.previous;
        }
        let next_before_sequence = next
            .filter(|pointer| pointer.sequence >= self.floor)
            .and_then(|_| entries.last().map(|entry| entry.sequence));
        Ok(HistoryPage {
            entries,
            next_before_sequence,
        })
    }
}

fn resolve_prepared(store: &Store, entry: &mut HistoryEntry, cancel: Cancel<'_>) -> Result<()> {
    for (name, expected) in [
        ("before.pen", &entry.before_revision),
        ("after.pen", &entry.after_revision),
    ] {
        let path = store.history_directory().join(&entry.id).join(name);
        if content::file_hash(&path, MAX_DOCUMENT_BYTES, cancel)? != *expected {
            return Err(corrupt("Corrupt recovery snapshot"));
        }
    }
    crate::cancel::check(cancel)?;
    let current = store.read()?.revision();
    entry.state = if current == entry.after_revision {
        TransactionState::Committed
    } else if current == entry.before_revision {
        TransactionState::Aborted
    } else {
        return Err(corrupt("The document differs from both recovery snapshots; automatic recovery would overwrite external changes")
            .details(json!({"checkpoint_id":entry.id,"current_revision":current})));
    };
    store.write_entry(&store.history_directory().join(&entry.id), entry)
}

fn migrate(store: &Store, cancel: Cancel<'_>) -> Result<(Index, Vec<String>)> {
    create_private_directory(&store.document_data_dir())?;
    let staging = tempfile::tempdir_in(store.document_data_dir())
        .map_err(|error| Error::io(&store.document_data_dir(), error))?;
    create_private_directory(&staging.path().join("sequence"))?;
    create_private_directory(&staging.path().join("requests"))?;
    let mut index = Index {
        version: INDEX_VERSION,
        document: store.path().into(),
        last_sequence: 0,
        tail: None,
        floor: 0,
        generation: 0,
        namespace: None,
    };
    let mut pointers = Vec::new();
    let history = store.history_directory();
    if exists(&history)? {
        require_directory(&history)?;
        for child in fs::read_dir(&history).map_err(|error| Error::io(&history, error))? {
            crate::cancel::check(cancel)?;
            let child = child.map_err(|error| Error::io(&history, error))?;
            let name = child.file_name().to_string_lossy().into_owned();
            if !child
                .file_type()
                .map_err(|error| Error::io(&child.path(), error))?
                .is_dir()
            {
                if valid_checkpoint_id(&name) {
                    return Err(corrupt(
                        "Checkpoint directories must be regular directories",
                    ));
                }
                continue;
            }
            if !exists(&child.path().join("metadata.json"))? {
                continue;
            }
            if !valid_checkpoint_id(&name) {
                return Err(corrupt("Invalid legacy checkpoint identity"));
            }
            let pointer = Pointer {
                sequence: name[..20]
                    .parse()
                    .map_err(|_| corrupt("Invalid checkpoint sequence"))?,
                id: name,
            };
            pointer.validate()?;
            pointers.push(pointer);
        }
    }
    pointers.sort_by_key(|pointer| pointer.sequence);
    let mut recovered = Vec::new();
    for pointer in pointers {
        crate::cancel::check(cancel)?;
        if pointer.sequence <= index.last_sequence {
            return Err(corrupt("Duplicate checkpoint sequence"));
        }
        let mut entry = read_checkpoint(store, &pointer, cancel)?;
        if entry.state == TransactionState::Prepared {
            resolve_prepared(store, &mut entry, cancel)?;
            recovered.push(entry.id.clone());
        }
        let record = Record {
            checkpoint: pointer.clone(),
            previous: index.tail.clone(),
            request_id: entry.request_id,
            fingerprint: entry.fingerprint,
            state: entry.state.clone(),
            pruned: false,
        };
        record.validate()?;
        write_json(
            &staging
                .path()
                .join("sequence")
                .join(format!("{:020}.json", pointer.sequence)),
            &record,
        )?;
        write_json(
            &staging
                .path()
                .join("requests")
                .join(format!("{}.json", hash(record.request_id.as_bytes()))),
            &pointer,
        )?;
        index.last_sequence = pointer.sequence;
        if entry.state == TransactionState::Committed {
            index.tail = Some(pointer);
        }
    }
    index.namespace = namespace(store)?;
    write_json(&staging.path().join("state.json"), &index)?;
    sync_directory(staging.path())?;
    crate::cancel::check(cancel)?;
    fs::rename(staging.path(), directory(store))
        .map_err(|error| Error::io(&directory(store), error))?;
    sync_directory(&store.document_data_dir())?;
    Ok((index, recovered))
}

pub(super) fn recover(store: &Store, cancel: Cancel<'_>) -> Result<Vec<String>> {
    let (mut index, mut recovered) = if exists(&directory(store))? {
        (Index::load(store, cancel)?, Vec::new())
    } else {
        migrate(store, cancel)?
    };
    prune::recover(store, &mut index, cancel)?;
    let pending = directory(store).join("pending.json");
    let record = if exists(&pending)? {
        let record: Record = read_json(&pending, MAX_DOCUMENT_BYTES, cancel)?;
        record.validate()?;
        Some(record)
    } else if let Some(tail) = &index.tail {
        // Also recognize the previous on-disk recovery representation at the tail.
        let entry = read_checkpoint(store, tail, cancel)?;
        if entry.state == TransactionState::Prepared {
            Some(Index::record(store, tail.sequence, cancel)?)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(mut record) = record {
        let mut entry = read_checkpoint(store, &record.checkpoint, cancel)?;
        if record.request_id != entry.request_id
            || record.fingerprint != entry.fingerprint
            || record.pruned
        {
            return Err(corrupt("Pending history differs from its checkpoint"));
        }
        if entry.state == TransactionState::Prepared {
            resolve_prepared(store, &mut entry, cancel)?;
            recovered.push(entry.id.clone());
        }
        record.state = entry.state;
        index.publish(store, &record)?;
        if exists(&pending)? {
            fs::remove_file(&pending).map_err(|error| Error::io(&pending, error))?;
            sync_directory(&directory(store))?;
        }
    }
    if index.namespace != namespace(store)? {
        // Older cooperating executables do not update the side index. Discover
        // their new checkpoint directories once, without rereading old payloads.
        let root = store.history_directory();
        let mut added = Vec::new();
        if exists(&root)? {
            for child in fs::read_dir(&root).map_err(|error| Error::io(&root, error))? {
                crate::cancel::check(cancel)?;
                let child = child.map_err(|error| Error::io(&root, error))?;
                let name = child.file_name().to_string_lossy().into_owned();
                if !valid_checkpoint_id(&name) {
                    continue;
                }
                if !child
                    .file_type()
                    .map_err(|error| Error::io(&child.path(), error))?
                    .is_dir()
                {
                    continue;
                }
                let sequence = name[..20]
                    .parse::<u64>()
                    .map_err(|_| corrupt("Invalid checkpoint sequence"))?;
                if sequence > index.last_sequence && exists(&child.path().join("metadata.json"))? {
                    added.push(Pointer { sequence, id: name });
                }
            }
        }
        added.sort_by_key(|pointer| pointer.sequence);
        for pointer in added {
            crate::cancel::check(cancel)?;
            if pointer.sequence <= index.last_sequence {
                return Err(corrupt("Duplicate checkpoint sequence"));
            }
            let mut entry = read_checkpoint(store, &pointer, cancel)?;
            if entry.state == TransactionState::Prepared {
                resolve_prepared(store, &mut entry, cancel)?;
                recovered.push(entry.id.clone());
            }
            let record = Record {
                checkpoint: pointer,
                previous: index.tail.clone(),
                request_id: entry.request_id,
                fingerprint: entry.fingerprint,
                state: entry.state,
                pruned: false,
            };
            index.publish(store, &record)?;
        }
        index.namespace = namespace(store)?;
        index.save(store)?;
    }
    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_committed_checkpoint_repairs_a_missing_request_and_sequence_publication() {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::init(
            temporary.path().join("design.pen"),
            Some(temporary.path().join("data")),
        )
        .unwrap();
        let initial = store.read().unwrap();
        let saved = store
            .transact("committed", &initial.revision(), "committed", |document| {
                let mut after = document.clone();
                after.set_property("", "name", &json!("committed"))?;
                Ok((after, json!({})))
            })
            .unwrap();
        let record = Index::record(&store, 1, None).unwrap();
        write_json(&directory(&store).join("pending.json"), &record).unwrap();
        fs::remove_file(Index::record_path(&store, 1)).unwrap();
        fs::remove_file(Index::request_path(&store, "committed")).unwrap();
        let mut index = Index::load(&store, None).unwrap();
        index.last_sequence = 0;
        index.tail = None;
        index.save(&store).unwrap();
        store.recover().unwrap();
        let result = store
            .replay_result_with_cancellation("committed", "committed", None)
            .unwrap()
            .unwrap();
        assert_eq!(result["replayed"], true);
        assert_eq!(result["checkpoint_id"], saved["checkpoint_id"]);
        assert_eq!(store.history(10, None).unwrap().len(), 1);
        assert!(!directory(&store).join("pending.json").exists());
    }

    #[test]
    fn a_foreign_pending_request_cannot_change_deduplication_indexes() {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::init(
            temporary.path().join("design.pen"),
            Some(temporary.path().join("data")),
        )
        .unwrap();
        store
            .transact(
                "owned",
                &store.read().unwrap().revision(),
                "owned",
                |document| Ok((document.clone(), json!({}))),
            )
            .unwrap();
        let mut record = Index::record(&store, 1, None).unwrap();
        record.request_id = "foreign".into();
        write_json(&directory(&store).join("pending.json"), &record).unwrap();
        assert_eq!(store.recover().unwrap_err().code, "RECOVERY_REQUIRED");
        assert!(!Index::request_path(&store, "foreign").exists());
    }
}
