use super::*;
use crate::{
    cli::{Arguments, Command},
    conversation::Conversation,
};
use clap::Parser;
use std::{cell::Cell, collections::BTreeSet, time::Duration};
use tempfile::TempDir;

const DAY: u64 = 1_900_000_000 / 86400 * 86400;

fn session(temp: &TempDir, document: &str) -> (Store, Conversation) {
    let path = temp.path().join(document);
    let data = temp.path().join("data");
    let store = if path.exists() {
        Store::open(&path, Some(data.clone())).unwrap()
    } else {
        Store::init(&path, Some(data.clone())).unwrap()
    };
    let parsed = Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        path.into_os_string(),
        "--adapter=uninstalled-retention-fixture".into(),
    ])
    .unwrap();
    let Command::Prompt(args) = parsed.command else {
        unreachable!()
    };
    let conversation =
        Conversation::open_with_executable(args, Some(data), std::env::current_exe().unwrap())
            .unwrap();
    (store, conversation)
}

fn write_time(path: &Path, seconds: u64) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            write_time(&path, seconds)
        } else {
            fs::File::open(path)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        }
    }
}

fn inactive(conversation: Conversation, age_days: u64) -> (String, PathBuf) {
    let id = conversation.id().to_owned();
    let directory = conversation.directory().to_path_buf();
    drop(conversation);
    fs::write(directory.join("history.txt"), b"saved local transcript").unwrap();
    write_time(&directory, DAY - age_days * 86400);
    (id, directory)
}

fn policy(age: Option<u32>, bytes: Option<u64>, keep: usize) -> RetentionSettings {
    RetentionSettings {
        max_age_days: age,
        max_total_bytes: bytes,
        keep_recent_per_document: keep,
    }
}

fn plan(data: &Path, policy: &RetentionSettings) -> Plan {
    plan_at(
        data,
        policy,
        DAY,
        &Budget {
            started: Instant::now(),
            cancel: None,
        },
    )
    .unwrap()
}

fn selected(plan: &Plan) -> BTreeSet<String> {
    plan.entries
        .iter()
        .filter(|entry| entry.selected)
        .map(|entry| entry.session.clone().unwrap())
        .collect()
}

fn apply_plan(data: &Path, policy: &RetentionSettings, plan: &Plan) -> Value {
    apply_at(
        data,
        policy,
        &plan.revision().unwrap(),
        None,
        DAY,
        || Ok(()),
    )
    .unwrap()
}

#[test]
fn retention_is_disabled_by_default_and_does_not_create_a_maintenance_store() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("missing");
    let settings = RetentionSettings::default();
    let review = plan(&data, &settings);
    assert!(!review.scanned && review.entries.is_empty());
    let result = apply_plan(&data, &settings, &review);
    assert!(result["deleted"].as_array().unwrap().is_empty());
    assert!(!data.exists());
    assert!(policy(Some(0), None, 1).validate().is_err());
    assert!(policy(None, Some(0), 1).validate().is_err());
    assert!(policy(None, None, 1001).validate().is_err());
}

#[test]
fn retention_age_spans_documents_and_missing_original_files_without_deleting_assets() {
    let temp = TempDir::new().unwrap();
    let (a, conversation) = session(&temp, "a.pen");
    let (first, first_path) = inactive(conversation, 20);
    let (b, conversation) = session(&temp, "b.pen");
    let (second, second_path) = inactive(conversation, 10);
    let (c, conversation) = session(&temp, "c.pen");
    let (_, current_path) = inactive(conversation, 1);
    let source = fs::read(a.path()).unwrap();
    fs::remove_file(b.path()).unwrap();
    let asset = temp.path().join("project-asset.png");
    let provider = temp.path().join("provider-history.jsonl");
    fs::write(&asset, b"project data").unwrap();
    fs::write(&provider, b"provider data").unwrap();
    let settings = policy(Some(7), None, 0);
    let review = plan(a.data_dir(), &settings);
    assert_eq!(selected(&review), BTreeSet::from([first, second]));
    assert!(review.total_bytes_exact);
    let result = apply_plan(a.data_dir(), &settings, &review);
    assert_eq!(result["deleted"].as_array().unwrap().len(), 2);
    assert!(!first_path.exists() && !second_path.exists() && current_path.exists());
    assert_eq!(fs::read(a.path()).unwrap(), source);
    assert!(c.path().is_file());
    assert_eq!(fs::read(asset).unwrap(), b"project data");
    assert_eq!(fs::read(provider).unwrap(), b"provider data");
}

#[test]
fn retention_aggregate_bytes_choose_oldest_across_documents_and_keep_recent_conversations() {
    let temp = TempDir::new().unwrap();
    let (a, conversation) = session(&temp, "a.pen");
    let (oldest, oldest_path) = inactive(conversation, 20);
    let (_, conversation) = session(&temp, "a.pen");
    let (_, recent_a) = inactive(conversation, 1);
    let (_, conversation) = session(&temp, "b.pen");
    let (other, other_path) = inactive(conversation, 10);
    let (_, conversation) = session(&temp, "b.pen");
    let (_, recent_b) = inactive(conversation, 2);
    let initial = plan(a.data_dir(), &policy(None, Some(u64::MAX), 1));
    let size = initial
        .entries
        .iter()
        .find(|entry| entry.session.as_deref() == Some(oldest.as_str()))
        .unwrap()
        .bytes
        .unwrap();
    let settings = policy(None, Some(initial.total_known_bytes - size), 1);
    let review = plan(a.data_dir(), &settings);
    assert_eq!(selected(&review), BTreeSet::from([oldest]));
    assert_eq!(review.remaining_over_limit, Some(false));
    let result = apply_plan(a.data_dir(), &settings, &review);
    assert_eq!(result["reclaimed_bytes"], size);
    assert!(!oldest_path.exists() && other_path.exists() && recent_a.exists() && recent_b.exists());
    assert!(
        review
            .entries
            .iter()
            .any(|entry| entry.session.as_deref() == Some(other.as_str()) && !entry.selected)
    );
}

#[test]
fn retention_protects_busy_drafts_pending_images_and_incomplete_native_continuity() {
    let temp = TempDir::new().unwrap();
    let (store, busy) = session(&temp, "a.pen");
    let busy_path = busy.directory().to_path_buf();
    let (_, mut draft) = session(&temp, "b.pen");
    draft.set_draft("未送信の依頼".into()).unwrap();
    let (_, draft_path) = inactive(draft, 10);
    let (_, image) = session(&temp, "c.pen");
    let (_, image_path) = inactive(image, 10);
    let png = include_bytes!("../../../../examples/assets/crop-source.png");
    let digest = hash(png);
    fs::create_dir(image_path.join("attachments")).unwrap();
    fs::write(
        image_path.join("attachments").join(format!("{digest}.png")),
        png,
    )
    .unwrap();
    let attachment = crate::attachments::Attachment {
        id: format!("img_{digest}"),
        display_name: "pending.png".into(),
        hash: digest,
        width: u32::from_be_bytes(png[16..20].try_into().unwrap()),
        height: u32::from_be_bytes(png[20..24].try_into().unwrap()),
        bytes: png.len(),
        source_bytes: Some(png.len()),
    };
    let metadata = image_path.join("session.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    state["pending_attachments"] = json!([attachment]);
    fs::write(&metadata, state.to_string()).unwrap();
    write_time(&image_path, DAY - 10 * 86400);
    let (_, continuity) = session(&temp, "d.pen");
    let (_, continuity_path) = inactive(continuity, 10);
    let metadata = continuity_path.join("session.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    state["continuity_pending"] = json!(true);
    fs::write(&metadata, state.to_string()).unwrap();
    write_time(&continuity_path, DAY - 10 * 86400);
    let (_, plain) = session(&temp, "e.pen");
    let (plain, plain_path) = inactive(plain, 10);
    let settings = policy(Some(1), Some(1), 0);
    let review = plan(store.data_dir(), &settings);
    assert_eq!(selected(&review), BTreeSet::from([plain]));
    assert!(!review.total_bytes_exact);
    assert_eq!(review.remaining_over_limit, Some(true));
    for reason in [
        "busy or changed",
        "unsent draft",
        "pending images",
        "incomplete native continuity",
    ] {
        assert!(
            review
                .entries
                .iter()
                .any(|entry| entry.protected_reason.as_deref() == Some(reason)),
            "{reason}"
        );
    }
    apply_plan(store.data_dir(), &settings, &review);
    assert!(
        !plain_path.exists()
            && [busy_path, draft_path, image_path, continuity_path]
                .iter()
                .all(|path| path.exists())
    );
}

#[test]
fn retention_rejects_stale_policy_and_same_length_same_time_content_changes_before_any_commit() {
    let temp = TempDir::new().unwrap();
    let (store, conversation) = session(&temp, "a.pen");
    let (_, path) = inactive(conversation, 10);
    let settings = policy(Some(1), None, 0);
    let review = plan(store.data_dir(), &settings);
    assert_eq!(
        apply_at(
            store.data_dir(),
            &policy(Some(2), None, 0),
            &review.revision().unwrap(),
            None,
            DAY,
            || Ok(())
        )
        .unwrap_err()
        .code,
        "REVISION_CONFLICT"
    );
    let file = path.join("history.txt");
    let before = fs::metadata(&file).unwrap().modified().unwrap();
    let length = fs::metadata(&file).unwrap().len() as usize;
    fs::write(&file, vec![b'x'; length]).unwrap();
    fs::File::open(&file).unwrap().set_modified(before).unwrap();
    assert_eq!(
        apply_at(
            store.data_dir(),
            &settings,
            &review.revision().unwrap(),
            None,
            DAY,
            || Ok(())
        )
        .unwrap_err()
        .code,
        "REVISION_CONFLICT"
    );
    assert!(path.exists());
}

#[test]
fn retention_rechecks_each_target_and_never_deletes_a_new_unsent_draft() {
    let temp = TempDir::new().unwrap();
    let (store, conversation) = session(&temp, "a.pen");
    let (_, first) = inactive(conversation, 20);
    let (_, conversation) = session(&temp, "b.pen");
    let (_, second) = inactive(conversation, 10);
    let settings = policy(Some(1), None, 0);
    let review = plan(store.data_dir(), &settings);
    let calls = Cell::new(0);
    let result = apply_at(
        store.data_dir(),
        &settings,
        &review.revision().unwrap(),
        None,
        DAY,
        || {
            if calls.get() == 0 {
                let path = first.join("session.json");
                let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                state["draft"] = json!("new unsent request");
                fs::write(path, state.to_string()).unwrap();
            }
            calls.set(calls.get() + 1);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result["deleted"].as_array().unwrap().len(), 1);
    assert_eq!(result["skipped"].as_array().unwrap().len(), 1);
    assert!(first.exists() && !second.exists());
}

#[test]
fn retention_checks_user_policy_after_hashing_and_again_after_preparing_the_commit() {
    for change_at in [1, 2] {
        let temp = TempDir::new().unwrap();
        let (store, conversation) = session(&temp, "a.pen");
        let (_, path) = inactive(conversation, 20);
        let settings = policy(Some(1), None, 0);
        let review = plan(store.data_dir(), &settings);
        let user = temp.path().join("config.toml");
        fs::write(
            &user,
            "[conversations.retention]\nmax_age_days=1\nkeep_recent_per_document=0\n",
        )
        .unwrap();
        let snapshot = crate::config::ConversationSnapshot::load_file(Some(user.clone())).unwrap();
        let calls = Cell::new(0);
        let result = apply_at(
            store.data_dir(),
            &settings,
            &review.revision().unwrap(),
            None,
            DAY,
            || {
                if calls.get() == change_at {
                    fs::write(&user, "[conversations.retention]\nmax_age_days=36500\n").unwrap();
                }
                calls.set(calls.get() + 1);
                snapshot.verify_unchanged()
            },
        )
        .unwrap_err();
        assert_eq!(result.code, "REVISION_CONFLICT");
        assert!(path.exists());
        assert!(
            result.details.as_ref().unwrap()["completed_result"]["deleted"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let cleanup = cleanup_all(store.data_dir(), None).unwrap();
        assert_eq!(cleanup["removed_count"], if change_at == 2 { 1 } else { 0 });
        assert!(path.exists());
    }
}

#[test]
fn retention_partial_cancel_and_config_revocation_report_already_committed_deletions() {
    for revoke in [false, true] {
        let temp = TempDir::new().unwrap();
        let (store, conversation) = session(&temp, "a.pen");
        let (_, first) = inactive(conversation, 20);
        let (_, conversation) = session(&temp, "b.pen");
        let (_, second) = inactive(conversation, 10);
        let settings = policy(Some(1), None, 0);
        let review = plan(store.data_dir(), &settings);
        let token = CancellationToken::new();
        let calls = Cell::new(0);
        let user = temp.path().join("config.toml");
        fs::write(
            &user,
            "[conversations.retention]\nmax_age_days=1\nkeep_recent_per_document=0\n",
        )
        .unwrap();
        let snapshot = crate::config::ConversationSnapshot::load_file(Some(user.clone())).unwrap();
        let result = apply_at(
            store.data_dir(),
            &settings,
            &review.revision().unwrap(),
            Some(&token),
            DAY,
            || {
                if !first.exists() {
                    if revoke {
                        fs::write(&user, "[conversations.retention]\nmax_age_days=36500\n")
                            .unwrap();
                    } else {
                        token.cancel();
                    }
                }
                calls.set(calls.get() + 1);
                snapshot.verify_unchanged()
            },
        )
        .unwrap_err();
        assert_eq!(
            result.code,
            if revoke {
                "REVISION_CONFLICT"
            } else {
                "CANCELLED"
            }
        );
        assert_eq!(
            result.details.as_ref().unwrap()["completed_result"]["deleted"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(!first.exists() && second.exists());
    }
}

#[test]
fn retention_counts_pending_cleanup_and_recovers_it_after_the_original_document_was_removed() {
    let temp = TempDir::new().unwrap();
    let (store, conversation) = session(&temp, "a.pen");
    let (id, path) = inactive(conversation, 20);
    let revision = deletion::review_delete(&store, &id, None).unwrap()["revision"].clone();
    let operation = uuid::Uuid::new_v4().to_string();
    let trash = store
        .document_data_dir()
        .join(format!("conversation-trash-{id}-{operation}"));
    fs::create_dir(&trash).unwrap();
    fs::rename(path, trash.join("payload")).unwrap();
    fs::remove_file(trash.join("payload/session.json")).unwrap();
    fs::write(trash.join("deletion.json"),json!({"version":1,"document":store.path(),"session":id,"operation":operation,"revision":revision}).to_string()).unwrap();
    fs::remove_file(store.path()).unwrap();
    let review = plan(store.data_dir(), &policy(None, Some(1), 0));
    assert!(selected(&review).is_empty());
    assert!(review.total_known_bytes > 0);
    assert_eq!(review.remaining_over_limit, Some(true));
    assert_eq!(review.entries[0].kind, "pending_cleanup");
    let result = cleanup_all(store.data_dir(), None).unwrap();
    assert_eq!(result["removed_count"], 1);
    assert!(!trash.exists());
}

#[cfg(unix)]
#[test]
fn retention_never_follows_symlink_or_foreign_namespaces_and_preserves_unknown_data() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let (store, conversation) = session(&temp, "a.pen");
    let (_, path) = inactive(conversation, 20);
    let outside = temp.path().join("outside");
    fs::rename(&path, &outside).unwrap();
    symlink(&outside, &path).unwrap();
    let before = fs::read(outside.join("session.json")).unwrap();
    let unknown = store
        .document_data_dir()
        .join("conversations/not-a-session");
    fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("keep"), b"unknown local data").unwrap();
    let settings = policy(Some(1), Some(1), 0);
    let review = plan(store.data_dir(), &settings);
    assert!(selected(&review).is_empty() && !review.total_bytes_exact);
    apply_plan(store.data_dir(), &settings, &review);
    assert_eq!(fs::read(outside.join("session.json")).unwrap(), before);
    assert_eq!(
        fs::read(unknown.join("keep")).unwrap(),
        b"unknown local data"
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        super::review(store.data_dir(), &settings, Some(&cancel))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert!(
        plan_at(
            store.data_dir(),
            &settings,
            DAY,
            &Budget {
                started: Instant::now() - Duration::from_secs(31),
                cancel: None
            }
        )
        .is_err()
    );
}
