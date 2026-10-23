//! Bound, per-turn metadata for design resources returned by an owned MCP process.

use crate::{
    Error, Result,
    store::{Store, atomic_replace, create_private_directory},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_JOURNAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    document: PathBuf,
    conversation: String,
    connection: String,
    turn: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UsageRecord {
    #[serde(rename = "type")]
    kind: String,
    source: String,
    turn: String,
    connection: String,
    tool: String,
    resources: Vec<ResourceRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependency_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_design: Option<ProjectIdentity>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ResourceRecord {
    Guide {
        id: String,
        version: String,
        hash: String,
        language: String,
    },
    Style {
        id: String,
        version: String,
        hash: String,
        language: String,
    },
    StyleCatalog {
        version: Option<String>,
        hash: String,
        hash_scope: String,
        styles: Vec<StyleSummary>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StyleSummary {
    id: String,
    version: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectIdentity {
    fingerprint: String,
    trust_fingerprint: String,
}

/// The owner publishes active turns; MCP children can only append to that turn.
pub struct UsageGrant {
    access: UsageAccess,
    active: Option<String>,
}

impl UsageGrant {
    pub fn create(store: &Store, directory: &Path, conversation: &str) -> Result<Self> {
        require_uuid(conversation)?;
        let directory = crate::paths::canonical(directory)?;
        let expected = store
            .document_data_dir()
            .join("conversations")
            .join(conversation);
        if directory != crate::paths::canonical(&expected)? {
            return Err(invalid(
                "Design usage must belong to the bound conversation",
            ));
        }
        let connection = uuid::Uuid::new_v4().to_string();
        let directory = directory.join(format!("design-usage-{connection}"));
        create_private_directory(&directory)?;
        new_file(&directory.join("lock"))?
            .sync_all()
            .map_err(|error| Error::io(&directory, error))?;
        let path = directory.join("active.json");
        let manifest = Manifest {
            version: 1,
            document: store.path().into(),
            conversation: conversation.into(),
            connection: connection.clone(),
            turn: None,
        };
        save_manifest(&path, &manifest)?;
        Ok(Self {
            access: UsageAccess {
                path,
                document: store.path().into(),
                conversation: conversation.into(),
                connection,
            },
            active: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.access.path
    }

    pub fn begin_turn(&mut self, turn: &str) -> Result<()> {
        require_uuid(turn)?;
        let _lock = self.access.lock(None)?;
        let mut manifest = self.access.manifest()?;
        if self.active.is_some() || manifest.turn.is_some() {
            return Err(invalid("The previous design usage turn has not ended"));
        }
        let path = self.access.journal(turn);
        new_file(&path)?
            .sync_all()
            .map_err(|error| Error::io(&path, error))?;
        manifest.turn = Some(turn.into());
        save_manifest(self.path(), &manifest)?;
        self.active = Some(turn.into());
        Ok(())
    }

    /// Freeze the journal before reading, including on failed or cancelled turns.
    pub fn finish_turn(&mut self) -> Result<Vec<Value>> {
        let Some(turn) = self.active.clone() else {
            return Ok(vec![]);
        };
        let _lock = self.access.lock(None)?;
        let mut manifest = self.access.manifest()?;
        if manifest.turn.as_deref() != Some(&turn) {
            return Err(invalid("The design usage turn changed unexpectedly"));
        }
        manifest.turn = None;
        save_manifest(self.path(), &manifest)?;
        self.active = None;
        let path = self.access.journal(&turn);
        let bytes = read_bounded(&path, MAX_JOURNAL_BYTES)?;
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            return Err(invalid(
                "The design usage journal contains an incomplete record",
            ));
        }
        let mut records = vec![];
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            if line.len() > MAX_RECORD_BYTES {
                return Err(invalid("A design usage record exceeds its limit"));
            }
            let value: UsageRecord = serde_json::from_slice(line)
                .map_err(|_| invalid("The design usage journal is invalid"))?;
            if value.kind != "design_resource"
                || value.source != "mcp"
                || value.turn != turn
                || value.connection != self.access.connection
                || !matches!(
                    value.tool.as_str(),
                    "get_guidance" | "get_style" | "list_styles"
                )
                || value.resources.is_empty()
                || value.resources.len() > 33
            {
                return Err(invalid(
                    "The design usage record belongs to a different turn",
                ));
            }
            for resource in &value.resources {
                match resource {
                    ResourceRecord::Guide {
                        id,
                        version,
                        hash,
                        language,
                    }
                    | ResourceRecord::Style {
                        id,
                        version,
                        hash,
                        language,
                    } => {
                        require_short(id, 128)?;
                        require_short(version, 64)?;
                        require_short(language, 128)?;
                        require_hash(hash)?;
                    }
                    ResourceRecord::StyleCatalog {
                        version,
                        hash,
                        hash_scope,
                        styles,
                    } => {
                        if let Some(version) = version {
                            require_short(version, 64)?;
                        }
                        require_hash(hash)?;
                        if hash_scope != "response" || styles.len() > 100 {
                            return Err(invalid("Too many style summaries in design usage"));
                        }
                        for style in styles {
                            require_short(&style.id, 128)?;
                            require_short(&style.version, 64)?;
                        }
                    }
                }
            }
            for revision in [&value.revision, &value.dependency_revision]
                .into_iter()
                .flatten()
            {
                require_hash(revision)?;
            }
            if let Some(design) = &value.project_design {
                require_hash(&design.fingerprint)?;
                require_hash(&design.trust_fingerprint)?;
            }
            records.push(serde_json::to_value(value).map_err(|error| invalid(error.to_string()))?);
        }
        Ok(records)
    }
}

impl Drop for UsageGrant {
    fn drop(&mut self) {
        // Keep bounded journals for interrupted-transcript diagnosis; revoke writes.
        let _ = fs::remove_file(self.path());
    }
}

#[derive(Clone)]
pub struct UsageAccess {
    path: PathBuf,
    document: PathBuf,
    conversation: String,
    connection: String,
}

impl UsageAccess {
    pub fn open(store: &Store, path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, 16 * 1024)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid design usage grant"))?;
        require_uuid(&manifest.conversation)?;
        require_uuid(&manifest.connection)?;
        let expected = store
            .document_data_dir()
            .join("conversations")
            .join(&manifest.conversation)
            .join(format!("design-usage-{}", manifest.connection))
            .join("active.json");
        let path = crate::paths::canonical(path)?;
        if path != crate::paths::canonical(expected)? || manifest.document != store.path() {
            return Err(invalid(
                "Design usage belongs to a different document or conversation",
            ));
        }
        let access = Self {
            path,
            document: store.path().into(),
            conversation: manifest.conversation,
            connection: manifest.connection,
        };
        access.manifest()?;
        Ok(access)
    }

    pub(crate) fn request(&self, cancel: &CancellationToken) -> Result<UsageRequest> {
        let _lock = self.lock(Some(cancel))?;
        let turn = self
            .manifest()?
            .turn
            .ok_or_else(|| invalid("No active design usage turn"))?;
        Ok(UsageRequest {
            access: self.clone(),
            turn,
        })
    }

    fn journal(&self, turn: &str) -> PathBuf {
        self.path
            .parent()
            .expect("usage directory")
            .join(format!("{turn}.jsonl"))
    }

    fn manifest(&self) -> Result<Manifest> {
        let bytes = read_bounded(&self.path, 16 * 1024)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid design usage grant"))?;
        if manifest.version != 1
            || manifest.document != self.document
            || manifest.conversation != self.conversation
            || manifest.connection != self.connection
        {
            return Err(invalid("The design usage grant changed its identity"));
        }
        if let Some(turn) = &manifest.turn {
            require_uuid(turn)?;
        }
        Ok(manifest)
    }

    fn lock(&self, cancel: Option<&CancellationToken>) -> Result<Lock> {
        let path = self.path.parent().expect("usage directory").join("lock");
        let file = regular_file(&path, true)?;
        let start = Instant::now();
        loop {
            crate::cancel::check(cancel)?;
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Lock(file)),
                Err(error)
                    if crate::store::lock_contended(&error)
                        && start.elapsed() < Duration::from_secs(2) =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(Error::new(
                        "LOCK_UNAVAILABLE",
                        format!("Cannot lock design usage: {error}"),
                    ));
                }
            }
        }
    }
}

pub(crate) struct UsageRequest {
    access: UsageAccess,
    turn: String,
}

impl UsageRequest {
    pub(crate) fn record(
        &self,
        tool: &str,
        value: &Value,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut resources = match tool {
            "get_guidance" => vec![identity("guide", value)],
            "get_style" => vec![identity("style", value)],
            "list_styles" => vec![
                json!({"kind":"style_catalog","version":value["version"],"hash":crate::document::hash(value.to_string().as_bytes()),"hash_scope":"response","styles":value["styles"].as_array().map(|styles| styles.iter().map(|style| json!({"id":style["id"],"version":style["version"]})).collect::<Vec<_>>())}),
            ],
            _ => return Err(invalid("Unsupported design usage tool")),
        };
        if tool == "get_guidance"
            && let Some(guides) = value["project_guides"].as_array()
        {
            resources.extend(guides.iter().map(|guide| identity("guide", guide)));
        }
        let mut record = json!({"type":"design_resource","source":"mcp","turn":self.turn,"connection":self.access.connection,"tool":tool,"resources":resources});
        for key in ["revision", "dependency_revision"] {
            if let Some(value) = value.get(key) {
                record[key] = value.clone();
            }
        }
        if let Some(design) = value.get("project_design") {
            record["project_design"] = json!({"fingerprint":design["fingerprint"],"trust_fingerprint":design["trust_fingerprint"]});
        }
        let mut bytes = serde_json::to_vec(&record).map_err(|error| invalid(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(invalid("Design usage metadata exceeds 64 KiB"));
        }
        let _lock = self.access.lock(Some(cancel))?;
        if self.access.manifest()?.turn.as_deref() != Some(&self.turn) {
            return Err(invalid(
                "The design usage turn ended before the resource was returned",
            ));
        }
        let path = self.access.journal(&self.turn);
        let mut file = regular_file(&path, true)?;
        let size = file
            .metadata()
            .map_err(|error| Error::io(&path, error))?
            .len();
        if size + bytes.len() as u64 > MAX_JOURNAL_BYTES as u64 {
            return Err(invalid("The design usage journal exceeded 4 MiB"));
        }
        file.write_all(&bytes)
            .and_then(|()| file.sync_data())
            .map_err(|error| Error::io(&path, error))
    }
}

fn identity(kind: &str, value: &Value) -> Value {
    json!({"kind":kind,"id":value["id"],"version":value["version"],"hash":value["hash"],"language":value["language"]})
}

fn require_uuid(value: &str) -> Result<()> {
    if uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err(invalid("Invalid design usage identity"))
    }
}

fn require_short(value: &str, max: usize) -> Result<()> {
    if value.is_empty() || value.len() > max {
        Err(invalid("Invalid design usage metadata"))
    } else {
        Ok(())
    }
}

fn require_hash(value: &str) -> Result<()> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid("Invalid design usage content hash"))
    }
}

fn save_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    atomic_replace(
        path,
        &serde_json::to_vec(manifest).map_err(|error| invalid(error.to_string()))?,
    )
}

fn new_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| Error::io(path, error))
}

fn regular_file(path: &Path, write: bool) -> Result<File> {
    if !fs::symlink_metadata(path)
        .map_err(|error| Error::io(path, error))?
        .is_file()
    {
        return Err(invalid("Design usage requires a regular file"));
    }
    let mut options = OpenOptions::new();
    options.read(true).append(write);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| Error::io(path, error))?;
    if !file
        .metadata()
        .map_err(|error| Error::io(path, error))?
        .is_file()
    {
        return Err(invalid("Design usage requires a regular file"));
    }
    Ok(file)
}

fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    regular_file(path, false)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(path, error))?;
    if bytes.len() > max {
        return Err(invalid("Design usage exceeds its size limit"));
    }
    Ok(bytes)
}

struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("AGENT_ERROR", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn grant() -> (TempDir, Store, UsageGrant) {
        let temp = TempDir::new().unwrap();
        let store = Store::init(
            temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        let conversation = uuid::Uuid::new_v4().to_string();
        let directory = store
            .document_data_dir()
            .join("conversations")
            .join(&conversation);
        create_private_directory(&directory).unwrap();
        let grant = UsageGrant::create(&store, &directory, &conversation).unwrap();
        (temp, store, grant)
    }

    #[test]
    fn concurrent_children_preserve_each_resource_without_body_data() {
        let (_temp, store, mut grant) = grant();
        let turn = uuid::Uuid::new_v4().to_string();
        grant.begin_turn(&turn).unwrap();
        let access = UsageAccess::open(&store, grant.path()).unwrap();
        let guide = crate::guidance::get_guidance(Some("operations")).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let access = access.clone();
                let guide = &guide;
                scope.spawn(move || {
                    for _ in 0..8 {
                        let cancel = CancellationToken::new();
                        access
                            .request(&cancel)
                            .unwrap()
                            .record("get_guidance", guide, &cancel)
                            .unwrap();
                    }
                });
            }
        });
        let records = grant.finish_turn().unwrap();
        assert_eq!(records.len(), 32);
        for record in records {
            assert_eq!(record["resources"][0]["id"], "operations");
            assert_eq!(record["resources"][0]["hash"], guide["hash"]);
            assert_eq!(record["turn"], turn);
            assert!(!record.to_string().contains("example_operations"));
            assert!(!record.to_string().contains("body"));
        }
    }

    #[test]
    fn old_requests_cannot_write_into_a_later_turn_and_revoked_grants_stay_revoked() {
        let (temp, store, mut grant) = grant();
        let other = Store::init(
            temp.path().join("other.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        assert!(UsageAccess::open(&other, grant.path()).is_err());
        let access = UsageAccess::open(&store, grant.path()).unwrap();
        let cancel = CancellationToken::new();
        assert!(access.request(&cancel).is_err());
        grant.begin_turn(&uuid::Uuid::new_v4().to_string()).unwrap();
        let old = access.request(&cancel).unwrap();
        assert!(grant.finish_turn().unwrap().is_empty());
        grant.begin_turn(&uuid::Uuid::new_v4().to_string()).unwrap();
        let guide = crate::guidance::get_guidance(None).unwrap();
        assert!(old.record("get_guidance", &guide, &cancel).is_err());
        let current = access.request(&cancel).unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(
            current
                .record("get_guidance", &guide, &cancelled)
                .unwrap_err()
                .code,
            "CANCELLED"
        );
        assert!(grant.finish_turn().unwrap().is_empty());
        let path = grant.path().to_owned();
        drop(grant);
        assert!(access.request(&cancel).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn oversized_or_corrupt_journals_fail_explicitly_and_a_new_turn_can_start() {
        let (_temp, store, mut grant) = grant();
        let turn = uuid::Uuid::new_v4().to_string();
        grant.begin_turn(&turn).unwrap();
        let access = UsageAccess::open(&store, grant.path()).unwrap();
        let journal = access.journal(&turn);
        regular_file(&journal, true)
            .unwrap()
            .set_len(MAX_JOURNAL_BYTES as u64)
            .unwrap();
        let cancel = CancellationToken::new();
        assert!(
            access
                .request(&cancel)
                .unwrap()
                .record(
                    "get_guidance",
                    &crate::guidance::get_guidance(None).unwrap(),
                    &cancel
                )
                .is_err()
        );
        assert!(grant.finish_turn().is_err());
        let turn = uuid::Uuid::new_v4().to_string();
        grant.begin_turn(&turn).unwrap();
        let path = access.journal(&turn);
        fs::write(&path, b"{\"type\":\"design_resource\"").unwrap();
        assert!(grant.finish_turn().is_err());
        let turn = uuid::Uuid::new_v4().to_string();
        grant.begin_turn(&turn).unwrap();
        let mut record = json!({"type":"design_resource","source":"mcp","turn":turn,"connection":access.connection,"tool":"get_guidance","resources":[],"body":"MUST_NOT_LOG"}).to_string();
        record.push('\n');
        fs::write(access.journal(&turn), record).unwrap();
        assert!(grant.finish_turn().is_err());
        grant.begin_turn(&uuid::Uuid::new_v4().to_string()).unwrap();
        access
            .request(&cancel)
            .unwrap()
            .record(
                "get_style",
                &crate::guidance::get_style("standard-ui").unwrap(),
                &cancel,
            )
            .unwrap();
        assert_eq!(grant.finish_turn().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn audit_files_cannot_be_symlinks_or_blocking_pipes() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let (temp, store, mut grant) = grant();
        let turn = uuid::Uuid::new_v4().to_string();
        grant.begin_turn(&turn).unwrap();
        let access = UsageAccess::open(&store, grant.path()).unwrap();
        let path = access.journal(&turn);
        fs::remove_file(&path).unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"original").unwrap();
        symlink(&outside, &path).unwrap();
        let cancel = CancellationToken::new();
        let guide = crate::guidance::get_guidance(None).unwrap();
        assert!(
            access
                .request(&cancel)
                .unwrap()
                .record("get_guidance", &guide, &cancel)
                .is_err()
        );
        assert_eq!(fs::read(&outside).unwrap(), b"original");
        fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(
            access
                .request(&cancel)
                .unwrap()
                .record("get_guidance", &guide, &cancel)
                .is_err()
        );
        assert!(grant.finish_turn().is_err());
    }
}
