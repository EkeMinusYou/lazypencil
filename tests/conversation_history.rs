use clap::Parser;
use lazypencil::{
    cli::{Arguments, Command as CliCommand},
    conversation::Conversation,
    document::hash,
    store::Store,
};
use serde_json::{Value, json};
use std::{path::Path, process::Command};

#[path = "support/fixture.rs"]
mod fixture_support;

fn cli(data: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn local_session(store: &Store) -> Conversation {
    let parsed = Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        store.path().as_os_str().to_owned(),
        "--adapter=uninstalled-retention-cli-fixture".into(),
    ])
    .unwrap();
    let CliCommand::Prompt(args) = parsed.command else {
        unreachable!()
    };
    Conversation::open_with_executable(
        args,
        Some(store.data_dir().into()),
        env!("CARGO_BIN_EXE_lazypencil").into(),
    )
    .unwrap()
}

fn lifecycle_report(name: &str, mut value: Value) {
    if let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
        let root = std::path::PathBuf::from(root).join("conversation-lifecycle");
        std::fs::create_dir_all(&root).unwrap();
        value["source_commit"] = json!(std::env::var("GITHUB_SHA").ok());
        let feature = if cfg!(feature = "rendering") {
            "default"
        } else {
            "core"
        };
        value["configuration"] = json!(feature);
        std::fs::write(
            root.join(format!("{name}-{feature}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn saved_conversation_cli_prunes_across_documents_only_after_an_unchanged_user_policy_review() {
    let temp = tempfile::TempDir::new().unwrap();
    let data = temp.path().join("data");
    let user = temp.path().join("user");
    std::fs::create_dir(&user).unwrap();
    let configuration = user.join("config.toml");
    let settings = "[fonts]\nfiles=['unavailable-font.ttf']\n[conversations.retention]\nmax_total_bytes=1\nkeep_recent_per_document=0\n";
    std::fs::write(&configuration, settings).unwrap();
    let mut stores = Vec::new();
    let mut directories = Vec::new();
    let mut ids = Vec::new();
    let mut originals = Vec::new();
    for name in ["a.pen", "b.pen", "draft.pen"] {
        let store = Store::init(temp.path().join(name), Some(data.clone())).unwrap();
        let mut conversation = local_session(&store);
        if name == "draft.pen" {
            conversation
                .set_draft("保護する未送信の依頼".into())
                .unwrap();
        }
        ids.push(conversation.id().to_string());
        directories.push(conversation.directory().to_path_buf());
        drop(conversation);
        originals.push(std::fs::read(store.path()).unwrap());
        stores.push(store);
    }
    let command = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .env("LAZYPENCIL_CONFIG_DIR", &user)
            .arg("--json")
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .output()
            .unwrap()
    };
    let read = || {
        let output = command(&["conversations", "prune"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let review = read();
    assert_eq!(review["scope"], "all_local_document_conversations");
    assert_eq!(review["selected_count"], 2);
    assert_eq!(review["protected_count"], 1);
    assert_eq!(review["applied"], false);
    assert!(directories.iter().all(|path| path.exists()));
    std::fs::write(
        &configuration,
        settings.replace("max_total_bytes=1", "max_total_bytes=2"),
    )
    .unwrap();
    let stale = command(&[
        "conversations",
        "prune",
        "--expected-revision",
        review["revision"].as_str().unwrap(),
    ]);
    assert!(!stale.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&stale.stdout).unwrap()["error"]["code"],
        "REVISION_CONFLICT"
    );
    assert!(directories.iter().all(|path| path.exists()));
    let review = read();
    let output = command(&[
        "conversations",
        "prune",
        "--expected-revision",
        review["revision"].as_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["deleted"].as_array().unwrap().len(), 2);
    assert!(result["skipped"].as_array().unwrap().is_empty());
    assert_eq!(result["remaining_over_limit_estimate"], true);
    assert!(!directories[0].exists() && !directories[1].exists() && directories[2].exists());
    for (store, original) in stores.iter().zip(&originals) {
        assert_eq!(std::fs::read(store.path()).unwrap(), *original);
    }
    let draft: Value =
        serde_json::from_slice(&std::fs::read(directories[2].join("session.json")).unwrap())
            .unwrap();
    assert_eq!(draft["draft"], "保護する未送信の依頼");
    assert!(draft["native_session_id"].is_null());
    lifecycle_report(
        "retention",
        json!({"scope":review["scope"],"reviewed_revision":review["revision"],"selected_ids":ids[..2],"protected_session":ids[2],"protected_draft_sha256":hash("保護する未送信の依頼".as_bytes()),"selected_count":2,"stale_policy_rejected":true,"deleted_count":2,"protected_draft_preserved":true,"documents_preserved":true,"agent_started":false,"remaining_over_limit":true}),
    );
}

#[cfg(unix)]
#[test]
fn saved_conversation_cli_reports_committed_cleanup_failure_and_recovers_it_globally_without_the_document()
 {
    use std::os::unix::fs::PermissionsExt;
    let uid = unsafe { libc::geteuid() };
    if uid == 0 {
        eprintln!("Readonly cleanup fault requires an unprivileged user");
        return;
    }
    let temp = tempfile::TempDir::new().unwrap();
    let data = temp.path().join("data");
    let store = Store::init(temp.path().join("design.pen"), Some(data.clone())).unwrap();
    let conversation = local_session(&store);
    let id = conversation.id().to_string();
    let directory = conversation.directory().to_path_buf();
    drop(conversation);
    let original = std::fs::read(store.path()).unwrap();
    let asset = temp.path().join("project-asset.png");
    std::fs::write(&asset, b"project asset remains").unwrap();
    let protected = directory.join("read-only");
    std::fs::create_dir(&protected).unwrap();
    std::fs::write(protected.join("keep"), b"failed cleanup payload").unwrap();
    std::fs::set_permissions(&protected, std::fs::Permissions::from_mode(0o500)).unwrap();
    let review = cli(
        &data,
        &[
            "conversations",
            "delete",
            store.path().to_str().unwrap(),
            &id,
        ],
    );
    let result = cli(
        &data,
        &[
            "conversations",
            "delete",
            store.path().to_str().unwrap(),
            &id,
            "--expected-revision",
            review["revision"].as_str().unwrap(),
        ],
    );
    let trash = std::fs::read_dir(store.document_data_dir())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("conversation-trash-{id}-"))
        })
        .unwrap()
        .path();
    std::fs::set_permissions(
        trash.join("payload/read-only"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert_eq!(result["deleted"], true);
    assert_eq!(result["cleanup_complete"], false);
    assert!(result["cleanup_error"].is_string());
    assert!(!directory.exists());
    assert_eq!(std::fs::read(store.path()).unwrap(), original);
    assert!(trash.join("deletion.json").is_file());
    std::fs::remove_file(store.path()).unwrap();
    let recovery = cli(&data, &["conversations", "cleanup", "--all"]);
    assert_eq!(recovery["removed_count"], 1);
    assert_eq!(recovery["retained_count"], 0);
    assert!(!trash.exists() && !store.path().exists());
    assert_eq!(std::fs::read(&asset).unwrap(), b"project asset remains");
    lifecycle_report(
        "cleanup-permission",
        json!({"uid":uid,"session":id,"deleted":true,"cleanup_failed":true,"journal_retained":true,"orphan_document_cleanup":true,"recovered":true,"project_asset_preserved":true}),
    );
}

#[test]
fn saved_conversation_cli_reviews_deletion_rejects_changes_and_preserves_the_document() {
    let temp = tempfile::TempDir::new().unwrap();
    let data = temp.path().join("data");
    let store = Store::init(temp.path().join("design.pen"), Some(data.clone())).unwrap();
    let parsed = Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        store.path().as_os_str().to_owned(),
        "--adapter=uninstalled-local-deletion-fixture".into(),
    ])
    .unwrap();
    let CliCommand::Prompt(args) = parsed.command else {
        unreachable!()
    };
    let mut conversation = Conversation::open_with_executable(
        args,
        Some(data.clone()),
        env!("CARGO_BIN_EXE_lazypencil").into(),
    )
    .unwrap();
    conversation.set_draft("削除前の下書き".into()).unwrap();
    let id = conversation.id().to_string();
    let directory = conversation.directory().to_path_buf();
    drop(conversation);
    let original = std::fs::read(store.path()).unwrap();
    let file = store.path().to_str().unwrap();
    let review = cli(&data, &["conversations", "delete", file, &id]);
    assert_eq!(review["deleted"], false);
    assert!(directory.is_dir());
    std::fs::write(
        directory.join("new-local-log.jsonl"),
        b"changed since review",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .arg("--data-dir")
        .arg(&data)
        .args([
            "conversations",
            "delete",
            file,
            &id,
            "--expected-revision",
            review["revision"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "REVISION_CONFLICT");
    assert!(directory.exists());
    let review = cli(&data, &["conversations", "delete", file, &id]);
    let result = cli(
        &data,
        &[
            "conversations",
            "delete",
            file,
            &id,
            "--expected-revision",
            review["revision"].as_str().unwrap(),
        ],
    );
    assert_eq!(result["deleted"], true);
    assert_eq!(result["cleanup_complete"], true);
    assert!(!directory.exists());
    assert_eq!(std::fs::read(store.path()).unwrap(), original);
    assert!(
        cli(&data, &["conversations", "cleanup", file])["removed"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn saved_conversation_cli_browses_complete_logs_without_starting_or_reconfiguring_an_agent() {
    let temp = tempfile::TempDir::new().unwrap();
    let data = temp.path().join("data");
    let path = temp.path().join("design.pen");
    let store = Store::init(&path, Some(data.clone())).unwrap();
    let adapter = temp
        .path()
        .join(format!("adapter{}", std::env::consts::EXE_SUFFIX));
    let marker = temp.path().join("agent-started");
    fixture_support::install(&adapter, &["mark".into(), marker.to_str().unwrap().into()]);
    let args = Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        path.clone().into_os_string(),
        "--adapter".into(),
        adapter.clone().into_os_string(),
    ])
    .unwrap();
    let CliCommand::Prompt(args) = args.command else {
        unreachable!()
    };
    let conversation = Conversation::open_with_executable(
        args,
        Some(data.clone()),
        Path::new(env!("CARGO_BIN_EXE_lazypencil")).into(),
    )
    .unwrap();
    let session = conversation.id();
    let metadata = std::fs::read(conversation.directory().join("session.json")).unwrap();
    let turn = uuid::Uuid::new_v4().to_string();
    let header = json!({"type":"start","session":session,"turn":turn,"document":store.path()});
    let prompt = json!({"type":"user_prompt","text":"日本語の保存履歴🙂".repeat(2000)});
    let bytes = format!("{header}\n{prompt}\n{{\"type\":\"unfinished\"").into_bytes();
    std::fs::write(
        conversation.directory().join(format!("turn-{turn}.jsonl")),
        &bytes,
    )
    .unwrap();
    // Read-only browsing must work after the original adapter has disappeared,
    // and while another frontend still owns the conversation's exclusive lease.
    std::fs::remove_file(adapter).unwrap();
    let file = store.path().to_str().unwrap();
    let list = cli(&data, &["conversations", "list", file]);
    assert_eq!(list["entries"][0]["id"], session);
    let turns = cli(&data, &["conversations", "turns", file, session]);
    assert_eq!(turns["entries"][0]["id"], turn);
    let revision = hash(&bytes);
    let mut restored = String::new();
    let mut offset = 0;
    loop {
        let page = cli(
            &data,
            &[
                "conversations",
                "read",
                file,
                session,
                &turn,
                "--offset",
                &offset.to_string(),
                "--limit",
                "8192",
                "--expected-revision",
                &revision,
            ],
        );
        assert_eq!(page["revision"], revision);
        restored.push_str(page["text"].as_str().unwrap());
        if let Some(next) = page["next_offset"].as_u64() {
            offset = next;
        } else {
            break;
        }
    }
    assert_eq!(restored.as_bytes(), bytes);
    let mut formatted = String::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut arguments = vec![
            "conversations",
            "view",
            file,
            session,
            &turn,
            "--limit",
            "8192",
        ];
        if let Some(cursor) = &cursor {
            arguments.extend(["--cursor", cursor]);
        }
        let page = cli(&data, &arguments);
        formatted.push_str(page["text"].as_str().unwrap());
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        formatted,
        format!(
            "\nTurn {turn}\n\nYou:\n{}\n{{\"type\":\"unfinished\"",
            prompt["text"].as_str().unwrap()
        )
    );
    let human = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(&data)
        .args([
            "conversations",
            "view",
            file,
            session,
            &turn,
            "--limit",
            "128",
        ])
        .output()
        .unwrap();
    assert!(human.status.success());
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .starts_with(&format!("\nTurn {turn}\n\nYou:\n日本語"))
    );
    assert!(
        String::from_utf8(human.stderr)
            .unwrap()
            .contains("Continue with --cursor")
    );
    assert_eq!(
        std::fs::read(conversation.directory().join("session.json")).unwrap(),
        metadata
    );
    assert!(!marker.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .arg("--data-dir")
        .arg(&data)
        .args([
            "conversations",
            "read",
            file,
            session,
            &turn,
            "--offset",
            "4",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
}
