use super::*;

fn fixture() -> (tempfile::TempDir, Store) {
    let temporary = tempfile::tempdir().unwrap();
    let store = Store::init(
        temporary.path().join("design.pen"),
        Some(temporary.path().join("data")),
    )
    .unwrap();
    for name in ["one", "two"] {
        let revision = store.read().unwrap().revision();
        store
            .transact(name, &revision, name, |document| {
                let mut after = document.clone();
                after.set_property("", "name", &json!(name))?;
                Ok((after, json!({})))
            })
            .unwrap();
    }
    (temporary, store)
}

fn commit_marker(store: &Store) -> Journal {
    let index = Index::load(store, None).unwrap();
    let journal = Journal {
        version: 1,
        document: store.path().into(),
        generation: index.generation,
        floor: 2,
        records: vec![Index::record(store, 1, None).unwrap()],
    };
    write_json(&directory(store).join("prune.json"), &journal).unwrap();
    journal
}

#[test]
fn cancellation_after_the_retention_commit_reports_committed_state_and_recovers() {
    let (_temporary, store) = fixture();
    let source = fs::read(store.path()).unwrap();
    let review = run(
        &store,
        &PruneRequest {
            keep: 1,
            expected_revision: None,
        },
        None,
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let result = run_with_commit(
        &store,
        &PruneRequest {
            keep: 1,
            expected_revision: review["revision"].as_str(),
        },
        Some(&cancel),
        || cancel.cancel(),
    )
    .unwrap();
    assert_eq!(result["applied"], true);
    assert_eq!(result["cleanup_complete"], false);
    assert_eq!(result["errors"][0]["code"], "CANCELLED");
    assert!(directory(&store).join("prune.json").is_file());
    assert_eq!(store.history(10, None).unwrap().len(), 1);
    assert!(!directory(&store).join("prune.json").exists());
    assert_eq!(fs::read(store.path()).unwrap(), source);
    assert_eq!(
        store
            .replay_result_with_cancellation("one", "one", None)
            .unwrap_err()
            .code,
        "REQUEST_EXPIRED"
    );
}

#[test]
fn cleanup_resumes_after_snapshot_and_metadata_files_have_already_been_removed() {
    let (_temporary, store) = fixture();
    let journal = commit_marker(&store);
    let id = &journal.records[0].checkpoint.id;
    let trash = directory(&store).join("trash");
    create_private_directory(&trash).unwrap();
    let payload = trash.join(id);
    fs::rename(store.history_directory().join(id), &payload).unwrap();
    fs::remove_file(payload.join("before.pen")).unwrap();
    fs::remove_file(payload.join("metadata.json")).unwrap();
    store.recover().unwrap();
    assert!(!payload.exists());
    assert!(!directory(&store).join("prune.json").exists());
    assert_eq!(store.history(10, None).unwrap().len(), 1);
    assert_eq!(store.read().unwrap().value()["name"], "two");
}

#[test]
fn unknown_cleanup_files_preserve_the_journal_and_expired_request_reservations() {
    let (_temporary, store) = fixture();
    let journal = commit_marker(&store);
    let source = store
        .history_directory()
        .join(&journal.records[0].checkpoint.id);
    let extra = source.join("user-notes.txt");
    fs::write(&extra, "preserve unknown data").unwrap();
    assert_eq!(store.recover().unwrap_err().code, "RECOVERY_REQUIRED");
    assert_eq!(fs::read_to_string(&extra).unwrap(), "preserve unknown data");
    assert!(directory(&store).join("prune.json").exists());
    assert_eq!(
        Index::load(&store, None)
            .unwrap()
            .replay(&store, "one", "one", None)
            .unwrap_err()
            .code,
        "REQUEST_EXPIRED"
    );
    fs::remove_file(extra).unwrap();
    store.recover().unwrap();
    assert!(!source.exists());
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn cleanup_rejects_symbolic_payloads_without_touching_their_targets() {
    let (temporary, store) = fixture();
    let journal = commit_marker(&store);
    let source = store
        .history_directory()
        .join(&journal.records[0].checkpoint.id);
    let original = temporary.path().join("original");
    fs::rename(&source, &original).unwrap();
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("before.pen"), "must survive").unwrap();
    std::os::unix::fs::symlink(&outside, &source).unwrap();
    assert_eq!(store.recover().unwrap_err().code, "RECOVERY_REQUIRED");
    assert_eq!(
        fs::read_to_string(outside.join("before.pen")).unwrap(),
        "must survive"
    );
    fs::remove_file(&source).unwrap();
    fs::rename(&original, &source).unwrap();
    store.recover().unwrap();
    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(outside.join("before.pen")).unwrap(),
        "must survive"
    );
}
