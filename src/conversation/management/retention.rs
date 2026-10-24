//! Reviewed aggregate retention across the selected local data directory.

use super::deletion::{self, Budget};
use crate::conversation::{history, storage};
use crate::{
    Error, Result, config::conversations::RetentionSettings, document::hash, store::Store,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

const MAX_RECORDS: usize = 10_000;
const MAX_DIRECTORY_ENTRIES: usize = 100_000;

#[derive(Serialize)]
struct Entry {
    record_key: String,
    document_key: String,
    document: Option<PathBuf>,
    session: Option<String>,
    name: String,
    kind: &'static str,
    bytes: Option<u64>,
    last_write_unix_seconds: Option<u64>,
    revision: Option<String>,
    protected_reason: Option<String>,
    selected: bool,
    selection_reason: Option<&'static str>,
}

impl Entry {
    fn protected(path: &Path, key: &str, kind: &'static str, reason: impl Into<String>) -> Self {
        Self {
            record_key: hash(path.as_os_str().as_encoded_bytes()),
            document_key: key.into(),
            document: None,
            session: None,
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            kind,
            bytes: None,
            last_write_unix_seconds: None,
            revision: None,
            protected_reason: Some(reason.into()),
            selected: false,
            selection_reason: None,
        }
    }
}

#[derive(Serialize)]
struct Plan {
    version: u32,
    scope: &'static str,
    data_directory: PathBuf,
    as_of_unix_seconds: u64,
    policy: RetentionSettings,
    scanned: bool,
    entries: Vec<Entry>,
    total_known_bytes: u64,
    total_bytes_exact: bool,
    planned_reclaimed_bytes: u64,
    remaining_known_bytes: u64,
    remaining_over_limit: Option<bool>,
}

impl Plan {
    fn revision(&self) -> Result<String> {
        Ok(hash(&serde_json::to_vec(self).map_err(|error| {
            Error::new("IO_ERROR", error.to_string())
        })?))
    }
    fn value(&self) -> Result<Value> {
        let mut value = serde_json::to_value(self)
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        value["revision"] = json!(self.revision()?);
        value["selected_count"] = json!(self.entries.iter().filter(|entry| entry.selected).count());
        value["protected_count"] = json!(
            self.entries
                .iter()
                .filter(|entry| entry.protected_reason.is_some())
                .count()
        );
        value["applied"] = json!(false);
        Ok(value)
    }
}

fn directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => Ok(true),
        Ok(_) => Err(Error::new(
            "AGENT_ERROR",
            "Retention requires regular, non-symlink data directories",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::io(path, error)),
    }
}

fn children(path: &Path, budget: &Budget<'_>) -> Result<Vec<PathBuf>> {
    if !directory(path)? {
        return Ok(vec![]);
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(path).map_err(|error| Error::io(path, error))? {
        budget.check()?;
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            return Err(Error::new(
                "AGENT_ERROR",
                "Retention directory exceeds 100,000 entries",
            ));
        }
        entries.push(item.map_err(|error| Error::io(path, error))?.path());
    }
    entries.sort();
    Ok(entries)
}

fn push(entries: &mut Vec<Entry>, entry: Entry) -> Result<()> {
    if entries.len() >= MAX_RECORDS {
        return Err(Error::new(
            "AGENT_ERROR",
            "Retention review exceeds 10,000 records; explicitly remove a smaller set first",
        ));
    }
    entries.push(entry);
    Ok(())
}

fn canonical_uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

fn conversation(
    data: &Path,
    doc_root: &Path,
    path: &Path,
    key: &str,
    budget: &Budget<'_>,
) -> Entry {
    let mut entry = Entry::protected(path, key, "conversation", "unavailable");
    let name = entry.name.clone();
    if !canonical_uuid(&name) {
        entry.kind = "unknown";
        entry.protected_reason = Some("unrecognized conversation entry".into());
        return entry;
    }
    entry.session = Some(name.clone());
    let inspect = (|| {
        let state = history::metadata(path)?;
        if state.id != name || !matches!(state.version, 1..=3) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Foreign conversation identity or format",
            ));
        }
        let store = Store::for_saved_conversation(state.document.clone(), data.into())?;
        if store.document_data_dir() != doc_root {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Conversation belongs to another document namespace",
            ));
        }
        entry.document = Some(state.document);
        entry.last_write_unix_seconds = fs::symlink_metadata(path.join("session.json"))
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| time.as_secs());
        let (_lease, directory, state) = deletion::target(&store, &name, budget)?;
        let inventory = deletion::scan(&directory, budget)?;
        entry.bytes = Some(inventory.bytes);
        entry.last_write_unix_seconds = inventory.latest_write_unix_seconds;
        entry.revision = Some(inventory.revision());
        entry.protected_reason = if !state.draft.is_empty() {
            Some("unsent draft".into())
        } else if !state.pending_attachments.is_empty() {
            Some("pending images".into())
        } else if state.continuity_pending {
            Some("incomplete native continuity".into())
        } else if entry.last_write_unix_seconds.is_none() {
            Some("unavailable write time".into())
        } else {
            None
        };
        Ok(())
    })();
    if let Err(error) = inspect {
        entry.protected_reason = Some(if error.code == "REVISION_CONFLICT" {
            "busy or changed".into()
        } else {
            format!("unavailable: {}", error.code)
        });
    }
    entry
}

fn discover(data: &Path, budget: &Budget<'_>) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    if !directory(data)? {
        return Ok(entries);
    }
    for doc_root in children(&data.join("documents"), budget)? {
        budget.check()?;
        let key = doc_root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if key.len() != 64
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            push(
                &mut entries,
                Entry::protected(
                    &doc_root,
                    &key,
                    "unknown",
                    "unrecognized document namespace",
                ),
            )?;
            continue;
        }
        match children(&doc_root, budget) {
            Ok(children) => {
                for path in children {
                    budget.check()?;
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    if name == "conversations" {
                        match self::children(&path, budget) {
                            Ok(sessions) => {
                                for session in sessions {
                                    push(
                                        &mut entries,
                                        conversation(data, &doc_root, &session, &key, budget),
                                    )?;
                                    budget.check()?;
                                }
                            }
                            Err(error) => push(
                                &mut entries,
                                Entry::protected(
                                    &path,
                                    &key,
                                    "unknown",
                                    format!("unavailable conversation namespace: {}", error.code),
                                ),
                            )?,
                        }
                    } else if name.starts_with("conversation-trash-") {
                        let mut entry = Entry::protected(
                            &path,
                            &key,
                            "pending_cleanup",
                            "deletion preparation or cleanup pending",
                        );
                        if directory(&path).is_ok_and(|exists| exists)
                            && let Ok(inventory) = deletion::scan(&path, budget)
                        {
                            entry.bytes = Some(inventory.bytes);
                            entry.revision = Some(inventory.revision());
                        }
                        push(&mut entries, entry)?;
                    }
                }
            }
            Err(error) => push(
                &mut entries,
                Entry::protected(
                    &doc_root,
                    &key,
                    "unknown",
                    format!("unavailable document namespace: {}", error.code),
                ),
            )?,
        }
    }
    budget.check()?;
    Ok(entries)
}

fn plan_at(data: &Path, policy: &RetentionSettings, day: u64, budget: &Budget<'_>) -> Result<Plan> {
    policy.validate()?;
    let data = crate::store::data_directory(Some(data.into()))?;
    let mut entries = if policy.has_limits() {
        discover(&data, budget)?
    } else {
        vec![]
    };
    let mut documents: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.kind == "conversation"
            && entry.revision.is_some()
            && entry.last_write_unix_seconds.is_some()
        {
            documents
                .entry(entry.document_key.clone())
                .or_default()
                .push(index);
        }
    }
    for indices in documents.values_mut() {
        indices.sort_by_key(|index| {
            (
                std::cmp::Reverse(entries[*index].last_write_unix_seconds),
                entries[*index].session.clone(),
            )
        });
        for index in indices.iter().take(policy.keep_recent_per_document) {
            if entries[*index].protected_reason.is_none() {
                entries[*index].protected_reason = Some("recent conversation".into());
            }
        }
    }
    let total_known_bytes =
        entries
            .iter()
            .filter_map(|entry| entry.bytes)
            .try_fold(0u64, |sum, size| {
                sum.checked_add(size)
                    .ok_or_else(|| Error::new("AGENT_ERROR", "Conversation byte total overflows"))
            })?;
    let exact = policy.has_limits() && entries.iter().all(|entry| entry.bytes.is_some());
    let mut reclaimed = 0u64;
    for entry in &mut entries {
        if entry.protected_reason.is_none()
            && policy.max_age_days.is_some_and(|days| {
                entry
                    .last_write_unix_seconds
                    .is_some_and(|last| last < day.saturating_sub(u64::from(days) * 86400))
            })
        {
            entry.selected = true;
            entry.selection_reason = Some("age limit");
            reclaimed += entry.bytes.unwrap_or(0);
        }
    }
    if let Some(limit) = policy.max_total_bytes {
        let mut eligible = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.protected_reason.is_none() && !entry.selected)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        eligible.sort_by_key(|index| {
            (
                entries[*index].last_write_unix_seconds,
                entries[*index].document_key.clone(),
                entries[*index].session.clone(),
            )
        });
        for index in eligible {
            if total_known_bytes.saturating_sub(reclaimed) <= limit {
                break;
            }
            entries[index].selected = true;
            entries[index].selection_reason = Some("aggregate byte limit");
            reclaimed += entries[index].bytes.unwrap_or(0);
        }
    }
    entries.sort_by(|a, b| {
        (&a.document_key, &a.name, &a.record_key).cmp(&(&b.document_key, &b.name, &b.record_key))
    });
    let remaining = total_known_bytes.saturating_sub(reclaimed);
    let over = policy.max_total_bytes.and_then(|limit| {
        if remaining > limit {
            Some(true)
        } else if exact {
            Some(false)
        } else {
            None
        }
    });
    budget.check()?;
    Ok(Plan {
        version: 1,
        scope: "all_local_document_conversations",
        data_directory: data,
        as_of_unix_seconds: day,
        policy: policy.clone(),
        scanned: policy.has_limits(),
        entries,
        total_known_bytes,
        total_bytes_exact: exact,
        planned_reclaimed_bytes: reclaimed,
        remaining_known_bytes: remaining,
        remaining_over_limit: over,
    })
}

fn current_day() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::new("IO_ERROR", "System clock precedes the Unix epoch"))?
        .as_secs()
        / 86400
        * 86400)
}

/// Review all local document stores. No limit means no scan or deletion.
pub fn review(
    data: &Path,
    policy: &RetentionSettings,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    plan_at(data, policy, current_day()?, &budget)?.value()
}

/// Apply only an unchanged reviewed plan; individual commits retain their own
/// revision and exclusive-lease guards. A batch is explicitly not atomic.
pub fn apply(
    data: &Path,
    policy: &RetentionSettings,
    expected_revision: &str,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    apply_checked(data, policy, expected_revision, cancel, || Ok(()))
}

pub(crate) fn apply_checked(
    data: &Path,
    policy: &RetentionSettings,
    expected_revision: &str,
    cancel: Option<&CancellationToken>,
    verify: impl Fn() -> Result<()>,
) -> Result<Value> {
    apply_at(
        data,
        policy,
        expected_revision,
        cancel,
        current_day()?,
        verify,
    )
}

fn apply_at(
    data: &Path,
    policy: &RetentionSettings,
    expected_revision: &str,
    cancel: Option<&CancellationToken>,
    day: u64,
    verify: impl Fn() -> Result<()>,
) -> Result<Value> {
    policy.validate()?;
    if expected_revision.len() != 64
        || !expected_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Pruning requires the reviewed SHA-256 revision",
        ));
    }
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    let data = crate::store::data_directory(Some(data.into()))?;
    let _lease = if policy.has_limits() && directory(&data)? {
        Some(storage::exclusive_lease(
            &data.join("conversations-maintenance.lock"),
        )?)
    } else {
        None
    };
    let plan = plan_at(&data, policy, day, &budget)?;
    if expected_revision != plan.revision()? {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Retention policy or saved data changed; review pruning again",
        ));
    }
    let mut deleted = Vec::new();
    let mut skipped = Vec::new();
    let mut reclaimed = 0u64;
    verify().map_err(|error|error.details(json!({"completed_result":{"deleted":deleted,"skipped":skipped,"reclaimed_bytes":reclaimed},"remaining_requires_review":true})))?;
    let mut selected = plan
        .entries
        .iter()
        .filter(|entry| entry.selected)
        .collect::<Vec<_>>();
    selected.sort_by_key(|entry| {
        (
            entry.last_write_unix_seconds,
            entry.document_key.clone(),
            entry.session.clone(),
        )
    });
    for entry in selected {
        if let Err(error) = budget.check() {
            return Err(error.details(json!({"completed_result":{"deleted":deleted,"skipped":skipped,"reclaimed_bytes":reclaimed},"remaining_requires_review":true})));
        }
        let operation = Store::for_saved_conversation(
            entry.document.clone().expect("validated document"),
            data.clone(),
        )
        .and_then(|store| {
            deletion::delete_verified(
                &store,
                entry.session.as_deref().expect("validated session"),
                entry.revision.as_deref().expect("reviewed revision"),
                &budget,
                || {
                    verify().map_err(|error| {
                        let cause = error.details.clone();
                        error.details(json!({"retention_policy_changed":true,"cause":cause}))
                    })
                },
            )
        });
        match operation {
            Ok(result)=>{if result["cleanup_complete"]==true {reclaimed+=entry.bytes.unwrap_or(0);}deleted.push(result);},
            Err(error) if error.code=="CANCELLED" || error.details.as_ref().is_some_and(|details|details["retention_policy_changed"]==true)=>return Err(error.details(json!({"completed_result":{"deleted":deleted,"skipped":skipped,"reclaimed_bytes":reclaimed},"remaining_requires_review":true}))),
            Err(error)=>skipped.push(json!({"document":entry.document,"session":entry.session,"error":error})),
        }
    }
    budget.check().and_then(|()|verify()).map_err(|error|error.details(json!({"completed_result":{"deleted":deleted,"skipped":skipped,"reclaimed_bytes":reclaimed},"remaining_requires_review":true})))?;
    Ok(
        json!({"scope":plan.scope,"data_directory":data,"policy":policy,"reviewed_revision":expected_revision,"applied":true,"deleted":deleted,"skipped":skipped,"reclaimed_bytes":reclaimed,"remaining_known_bytes_estimate":plan.total_known_bytes.saturating_sub(reclaimed),"reviewed_bytes_exact":plan.total_bytes_exact,"remaining_over_limit_estimate":policy.max_total_bytes.and_then(|limit|if plan.total_known_bytes.saturating_sub(reclaimed)>limit {Some(true)} else if plan.total_bytes_exact {Some(false)} else {None}),"concurrent_changes_possible":true}),
    )
}

/// Recover committed deletion journals even after their original .pen moved.
/// Unknown, foreign and busy records remain protected and are reported.
pub fn cleanup_all(data: &Path, cancel: Option<&CancellationToken>) -> Result<Value> {
    use std::io::Read;
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    let data = crate::store::data_directory(Some(data.into()))?;
    let mut reports = Vec::new();
    let mut removed = 0u64;
    let mut retained = 0u64;
    if !directory(&data)? {
        return Ok(
            json!({"scope":"all_local_committed_deletions","data_directory":data,"documents":reports,"removed_count":removed,"retained_count":retained}),
        );
    }
    let _lease = storage::exclusive_lease(&data.join("conversations-maintenance.lock"))?;
    let result = (|| {
        for doc_root in children(&data.join("documents"), &budget)? {
            budget.check()?;
            if reports.len() >= MAX_RECORDS {
                return Err(Error::new(
                    "AGENT_ERROR",
                    "Global cleanup exceeds 10,000 document reports",
                ));
            }
            let paths = match children(&doc_root, &budget) {
                Ok(paths) => paths,
                Err(error) => {
                    reports.push(json!({"namespace":doc_root,"error":error}));
                    retained += 1;
                    continue;
                }
            };
            let tombstones = paths
                .into_iter()
                .filter(|path| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .starts_with("conversation-trash-")
                })
                .collect::<Vec<_>>();
            if tombstones.is_empty() {
                continue;
            }
            if reports.len() >= MAX_RECORDS {
                return Err(Error::new(
                    "AGENT_ERROR",
                    "Global cleanup exceeds 10,000 document reports",
                ));
            }
            let mut store = None;
            for path in &tombstones {
                budget.check()?;
                if !directory(path).is_ok_and(|exists| exists) {
                    continue;
                }
                let inspected = (|| {
                    let journal = path.join("deletion.json");
                    let mut bytes = Vec::new();
                    crate::store::open_regular_file(&journal)?
                        .take(16 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|error| Error::io(&journal, error))?;
                    if bytes.len() > 16 * 1024 {
                        return Err(Error::new("AGENT_ERROR", "Deletion journal exceeds 16 KiB"));
                    }
                    let value: Value = serde_json::from_slice(&bytes)
                        .map_err(|_| Error::new("AGENT_ERROR", "Invalid deletion journal"))?;
                    let document = value["document"].as_str().ok_or_else(|| {
                        Error::new("AGENT_ERROR", "Missing deletion document identity")
                    })?;
                    let saved = Store::for_saved_conversation(document.into(), data.clone())?;
                    if saved.document_data_dir() != doc_root {
                        return Err(Error::new("INVALID_ARGUMENT", "Foreign deletion journal"));
                    }
                    Ok(saved)
                })();
                if let Ok(saved) = inspected {
                    store = Some(saved);
                    break;
                }
            }
            if let Some(store) = store {
                match deletion::cleanup_with_budget(&store, &budget) {
                    Ok(report) => {
                        removed += report["removed_count"].as_u64().unwrap_or(0);
                        retained += report["retained_count"].as_u64().unwrap_or(0);
                        reports.push(report);
                    }
                    Err(error) => {
                        let cause = error.details.clone();
                        return Err(error.details(json!({"completed_result":{"documents":reports,"removed_count":removed,"retained_count":retained},"incomplete_document_result":cause})));
                    }
                }
            } else {
                retained += tombstones.len() as u64;
                reports.push(json!({"namespace":doc_root,"retained_count":tombstones.len(),"reason":"No valid document-bound deletion journal; local data was retained"}));
            }
        }
        budget.check()
    })();
    if let Err(error) = result {
        if error
            .details
            .as_ref()
            .is_some_and(|details| details.get("completed_result").is_some())
        {
            return Err(error);
        }
        return Err(error.details(json!({"completed_result":{"documents":reports,"removed_count":removed,"retained_count":retained}})));
    }
    Ok(
        json!({"scope":"all_local_committed_deletions","data_directory":data,"documents":reports,"removed_count":removed,"retained_count":retained}),
    )
}

#[cfg(test)]
mod tests;
