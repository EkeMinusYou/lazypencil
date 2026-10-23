use lazypencil::{
    document::Document,
    operations::{self, Operation},
    service,
};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;
#[cfg(unix)]
#[path = "support/lock_wait.rs"]
mod lock_wait;

fn operations(value: Value) -> Vec<Operation> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn cli_argument_errors_respect_json_before_or_after_the_subcommand() {
    for (index, arguments) in [
        vec!["not-a-command"],
        vec!["render"],
        vec!["icons", "list", "--limit", "nope"],
        vec!["inspect", "design.pen", "--depth", "NaN"],
        vec!["icons", "list", "--unknown"],
    ]
    .iter()
    .enumerate()
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        if index % 2 == 0 {
            command.arg("--json");
        }
        command.args(arguments);
        if index % 2 != 0 {
            command.arg("--json");
        }
        let output = command.env("CLICOLOR_FORCE", "1").output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stderr.is_empty(), "{arguments:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        let message = value["error"]["message"].as_str().unwrap();
        assert!(message.contains("error:"));
        assert!(!message.contains('\x1b'));
        assert!(
            !value["error"]["details"]["kind"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn cli_help_version_and_end_of_options_keep_their_normal_contract() {
    for flag in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", flag])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(String::from_utf8_lossy(&output.stdout).contains("lazypencil"));
        assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
    }
    for arguments in [
        vec!["icons", "list", "--limit", "nope"],
        vec!["--", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn idempotency_fingerprints_ignore_json_object_key_order_but_keep_operation_order() {
    use lazypencil::operations::EditRequest;
    let a = r#"{"request_id":"x","expected_revision":"rev","operations":[{"op":"update","target":"x","set":{"width":10,"height":20}}]}"#;
    let b = r#"{"request_id":"x","expected_revision":"rev","operations":[{"op":"update","target":"x","set":{"height":20,"width":10}}]}"#;
    assert_eq!(
        serde_json::from_str::<EditRequest>(a)
            .unwrap()
            .fingerprint(),
        serde_json::from_str::<EditRequest>(b)
            .unwrap()
            .fingerprint()
    );
    let mut ordered = serde_json::from_str::<EditRequest>(a).unwrap();
    ordered.operations.push(
        serde_json::from_value(json!({"op":"update","target":"x","set":{"width":30}})).unwrap(),
    );
    let mut reversed = ordered.clone();
    reversed.operations.reverse();
    assert_ne!(ordered.fingerprint(), reversed.fingerprint());
    reversed = ordered.clone();
    reversed.expected_dependency_revision = Some("different-library".into());
    assert_ne!(ordered.fingerprint(), reversed.fingerprint());
}

#[cfg(unix)]
#[test]
fn cli_apply_restore_and_history_cancel_writer_lock_waits_without_changing_saved_work() {
    use lazypencil::store::Store;
    use std::time::{Duration, Instant};
    let temp = TempDir::new().unwrap();
    let store = Store::init(
        temp.path().join("design.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let mut first = Value::Null;
    for index in 1..=2 {
        let value = store
            .transact(
                &format!("edit-{index}"),
                &store.read().unwrap().revision(),
                "test",
                |document| {
                    let mut after = document.clone();
                    after.set_property("", "note", &json!(index))?;
                    Ok((after, json!({})))
                },
            )
            .unwrap();
        if index == 1 {
            first = value;
        }
    }
    let before = store.read().unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.lock_file_path())
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let operations = temp.path().join("operations.json");
    fs::write(&operations, json!([{"op":"import_component","mode":"copy","source":{"style_id":"standard-ui","node_id":"button"}}]).to_string()).unwrap();
    for command in [
        "apply",
        #[cfg(feature = "rendering")]
        "apply-runtime",
        "restore",
        "history",
    ] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        process
            .arg("--data-dir")
            .arg(store.data_dir())
            .args([
                "--json",
                if command.starts_with("apply") {
                    "apply"
                } else {
                    command
                },
            ])
            .arg(store.path());
        if command == "restore" {
            process.args([
                "--checkpoint",
                first["checkpoint_id"].as_str().unwrap(),
                "--expected-revision",
                &before.revision(),
            ]);
        }
        if command.starts_with("apply") {
            process
                .arg("--operations")
                .arg(&operations)
                .args(["--expected-revision", &before.revision()]);
            if command == "apply-runtime" {
                process.args([
                    "--expected-runtime-dependency-revision",
                    "must-not-evaluate",
                ]);
            }
        }
        let child = lock_wait::WaitingProcess::spawn(&mut process);
        let start = Instant::now();
        assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
        let output = child.wait_with_output();
        assert!(start.elapsed() < Duration::from_millis(1500));
        assert_eq!(
            output.status.code(),
            Some(130),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], "CANCELLED");
        assert_eq!(store.read().unwrap().text(), before.text());
    }
    drop(lock);
    assert_eq!(store.history(10, None).unwrap().len(), 2);
    assert!(!temp.path().join("assets").exists());
}

fn scene() -> Document {
    Document::parse(
        r#"{
      "version":"2.21",
      "variables":{"label":{"type":"string","value":"Continue"}},
      "children":[
        {"id":"button","type":"frame","reusable":true,"children":[
          {"id":"label","type":"text","content":"$label"}
        ]},
        {"id":"instance","type":"ref","ref":"button"}
      ]
    }"#
        .into(),
    )
    .unwrap()
}

#[test]
fn a_batch_can_create_a_component_and_reference_its_generated_id() {
    let batch = operations(json!([
        {"op":"insert","client_ref":"button","node":{"type":"frame","reusable":true,"children":[{"type":"text","content":"Continue"}]}},
        {"op":"insert","client_ref":"instance","node":{"type":"ref","ref":{"result_of":"button"}}}
    ]));
    let (after, result) = operations::apply(&Document::empty(), &batch).unwrap();
    assert_eq!(after.node_count(), 3);
    let component = result["bindings"]["button"].as_str().unwrap();
    let instance = result["bindings"]["instance"].as_str().unwrap();
    assert_eq!(after.node(instance).unwrap()["ref"], component);
    assert_eq!(after.node(component).unwrap()["reusable"], true);
}

#[test]
fn deleting_a_referenced_component_requires_resolving_the_reference_in_the_same_batch() {
    let before = scene();
    let error = operations::apply(
        &before,
        &operations(json!([{"op":"delete","target":"button"}])),
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_OPERATION");
    assert_eq!(before.node_count(), 3);
    let (after, _) = operations::apply(
        &before,
        &operations(json!([
            {"op":"delete","target":"button"},{"op":"delete","target":"instance"}
        ])),
    )
    .unwrap();
    assert_eq!(after.node_count(), 0);
}

#[test]
fn copy_remaps_internal_refs_and_preserves_comments_and_numeric_spelling() {
    let doc = Document::parse(
        r#"{"version":"2.21","children":[
      {"id":"scene","type":"frame","children":[
        // retained inside the copied subtree
        {"id":"component","type":"rectangle","width":1.000e2,"reusable":true},
        {"id":"ref","type":"ref","ref":"component"}
      ]}
    ]}"#
        .into(),
    )
    .unwrap();
    let (after, result) = operations::apply(
        &doc,
        &operations(json!([{"op":"copy","target":"scene","client_ref":"copy"}])),
    )
    .unwrap();
    assert_eq!(after.node_count(), 6);
    let copy = after
        .node(result["bindings"]["copy"].as_str().unwrap())
        .unwrap();
    assert_ne!(copy["children"][0]["id"], "component");
    assert_eq!(copy["children"][1]["ref"], copy["children"][0]["id"]);
    assert_eq!(
        after
            .text()
            .matches("// retained inside the copied subtree")
            .count(),
        2
    );
    assert_eq!(after.text().matches("1.000e2").count(), 2);
}

#[test]
fn moves_preserve_source_and_reject_ancestry_cycles() {
    let doc = scene();
    assert!(
        operations::apply(
            &doc,
            &operations(json!([{"op":"move","target":"button","parent":"label"}]))
        )
        .is_err()
    );
    let (after, _) = operations::apply(
        &doc,
        &operations(json!([{"op":"move","target":"label","index":0}])),
    )
    .unwrap();
    assert_eq!(after.value()["children"][0]["id"], "label");
    assert!(
        after.node("button").unwrap()["children"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(after.node("label").unwrap()["content"], "$label");
}

#[test]
fn inherited_targets_never_silently_edit_the_instance_root() {
    let request = json!({"op":"update","target":{"node_id":"instance","instance_path":["label"]},"set":{"content":"wrong"}});
    let (after, _) =
        operations::apply(&scene(), &[serde_json::from_value(request).unwrap()]).unwrap();
    assert_eq!(
        after.node("instance").unwrap()["descendants"]["label"]["content"],
        "wrong"
    );
    assert!(after.node("instance").unwrap().get("content").is_none());
    assert_eq!(after.node("label").unwrap()["content"], "$label");
    let forbidden =
        json!({"op":"delete","target":{"node_id":"instance","instance_path":["label"]}});
    assert!(operations::apply(&after, &[serde_json::from_value(forbidden).unwrap()]).is_err());
    let unknown =
        json!({"op":"update","target":{"node_id":"instance","library_alias":"external"},"set":{}});
    assert!(serde_json::from_value::<Operation>(unknown).is_err());
}

#[test]
fn malformed_variables_and_reference_cycles_fail_validation() {
    for definition in [
        json!({"type":"number","value":"invalid"}),
        json!({"type":"color","value":"red"}),
        json!({"type":"boolean","value":1}),
        json!({"type":"number","value":[]}),
    ] {
        assert!(
            operations::apply(
                &Document::empty(),
                &operations(json!([{"op":"set_variable","name":"x","definition":definition}]))
            )
            .is_err()
        );
    }
    let error = operations::apply(
        &Document::empty(),
        &operations(json!([
            {"op":"set_variable","name":"a","definition":{"type":"number","value":"$b"}},
            {"op":"set_variable","name":"b","definition":{"type":"number","value":"$a"}}
        ])),
    )
    .unwrap_err();
    assert_eq!(error.code, "REFERENCE_CYCLE");
    let error = operations::apply(
        &scene(),
        &operations(json!([
            {"op":"update","target":"button","set":{"ref":"instance"}},
            {"op":"update","target":"instance","set":{"reusable":true,"ref":"instance"}}
        ])),
    )
    .unwrap_err();
    assert_eq!(error.code, "REFERENCE_CYCLE");
}

#[test]
fn node_pagination_rejects_changed_revisions_and_changed_filters() {
    let doc = scene();
    let first = service::read_nodes(&doc, None, None, None, 0, 1, None).unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = service::read_nodes(&doc, None, None, None, 0, 1, Some(cursor)).unwrap();
    assert_eq!(second["nodes"][0]["id"], "label");
    assert_eq!(
        service::read_nodes(&doc, None, None, Some("text"), 0, 1, Some(cursor))
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
    let mut changed = doc.clone();
    changed.set_property("", "name", &json!("new")).unwrap();
    assert_eq!(
        service::read_nodes(&changed, None, None, None, 0, 1, Some(cursor))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
}

fn cli_run(dir: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(dir.path())
        .env("LAZYPENCIL_DATA_DIR", dir.path().join("data"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap()
}

fn json_output(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn real_cli_creates_edits_deduplicates_and_restores() {
    let dir = TempDir::new().unwrap();
    let initial = json_output(cli_run(&dir, &["init", "design.pen"]));
    let revision = initial["revision"].as_str().unwrap();
    fs::write(
        dir.path().join("ops.json"),
        json!([
            {"op":"insert","client_ref":"title","node":{"type":"text","content":"こんにちは"}}
        ])
        .to_string(),
    )
    .unwrap();
    let args = [
        "apply",
        "design.pen",
        "--operations",
        "ops.json",
        "--expected-revision",
        revision,
        "--request-id",
        "first",
    ];
    let applied = json_output(cli_run(&dir, &args));
    assert_eq!(json_output(cli_run(&dir, &args))["replayed"], true);
    let after = json_output(cli_run(
        &dir,
        &["inspect", "design.pen", "--nodes", "--depth", "0"],
    ));
    let id = applied["bindings"]["title"].as_str().unwrap();
    assert_eq!(after["nodes"][0]["id"], id);
    assert_eq!(after["nodes"][0]["content"], "こんにちは");
    fs::write(
        dir.path().join("ops.json"),
        json!([{ "op":"update","target":id,"set":{"content":"Changed"}}]).to_string(),
    )
    .unwrap();
    let changed = json_output(cli_run(
        &dir,
        &[
            "apply",
            "design.pen",
            "--operations",
            "ops.json",
            "--expected-revision",
            applied["revision"].as_str().unwrap(),
        ],
    ));
    json_output(cli_run(
        &dir,
        &[
            "restore",
            "design.pen",
            "--checkpoint",
            applied["checkpoint_id"].as_str().unwrap(),
            "--expected-revision",
            changed["revision"].as_str().unwrap(),
        ],
    ));
    let restored = json_output(cli_run(&dir, &["inspect", "design.pen", "--node", id]));
    assert_eq!(restored["nodes"][0]["content"], "こんにちは");
    assert_eq!(
        json_output(cli_run(&dir, &["history", "design.pen"]))["entries"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn real_cli_dry_run_and_failed_batch_leave_no_edit_history() {
    let dir = TempDir::new().unwrap();
    let initial = json_output(cli_run(&dir, &["init", "design.pen"]));
    let revision = initial["revision"].as_str().unwrap();
    fs::write(
        dir.path().join("ops.json"),
        json!([{ "op":"insert","node":{"type":"rectangle","width":100}}]).to_string(),
    )
    .unwrap();
    let preview = json_output(cli_run(
        &dir,
        &[
            "apply",
            "design.pen",
            "--operations",
            "ops.json",
            "--expected-revision",
            revision,
            "--dry-run",
        ],
    ));
    assert_eq!(preview["dry_run"], true);
    assert_eq!(
        json_output(cli_run(&dir, &["inspect", "design.pen"]))["revision"],
        revision
    );
    assert_eq!(
        json_output(cli_run(&dir, &["history", "design.pen"]))["entries"],
        json!([])
    );
    fs::write(
        dir.path().join("ops.json"),
        json!([
            {"op":"insert","node":{"type":"rectangle","width":100}},
            {"op":"delete","target":"missing"}
        ])
        .to_string(),
    )
    .unwrap();
    let failure = cli_run(
        &dir,
        &[
            "apply",
            "design.pen",
            "--operations",
            "ops.json",
            "--expected-revision",
            revision,
        ],
    );
    assert_eq!(failure.status.code(), Some(4));
    assert_eq!(
        serde_json::from_slice::<Value>(&failure.stdout).unwrap()["error"]["code"],
        "INVALID_OPERATION"
    );
    assert_eq!(
        json_output(cli_run(&dir, &["inspect", "design.pen"]))["revision"],
        revision
    );
    assert_eq!(
        json_output(cli_run(&dir, &["history", "design.pen"]))["entries"],
        json!([])
    );
}
