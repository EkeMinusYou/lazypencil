//! Public history compatibility, bounded reads and reviewed retention contracts.
use super::*;
use lazypencil::store::PruneRequest;

fn prune(store: &Store, keep: usize, expected: Option<&str>) -> lazypencil::Result<Value> {
    store.prune_history(
        &PruneRequest {
            keep,
            expected_revision: expected,
        },
        None,
    )
}

#[test]
fn legacy_history_migrates_once_and_keeps_replay_pagination_and_restoration() {
    let (_temp, store) = setup();
    let first = edit(&store, "first", "one");
    let second = edit(&store, "second", "two");
    let third = edit(&store, "third", "three");
    fs::remove_dir_all(store.document_data_dir().join("history-index")).unwrap();
    fs::write(store.document_data_dir().join("history/README"), "keep me").unwrap();
    let reopened = Store::open(store.path(), Some(store.data_dir().into())).unwrap();
    let latest = reopened.history(1, None).unwrap();
    assert_eq!(latest[0].id, third["checkpoint_id"]);
    let previous = reopened.history(1, Some(latest[0].sequence)).unwrap();
    assert_eq!(previous[0].id, second["checkpoint_id"]);
    let oldest = reopened.history(10, Some(previous[0].sequence)).unwrap();
    assert_eq!(oldest.len(), 1);
    assert_eq!(oldest[0].id, first["checkpoint_id"]);
    let replay = reopened
        .replay_result_with_cancellation("first", &hash(b"one"), None)
        .unwrap()
        .unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["current_revision"], third["revision"]);
    reopened
        .restore(
            "restore-migrated",
            &reopened.read().unwrap().revision(),
            oldest[0].id.as_str(),
        )
        .unwrap();
    assert_eq!(reopened.read().unwrap().value()["name"], "one");
    assert_eq!(reopened.history(10, None).unwrap().len(), 4);
}

#[test]
fn indexed_pages_replays_and_new_saves_do_not_deserialize_unselected_old_results() {
    let (_temp, store) = setup();
    let oldest = edit(&store, "oldest", "oldest");
    let recent = edit(&store, "recent", "recent");
    let path = store
        .document_data_dir()
        .join("history")
        .join(oldest["checkpoint_id"].as_str().unwrap())
        .join("metadata.json");
    fs::write(&path, "damaged old metadata").unwrap();
    // Reopening must use the disk index, rather than a process-local cache or a
    // complete history scan. Explicit access still diagnoses the damaged row.
    let reopened = Store::open(store.path(), Some(store.data_dir().into())).unwrap();
    assert_eq!(
        reopened.history(1, None).unwrap()[0].id,
        recent["checkpoint_id"]
    );
    assert!(
        reopened
            .replay_result_with_cancellation("recent", &hash(b"recent"), None)
            .unwrap()
            .is_some()
    );
    edit(&reopened, "new", "new");
    assert_eq!(
        reopened.history(1, Some(2)).unwrap_err().code,
        "RECOVERY_REQUIRED"
    );
}

#[test]
fn older_cooperating_writers_are_discovered_without_rebuilding_old_results() {
    let (_temp, store) = setup();
    edit(&store, "indexed", "indexed");
    let before = store.read().unwrap();
    let mut after = before.clone();
    after
        .set_property("", "name", &json!("legacy writer"))
        .unwrap();
    let id = format!("{:020}-{}", 2, uuid::Uuid::new_v4().simple());
    let path = store.document_data_dir().join("history").join(&id);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("before.pen"), before.text()).unwrap();
    fs::write(path.join("after.pen"), after.text()).unwrap();
    let entry = lazypencil::store::HistoryEntry {
        id: id.clone(),
        sequence: 2,
        created_at: "2026-11-08T00:00:00Z".into(),
        request_id: "legacy".into(),
        fingerprint: hash(b"legacy"),
        state: TransactionState::Committed,
        before_revision: before.revision(),
        after_revision: after.revision(),
        result: json!({"revision":after.revision()}),
        document_path: Some(store.path().into()),
    };
    fs::write(
        path.join("metadata.json"),
        serde_json::to_vec(&entry).unwrap(),
    )
    .unwrap();
    fs::write(store.path(), after.text()).unwrap();
    assert_eq!(store.history(1, None).unwrap()[0].id, id);
    assert!(
        store
            .replay_result_with_cancellation("legacy", &hash(b"legacy"), None)
            .unwrap()
            .is_some()
    );
    edit(&store, "following", "following");
    assert_eq!(store.history(1, None).unwrap()[0].sequence, 3);
}

#[test]
fn reviewed_pruning_keeps_newest_checkpoints_and_expired_requests_cannot_run_again() {
    let (_temp, store) = setup();
    let before = store.read().unwrap();
    // A no-op still has the current revision after pruning. Revision conflicts
    // alone cannot prevent its duplicate execution; the reservation must survive.
    let no_op = store
        .transact("no-op", &before.revision(), "same", |document| {
            Ok((document.clone(), json!({})))
        })
        .unwrap();
    let middle = edit(&store, "middle", "middle");
    let newest = edit(&store, "newest", "newest");
    let bytes = fs::read(store.path()).unwrap();
    let review = prune(&store, 1, None).unwrap();
    assert_eq!(review["applied"], false);
    assert_eq!(review["candidate_count"], 2);
    assert_eq!(store.history(10, None).unwrap().len(), 3);
    let applied = prune(&store, 1, review["revision"].as_str()).unwrap();
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["cleanup_complete"], true);
    assert_eq!(applied["pruned_count"], 2);
    assert_eq!(fs::read(store.path()).unwrap(), bytes);
    assert_eq!(
        store.history(10, None).unwrap()[0].id,
        newest["checkpoint_id"]
    );
    assert!(store.history(10, Some(3)).unwrap().is_empty());
    for id in [
        no_op["checkpoint_id"].as_str().unwrap(),
        middle["checkpoint_id"].as_str().unwrap(),
    ] {
        assert!(!store.document_data_dir().join("history").join(id).exists());
        assert_eq!(
            store
                .restore("pruned-restore", &store.read().unwrap().revision(), id)
                .unwrap_err()
                .code,
            "SNAPSHOT_UNAVAILABLE"
        );
    }
    assert_eq!(
        store
            .transact(
                "no-op",
                &store.read().unwrap().revision(),
                "same",
                |_| panic!("Expired requests must never execute again")
            )
            .unwrap_err()
            .code,
        "REQUEST_EXPIRED"
    );
    assert_eq!(
        store
            .transact(
                "no-op",
                &store.read().unwrap().revision(),
                "different",
                |_| panic!("Conflicting IDs must never execute")
            )
            .unwrap_err()
            .code,
        "REQUEST_ID_CONFLICT"
    );
    let replay = store
        .replay_result_with_cancellation("newest", &hash(b"newest"), None)
        .unwrap()
        .unwrap();
    assert_eq!(replay["checkpoint_id"], newest["checkpoint_id"]);
    store
        .restore(
            "restore-kept",
            &store.read().unwrap().revision(),
            newest["checkpoint_id"].as_str().unwrap(),
        )
        .unwrap();
    assert_eq!(store.history(10, None).unwrap().len(), 2);
}

#[test]
fn pruning_rejects_stale_reviews_and_same_length_metadata_changes() {
    let (_temp, store) = setup();
    let first = edit(&store, "one", "one");
    edit(&store, "two", "two");
    let old = prune(&store, 1, None).unwrap();
    edit(&store, "three", "three");
    assert_eq!(
        prune(&store, 1, old["revision"].as_str()).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let review = prune(&store, 1, None).unwrap();
    let path = store
        .document_data_dir()
        .join("history")
        .join(first["checkpoint_id"].as_str().unwrap())
        .join("metadata.json");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let changed = original.replace("\"changed\": [", "\"Changed\": [");
    assert_ne!(original, changed);
    assert_eq!(original.len(), changed.len());
    fs::write(&path, changed).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        prune(&store, 1, review["revision"].as_str())
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(store.history(10, None).unwrap().len(), 3);
}

#[test]
fn cancelled_retention_never_commits_a_reviewed_plan() {
    let (_temp, store) = setup();
    edit(&store, "one", "one");
    edit(&store, "two", "two");
    let review = prune(&store, 1, None).unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        store
            .prune_history(
                &PruneRequest {
                    keep: 1,
                    expected_revision: review["revision"].as_str()
                },
                Some(&cancel)
            )
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(store.history(10, None).unwrap().len(), 2);
    assert!(
        !store
            .document_data_dir()
            .join("history-index/prune.json")
            .exists()
    );
    assert_eq!(prune(&store, 0, None).unwrap_err().code, "INVALID_ARGUMENT");
}

#[test]
fn indexed_cursors_skip_aborted_entries_and_keep_arbitrary_numeric_cutoffs() {
    let (_temp, store) = setup();
    let first = edit(&store, "first", "first");
    let revision = store.read().unwrap().revision();
    store
        .transact_guarded(
            "aborted",
            &revision,
            "aborted",
            |document| Ok((document.clone(), json!({}))),
            || Err(Error::new("CANCELLED", "test cancellation")),
        )
        .unwrap_err();
    let third = edit(&store, "third", "third");
    assert_eq!(
        store.history(10, Some(2)).unwrap()[0].id,
        first["checkpoint_id"]
    );
    assert_eq!(
        store.history(10, Some(3)).unwrap()[0].id,
        first["checkpoint_id"]
    );
    assert_eq!(
        store.history(1, Some(100)).unwrap()[0].id,
        third["checkpoint_id"]
    );
    assert!(store.history(10, Some(1)).unwrap().is_empty());
    assert!(store.history(10, Some(0)).unwrap().is_empty());
    assert!(store.history(0, None).unwrap().is_empty());
    let review = prune(&store, 1, None).unwrap();
    assert_eq!(review["candidate_count"], 2);
    prune(&store, 1, review["revision"].as_str()).unwrap();
    store
        .transact(
            "aborted",
            &store.read().unwrap().revision(),
            "aborted",
            |document| Ok((document.clone(), json!({}))),
        )
        .unwrap();
    assert_eq!(store.history(10, None).unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn consecutive_snapshots_share_history_storage_without_linking_the_live_document() {
    use std::os::unix::fs::MetadataExt;
    let (_temp, store) = setup();
    let first = edit(&store, "one", "one");
    let second = edit(&store, "two", "two");
    let root = store.document_data_dir().join("history");
    let first_after = root
        .join(first["checkpoint_id"].as_str().unwrap())
        .join("after.pen");
    let second_before = root
        .join(second["checkpoint_id"].as_str().unwrap())
        .join("before.pen");
    assert_eq!(
        fs::metadata(&first_after).unwrap().ino(),
        fs::metadata(&second_before).unwrap().ino()
    );
    assert_ne!(
        fs::metadata(&first_after).unwrap().ino(),
        fs::metadata(store.path()).unwrap().ino()
    );
    let retained = fs::read(&second_before).unwrap();
    let review = prune(&store, 1, None).unwrap();
    prune(&store, 1, review["revision"].as_str()).unwrap();
    assert!(!first_after.exists());
    assert_eq!(fs::read(second_before).unwrap(), retained);
}

#[test]
fn byte_bounded_history_pages_keep_a_cursor_and_return_every_large_result_once() {
    let (_temp, store) = setup();
    for index in 0..4 {
        let before = store.read().unwrap();
        store
            .transact(
                &format!("large-{index}"),
                &before.revision(),
                "large",
                |document| {
                    let mut after = document.clone();
                    after.set_property("", "name", &json!(index))?;
                    Ok((after, json!({"payload":"x".repeat(2 * 1024 * 1024)})))
                },
            )
            .unwrap();
    }
    let mut before = None;
    let mut sequences = Vec::new();
    loop {
        let page = store
            .history_page_with_cancellation(1000, before, None)
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(serde_json::to_vec(&page.entries).unwrap().len() < 4 * 1024 * 1024);
        sequences.extend(page.entries.iter().map(|entry| entry.sequence));
        before = page.next_before_sequence;
        if before.is_none() {
            break;
        }
    }
    assert_eq!(sequences, [4, 3, 2, 1]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(store.data_dir())
        .arg("history")
        .arg(store.path())
        .args(["--limit", "1000"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.len() < 4 * 1024 * 1024);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(result["next_before_sequence"], 4);
}

#[test]
fn retention_preserves_unpublished_preparations_without_blocking_known_checkpoints() {
    let (_temporary, store) = setup();
    edit(&store, "one", "one");
    edit(&store, "two", "two");
    let orphan = store.document_data_dir().join("history").join(format!(
        "{:020}-{}",
        3,
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("before.pen"), "unpublished preparation").unwrap();
    let review = prune(&store, 1, None).unwrap();
    assert_eq!(review["candidate_count"], 1);
    assert_eq!(review["retained_unpublished_count"], 1);
    let applied = prune(&store, 1, review["revision"].as_str()).unwrap();
    assert_eq!(applied["cleanup_complete"], true);
    assert_eq!(
        fs::read_to_string(orphan.join("before.pen")).unwrap(),
        "unpublished preparation"
    );
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn cli_reviews_and_commits_retention() {
    use std::process::Command;
    let (_temp, store) = setup();
    edit(&store, "one", "one");
    edit(&store, "two", "two");
    let run = |expected: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        command
            .args(["--json", "--data-dir"])
            .arg(store.data_dir())
            .arg("history")
            .arg(store.path())
            .args(["--prune", "--keep", "1"]);
        if let Some(revision) = expected {
            command.args(["--expected-revision", revision]);
        }
        command.output().unwrap()
    };
    let review = run(None);
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stdout)
    );
    let review: Value = serde_json::from_slice(&review.stdout).unwrap();
    assert_eq!(review["candidate_count"], 1);
    assert_eq!(store.history(10, None).unwrap().len(), 2);
    let applied = run(review["revision"].as_str());
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stdout)
    );
    let applied: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["cleanup_complete"], true);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}
