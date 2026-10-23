use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
use lazypencil::agent::{AgentEvent, AgentHost, AgentOptions};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Duration,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn python() -> PathBuf {
    std::env::var_os("LAZYPENCIL_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| "python3".into())
}
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/acp_fixture.py")
}
fn options(mode: &str, directory: &Path) -> AgentOptions {
    let mut options = AgentOptions::new(python(), directory);
    options.args = vec![fixture().to_str().unwrap().into(), mode.into()];
    options.setup_timeout = Duration::from_secs(5);
    options.turn_timeout = Duration::from_secs(5);
    options.cancel_grace = Duration::from_millis(300);
    options
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
async fn message(host: &mut AgentHost) -> Value {
    match tokio::time::timeout(Duration::from_secs(10), host.next_event())
        .await
        .unwrap()
        .expect("agent event")
    {
        AgentEvent::Message(message) => message,
        event => panic!("Unexpected event: {event:?}"),
    }
}
async fn ready(host: &mut AgentHost) {
    assert_eq!(message(host).await["type"], "initialized");
    loop {
        let event = message(host).await;
        if event["type"] == "ready" {
            break;
        }
        assert_eq!(event["type"], "auth_status");
    }
}

#[test]
fn reported_authentication_must_match_the_requested_route_before_prompting() {
    for (mode, subscription, allowed) in [
        ("auth-account", true, true),
        ("auth-api", false, true),
        ("auth-account", false, false),
        ("auth-api", true, false),
        ("auth-external", true, false),
        ("auth-gateway", true, false),
        ("auth-none", true, false),
        ("auth-unknown", true, false),
        ("auth-absent", true, false),
    ] {
        let directory = TempDir::new().unwrap();
        runtime().block_on(async {
            let mut opts = options(mode, directory.path());
            opts.subscription = subscription;
            opts.auth_status_timeout = Duration::from_millis(100);
            let mut host = AgentHost::spawn(opts).unwrap();
            ready(&mut host).await;
            host.prompt(prompt(), CancellationToken::new())
                .await
                .unwrap();
            let event = message(&mut host).await;
            if allowed {
                assert_eq!(event["type"], "update", "mode={mode}: {event}");
                assert_eq!(message(&mut host).await["type"], "turn_complete");
            } else {
                assert_eq!(event["type"], "turn_error", "mode={mode}: {event}");
                assert_eq!(event["error"]["details"]["reason"], "auth_route_mismatch");
                assert!(!directory.path().join("sent-turn.txt").exists());
            }
            host.close().await.unwrap();
        });
    }
}

#[test]
fn relative_user_configuration_is_pinned_before_adapter_working_directory_changes() {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("nested")).unwrap();
    std::fs::create_dir_all(root.join("settings/fonts")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/regular.ttf"),
        root.join("settings/fonts/regular.ttf"),
    )
    .unwrap();
    std::fs::write(
        root.join("settings/config.toml"),
        "[fonts]\nfiles=['fonts/regular.ttf']\ndefault_family='LP Test Sans'\nlocale='en-US'\n",
    )
    .unwrap();
    let document = root.join("nested/design.pen");
    lazypencil::store::Store::init(&document, Some(root.join("data"))).unwrap();
    let saved = std::fs::read(&document).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(root)
        .env("LAZYPENCIL_CONFIG_DIR", "settings")
        .args([
            "--json",
            "--data-dir",
            "data",
            "prompt",
            "nested/design.pen",
            "--adapter",
        ])
        .arg(python())
        .arg(format!("--adapter-arg={}", fixture().display()))
        .arg("--adapter-arg=user-config")
        .args([
            "--non-interactive",
            "--timeout",
            "10",
            "設定を確認してください",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        std::fs::read_to_string(root.join("nested/fixture-errors.log")).unwrap_or_default()
    );
    let received: Value = serde_json::from_slice(
        &std::fs::read(root.join("nested/received-user-configuration.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        received["user"]["path"],
        json!(lazypencil::paths::canonical(root.join("settings/config.toml")).unwrap())
    );
    assert_eq!(
        received["font_files"][0],
        json!(lazypencil::paths::canonical(root.join("settings/fonts/regular.ttf")).unwrap())
    );
    assert!(!root.join("nested/settings").exists());
    assert!(!root.join("nested/data").exists());
    assert_eq!(std::fs::read(&document).unwrap(), saved);
}

#[test]
fn authentication_changes_cancel_an_active_turn_without_retrying() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("auth-change", directory.path())).unwrap();
        ready(&mut host).await;
        host.prompt(prompt(), CancellationToken::new())
            .await
            .unwrap();
        let mut changed = false;
        loop {
            let event = message(&mut host).await;
            if event["type"] == "auth_status" {
                assert_eq!(event["authentication"]["source"], "api_key");
                changed = true;
            } else {
                assert_eq!(event["type"], "turn_error");
                assert_eq!(event["error"]["details"]["reason"], "auth_route_mismatch");
                break;
            }
        }
        assert!(changed);
        host.close().await.unwrap();
    });
}

#[test]
fn cli_records_only_nonsensitive_authentication_metadata() {
    let directory = TempDir::new().unwrap();
    let output = cli(
        &directory,
        "auth-account",
        &["--create", "--require-auth-status"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let values = events(&output);
    let complete = values
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    assert_eq!(complete["authentication"]["source"], "subscription");
    let transcript = std::fs::read_to_string(complete["transcript"].as_str().unwrap()).unwrap();
    for text in [
        String::from_utf8_lossy(&output.stdout).as_ref(),
        &transcript,
    ] {
        assert!(!text.contains("must-not-log"));
        assert!(!text.contains("fixture-not-a-token"));
        assert!(text.contains("test-plan"));
    }
}

#[test]
fn model_and_mode_selection_are_validated_and_confirmed() {
    let directory = TempDir::new().unwrap();
    let output = cli(
        &directory,
        "settings-check",
        &[
            "--create",
            "--model",
            "fixture-model-2",
            "--mode",
            "workspace-write",
        ],
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        std::fs::read_to_string(directory.path().join("fixture-errors.log")).unwrap_or_default()
    );
    let directory = TempDir::new().unwrap();
    let output = cli(
        &directory,
        "settings-check",
        &["--create", "--model", "not-advertised"],
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!directory.path().join("sent-turn.txt").exists());
}

#[test]
fn document_tool_preauthorization_never_uses_a_shell_title() {
    for mode in ["document-permission", "correlated-document"] {
        let directory = TempDir::new().unwrap();
        let output = cli(&directory, mode, &["--create"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            events(&output)
                .iter()
                .any(|event| event["type"] == "permission_response"
                    && event["document_tool_preauthorized"] == true
                    && event["option_id"] == "allow")
        );
    }
    let directory = TempDir::new().unwrap();
    let output = cli(&directory, "spoofed-permission", &["--create"]);
    assert_eq!(output.status.code(), Some(5));
    assert!(
        events(&output)
            .iter()
            .any(|event| event["type"] == "permission_response"
                && event["document_tool_preauthorized"] == false
                && event["option_id"] == "deny")
    );
}
fn prompt() -> Vec<ContentBlock> {
    vec![ContentBlock::Text(TextContent::new("日本語のテスト"))]
}

#[test]
fn connection_reuses_session_and_handles_multiple_turns() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut opts = options("echo", directory.path());
        opts.auth_method = Some("fixture-auth".into());
        let mut host = AgentHost::spawn(opts).unwrap();
        ready(&mut host).await;
        for index in 1..=2 {
            host.prompt(prompt(), CancellationToken::new())
                .await
                .unwrap();
            let value = message(&mut host).await;
            assert_eq!(
                value["notification"]["update"]["content"]["text"],
                format!("完了 {index}\n")
            );
            assert_eq!(message(&mut host).await["type"], "turn_complete");
        }
        host.close().await.unwrap();
    });
}

#[test]
fn cancellation_waits_for_protocol_completion_and_allows_next_turn() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("cancel", directory.path())).unwrap();
        ready(&mut host).await;
        for _ in 0..2 {
            let cancel = CancellationToken::new();
            host.prompt(prompt(), cancel.clone()).await.unwrap();
            assert_eq!(message(&mut host).await["type"], "update");
            cancel.cancel();
            let result = message(&mut host).await;
            assert_eq!(result["type"], "turn_error");
            assert_eq!(result["error"]["code"], "CANCELLED");
        }
        host.close().await.unwrap();
    });
}

#[test]
fn cancellation_answers_pending_permissions_without_user_input() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("cancel-permission", directory.path())).unwrap();
        ready(&mut host).await;
        let cancel = CancellationToken::new();
        host.prompt(prompt(), cancel.clone()).await.unwrap();
        message(&mut host).await;
        let event = host.next_event().await.unwrap();
        assert!(matches!(event, AgentEvent::Permission { .. }));
        cancel.cancel();
        assert_eq!(message(&mut host).await["error"]["code"], "CANCELLED");
        drop(event);
        host.close().await.unwrap();
    });
}

#[test]
fn turn_timeout_and_uncooperative_cancel_terminate_adapter() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut opts = options("hang-cancel", directory.path());
        opts.turn_timeout = Duration::from_millis(100);
        let mut host = AgentHost::spawn(opts).unwrap();
        ready(&mut host).await;
        host.prompt(prompt(), CancellationToken::new())
            .await
            .unwrap();
        message(&mut host).await;
        let event = message(&mut host).await;
        assert_eq!(event["type"], "connection_error");
        assert!(
            event["error"]["message"]
                .as_str()
                .unwrap()
                .contains("timed out")
        );
        let error = host.close().await.unwrap_err();
        assert!(error.message.contains("timed out"), "{error}");
    });
}

#[test]
fn capabilities_prevent_image_fallback_and_unsupported_session_load() {
    use agent_client_protocol::schema::v1::ImageContent;
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("no-images", directory.path())).unwrap();
        ready(&mut host).await;
        host.prompt(
            vec![ContentBlock::Image(ImageContent::new("AA==", "image/png"))],
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(message(&mut host).await["type"], "turn_error");
        host.prompt(prompt(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            message(&mut host).await["notification"]["update"]["content"]["text"],
            "完了 1\n"
        );
        assert_eq!(message(&mut host).await["type"], "turn_complete");
        host.close().await.unwrap();
        let mut opts = options("no-load", directory.path());
        opts.session_id = Some("fixture-native".into());
        let mut host = AgentHost::spawn(opts).unwrap();
        assert_eq!(message(&mut host).await["type"], "initialized");
        let error = message(&mut host).await;
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot reload sessions")
        );
        assert!(host.close().await.is_err());
    });
}

#[test]
fn future_updates_and_extension_notifications_are_preserved() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("unknown-notification", directory.path())).unwrap();
        ready(&mut host).await;
        host.prompt(prompt(), CancellationToken::new())
            .await
            .unwrap();
        let extension = message(&mut host).await;
        assert_eq!(extension["type"], "agent_notification");
        assert_eq!(extension["method"], "_fixture/future");
        assert_eq!(
            message(&mut host).await["notification"]["update"]["value"],
            "preserve me"
        );
        assert_eq!(message(&mut host).await["type"], "update");
        assert_eq!(message(&mut host).await["type"], "turn_complete");
        host.close().await.unwrap();
    });
}

#[test]
fn setup_timeout_and_pre_cancelled_prompts_never_invoke_a_turn() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut opts = options("setup-hang", directory.path());
        opts.setup_timeout = Duration::from_millis(100);
        let mut host = AgentHost::spawn(opts).unwrap();
        assert_eq!(message(&mut host).await["type"], "connection_error");
        assert!(
            host.close()
                .await
                .unwrap_err()
                .message
                .contains("setup timed out")
        );
        let mut host = AgentHost::spawn(options("echo", directory.path())).unwrap();
        ready(&mut host).await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        host.prompt(prompt(), cancel).await.unwrap();
        assert_eq!(message(&mut host).await["error"]["code"], "CANCELLED");
        host.prompt(prompt(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            message(&mut host).await["notification"]["update"]["content"]["text"],
            "完了 1\n"
        );
        assert_eq!(message(&mut host).await["type"], "turn_complete");
        host.close().await.unwrap();
    });
}

#[cfg(unix)]
#[test]
fn closing_the_host_reaps_its_adapter_process_group() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("grandchild", directory.path())).unwrap();
        ready(&mut host).await;
        let path = directory.path().join("fixture-child.pid");
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid: i32 = std::fs::read_to_string(path).unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        host.close().await.unwrap();
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("Adapter child {pid} survived host shutdown");
    });
}

fn cli(directory: &TempDir, mode: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(directory.path().join("data"))
        .args(["--json", "prompt"])
        .arg(directory.path().join("design.pen"))
        .args(["--adapter"])
        .arg(python())
        .arg(format!("--adapter-arg={}", fixture().display()))
        .arg(format!("--adapter-arg={mode}"))
        .args(["--non-interactive", "--timeout", "10"])
        .args(extra)
        .arg("日本語でデザインを作成してください")
        .output()
        .unwrap()
}
fn events(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect()
}

#[test]
fn cli_prompt_passes_trusted_project_rules_and_default_style_and_blocks_changed_rules_before_spawning()
 {
    let directory = TempDir::new().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    std::fs::write(
        root.join("rules.txt"),
        "ACP_PROJECT_RULE: 日本語の原稿を維持する。",
    )
    .unwrap();
    std::fs::write(root.join("reference.txt"), "REFERENCE_NOT_AUTOMATIC").unwrap();
    std::fs::write(root.join("components.pen"),json!({"version":"2.21","children":[{"id":"button","type":"frame","reusable":true,"width":40,"height":20}]}).to_string()).unwrap();
    std::fs::write(root.join("brand.json"),json!({"id":"brand","name":"Brand","description":"Project style","tokens":{},"instructions":"Use project tokens.","example":"components.pen"}).to_string()).unwrap();
    std::fs::write(root.join("lazypencil.toml"),"[design]\nstyles=['brand.json']\ndefault_style='project:brand'\n[[design.guides]]\nid='brand'\npath='rules.txt'\nlanguage='ja'\n[[design.guides]]\nid='reference'\npath='reference.txt'\nautomatic=false\n").unwrap();
    let output = cli(&directory, "project-guidance", &["--create"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("PROJECT_NOT_TRUSTED"));
    assert!(!root.join("sent-turn.txt").exists());
    assert!(!root.join("fixture-errors.log").exists());
    let grant = || {
        let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(root.join("data"))
            .args(["config", "trust"])
            .arg(root.join("design.pen"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    };
    grant();
    let output = cli(&directory, "project-guidance", &[]);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        std::fs::read_to_string(root.join("fixture-errors.log")).unwrap_or_default()
    );
    let first = events(&output);
    let start = first.iter().find(|event| event["type"] == "start").unwrap();
    assert_eq!(start["style"]["id"], "project:brand");
    assert_eq!(
        start["guidance"]["project_guides"][0]["id"],
        "project:brand"
    );
    assert!(start["guidance"]["project_guides"][0].get("body").is_none());
    let usage = first
        .iter()
        .filter(|event| event["type"] == "design_resource")
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 4);
    assert_eq!(usage[0]["resources"][1]["id"], "project:brand");
    assert_eq!(usage[3]["resources"][0]["id"], "project:reference");
    assert!(usage.iter().all(|event| event["turn"] == start["turn"]));
    assert!(
        !serde_json::to_string(&usage)
            .unwrap()
            .contains("REFERENCE_NOT_AUTOMATIC")
    );
    let received = std::fs::read(root.join("received-project-guidance.json")).unwrap();
    let fetched: Value = serde_json::from_slice(&received).unwrap();
    assert_eq!(usage[0]["resources"][0]["hash"], fetched["workflow_hash"]);
    assert_eq!(usage[1]["resources"][0]["hash"], fetched["style_hash"]);
    assert_eq!(
        usage[2]["resources"][0]["hash"],
        lazypencil::document::hash(fetched["catalog"].to_string().as_bytes())
    );
    assert_eq!(usage[3]["resources"][0]["hash"], fetched["reference_hash"]);
    std::fs::write(root.join("rules.txt"), "ACP_PROJECT_RULE: Changed rules").unwrap();
    let output = cli(&directory, "project-guidance", &[]);
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(root.join("received-project-guidance.json")).unwrap(),
        received
    );
    grant();
    let output = cli(
        &directory,
        "project-guidance-override",
        &["--style", "content-page"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let second = events(&output);
    assert_eq!(
        second
            .iter()
            .find(|event| event["type"] == "start")
            .unwrap()["style"]["id"],
        "content-page"
    );
    std::fs::create_dir(root.join("nested")).unwrap();
    lazypencil::store::Store::init(root.join("nested/design.pen"), Some(root.join("data")))
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(root)
        .args([
            "--json",
            "--data-dir",
            "data",
            "prompt",
            "nested/design.pen",
            "--adapter",
        ])
        .arg(python())
        .arg(format!("--adapter-arg={}", fixture().display()))
        .arg("--adapter-arg=project-guidance")
        .args([
            "--non-interactive",
            "--timeout",
            "10",
            "日本語で作成してください",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        std::fs::read_to_string(root.join("nested/fixture-errors.log")).unwrap_or_default()
    );
    assert!(!root.join("nested/data").exists());
}

#[test]
fn design_resource_audit_reaches_transcripts_for_success_failure_and_cancelled_turns() {
    for mode in [
        "audit-success",
        "audit-failure",
        "audit-cancel",
        "audit-corrupt",
    ] {
        let directory = TempDir::new().unwrap();
        let output = cli(&directory, mode, &["--create"]);
        assert_eq!(
            output.status.success(),
            mode == "audit-success",
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            std::fs::read_to_string(directory.path().join("fixture-errors.log"))
                .unwrap_or_default()
        );
        let all = events(&output);
        let start = all.iter().find(|event| event["type"] == "start").unwrap();
        let terminal = all
            .iter()
            .find(|event| matches!(event["type"].as_str(), Some("complete" | "failed")))
            .unwrap();
        let transcript = Path::new(terminal["transcript"].as_str().unwrap());
        let text = std::fs::read_to_string(transcript).unwrap();
        let recorded = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|event| event["type"] == "design_resource")
            .collect::<Vec<_>>();
        let emitted = all
            .iter()
            .filter(|event| event["type"] == "design_resource")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(recorded, emitted);
        if mode == "audit-corrupt" {
            assert!(recorded.is_empty());
            assert!(!text.contains("MUST_NOT_LOG"));
            assert!(
                terminal["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("audit")
            );
        } else {
            assert_eq!(recorded.len(), 3);
            assert_eq!(recorded[0]["resources"][0]["id"], "typography");
            assert_eq!(
                recorded[0]["resources"][0]["hash"],
                lazypencil::guidance::get_guidance(Some("typography")).unwrap()["hash"]
            );
            assert_eq!(recorded[1]["resources"][0]["id"], "standard-ui");
            assert_eq!(recorded[2]["resources"][0]["kind"], "style_catalog");
            assert!(recorded.iter().all(|event| event["turn"] == start["turn"]));
        }
        let doc: Value =
            serde_json::from_slice(&std::fs::read(directory.path().join("design.pen")).unwrap())
                .unwrap();
        assert_eq!(
            doc["children"].as_array().unwrap().len(),
            1,
            "Committed edits must survive audit and agent failures"
        );
        let state: Value = serde_json::from_slice(
            &std::fs::read(transcript.parent().unwrap().join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            state["draft"].as_str().unwrap().is_empty(),
            mode == "audit-success"
        );
        for entry in std::fs::read_dir(transcript.parent().unwrap()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir()
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("design-usage-")
            {
                assert!(
                    !path.join("active.json").exists(),
                    "Closing the connection must revoke audit writes"
                );
            }
        }
        if mode == "audit-success" {
            let output = cli(
                &directory,
                mode,
                &["--session", terminal["session"].as_str().unwrap()],
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            let next = events(&output);
            let next_start = next.iter().find(|event| event["type"] == "start").unwrap();
            assert_ne!(next_start["turn"], start["turn"]);
            let next_usage = next
                .iter()
                .filter(|event| event["type"] == "design_resource")
                .collect::<Vec<_>>();
            assert_eq!(next_usage.len(), 3);
            assert!(
                next_usage
                    .iter()
                    .all(|event| event["turn"] == next_start["turn"])
            );
            assert_ne!(next_usage[0]["connection"], recorded[0]["connection"]);
        }
    }
}

#[test]
fn prompt_cli_edits_through_real_mcp_and_resumes_bound_conversation() {
    let directory = TempDir::new().unwrap();
    let output = cli(&directory, "edit", &["--create", "--style", "standard-ui"]);
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        std::fs::read_to_string(directory.path().join("fixture-errors.log")).unwrap_or_default()
    );
    let first = events(&output);
    let complete = first
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    assert_ne!(
        complete["document"]["revision"],
        complete["before_revision"]
    );
    let session = complete["session"].as_str().unwrap();
    let output = cli(&directory, "edit", &["--session", session]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        events(&output)
            .iter()
            .any(|event| event["notification"]["update"]["content"]["text"] == "replayed\n")
    );
    let document: Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("design.pen")).unwrap(),
    )
    .unwrap();
    assert_eq!(document["children"].as_array().unwrap().len(), 2);
    let transcript = std::fs::read_to_string(complete["transcript"].as_str().unwrap()).unwrap();
    assert!(transcript.contains("日本語でデザイン"));
    assert!(transcript.contains("standard-ui"));
    let grant_files = std::fs::read_dir(
        Path::new(complete["transcript"].as_str().unwrap())
            .parent()
            .unwrap(),
    )
    .unwrap()
    .filter_map(std::result::Result::ok)
    .filter(|file| file.file_name().to_string_lossy().starts_with("grant-"))
    .count();
    assert_eq!(
        grant_files, 0,
        "Connection attachment grants must be revoked"
    );
}

#[test]
fn noninteractive_permission_denial_is_an_error_even_if_agent_reports_end_turn() {
    for mode in ["permission", "permission-cancel"] {
        let directory = TempDir::new().unwrap();
        let output = cli(&directory, mode, &["--create"]);
        assert_eq!(output.status.code(), Some(5));
        let events = events(&output);
        assert!(
        events
            .iter()
            .any(|event| event["type"] == "permission_response" && event["option_id"] == "deny")
    );
        assert!(!events.iter().any(|event| event["type"] == "complete"));
        assert_eq!(events.last().unwrap()["error"]["code"], "AGENT_ERROR");
    }
}

#[cfg(unix)]
#[test]
fn cli_interrupt_retains_committed_batches_and_reports_the_actual_revision() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let directory = TempDir::new().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(directory.path().join("data"))
        .args(["--json", "prompt"])
        .arg(directory.path().join("design.pen"))
        .args(["--create", "--non-interactive", "--adapter"])
        .arg(python())
        .arg(format!("--adapter-arg={}", fixture().display()))
        .arg("--adapter-arg=edit-cancel")
        .arg("日本語のデザイン")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    loop {
        let value: Value =
            serde_json::from_str(&receiver.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
        if value["notification"]["update"]["content"]["text"] == "committed\n" {
            break;
        }
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("CLI did not exit after SIGINT");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(130));
    reader.join().unwrap();
    let remaining: Vec<Value> = receiver
        .try_iter()
        .map(|line| serde_json::from_str(&line).unwrap())
        .collect();
    let failed = remaining
        .iter()
        .find(|event| event["type"] == "failed")
        .unwrap();
    assert_ne!(failed["document"]["revision"], failed["before_revision"]);
    let store = lazypencil::store::Store::open(
        directory.path().join("design.pen"),
        Some(directory.path().join("data")),
    )
    .unwrap();
    assert_eq!(
        store.read().unwrap().revision(),
        failed["document"]["revision"]
    );
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn subscription_requests_remove_inherited_api_key_overrides() {
    let directory = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .env("OPENAI_API_KEY", "fixture-openai-secret")
        .env("CODEX_API_KEY", "fixture-codex-secret")
        .env("ANTHROPIC_API_KEY", "fixture-anthropic-secret")
        .env("DEFAULT_AUTH_REQUEST", "fixture-default-login")
        .env("ANTHROPIC_BASE_URL", "https://invalid.example.test")
        .arg("--data-dir")
        .arg(directory.path().join("data"))
        .args(["--json", "prompt"])
        .arg(directory.path().join("design.pen"))
        .args(["--create", "--non-interactive", "--adapter"])
        .arg(python())
        .arg(format!("--adapter-arg={}", fixture().display()))
        .arg("--adapter-arg=env")
        .arg("日本語")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-openai-secret"));
}

#[test]
fn bad_protocol_and_crashing_peers_fail_without_hanging() {
    for mode in ["wrong-version", "crash", "malformed", "bad-envelope"] {
        let directory = TempDir::new().unwrap();
        let output = cli(&directory, mode, &["--create"]);
        assert_eq!(
            output.status.code(),
            Some(5),
            "mode={mode}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            !events(&output)
                .iter()
                .any(|event| event["type"] == "complete")
        );
    }
}

#[test]
fn create_never_overwrites_and_missing_adapter_is_diagnosed() {
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("design.pen");
    std::fs::write(&file, "preserve me").unwrap();
    let output = cli(&directory, "echo", &["--create"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "preserve me");
    let directory = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(directory.path().join("data"))
        .args(["--json", "prompt"])
        .arg(directory.path().join("design.pen"))
        .args(["--create", "--adapter"])
        .arg("lazypencil-adapter-that-does-not-exist")
        .arg("test")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
    assert_eq!(
        events(&output).last().unwrap()["error"]["code"],
        "AGENT_UNAVAILABLE"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn image_prompt_imports_registered_attachment_and_returns_real_preview() {
    let directory = TempDir::new().unwrap();
    let image = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png");
    assert!(image.is_file(), "{image:?}");
    let output = cli(
        &directory,
        "image-edit",
        &["--create", "--image", image.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        std::fs::read_to_string(directory.path().join("fixture-errors.log")).unwrap_or_default()
    );
    let values = events(&output);
    let complete = values
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    let transcript = std::fs::read_to_string(complete["transcript"].as_str().unwrap()).unwrap();
    assert!(!transcript.contains("iVBORw0"));
    let document: Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("design.pen")).unwrap(),
    )
    .unwrap();
    let uri = document["children"][0]["fill"]["url"].as_str().unwrap();
    assert!(directory.path().join(uri).is_file());
}

#[test]
#[ignore = "Requires an explicitly installed codex-acp; creates a temporary native session without sending an LLM prompt"]
fn installed_codex_adapter_negotiates_v1_and_bound_mcp() {
    installed_adapter("LAZYPENCIL_TEST_CODEX_ACP", "codex-acp");
}

#[test]
#[ignore = "Requires an explicitly installed claude-agent-acp; creates a temporary native session without sending an LLM prompt"]
fn installed_claude_adapter_negotiates_v1_and_bound_mcp() {
    installed_adapter("LAZYPENCIL_TEST_CLAUDE_ACP", "claude-agent-acp");
}

fn installed_adapter(variable: &str, default: &str) {
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("design.pen");
    let store = lazypencil::store::Store::init(&file, Some(directory.path().join("data"))).unwrap();
    runtime().block_on(async {
        let adapter = std::env::var_os(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| default.into());
        let mut opts = AgentOptions::new(adapter, directory.path());
        opts.model = std::env::var("LAZYPENCIL_TEST_AGENT_MODEL").ok();
        opts.session_mode = std::env::var("LAZYPENCIL_TEST_AGENT_MODE").ok();
        opts.require_auth_status = true;
        opts.mcp_servers = vec![agent_client_protocol::schema::v1::McpServer::Stdio(
            agent_client_protocol::schema::v1::McpServerStdio::new(
                "lazypencil",
                env!("CARGO_BIN_EXE_lazypencil"),
            )
            .args(vec![
                "--data-dir".into(),
                store.data_dir().to_str().unwrap().into(),
                "mcp".into(),
                "--file".into(),
                file.to_str().unwrap().into(),
            ]),
        )];
        let mut host = AgentHost::spawn(opts).unwrap();
        let mut initialized = false;
        loop {
            let event = message(&mut host).await;
            match event["type"].as_str() {
                Some("initialized") => {
                    assert_eq!(event["response"]["protocolVersion"], 1);
                    initialized = true;
                }
                Some("ready") => {
                    assert!(initialized);
                    break;
                }
                Some("connection_error" | "turn_error") => {
                    panic!("Adapter setup failed: {}", event["error"]["message"])
                }
                // Authentication status and command lists may arrive during session setup.
                _ => {}
            }
        }
        host.close().await.unwrap();
    });
    assert_eq!(store.read().unwrap().value()["children"], json!([]));
}
