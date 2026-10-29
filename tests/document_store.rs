use lazypencil::{
    Error,
    document::{Document, hash},
    store::{Store, TransactionState},
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, Barrier},
};
use tempfile::TempDir;

fn fixture() -> String {
    "{\r\n  // 日本語のコメント\r\n  \"version\": \"2.21\",\r\n  \"future\": {\"number\": 1.000e+3},\r\n  \"children\": [\r\n    {\"id\": \"a\", \"type\": \"rectangle\", \"width\": 100, \"unknown\": 42},\r\n    // untouched\r\n    {\"id\": \"b\", \"type\": \"future_shape\", \"magic\": [1, 2, 3]},\r\n  ]\r\n}\r\n".into()
}

fn setup() -> (TempDir, Store) {
    let directory = TempDir::new().unwrap();
    let store = Store::init(
        directory.path().join("design.pen"),
        Some(directory.path().join("data")),
    )
    .unwrap();
    (directory, store)
}

#[test]
fn different_history_directories_cannot_bypass_the_document_writer_lock() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("design.pen");
    let first = Store::init(&path, Some(temp.path().join("first-history"))).unwrap();
    let second = Store::open(&path, Some(temp.path().join("second-history"))).unwrap();
    assert_eq!(first.lock_file_path(), second.lock_file_path());
    let revision = first.read().unwrap().revision();
    let barrier = Arc::new(Barrier::new(2));
    let threads = [first.clone(), second.clone()]
        .into_iter()
        .enumerate()
        .map(|(i, store)| {
            let barrier = barrier.clone();
            let revision = revision.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.transact(
                    &format!("writer-{i}"),
                    &revision,
                    &format!("payload-{i}"),
                    |doc| {
                        let mut after = doc.clone();
                        after.set_property("", "writer", &json!(i))?;
                        Ok((after, json!({})))
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().is_err_and(|e| e.code == "REVISION_CONFLICT"))
            .count(),
        1
    );
    assert_eq!(
        first.history(10, None).unwrap().len() + second.history(10, None).unwrap().len(),
        1
    );
}

fn edit(store: &Store, id: &str, name: &str) -> Value {
    let before = store.read().unwrap();
    store
        .transact(id, &before.revision(), &hash(name.as_bytes()), |doc| {
            let mut after = doc.clone();
            after.set_property("", "name", &json!(name))?;
            Ok((after, json!({"changed": ["name"]})))
        })
        .unwrap()
}

#[test]
fn preserves_exact_source_and_only_changes_the_requested_value() {
    let text = fixture();
    let mut doc = Document::parse(text.clone()).unwrap();
    assert_eq!(doc.text(), text);
    let pointer = doc.location("a").unwrap().pointer.clone();
    doc.set_property(&pointer, "width", &json!(125)).unwrap();
    assert_eq!(doc.text(), text.replace("\"width\": 100", "\"width\": 125"));
    assert_eq!(doc.node("b").unwrap()["magic"], json!([1, 2, 3]));
    assert_eq!(doc.diagnostics()[0].code, "UNSUPPORTED_FEATURE");
    assert_eq!(doc.known_node("b").unwrap_err().code, "UNSUPPORTED_FEATURE");
}

#[test]
fn structural_changes_keep_comments_unknown_fields_and_numeric_spelling() {
    let mut doc = Document::parse(fixture()).unwrap();
    doc.insert_element(
        "/children",
        1,
        r#"{"id":"new","type":"text","content":"こんにちは"}"#,
    )
    .unwrap();
    doc.set_property("", "metadata", &json!({"custom": true}))
        .unwrap();
    let location = doc.location("new").unwrap().pointer.clone();
    doc.remove_element(&location).unwrap();
    doc.remove_property("", "metadata").unwrap();
    assert!(doc.text().contains("// 日本語のコメント"));
    assert!(doc.text().contains("// untouched"));
    assert!(doc.text().contains("1.000e+3"));
    assert_eq!(doc.node_count(), 2);
    assert_eq!(doc.value()["future"]["number"], json!(1000.0));
}

#[test]
fn array_edits_work_at_every_position_and_with_trailing_comments() {
    for children in [
        "[]",
        "[\n]",
        "[ /* retained */ ]",
        "[\n  // retained\n]",
        "[\n  /* retained */\n]",
    ] {
        let mut doc =
            Document::parse(format!(r#"{{"version":"2.21","children":{children}}}"#)).unwrap();
        for (index, id) in [(0, "a"), (0, "b"), (1, "c"), (3, "d")] {
            doc.insert_element(
                "/children",
                index,
                &json!({"id":id,"type":"rectangle"}).to_string(),
            )
            .unwrap();
        }
        assert_eq!(
            doc.value()["children"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["b", "c", "a", "d"]
        );
        for id in ["b", "d", "c", "a"] {
            let pointer = doc.location(id).unwrap().pointer.clone();
            doc.remove_element(&pointer).unwrap();
        }
        assert_eq!(doc.node_count(), 0);
        if children.contains("retained") {
            assert!(doc.text().contains("retained"));
        }
    }
}

#[test]
fn rejects_ambiguous_or_non_jsonc_input_without_panicking() {
    for source in [
        r#"{"version":"2.21","version":"2.20","children":[]}"#,
        r#"{"version":"2.21","children":[{"id":"x","type":"text"},{"id":"x","type":"text"}]}"#,
        r#"{"version":"2.21",children:[]}"#,
        r#"{"version":"2.21","children":[],"bad":NaN}"#,
        r#"{"version":"2.21","children":[] "missing":true}"#,
    ] {
        assert_eq!(
            Document::parse(source.into()).unwrap_err().code,
            "INVALID_DOCUMENT"
        );
    }
    let deep = format!("{}0{}", "[".repeat(300), "]".repeat(300));
    assert_eq!(Document::parse(deep).unwrap_err().code, "INVALID_DOCUMENT");
}

#[test]
fn json_pointer_keys_and_multibyte_offsets_are_unambiguous() {
    let mut doc = Document::parse(
        r#"{"version":"2.21","note":"💡日本語","children":[],"a/b~c":{"x":1}}"#.into(),
    )
    .unwrap();
    doc.set_property("/a~1b~0c", "x", &json!(2)).unwrap();
    assert!(doc.text().contains("\"a/b~c\":{\"x\":2}"));
    assert!(doc.text().contains("💡日本語"));
}

#[test]
fn init_does_not_clobber_existing_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("design.pen");
    fs::write(&path, b"precious existing bytes").unwrap();
    assert!(Store::init(&path, Some(dir.path().join("data"))).is_err());
    assert_eq!(fs::read(path).unwrap(), b"precious existing bytes");
}

#[test]
fn stale_edits_and_failed_batches_do_not_change_the_document() {
    let (_dir, store) = setup();
    let stale = store.read().unwrap().revision();
    edit(&store, "first", "first");
    let saved = fs::read(store.path()).unwrap();
    let error = store
        .transact("second", &stale, "fingerprint", |_| {
            panic!("stale edits must not run")
        })
        .unwrap_err();
    assert_eq!(error.code, "REVISION_CONFLICT");
    let current = store.read().unwrap().revision();
    let error = store
        .transact("failed", &current, "fingerprint", |_| {
            Err(Error::new("INVALID_OPERATION", "failure"))
        })
        .unwrap_err();
    assert_eq!(error.code, "INVALID_OPERATION");
    assert_eq!(fs::read(store.path()).unwrap(), saved);
    assert_eq!(store.history(100, None).unwrap().len(), 1);
}

#[test]
fn replay_is_deduplicated_even_after_subsequent_edits() {
    let (_dir, store) = setup();
    let expected = store.read().unwrap().revision();
    let first = edit(&store, "once", "one");
    edit(&store, "later", "two");
    let current = store.read().unwrap().revision();
    let replay = store
        .transact("once", &expected, &hash(b"one"), |_| {
            panic!("duplicate edit must not run")
        })
        .unwrap();
    assert_eq!(replay["revision"], first["revision"]);
    assert_eq!(replay["current_revision"], current);
    assert_eq!(replay["replayed"], true);
    assert_eq!(store.read().unwrap().value()["name"], "two");
    assert_eq!(store.history(100, None).unwrap().len(), 2);
    assert_eq!(
        store
            .transact("once", &current, "different", |_| panic!())
            .unwrap_err()
            .code,
        "REQUEST_ID_CONFLICT"
    );
}

#[test]
fn concurrent_writers_with_the_same_revision_have_one_winner() {
    let (_dir, store) = setup();
    let expected = store.read().unwrap().revision();
    let barrier = Arc::new(Barrier::new(2));
    let threads = (0..2)
        .map(|index| {
            let (store, expected, barrier) = (store.clone(), expected.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                store.transact(
                    &format!("writer-{index}"),
                    &expected,
                    &format!("fp-{index}"),
                    |doc| {
                        let mut doc = doc.clone();
                        doc.set_property("", "writer", &json!(index))?;
                        Ok((doc, json!({})))
                    },
                )
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().is_err_and(|e| e.code == "REVISION_CONFLICT"))
            .count(),
        1
    );
    assert_eq!(store.history(100, None).unwrap().len(), 1);
}

fn mark_prepared(store: &Store, checkpoint: &str) -> std::path::PathBuf {
    let path = store.document_data_dir().join("history").join(checkpoint);
    let meta = path.join("metadata.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&meta).unwrap()).unwrap();
    value["state"] = json!("prepared");
    fs::write(meta, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

#[test]
fn recovery_recognizes_a_saved_document_with_an_uncommitted_history_record() {
    let (_dir, store) = setup();
    let result = edit(&store, "saved", "saved");
    let id = result["checkpoint_id"].as_str().unwrap();
    mark_prepared(&store, id);
    assert_eq!(store.recover().unwrap(), vec![id]);
    assert_eq!(
        store.history(10, None).unwrap()[0].state,
        TransactionState::Committed
    );
    assert_eq!(store.read().unwrap().value()["name"], "saved");
}

#[test]
fn recovery_aborts_an_unsaved_edit_and_allows_an_idempotent_retry() {
    let (_dir, store) = setup();
    let result = edit(&store, "retry", "saved");
    let dir = mark_prepared(&store, result["checkpoint_id"].as_str().unwrap());
    fs::copy(dir.join("before.pen"), store.path()).unwrap();
    store.recover().unwrap();
    edit(&store, "retry", "saved");
    let revision = store.read().unwrap().revision();
    assert_eq!(
        store
            .transact("retry", &revision, &hash(b"saved"), |_| panic!())
            .unwrap()["replayed"],
        true
    );
    assert_eq!(store.history(100, None).unwrap().len(), 1);
}

#[test]
fn recovery_never_overwrites_unrelated_external_changes() {
    let (_dir, store) = setup();
    let result = edit(&store, "prepared", "saved");
    mark_prepared(&store, result["checkpoint_id"].as_str().unwrap());
    let external = r#"{"version":"2.21","children":[],"name":"external"}"#;
    fs::write(store.path(), external).unwrap();
    assert_eq!(store.recover().unwrap_err().code, "RECOVERY_REQUIRED");
    assert_eq!(fs::read_to_string(store.path()).unwrap(), external);
}

#[test]
fn restoring_a_checkpoint_is_a_new_transaction_and_checks_its_content() {
    let (_dir, store) = setup();
    let first = edit(&store, "first", "one");
    edit(&store, "second", "two");
    let checkpoint = first["checkpoint_id"].as_str().unwrap();
    store
        .restore("restore", &store.read().unwrap().revision(), checkpoint)
        .unwrap();
    assert_eq!(store.read().unwrap().value()["name"], "one");
    assert_eq!(store.history(100, None).unwrap().len(), 3);
    let path = store
        .document_data_dir()
        .join("history")
        .join(checkpoint)
        .join("after.pen");
    fs::write(path, Document::empty().text()).unwrap();
    assert_eq!(
        store
            .restore("corrupt", &store.read().unwrap().revision(), checkpoint)
            .unwrap_err()
            .code,
        "RECOVERY_REQUIRED"
    );
    assert_eq!(store.read().unwrap().value()["name"], "one");
    assert_eq!(
        store
            .restore(
                "invalid",
                &store.read().unwrap().revision(),
                &"日".repeat(18)
            )
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn cancelling_before_atomic_replace_aborts_preparation_without_committing() {
    use tokio_util::sync::CancellationToken;
    let (_temp, store) = setup();
    let before = store.read().unwrap();
    let cancel = CancellationToken::new();
    let error = store
        .transact_guarded_with_cancellation(
            "cancel-prepared",
            &before.revision(),
            "cancel-fingerprint",
            Some(&cancel),
            |document| {
                let mut after = document.clone();
                after.set_property("", "name", &json!("Must not save"))?;
                Ok((after, json!({})))
            },
            || {
                cancel.cancel();
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert_eq!(store.read().unwrap().text(), before.text());
    assert!(store.history(10, None).unwrap().is_empty());
    let records = fs::read_dir(store.document_data_dir().join("history"))
        .unwrap()
        .map(|entry| {
            let metadata = fs::read(entry.unwrap().path().join("metadata.json")).unwrap();
            serde_json::from_slice::<lazypencil::store::HistoryEntry>(&metadata).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, TransactionState::Aborted);
    assert!(store.recover().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn cancellation_interrupts_a_writer_lock_wait_before_running_the_edit() {
    use fs2::FileExt;
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;
    let (_temp, store) = setup();
    // init does not acquire this writer lock, so ensure its directory exists first.
    store.history(1, None).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.lock_file_path())
        .unwrap();
    lock.lock_exclusive().unwrap();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let before = store.read().unwrap();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        token.cancel();
    });
    let start = Instant::now();
    let error = store
        .transact_guarded_with_cancellation(
            "cancel-locked",
            &before.revision(),
            "locked-fingerprint",
            Some(&cancel),
            |_| panic!("Cancelled lock waits must not run edits"),
            || Ok(()),
        )
        .unwrap_err();
    thread.join().unwrap();
    assert_eq!(error.code, "CANCELLED");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(store.read().unwrap().text(), before.text());
}

#[cfg(unix)]
#[test]
fn document_reads_reject_named_pipes_without_waiting_for_a_writer() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("pipe.pen");
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = sender.send(lazypencil::store::read_text(&path));
    });
    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("Opening a named pipe must not block");
    assert_eq!(result.unwrap_err().code, "INVALID_DOCUMENT");
    thread.join().unwrap();
}

#[cfg(unix)]
#[test]
fn atomic_replacement_preserves_document_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, store) = setup();
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o640)).unwrap();
    edit(&store, "permissions", "saved");
    assert_eq!(
        fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn saves_respect_read_only_documents_and_keep_private_recovery_files() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, store) = setup();
    let first = edit(&store, "private", "private");
    let directory = store
        .document_data_dir()
        .join("history")
        .join(first["checkpoint_id"].as_str().unwrap());
    for name in ["before.pen", "after.pen", "metadata.json"] {
        assert_eq!(
            fs::metadata(directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o444)).unwrap();
    let before = store.read().unwrap();
    let error = store
        .transact("read-only", &before.revision(), "fp", |_| {
            panic!("read-only edits must not run")
        })
        .unwrap_err();
    assert_eq!(error.code, "IO_ERROR");
    assert_eq!(store.read().unwrap().text(), before.text());
}

#[test]
fn supported_versions_survive_transactions_restores_and_rejected_edits_verbatim() {
    for version in ["2.19", "2.21"] {
        let (_temp, store) = setup();
        let original = fixture().replace("2.21", version);
        fs::write(store.path(), &original).unwrap();
        let document = store.read().unwrap();
        document.ensure_writable().unwrap();
        assert_eq!(document.summary()["writable"], true);
        let first = edit(&store, "first", "first");
        let first_text = store.read().unwrap().text().to_owned();
        assert_eq!(store.read().unwrap().version(), version);
        assert!(first_text.contains("// 日本語のコメント"));
        assert!(first_text.contains("1.000e+3"));
        assert_eq!(
            store.read().unwrap().node("b").unwrap()["magic"],
            json!([1, 2, 3])
        );
        let revision = store.read().unwrap().revision();
        assert!(
            store
                .transact("rejected", &revision, "rejected", |doc| {
                    let mut changed = doc.clone();
                    changed.set_property("", "name", &json!("discarded"))?;
                    Err(Error::new("INVALID_OPERATION", "Rejected after editing"))
                })
                .is_err()
        );
        assert_eq!(fs::read_to_string(store.path()).unwrap(), first_text);
        edit(&store, "second", "second");
        store
            .restore(
                "restore",
                &store.read().unwrap().revision(),
                first["checkpoint_id"].as_str().unwrap(),
            )
            .unwrap();
        // A checkpoint restores its exact saved source, including wire version.
        assert_eq!(fs::read_to_string(store.path()).unwrap(), first_text);
    }
}

#[test]
fn unrecognized_wire_versions_remain_read_only_without_running_the_edit() {
    for version in ["2.18", "2.20", "2.22", "9", "2.19.0"] {
        let (_temp, store) = setup();
        let original = fixture().replace("2.21", version);
        fs::write(store.path(), &original).unwrap();
        let document = store.read().unwrap();
        assert_eq!(document.summary()["writable"], false);
        assert_eq!(
            document.ensure_writable().unwrap_err().code,
            "UNSUPPORTED_VERSION"
        );
        let error = store
            .transact("unknown", &document.revision(), "unknown", |_| {
                panic!("Unknown wire versions must never enter a transaction")
            })
            .unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_VERSION");
        assert_eq!(fs::read_to_string(store.path()).unwrap(), original);
        assert!(store.history(10, None).unwrap().is_empty());
    }
}

#[test]
fn history_records_its_owner_accepts_legacy_records_and_rejects_a_foreign_owner() {
    let (_temp, store) = setup();
    let revision = store.read().unwrap().revision();
    let result = store
        .transact("owned", &revision, "owned", |document| {
            let mut after = document.clone();
            after.set_property("", "name", &json!("owned"))?;
            Ok((after, json!({})))
        })
        .unwrap();
    let id = result["checkpoint_id"].as_str().unwrap();
    let path = store
        .document_data_dir()
        .join("history")
        .join(id)
        .join("metadata.json");
    let mut entry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(entry["document_path"], json!(store.path()));
    entry.as_object_mut().unwrap().remove("document_path");
    fs::write(&path, entry.to_string()).unwrap();
    assert!(store.history(10, None).unwrap()[0].document_path.is_none());
    entry["document_path"] = json!(store.path().with_file_name("foreign.pen"));
    fs::write(path, entry.to_string()).unwrap();
    assert_eq!(
        store.history(10, None).unwrap_err().code,
        "RECOVERY_REQUIRED"
    );
    assert_eq!(store.read().unwrap().value()["name"], "owned");
}
