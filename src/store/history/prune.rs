//! Reviewed retention; a durable journal reserves expired request IDs before cleanup.
use super::*;
use sha2::{Digest, Sha256};

pub struct PruneRequest<'a> {
    pub keep: usize,
    pub expected_revision: Option<&'a str>,
}

#[derive(Serialize)]
struct Candidate {
    record: Record,
    metadata_hash: String,
    bytes: u64,
}

#[derive(Serialize)]
struct Plan<'a> {
    version: u32,
    document: &'a Path,
    data_directory: &'a Path,
    document_revision: &'a str,
    index: &'a Index,
    keep: usize,
    retained: &'a std::collections::BTreeSet<String>,
    candidates: &'a [Candidate],
    unpublished: &'a [String],
}

fn plan_hash(plan: &Plan<'_>) -> Result<String> {
    struct Writer(Sha256);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = std::io::BufWriter::with_capacity(64 * 1024, Writer(Sha256::new()));
    serde_json::to_writer(&mut writer, plan)
        .map_err(|_| corrupt("Cannot encode checkpoint retention plan"))?;
    writer
        .flush()
        .map_err(|_| corrupt("Cannot hash checkpoint retention plan"))?;
    Ok(crate::document::digest_hex(
        writer
            .into_inner()
            .map_err(|_| corrupt("Cannot finish checkpoint retention hash"))?
            .0
            .finalize(),
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    document: PathBuf,
    generation: u64,
    floor: u64,
    records: Vec<Record>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    document: PathBuf,
    journal_hash: String,
    expired: usize,
    removed: usize,
}

fn check(cancel: Cancel<'_>, started: Instant) -> Result<()> {
    crate::cancel::check(cancel)?;
    if started.elapsed() >= Duration::from_secs(30) {
        return Err(Error::new(
            "RESOURCE_LIMIT",
            "Checkpoint retention exceeded 30 seconds; retain more checkpoints and prune in smaller batches",
        ));
    }
    Ok(())
}

fn changed() -> Error {
    Error::new(
        "REVISION_CONFLICT",
        "Checkpoint retention inputs changed; review history --prune again",
    )
}

fn private_payload(path: &Path) -> Result<()> {
    require_directory(path)?;
    for child in fs::read_dir(path).map_err(|error| Error::io(path, error))? {
        let child = child.map_err(|error| Error::io(path, error))?;
        if !matches!(
            child.file_name().to_str(),
            Some("before.pen" | "after.pen" | "metadata.json")
        ) || !child
            .file_type()
            .map_err(|error| Error::io(&child.path(), error))?
            .is_file()
        {
            return Err(corrupt(
                "Checkpoint cleanup encountered an unknown or non-regular file",
            ));
        }
    }
    Ok(())
}

pub(in crate::store) fn run(
    store: &Store,
    request: &PruneRequest<'_>,
    cancel: Cancel<'_>,
) -> Result<Value> {
    run_with_commit(store, request, cancel, || {})
}

fn run_with_commit(
    store: &Store,
    request: &PruneRequest<'_>,
    cancel: Cancel<'_>,
    committed: impl FnOnce(),
) -> Result<Value> {
    if request.keep == 0 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Checkpoint retention must keep at least one committed checkpoint",
        ));
    }
    let started = Instant::now();
    check(cancel, started)?;
    let _lock = store.lock_mode_cancellable(false, cancel)?;
    super::recover(store, cancel)?;
    let index = Index::load(store, cancel)?;
    let document_revision = store.read()?.revision();
    let mut retained = std::collections::BTreeSet::new();
    let mut next = index.tail.clone();
    let mut floor = index.floor;
    while let Some(pointer) = next {
        check(cancel, started)?;
        if retained.len() >= request.keep || pointer.sequence < index.floor {
            break;
        }
        let record = Index::record(store, pointer.sequence, cancel)?;
        if record.checkpoint != pointer
            || record.state != TransactionState::Committed
            || record.pruned
        {
            return Err(corrupt("Invalid checkpoint retention chain"));
        }
        floor = pointer.sequence;
        retained.insert(pointer.id);
        next = record.previous;
    }
    let mut candidates = Vec::new();
    let mut unpublished = Vec::new();
    let mut remaining_record_bytes = 64 * 1024 * 1024;
    let history = store.history_directory();
    if exists(&history)? {
        for child in fs::read_dir(&history).map_err(|error| Error::io(&history, error))? {
            check(cancel, started)?;
            let child = child.map_err(|error| Error::io(&history, error))?;
            let id = child.file_name().to_string_lossy().into_owned();
            if retained.contains(&id) {
                continue;
            }
            if !child
                .file_type()
                .map_err(|error| Error::io(&child.path(), error))?
                .is_dir()
                || !valid_checkpoint_id(&id)
                || !exists(&child.path().join("metadata.json"))?
            {
                unpublished.push(id);
                if unpublished.len() > 100_000 {
                    return Err(Error::new(
                        "RESOURCE_LIMIT",
                        "Too many unpublished history entries to review",
                    ));
                }
                continue;
            }
            let sequence = id[..20]
                .parse()
                .map_err(|_| corrupt("Invalid checkpoint sequence"))?;
            let pointer = Pointer { id, sequence };
            let record = Index::record(store, sequence, cancel)?;
            if record.checkpoint != pointer
                || record.pruned
                || record.state == TransactionState::Prepared
            {
                return Err(corrupt("Checkpoint retention index mismatch"));
            }
            crate::query::check_response_size(
                &record,
                remaining_record_bytes,
                cancel,
                "Checkpoint retention records exceed 64 MiB; retain more checkpoints for this batch",
            )?;
            remaining_record_bytes -= crate::query::serialized_size(&record);
            let entry = read_checkpoint(store, &pointer, cancel)?;
            if entry.state != record.state
                || entry.request_id != record.request_id
                || entry.fingerprint != record.fingerprint
            {
                return Err(corrupt("Checkpoint retention metadata mismatch"));
            }
            private_payload(&child.path())?;
            let mut bytes = 0u64;
            for (name, expected) in [
                ("before.pen", entry.before_revision),
                ("after.pen", entry.after_revision),
            ] {
                check(cancel, started)?;
                let path = child.path().join(name);
                if content::file_hash(&path, MAX_DOCUMENT_BYTES, cancel)? != expected {
                    return Err(corrupt(
                        "Cannot prune a checkpoint with corrupt snapshot content",
                    ));
                }
                bytes = bytes.saturating_add(
                    fs::metadata(&path)
                        .map_err(|error| Error::io(&path, error))?
                        .len(),
                );
            }
            let metadata = child.path().join("metadata.json");
            let metadata_hash = content::file_hash(&metadata, MAX_DOCUMENT_BYTES, cancel)?;
            bytes = bytes.saturating_add(
                fs::metadata(&metadata)
                    .map_err(|error| Error::io(&metadata, error))?
                    .len(),
            );
            candidates.push(Candidate {
                record,
                metadata_hash,
                bytes,
            });
            if candidates.len() > 100_000 {
                return Err(Error::new(
                    "RESOURCE_LIMIT",
                    "Checkpoint retention is limited to 100000 removals; retain more checkpoints for this batch",
                ));
            }
        }
    }
    candidates.sort_by_key(|candidate| candidate.record.checkpoint.sequence);
    unpublished.sort();
    check(cancel, started)?;
    require_revision(&store.read()?, &document_revision)?;
    let revision = plan_hash(&Plan {
        version: 1,
        document: store.path(),
        data_directory: store.data_dir(),
        document_revision: &document_revision,
        index: &index,
        keep: request.keep,
        retained: &retained,
        candidates: &candidates,
        unpublished: &unpublished,
    })?;
    if request
        .expected_revision
        .is_some_and(|expected| expected != revision)
    {
        return Err(changed());
    }
    let mut result = json!({"applied":false,"cleanup_complete":true,"revision":revision,
        "document_revision":document_revision,"keep":request.keep,"retained_count":retained.len(),
        "candidate_count":candidates.len(),"candidate_bytes":candidates.iter().map(|candidate|candidate.bytes).sum::<u64>(),
        "candidate_checkpoints":candidates.iter().take(200).map(|candidate|&candidate.record.checkpoint.id).collect::<Vec<_>>(),
        "candidates_truncated":candidates.len()>200,"pruned_count":0,
        "retained_unpublished_count":unpublished.len(),"retained_unpublished":unpublished.iter().take(200).collect::<Vec<_>>(),
        "retained_unpublished_truncated":unpublished.len()>200,"errors":[]});
    if request.expected_revision.is_none() {
        return Ok(result);
    }
    if candidates.is_empty() {
        result["applied"] = json!(true);
        return Ok(result);
    }
    let journal = Journal {
        version: 1,
        document: store.path().into(),
        generation: index.generation,
        floor,
        records: candidates
            .into_iter()
            .map(|candidate| candidate.record)
            .collect(),
    };
    crate::query::check_response_size(
        &journal,
        64 * 1024 * 1024,
        cancel,
        "Checkpoint retention journal exceeds 64 MiB; retain more checkpoints for this batch",
    )?;
    check(cancel, started)?;
    // Publishing this journal commits retention, including request-ID reservations.
    let journal_path = directory(store).join("prune.json");
    if let Err(error) = write_json(&journal_path, &journal) {
        if !exists(&journal_path)? {
            return Err(error);
        }
        result["applied"] = json!(true);
        result["cleanup_complete"] = json!(false);
        result["pruned_count"] = json!(journal.records.len());
        result["errors"] = json!([error]);
        return Ok(result);
    }
    result["applied"] = json!(true);
    result["pruned_count"] = json!(journal.records.len());
    committed();
    let mut index = index;
    if let Err(error) = recover(store, &mut index, cancel) {
        result["cleanup_complete"] = json!(false);
        result["errors"] = json!([error]);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod tests;

pub(super) fn recover(store: &Store, index: &mut Index, cancel: Cancel<'_>) -> Result<()> {
    let path = directory(store).join("prune.json");
    let progress_path = directory(store).join("prune-progress.json");
    if !exists(&path)? {
        if exists(&progress_path)? {
            let progress: Progress = read_json(&progress_path, 64 * 1024, cancel)?;
            if progress.document != store.path() {
                return Err(corrupt("Foreign checkpoint cleanup progress"));
            }
            fs::remove_file(&progress_path).map_err(|error| Error::io(&progress_path, error))?;
            sync_directory(&directory(store))?;
        }
        return Ok(());
    }
    let started = Instant::now();
    let journal: Journal = read_json(&path, 64 * 1024 * 1024, cancel)?;
    if journal.version != 1
        || journal.document != store.path()
        || journal.records.len() > 100_000
        || !(index.generation == journal.generation
            || journal.generation.checked_add(1) == Some(index.generation))
        || journal.floor > index.last_sequence
        || exists(&directory(store).join("pending.json"))?
    {
        return Err(corrupt("Invalid checkpoint retention journal"));
    }
    for record in &journal.records {
        check(cancel, started)?;
        record.validate()?;
        if record.state == TransactionState::Prepared
            || record.state == TransactionState::Committed
                && record.checkpoint.sequence >= journal.floor
        {
            return Err(corrupt(
                "Checkpoint retention journal would remove retained history",
            ));
        }
    }
    let journal_hash = content::file_hash(&path, 64 * 1024 * 1024, cancel)?;
    let mut progress = if exists(&progress_path)? {
        let progress: Progress = read_json(&progress_path, 64 * 1024, cancel)?;
        if progress.document != store.path()
            || progress.journal_hash != journal_hash
            || progress.expired > journal.records.len()
            || progress.removed > journal.records.len()
            || progress.removed > 0 && progress.expired != journal.records.len()
        {
            return Err(corrupt("Invalid checkpoint cleanup progress"));
        }
        progress
    } else {
        Progress {
            document: store.path().into(),
            journal_hash,
            expired: 0,
            removed: 0,
        }
    };
    // Expired results cannot be replayed even after an interrupted cleanup.
    for (position, record) in journal.records.iter().enumerate().skip(progress.expired) {
        check(cancel, started)?;
        let actual = Index::record(store, record.checkpoint.sequence, cancel)?;
        if actual.checkpoint != record.checkpoint
            || actual.request_id != record.request_id
            || actual.fingerprint != record.fingerprint
            || actual.state != record.state
        {
            return Err(corrupt("Checkpoint retention reservation changed"));
        }
        if !actual.pruned {
            let mut expired = record.clone();
            expired.pruned = true;
            write_json(
                &Index::record_path(store, expired.checkpoint.sequence),
                &expired,
            )?;
        }
        progress.expired = position + 1;
        write_json(&progress_path, &progress)?;
    }
    index.floor = journal.floor;
    index.generation = journal
        .generation
        .checked_add(1)
        .ok_or_else(|| corrupt("History generation is exhausted"))?;
    index.save(store)?;
    let trash = directory(store).join("trash");
    create_private_directory(&trash)?;
    require_directory(&trash)?;
    for (position, record) in journal.records.iter().enumerate().skip(progress.removed) {
        check(cancel, started)?;
        let source = store.history_directory().join(&record.checkpoint.id);
        let payload = trash.join(&record.checkpoint.id);
        if exists(&source)? {
            private_payload(&source)?;
            if exists(&payload)? {
                return Err(corrupt("Checkpoint retention has duplicate payloads"));
            }
            fs::rename(&source, &payload).map_err(|error| Error::io(&source, error))?;
            sync_directory(&store.history_directory())?;
            sync_directory(&trash)?;
        }
        if exists(&payload)? {
            private_payload(&payload)?;
            for name in ["before.pen", "after.pen", "metadata.json"] {
                check(cancel, started)?;
                let file = payload.join(name);
                match fs::remove_file(&file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::io(&file, error)),
                }
            }
            fs::remove_dir(&payload).map_err(|error| Error::io(&payload, error))?;
            sync_directory(&trash)?;
        }
        progress.removed = position + 1;
        write_json(&progress_path, &progress)?;
    }
    // Reconcile once after cleanup, including any older writer that appended
    // checkpoints while a previously committed cleanup was interrupted.
    index.namespace = None;
    index.save(store)?;
    fs::remove_file(&path).map_err(|error| Error::io(&path, error))?;
    sync_directory(&directory(store))?;
    fs::remove_file(&progress_path).map_err(|error| Error::io(&progress_path, error))?;
    sync_directory(&directory(store))
}
