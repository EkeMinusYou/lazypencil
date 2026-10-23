use crate::{
    Error, Result,
    document::{Document, MAX_DOCUMENT_BYTES, hash},
};
use directories::ProjectDirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub(crate) mod content;

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
    data_dir: PathBuf,
    document_key: String,
    lock_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub sequence: u64,
    pub created_at: String,
    pub request_id: String,
    pub fingerprint: String,
    pub state: TransactionState,
    pub before_revision: String,
    pub after_revision: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Prepared,
    Committed,
    Aborted,
}

impl Store {
    pub fn open(path: impl AsRef<Path>, data_dir: Option<PathBuf>) -> Result<Self> {
        let path = crate::paths::canonical(path)?;
        if !path.is_file() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Document path is not a file",
            ));
        }
        Self::at_canonical_path(path, data_dir)
    }

    fn at_canonical_path(path: PathBuf, data_dir: Option<PathBuf>) -> Result<Self> {
        if path.to_str().is_none() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Document paths must be UTF-8",
            ));
        }
        let data_dir = data_directory(data_dir)?;
        let document_key = hash(path.as_os_str().as_encoded_bytes());
        Ok(Self {
            path,
            data_dir,
            document_key,
            lock_dir: default_lock_dir()?,
        })
    }

    pub fn init(path: impl AsRef<Path>, data_dir: Option<PathBuf>) -> Result<Self> {
        let supplied = path.as_ref();
        let filename = supplied
            .file_name()
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Missing document filename"))?;
        let parent = supplied
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let canonical_parent = crate::paths::canonical(parent)?;
        let store = Self::at_canonical_path(canonical_parent.join(filename), data_dir)?;
        let _lock = store.lock()?;
        let doc = Document::empty();
        let mut temp = NamedTempFile::new_in(&canonical_parent)
            .map_err(|e| Error::io(&canonical_parent, e))?;
        temp.write_all(doc.text().as_bytes())
            .map_err(|e| Error::io(&store.path, e))?;
        temp.as_file()
            .sync_all()
            .map_err(|e| Error::io(&store.path, e))?;
        temp.persist_noclobber(&store.path).map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new(
                    "INVALID_ARGUMENT",
                    "The destination already exists; init never overwrites files",
                )
            } else {
                Error::io(&store.path, e.error)
            }
        })?;
        sync_directory(&canonical_parent)?;
        Self::open(store.path(), Some(store.data_dir.clone()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    pub fn document_data_dir(&self) -> PathBuf {
        self.data_dir.join("documents").join(&self.document_key)
    }
    pub fn lock_file_path(&self) -> PathBuf {
        self.lock_dir.join(format!("{}.lock", self.document_key))
    }

    pub fn read(&self) -> Result<Document> {
        Document::parse(read_text(&self.path)?)
    }

    /// Check retries before expensive preparation outside the document lock.
    /// Transactions repeat this check after taking their own lock.
    pub fn replay_result(&self, request_id: &str, fingerprint: &str) -> Result<Option<Value>> {
        self.replay_result_with_cancellation(request_id, fingerprint, None)
    }

    pub fn replay_result_with_cancellation(
        &self,
        request_id: &str,
        fingerprint: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Option<Value>> {
        validate_request_id(request_id)?;
        let _lock = self.lock_mode_cancellable(false, cancel)?;
        self.recover_locked()?;
        crate::cancel::check(cancel)?;
        self.replay_locked(&self.entries_locked()?, request_id, fingerprint)
    }

    fn replay_locked(
        &self,
        entries: &[HistoryEntry],
        request_id: &str,
        fingerprint: &str,
    ) -> Result<Option<Value>> {
        if let Some(previous) = entries.iter().rev().find(|e| e.request_id == request_id) {
            if previous.fingerprint != fingerprint {
                return Err(Error::new(
                    "REQUEST_ID_CONFLICT",
                    "This request ID was used with different content",
                ));
            }
            if previous.state == TransactionState::Committed {
                let mut result = previous.result.clone();
                result["replayed"] = json!(true);
                result["current_revision"] = json!(self.read()?.revision());
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    pub fn transact<F>(
        &self,
        request_id: &str,
        expected_revision: &str,
        fingerprint: &str,
        edit: F,
    ) -> Result<Value>
    where
        F: FnOnce(&Document) -> Result<(Document, Value)>,
    {
        self.transact_guarded(request_id, expected_revision, fingerprint, edit, || Ok(()))
    }

    pub fn transact_guarded<F, G>(
        &self,
        request_id: &str,
        expected_revision: &str,
        fingerprint: &str,
        edit: F,
        verify: G,
    ) -> Result<Value>
    where
        F: FnOnce(&Document) -> Result<(Document, Value)>,
        G: FnOnce() -> Result<()>,
    {
        self.transact_guarded_with_cancellation(
            request_id,
            expected_revision,
            fingerprint,
            None,
            edit,
            verify,
        )
    }

    pub fn transact_guarded_with_cancellation<F, G>(
        &self,
        request_id: &str,
        expected_revision: &str,
        fingerprint: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
        edit: F,
        verify: G,
    ) -> Result<Value>
    where
        F: FnOnce(&Document) -> Result<(Document, Value)>,
        G: FnOnce() -> Result<()>,
    {
        validate_request_id(request_id)?;
        crate::cancel::check(cancel)?;
        let _lock = self.lock_mode_cancellable(false, cancel)?;
        self.recover_locked()?;
        crate::cancel::check(cancel)?;
        let entries = self.entries_locked()?;
        if let Some(result) = self.replay_locked(&entries, request_id, fingerprint)? {
            return Ok(result);
        }
        let before = self.read()?;
        before.ensure_writable()?;
        require_revision(&before, expected_revision)?;
        if fs::metadata(&self.path)
            .map_err(|e| Error::io(&self.path, e))?
            .permissions()
            .readonly()
        {
            return Err(Error::new("IO_ERROR", "The document is read-only"));
        }
        crate::cancel::check(cancel)?;
        let (after, mut result) = edit(&before)?;
        after.ensure_writable()?;
        crate::cancel::check(cancel)?;
        let sequence = entries.last().map_or(1, |e| e.sequence + 1);
        let id = format!("{sequence:020}-{}", Uuid::new_v4().simple());
        let mut entry = HistoryEntry {
            id: id.clone(),
            sequence,
            created_at: timestamp()?,
            request_id: request_id.into(),
            fingerprint: fingerprint.into(),
            state: TransactionState::Prepared,
            before_revision: before.revision(),
            after_revision: after.revision(),
            result: Value::Null,
        };
        result["revision"] = json!(entry.after_revision);
        result["request_id"] = json!(request_id);
        result["checkpoint_id"] = json!(id);
        result["replayed"] = json!(false);
        entry.result = result.clone();
        let directory = self.history_directory().join(&id);
        create_private_directory(&directory)?;
        write_new_synced(&directory.join("before.pen"), before.text().as_bytes())?;
        write_new_synced(&directory.join("after.pen"), after.text().as_bytes())?;
        self.write_entry(&directory, &entry)?;
        sync_directory(&directory)?;
        sync_directory(&self.history_directory())?;

        let current = self.read()?;
        if current.revision() != entry.before_revision {
            entry.state = TransactionState::Aborted;
            self.write_entry(&directory, &entry)?;
            return Err(conflict(&entry.before_revision, &current.revision()));
        }
        if let Err(error) = verify().and_then(|()| crate::cancel::check(cancel)) {
            entry.state = TransactionState::Aborted;
            self.write_entry(&directory, &entry)?;
            return Err(error);
        }
        // Once replacement begins, finish the journal commit even if cancellation arrives.
        if let Err(error) = atomic_replace(&self.path, after.text().as_bytes()) {
            if self
                .read()
                .is_ok_and(|d| d.revision() == entry.after_revision)
            {
                return Err(Error::new("RECOVERY_REQUIRED", "The document was replaced but its durability could not be confirmed")
                    .details(json!({"checkpoint_id": id, "revision": entry.after_revision, "cause": error.to_string()})));
            }
            entry.state = TransactionState::Aborted;
            self.write_entry(&directory, &entry)?;
            return Err(error);
        }
        entry.state = TransactionState::Committed;
        self.write_entry(&directory, &entry).map_err(|error| Error::new("RECOVERY_REQUIRED",
            "The document was saved, but its history commit needs recovery"
        ).details(json!({"checkpoint_id": id, "revision": entry.after_revision, "cause": error.to_string()})))?;
        Ok(result)
    }

    pub fn history(&self, limit: usize, before_sequence: Option<u64>) -> Result<Vec<HistoryEntry>> {
        self.history_with_cancellation(limit, before_sequence, None)
    }
    pub fn history_with_cancellation(
        &self,
        limit: usize,
        before_sequence: Option<u64>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<HistoryEntry>> {
        let _lock = self.lock_mode_cancellable(false, cancel)?;
        self.recover_locked()?;
        crate::cancel::check(cancel)?;
        Ok(self
            .entries_locked()?
            .into_iter()
            .rev()
            .filter(|e| {
                e.state == TransactionState::Committed
                    && before_sequence.is_none_or(|s| e.sequence < s)
            })
            .take(limit.min(1000))
            .collect())
    }

    pub fn restore(
        &self,
        request_id: &str,
        expected_revision: &str,
        checkpoint: &str,
    ) -> Result<Value> {
        if !valid_checkpoint_id(checkpoint) {
            return Err(Error::new("INVALID_ARGUMENT", "Invalid checkpoint ID"));
        }
        let fingerprint = hash(
            json!({"restore": checkpoint, "expected_revision": expected_revision})
                .to_string()
                .as_bytes(),
        );
        self.transact(request_id, expected_revision, &fingerprint, |_| {
            Ok((
                self.checkpoint_document(checkpoint)?,
                json!({"restored_checkpoint": checkpoint}),
            ))
        })
    }

    pub fn checkpoint_document(&self, checkpoint: &str) -> Result<Document> {
        if !valid_checkpoint_id(checkpoint) {
            return Err(Error::new("INVALID_ARGUMENT", "Invalid checkpoint ID"));
        }
        let directory = self.history_directory().join(checkpoint);
        let entry = read_entry(&directory)?;
        if entry.state != TransactionState::Committed {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The checkpoint is not committed",
            ));
        }
        let text = read_text(&directory.join("after.pen"))?;
        if hash(text.as_bytes()) != entry.after_revision {
            return Err(Error::new(
                "RECOVERY_REQUIRED",
                "The checkpoint content does not match its recorded hash",
            ));
        }
        Document::parse(text)
    }

    pub fn recover(&self) -> Result<Vec<String>> {
        let _lock = self.lock()?;
        self.recover_locked()
    }

    fn recover_locked(&self) -> Result<Vec<String>> {
        let mut recovered = Vec::new();
        for mut entry in self.entries_locked()? {
            if entry.state != TransactionState::Prepared {
                continue;
            }
            let directory = self.history_directory().join(&entry.id);
            for (name, expected) in [
                ("before.pen", &entry.before_revision),
                ("after.pen", &entry.after_revision),
            ] {
                if hash(read_text(&directory.join(name))?.as_bytes()) != *expected {
                    return Err(Error::new(
                        "RECOVERY_REQUIRED",
                        format!("Corrupt recovery content in {}", entry.id),
                    ));
                }
            }
            let current = self.read()?.revision();
            entry.state = if current == entry.after_revision {
                TransactionState::Committed
            } else if current == entry.before_revision {
                TransactionState::Aborted
            } else {
                return Err(Error::new("RECOVERY_REQUIRED", "The document differs from both recovery snapshots; automatic recovery would overwrite external changes")
                        .details(json!({"checkpoint_id": entry.id, "current_revision": current})));
            };
            self.write_entry(&directory, &entry)?;
            recovered.push(entry.id);
        }
        Ok(recovered)
    }

    fn history_directory(&self) -> PathBuf {
        self.document_data_dir().join("history")
    }

    fn entries_locked(&self) -> Result<Vec<HistoryEntry>> {
        let directory = self.history_directory();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for child in fs::read_dir(&directory).map_err(|e| Error::io(&directory, e))? {
            let child = child.map_err(|e| Error::io(&directory, e))?;
            let path = child.path();
            if !path.is_dir() || !path.join("metadata.json").exists() {
                continue;
            }
            let entry = read_entry(&path)?;
            if path.file_name().and_then(|s| s.to_str()) != Some(&entry.id)
                || !valid_checkpoint_id(&entry.id)
            {
                return Err(Error::new(
                    "RECOVERY_REQUIRED",
                    "History identity does not match its directory",
                ));
            }
            entries.push(entry);
        }
        entries.sort_by_key(|e| e.sequence);
        Ok(entries)
    }

    fn write_entry(&self, directory: &Path, entry: &HistoryEntry) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(entry)
            .map_err(|e| Error::new("INTERNAL_ERROR", e.to_string()))?;
        atomic_replace(&directory.join("metadata.json"), &bytes)
    }

    fn lock(&self) -> Result<File> {
        self.lock_mode(false)
    }

    pub fn lock_dependencies(&self, paths: &[PathBuf]) -> Result<Vec<File>> {
        self.lock_dependencies_with_cancellation(paths, None)
    }
    pub fn lock_dependencies_with_cancellation(
        &self,
        paths: &[PathBuf],
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<File>> {
        let mut paths = paths
            .iter()
            .map(|path| {
                crate::cancel::check(cancel)?;
                canonical_lock_path(path)
            })
            .collect::<Result<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        paths.retain(|path| path != &self.path);
        paths
            .into_iter()
            .map(|path| lock_path(&path, &self.lock_dir, true, cancel))
            .collect()
    }

    pub(crate) fn lock_publication(
        &self,
        path: &Path,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<File> {
        lock_path(&canonical_lock_path(path)?, &self.lock_dir, false, cancel)
    }

    pub(crate) fn lock_configuration(path: &Path) -> Result<File> {
        lock_path(
            &canonical_lock_path(path)?,
            &default_lock_dir()?,
            false,
            Some(&crate::cancel::token()),
        )
    }

    fn lock_mode(&self, shared: bool) -> Result<File> {
        self.lock_mode_cancellable(shared, None)
    }
    fn lock_mode_cancellable(
        &self,
        shared: bool,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<File> {
        lock_path(&self.path, &self.lock_dir, shared, cancel)
    }
}

fn lock_path(
    resource: &Path,
    directory: &Path,
    shared: bool,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<File> {
    crate::cancel::check(cancel)?;
    if resource.to_str().is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Document paths must be UTF-8",
        ));
    }
    create_private_directory(directory)?;
    let path = directory.join(format!(
        "{}.lock",
        hash(resource.as_os_str().as_encoded_bytes())
    ));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|error| Error::io(&path, error))?;
    let start = Instant::now();
    let mut reported = false;
    loop {
        crate::cancel::check(cancel)?;
        let result = if shared {
            FileExt::try_lock_shared(&file)
        } else {
            FileExt::try_lock_exclusive(&file)
        };
        match result {
            Ok(()) => return Ok(file),
            Err(error) if lock_contended(&error) && start.elapsed() < Duration::from_secs(5) => {
                if !reported && crate::cancel::cli_handler_installed() {
                    eprintln!("Waiting for file lock: {}", resource.display());
                    reported = true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                return Err(Error::new(
                    "LOCK_UNAVAILABLE",
                    format!("Could not lock {}: {error}", resource.display()),
                ));
            }
        }
    }
}

fn canonical_lock_path(path: &Path) -> Result<PathBuf> {
    match crate::paths::MissingPath::capture(path)? {
        Some(snapshot) => Ok(snapshot.target),
        None => crate::paths::canonical(path),
    }
}

pub(crate) fn lock_contended(error: &std::io::Error) -> bool {
    // Windows ERROR_LOCK_VIOLATION is not ErrorKind::WouldBlock on the MSRV.
    // Compare fs2's platform-specific code rather than retrying unrelated errors.
    error
        .raw_os_error()
        .is_some_and(|code| Some(code) == fs2::lock_contended_error().raw_os_error())
}

pub fn default_data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LAZYPENCIL_DATA_DIR") {
        return Ok(path.into());
    }
    ProjectDirs::from("dev", "lazypencil", "lazypencil")
        .map(|d| d.data_local_dir().to_path_buf())
        .ok_or_else(|| {
            Error::new(
                "CONFIGURATION_ERROR",
                "Cannot determine user data directory; set LAZYPENCIL_DATA_DIR",
            )
        })
}

pub fn data_directory(supplied: Option<PathBuf>) -> Result<PathBuf> {
    let path = supplied.map(Ok).unwrap_or_else(default_data_dir)?;
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| Error::io(&path, error))?
            .join(path)
    };
    if path.to_str().is_none() {
        return Err(Error::new(
            "CONFIGURATION_ERROR",
            "The data directory must be a UTF-8 path",
        ));
    }
    Ok(path)
}

pub fn default_lock_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LAZYPENCIL_LOCK_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(Error::new(
                "CONFIGURATION_ERROR",
                "LAZYPENCIL_LOCK_DIR must be absolute",
            ));
        }
        if path.to_str().is_none() {
            return Err(Error::new(
                "CONFIGURATION_ERROR",
                "LAZYPENCIL_LOCK_DIR must be UTF-8",
            ));
        }
        return Ok(path);
    }
    ProjectDirs::from("dev", "lazypencil", "lazypencil")
        .map(|dirs| dirs.cache_dir().join("locks"))
        .ok_or_else(|| {
            Error::new(
                "CONFIGURATION_ERROR",
                "Cannot determine a stable per-user lock directory; set LAZYPENCIL_LOCK_DIR",
            )
        })
}

pub fn require_revision(document: &Document, expected: &str) -> Result<()> {
    if document.revision() == expected {
        Ok(())
    } else {
        Err(conflict(expected, &document.revision()))
    }
}

fn conflict(expected: &str, actual: &str) -> Error {
    Error::new(
        "REVISION_CONFLICT",
        "The document changed; read its current revision and re-plan",
    )
    .details(json!({"expected_revision": expected, "actual_revision": actual}))
}

pub fn read_text(path: &Path) -> Result<String> {
    let file = open_regular_file(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::new(
            "INVALID_DOCUMENT",
            "File exceeds the size limit",
        ));
    }
    String::from_utf8(bytes).map_err(|e| {
        Error::new(
            "INVALID_DOCUMENT",
            format!("{} is not UTF-8: {e}", path.display()),
        )
    })
}

pub(crate) fn open_regular_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| Error::io(path, error))?;
    if !file
        .metadata()
        .map_err(|error| Error::io(path, error))?
        .is_file()
    {
        return Err(Error::new(
            "INVALID_DOCUMENT",
            "Document and asset input must be a regular file",
        ));
    }
    Ok(file)
}

pub fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("IO_ERROR", "Missing parent directory"))?;
    let mut file = NamedTempFile::new_in(parent).map_err(|e| Error::io(parent, e))?;
    if let Ok(metadata) = fs::metadata(path) {
        file.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| Error::io(path, e))?;
    }
    file.write_all(bytes).map_err(|e| Error::io(path, e))?;
    file.as_file().sync_all().map_err(|e| Error::io(path, e))?;
    file.persist(path).map_err(|e| Error::io(path, e.error))?;
    sync_directory(parent)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| Error::io(path, e))?;
    file.write_all(bytes).map_err(|e| Error::io(path, e))?;
    file.sync_all().map_err(|e| Error::io(path, e))
}

pub fn create_private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| Error::io(path, e))
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| Error::io(path, e))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn read_entry(directory: &Path) -> Result<HistoryEntry> {
    let path = directory.join("metadata.json");
    serde_json::from_str(&read_text(&path)?).map_err(|e| {
        Error::new(
            "RECOVERY_REQUIRED",
            format!("Cannot read history metadata {}: {e}", path.display()),
        )
    })
}

fn timestamp() -> Result<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| Error::new("INTERNAL_ERROR", e.to_string()))
}

fn validate_request_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        Err(Error::new(
            "INVALID_ARGUMENT",
            "Request ID must contain 1..128 bytes and no control characters",
        ))
    } else {
        Ok(())
    }
}

fn valid_checkpoint_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 53
        && bytes[20] == b'-'
        && bytes[..20].iter().all(u8::is_ascii_digit)
        && bytes[21..].iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod lock_error_tests {
    #[test]
    fn fs2_contention_is_retryable_and_unrelated_io_errors_are_not() {
        use std::io::{Error, ErrorKind};
        let file = tempfile::NamedTempFile::new().unwrap();
        let other = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(file.path())
            .unwrap();
        fs2::FileExt::lock_exclusive(file.as_file()).unwrap();
        let busy = fs2::FileExt::try_lock_shared(&other).unwrap_err();
        assert!(super::lock_contended(&busy));
        assert!(super::lock_contended(&fs2::lock_contended_error()));
        assert!(!super::lock_contended(&Error::new(
            ErrorKind::PermissionDenied,
            "access denied"
        )));
        assert!(!super::lock_contended(&Error::other("unrelated failure")));
        assert!(!super::lock_contended(&Error::from_raw_os_error(0)));
    }
}
