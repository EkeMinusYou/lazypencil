use clap::Parser;
use lazypencil::{
    cli::{Arguments, Command as CliCommand},
    conversation::Conversation,
    document::hash,
    store::Store,
};
use serde_json::{Value, json};
use std::{path::Path, process::Command};

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
    let adapter = temp.path().join("adapter.py");
    let marker = temp.path().join("agent-started");
    std::fs::write(
        &adapter,
        format!(
            "from pathlib import Path\nPath({:?}).write_text('started')\n",
            marker.to_str().unwrap()
        ),
    )
    .unwrap();
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
