//! Explicit, revision-checked maintenance of content-addressed project assets.

use crate::{
    Error, Result, document::hash, project::secure_filesystem as filesystem, store::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

mod scan;
mod trash;

const MAX_ENTRIES: usize = 100_000;
const MAX_BYTES: usize = 512 * 1024 * 1024;
const MAX_FILE: usize = 64 * 1024 * 1024;
const TRASH: &str = ".lazypencil-trash-";

struct Budget<'a> {
    started: Instant,
    cancel: Option<&'a CancellationToken>,
    entries: usize,
    bytes: usize,
}
impl Budget<'_> {
    fn check(&self) -> Result<()> {
        crate::cancel::check(self.cancel)?;
        if self.started.elapsed() >= Duration::from_secs(30) {
            return Err(Error::new(
                "RESOURCE_LIMIT",
                "Asset maintenance exceeded 30 seconds",
            ));
        }
        Ok(())
    }
    fn entries(&mut self, count: usize) -> Result<()> {
        self.check()?;
        self.entries = self.entries.saturating_add(count);
        if self.entries > MAX_ENTRIES {
            return Err(Error::new(
                "RESOURCE_LIMIT",
                "Asset maintenance exceeds 100000 entries",
            ));
        }
        Ok(())
    }
    fn read(&mut self, path: &Path, limit: usize) -> Result<Vec<u8>> {
        self.check()?;
        let bytes = filesystem::read(
            path,
            limit.min(MAX_BYTES.saturating_sub(self.bytes)),
            self.cancel,
        )?;
        self.bytes = self.bytes.saturating_add(bytes.len());
        self.check()?;
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    sha256: String,
    bytes: u64,
}
impl Content {
    fn of(bytes: &[u8]) -> Self {
        Self {
            sha256: hash(bytes),
            bytes: bytes.len() as u64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    path: PathBuf,
    digest: String,
    bundle: bool,
    files: BTreeMap<String, Content>,
}
impl Candidate {
    fn bytes(&self) -> u64 {
        self.files.values().map(|file| file.bytes).sum()
    }
}

fn digest_name(name: &str) -> Option<&str> {
    let (digest, extension) = name.split_once('.')?;
    (digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && matches!(
            extension,
            "png" | "jpg" | "jpeg" | "webp" | "js" | "glsl" | "lib.pen" | "bundle"
        ))
    .then_some(digest)
}

fn changed() -> Error {
    Error::new(
        "REVISION_CONFLICT",
        "Asset maintenance inputs changed; review assets prune again",
    )
}

/// Review by default. Applying requires the exact revision returned by review.
/// The caller supplies the complete local project boundary, including sibling
/// documents that can share an assets/lazypencil directory. Inbound references
/// from outside that boundary or another user-data directory are not inferred.
pub fn run(
    file: &Path,
    project_root: &Path,
    data_dir: Option<PathBuf>,
    expected_revision: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    let mut budget = Budget {
        started: Instant::now(),
        cancel,
        entries: 0,
        bytes: 0,
    };
    budget.check()?;
    let store = Store::open_for_asset_maintenance(file, data_dir)?;
    let root = crate::paths::canonical(project_root)?;
    filesystem::entries(&root, MAX_ENTRIES, cancel)?;
    if !store.path().starts_with(&root) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Project root must contain the bound document",
        ));
    }
    let scan = scan::collect(&store, &root, &mut budget)?;
    let revision = hash(
        serde_json::to_vec(&json!({
            "version":1,"root":root,"document":store.path(),"data_dir":store.data_dir(),
            "sources":scan.sources,"projects":scan.identities,"owned":scan.owned,
            "pending":scan.pending,"pending_payloads":scan.pending_payloads,
            "retained":scan.retained,"uncertain":scan.uncertain,
            "references":scan.references,"paths":scan.paths
        }))
        .map_err(|error| Error::new("INTERNAL_ERROR", error.to_string()))?
        .as_slice(),
    );
    if expected_revision.is_some_and(|expected| expected != revision) {
        return Err(changed());
    }
    let candidates = scan
        .owned
        .iter()
        .filter(|candidate| !scan.protected(candidate))
        .cloned()
        .collect::<Vec<_>>();
    let pending = scan
        .pending
        .iter()
        .map(|journal| {
            json!({
                "path":journal.directory(),"asset":journal.asset.path,
                "action":if scan.protected(&journal.asset) {"restore"} else {"cleanup"}
            })
        })
        .collect::<Vec<_>>();
    let mut result = json!({
        "project_root":root,"data_dir":store.data_dir(),"revision":revision,
        "applied":false,"candidates":candidates.iter().map(|candidate| json!({
            "path":candidate.path,"bundle":candidate.bundle,"bytes":candidate.bytes()
        })).collect::<Vec<_>>(),
        "candidate_bytes":candidates.iter().map(Candidate::bytes).sum::<u64>(),
        "retained":scan.retained,"uncertain":scan.uncertain,"pending":pending,
        "deleted":[],"restored":[],"cleanup_pending":[],"errors":[]
    });
    if expected_revision.is_none() {
        return Ok(result);
    }
    // Validate the whole source closure again before each committed rename.
    // Each operation is independent: a cancellation reports earlier commits.
    result["applied"] = json!(true);
    for journal in &scan.pending {
        let operation: Result<()> = (|| {
            scan.verify(&mut budget)?;
            if scan.protected(&journal.asset) {
                trash::restore(journal, &mut budget)?;
                result["restored"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(journal.asset.path));
            } else {
                trash::finish(journal, &mut budget)?;
                result["deleted"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(journal.asset.path));
            }
            Ok(())
        })();
        if let Err(error) = operation {
            result["errors"].as_array_mut().unwrap().push(json!(error));
            result["cleanup_pending"]
                .as_array_mut()
                .unwrap()
                .push(json!(journal.directory()));
            return Ok(result);
        }
    }
    for candidate in &candidates {
        let operation: Result<()> = (|| {
            scan.verify(&mut budget)?;
            scan::verify_candidate(candidate, false, &mut budget)?;
            let journal = trash::quarantine(candidate, &root, store.path(), &mut budget)?;
            result["deleted"]
                .as_array_mut()
                .unwrap()
                .push(json!(candidate.path));
            if let Err(error) = trash::finish(&journal, &mut budget) {
                result["errors"].as_array_mut().unwrap().push(json!(error));
                result["cleanup_pending"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(journal.directory()));
            }
            Ok(())
        })();
        if let Err(error) = operation {
            if let Some(path) = error
                .details
                .as_ref()
                .and_then(|details| details.get("journal"))
            {
                result["cleanup_pending"]
                    .as_array_mut()
                    .unwrap()
                    .push(path.clone());
            }
            result["errors"].as_array_mut().unwrap().push(json!(error));
            break;
        }
        if !result["errors"].as_array().unwrap().is_empty() {
            break;
        }
    }
    Ok(result)
}
