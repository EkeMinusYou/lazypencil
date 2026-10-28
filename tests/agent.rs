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

#[path = "support/fixture.rs"]
mod fixture_support;

fn helper() -> PathBuf {
    fixture_support::helper()
}
fn fixture() -> PathBuf {
    "acp".into()
}
fn options(mode: &str, directory: &Path) -> AgentOptions {
    let mut options = AgentOptions::new(helper(), directory);
    options.supervisor_executable = Some(env!("CARGO_BIN_EXE_lazypencil").into());
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
fn embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits() {
    use clap::Parser;
    use lazypencil::conversation::{Conversation, ConversationEvent};

    let directory = TempDir::new().unwrap();
    let file = directory.path().join("design.pen");
    let arguments = lazypencil::cli::Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        file.clone().into_os_string(),
        "--create".into(),
        "--adapter".into(),
        helper().into_os_string(),
        format!("--adapter-arg={}", fixture().display()).into(),
        "--adapter-arg=edit".into(),
        "--non-interactive".into(),
        "--timeout=5".into(),
        "日本語の変更をしてください".into(),
    ])
    .unwrap();
    let lazypencil::cli::Command::Prompt(args) = arguments.command else {
        unreachable!()
    };
    let mut conversation = Conversation::open_with_executable(
        args,
        Some(directory.path().join("data")),
        env!("CARGO_BIN_EXE_lazypencil").into(),
    )
    .unwrap();
    runtime().block_on(async {
        // This runs without Skia and exercises both actual subprocess routes.
        // Always close the host, including when the bounded reproduction fails.
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            conversation.send(None).await?;
            loop {
                if let ConversationEvent::Message(value) = conversation.next_event().await?
                    && matches!(value["type"].as_str(), Some("complete" | "failed"))
                {
                    break conversation.result().cloned().expect("terminal result");
                }
            }
        })
        .await;
        let closed = conversation.close().await;
        result
            .expect("embedded conversation exceeded 10 seconds")
            .unwrap();
        closed.unwrap();
    });
    let document: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(document["children"][0]["name"], "日本語の変更");
    assert_eq!(document["children"][0]["fill"], "#E65100");
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
        .arg(helper())
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
    cli_output(directory, mode, extra, true)
}

fn cli_output(directory: &TempDir, mode: &str, extra: &[&str], json: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command.arg("--data-dir").arg(directory.path().join("data"));
    if json {
        command.arg("--json");
    }
    command
        .arg("prompt")
        .arg(directory.path().join("design.pen"))
        .args(["--adapter"])
        .arg(helper())
        .arg(format!("--adapter-arg={}", fixture().display()))
        .arg(format!("--adapter-arg={mode}"))
        .args(["--non-interactive", "--timeout", "10"])
        .args(extra)
        .arg("日本語でデザインを作成してください")
        .output()
        .unwrap()
}

#[test]
fn adapter_errors_keep_numeric_codes_without_echoing_private_payloads_or_host_error_codes() {
    let cases: [(&str, &[&str], &str); 8] = [
        ("protocol-error-initialize", &[], "initialize"),
        (
            "protocol-error-authenticate",
            &["--auth-method", "fixture-auth"],
            "authenticate",
        ),
        ("protocol-error-new", &[], "session/new"),
        ("protocol-error-load", &[], "session/load"),
        (
            "protocol-error-mode",
            &["--mode", "workspace-write"],
            "session/set_mode",
        ),
        (
            "protocol-error-model",
            &["--model", "fixture-model-2"],
            "session/set_config_option",
        ),
        ("protocol-error-prompt", &[], "session/prompt"),
        ("protocol-error-frame", &[], "initialize"),
    ];
    for (mode, extra, phase) in cases {
        for json in [true, false] {
            // Setup phases and prompt failures use the same public entry point
            // as a real adapter. Every case owns its data, transcript and child.
            let directory = TempDir::new().unwrap();
            let file = directory.path().join("design.pen");
            let original = br##"{"version":"2.21","children":[{"id":"keep","type":"rectangle","width":40,"height":20,"fill":"#2563EB"}]}"##;
            std::fs::write(&file, original).unwrap();
            let session = if mode == "protocol-error-load" {
                let first = cli(&directory, "auth-account", &[]);
                assert!(
                    first.status.success(),
                    "{}",
                    String::from_utf8_lossy(&first.stdout)
                );
                Some(
                    events(&first)
                        .into_iter()
                        .find(|event| event["type"] == "complete")
                        .unwrap()["session"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            } else {
                None
            };
            let mut extra = extra.to_vec();
            if let Some(session) = &session {
                extra.extend(["--session", session]);
            }
            let output = cli_output(&directory, mode, &extra, json);
            assert!(!output.status.success(), "{mode}");
            let rejected: Value = serde_json::from_slice(
                &std::fs::read(directory.path().join("rejected-acp-phase.json")).unwrap_or_else(|error| {
                    panic!("{mode}, json={json}: missing rejection record: {error}; stdout={}; stderr={}; fixture={}",
                        String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr),
                        std::fs::read_to_string(directory.path().join("fixture-errors.log")).unwrap_or_default())
                }),
            )
            .unwrap();
            assert_eq!(rejected["phase"], phase, "{mode}");
            assert_eq!(std::fs::read(&file).unwrap(), original, "{mode}");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            for text in [&*stdout, &*stderr] {
                assert!(!text.contains("MUST_NOT_ECHO"), "{mode}");
                assert!(!text.contains("fixture-private-credential"), "{mode}");
                assert!(!text.contains("private document excerpt"), "{mode}");
            }
            if !json {
                assert!(stderr.contains("AGENT_ERROR"), "{mode}: {stderr}");
                continue;
            }
            let values = events(&output);
            let failed = values
                .iter()
                .find(|event| event["type"] == "failed")
                .unwrap();
            let error = &failed["error"];
            assert_eq!(error["code"], "AGENT_ERROR", "{mode}");
            if mode != "protocol-error-frame" {
                assert_eq!(error["details"]["phase"], phase, "{mode}: {error}");
                assert_eq!(
                    error["details"]["acp_error_code"], -32603,
                    "{mode}: {error}"
                );
            }
            let transcript = Path::new(failed["transcript"].as_str().unwrap());
            let metadata = transcript.parent().unwrap().join("session.json");
            let state: Value = serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
            assert_eq!(
                state["draft"], "日本語でデザインを作成してください",
                "{mode}"
            );
            for path in [transcript, metadata.as_path()] {
                let text = std::fs::read_to_string(path).unwrap();
                assert!(!text.contains("MUST_NOT_ECHO"), "{mode}");
                assert!(!text.contains("fixture-private-credential"), "{mode}");
                assert!(!text.contains("private document excerpt"), "{mode}");
            }
        }
    }
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
        .arg(helper())
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

#[test]
fn session_restart_continues_from_current_document_and_bounded_history_without_replaying_edits() {
    let directory = TempDir::new().unwrap();
    #[cfg(feature = "rendering")]
    let image = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png");
    #[cfg(feature = "rendering")]
    let extra = ["--create", "--image", image.to_str().unwrap()];
    #[cfg(not(feature = "rendering"))]
    let extra = ["--create"];
    let first = cli(&directory, "resume-no-load", &extra);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let first_events = events(&first);
    let complete = first_events
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    let session = complete["session"].as_str().unwrap();
    let transcript = Path::new(complete["transcript"].as_str().unwrap());
    let original_transcript = std::fs::read(transcript).unwrap();
    let conversation = transcript.parent().unwrap();
    let first_metadata: Value =
        serde_json::from_slice(&std::fs::read(conversation.join("session.json")).unwrap()).unwrap();
    let interrupted = uuid::Uuid::new_v4().to_string();
    let start = first_events
        .iter()
        .find(|event| event["type"] == "start")
        .unwrap();
    let mut historical_start = start.clone();
    historical_start["turn"] = json!(interrupted);
    std::fs::write(conversation.join(format!("turn-{interrupted}.jsonl")), format!("{historical_start}\n{}\n{}\n",
        json!({"type":"user_prompt","text":"HISTORICAL_FAILED_REQUEST_DO_NOT_REPEAT"}),
        json!({"type":"failed","session":session,"turn":interrupted,"document":complete["document"]}))).unwrap();
    let path = directory.path().join("design.pen");
    let mut document: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    document["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"outside-edit","type":"rectangle","width":10,"height":10}));
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let current_revision =
        lazypencil::store::Store::open(&path, Some(directory.path().join("data")))
            .unwrap()
            .read()
            .unwrap()
            .revision();
    std::fs::write(directory.path().join("fixture-restart"), "").unwrap();
    let second = cli(&directory, "resume-no-load", &["--session", session]);
    assert!(
        second.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&second.stdout),
        std::fs::read_to_string(directory.path().join("fixture-errors.log")).unwrap()
    );
    let second_events = events(&second);
    let restart = second_events
        .iter()
        .find(|event| event["type"] == "session_restarted")
        .unwrap();
    assert_eq!(restart["reason"], "load_session_unsupported");
    assert_eq!(restart["previous_native_session_id"], "fixture-native");
    assert_eq!(restart["native_session_id"], "fixture-restarted");
    assert_eq!(restart["document_revision"], current_revision);
    let history = restart["history"]["recent_turns"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().any(|turn| turn["status"] == "failed"
        && turn["request"]["text"] == "HISTORICAL_FAILED_REQUEST_DO_NOT_REPEAT"));
    let prompts = std::fs::read_to_string(directory.path().join("received-prompts.jsonl")).unwrap();
    let prompts: Vec<Value> = prompts
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        prompts.len(),
        2,
        "Only the two explicitly requested turns may be sent"
    );
    let context = prompts[1]["prompt"][1]["text"].as_str().unwrap();
    assert!(context.contains("not restored model memory"));
    assert!(context.contains("Do not replay historical prompts or edits"));
    assert!(context.contains(&current_revision));
    assert!(context.contains("HISTORICAL_FAILED_REQUEST_DO_NOT_REPEAT"));
    let methods =
        std::fs::read_to_string(directory.path().join("received-session-methods.jsonl")).unwrap();
    assert_eq!(methods.lines().count(), 2);
    assert!(
        methods
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).unwrap()["method"] == "session/new")
    );
    let current: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(current["children"].as_array().unwrap().len(), 3);
    assert!(
        current["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["id"] == "outside-edit")
    );
    assert_eq!(std::fs::read(transcript).unwrap(), original_transcript);
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(conversation.join("session.json")).unwrap()).unwrap();
    assert_eq!(metadata["native_session_id"], "fixture-restarted");
    assert_eq!(metadata["draft"], "");
    assert_eq!(metadata["attachments"], first_metadata["attachments"]);
    assert!(
        !prompts[1]["prompt"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["type"] == "image")
    );
    #[cfg(feature = "rendering")]
    {
        assert_eq!(metadata["attachments"].as_array().unwrap().len(), 1);
        assert!(
            prompts[0]["prompt"]
                .as_array()
                .unwrap()
                .iter()
                .any(|block| block["type"] == "image")
        );
        let attachment: lazypencil::attachments::Attachment =
            serde_json::from_value(metadata["attachments"][0].clone()).unwrap();
        attachment.read(conversation).unwrap();
    }
}

#[test]
fn session_restart_does_not_hide_session_load_failures_or_send_the_pending_prompt() {
    let directory = TempDir::new().unwrap();
    let first = cli(&directory, "resume-load-error", &["--create"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let first_events = events(&first);
    let complete = first_events
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    let session = complete["session"].as_str().unwrap();
    let directory_path = Path::new(complete["transcript"].as_str().unwrap())
        .parent()
        .unwrap();
    let original = std::fs::read(directory.path().join("design.pen")).unwrap();
    std::fs::write(directory.path().join("fixture-restart"), "").unwrap();
    let second = cli(&directory, "resume-load-error", &["--session", session]);
    assert!(!second.status.success());
    let second_events = events(&second);
    assert!(
        !second_events
            .iter()
            .any(|event| event["type"] == "session_restarted")
    );
    assert!(second_events.iter().any(|event| event["type"] == "failed"));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("received-prompts.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let methods =
        std::fs::read_to_string(directory.path().join("received-session-methods.jsonl")).unwrap();
    assert_eq!(methods.lines().count(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(methods.lines().last().unwrap()).unwrap()["method"],
        "session/load"
    );
    assert_eq!(
        std::fs::read(directory.path().join("design.pen")).unwrap(),
        original
    );
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(directory_path.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["native_session_id"], "fixture-native");
    assert_eq!(metadata["draft"], "日本語でデザインを作成してください");
    let third = cli(
        &directory,
        "resume-load-error",
        &["--session", session, "--restart-session"],
    );
    assert!(
        third.status.success(),
        "{}",
        String::from_utf8_lossy(&third.stdout)
    );
    let third_events = events(&third);
    let restart = third_events
        .iter()
        .find(|event| event["type"] == "session_restarted")
        .unwrap();
    assert_eq!(restart["reason"], "user_requested");
    assert_eq!(restart["native_session_id"], "fixture-restarted");
    assert_eq!(
        std::fs::read_to_string(directory.path().join("received-prompts.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    let current: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("design.pen")).unwrap())
            .unwrap();
    assert_eq!(current["children"].as_array().unwrap().len(), 2);
}

#[test]
fn session_restart_requires_an_explicit_local_conversation_for_cli_and_native_hosts() {
    use clap::Parser;
    let arguments = lazypencil::cli::Arguments::try_parse_from([
        "lazypencil",
        "prompt",
        "design.pen",
        "--restart-session",
        "Continue",
    ]);
    assert!(arguments.is_err());
    let directory = TempDir::new().unwrap();
    let mut options = options("ok", directory.path());
    options.restart_session = true;
    let error = AgentHost::spawn(options)
        .err()
        .expect("Missing native session must fail before spawning");
    assert_eq!(error.code, "INVALID_ARGUMENT");
    assert!(!directory.path().join("fixture-errors.log").exists());
}

#[test]
fn session_restart_cancellation_before_sending_preserves_context_for_the_next_loaded_session() {
    use clap::Parser;
    use lazypencil::conversation::{Conversation, ConversationEvent};
    let directory = TempDir::new().unwrap();
    let first = cli(&directory, "resume-native", &["--create"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let first_events = events(&first);
    let complete = first_events
        .iter()
        .find(|event| event["type"] == "complete")
        .unwrap();
    let session = complete["session"].as_str().unwrap();
    let state_directory = Path::new(complete["transcript"].as_str().unwrap())
        .parent()
        .unwrap();
    let document_path = directory.path().join("design.pen");
    let original = std::fs::read(&document_path).unwrap();
    std::fs::write(directory.path().join("fixture-restart"), "").unwrap();
    let arguments = lazypencil::cli::Arguments::try_parse_from([
        "lazypencil".into(),
        "prompt".into(),
        document_path.clone().into_os_string(),
        "--adapter".into(),
        helper().into_os_string(),
        format!("--adapter-arg={}", fixture().display()).into(),
        "--adapter-arg=resume-native".into(),
        "--session".into(),
        session.into(),
        "--restart-session".into(),
        "--non-interactive".into(),
        "日本語の中断される依頼".into(),
    ])
    .unwrap();
    let lazypencil::cli::Command::Prompt(args) = arguments.command else {
        unreachable!()
    };
    let mut conversation = Conversation::open_with_executable(
        args,
        Some(directory.path().join("data")),
        env!("CARGO_BIN_EXE_lazypencil").into(),
    )
    .unwrap();
    runtime().block_on(async {
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            conversation.send(None).await.unwrap();
            let mut restarted = false;
            loop {
                if let ConversationEvent::Message(event) = conversation.next_event().await.unwrap()
                {
                    if event["type"] == "initialized" {
                        // Cancel before Conversation handles ready and enqueues
                        // the turn, independently of adapter task scheduling.
                        conversation.cancel();
                    }
                    if event["type"] == "session_restarted" {
                        restarted = true;
                        let saved: Value = serde_json::from_slice(
                            &std::fs::read(state_directory.join("session.json")).unwrap(),
                        )
                        .unwrap();
                        assert_eq!(saved["native_session_id"], "fixture-restarted");
                        assert_eq!(saved["continuity_pending"], true);
                    }
                    if event["type"] == "failed" {
                        assert!(restarted);
                        break;
                    }
                }
            }
        })
        .await;
        conversation.close().await.unwrap();
        result.expect("Restart/cancellation exceeded ten seconds");
    });
    drop(conversation);
    assert_eq!(std::fs::read(&document_path).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("received-prompts.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let pending: Value =
        serde_json::from_slice(&std::fs::read(state_directory.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(pending["native_session_id"], "fixture-restarted");
    assert_eq!(pending["continuity_pending"], true);
    assert_eq!(pending["draft"], "日本語の中断される依頼");
    let third = cli(&directory, "resume-native", &["--session", session]);
    assert!(
        third.status.success(),
        "{}",
        String::from_utf8_lossy(&third.stdout)
    );
    let third_events = events(&third);
    assert!(
        !third_events
            .iter()
            .any(|event| event["type"] == "session_restarted")
    );
    let context = third_events
        .iter()
        .find(|event| event["type"] == "local_context")
        .unwrap();
    assert_eq!(context["native_session_id"], "fixture-restarted");
    assert_eq!(context["reason"], "restart_pending");
    assert_eq!(
        third_events
            .iter()
            .find(|event| event["type"] == "ready")
            .unwrap()["session_origin"],
        "loaded"
    );
    let prompts = std::fs::read_to_string(directory.path().join("received-prompts.jsonl")).unwrap();
    let prompts: Vec<Value> = prompts
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[1]["prompt"][1]["text"]
            .as_str()
            .unwrap()
            .contains("prior restart did not complete")
    );
    assert!(
        prompts[1]["prompt"][1]["text"]
            .as_str()
            .unwrap()
            .contains("Do not replay historical prompts or edits")
    );
    let final_state: Value =
        serde_json::from_slice(&std::fs::read(state_directory.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(final_state["continuity_pending"], false);
    assert_eq!(final_state["draft"], "");
    let current: Value = serde_json::from_slice(&std::fs::read(&document_path).unwrap()).unwrap();
    assert_eq!(current["children"].as_array().unwrap().len(), 2);
    let fourth = cli(&directory, "resume-native", &["--session", session]);
    assert!(
        fourth.status.success(),
        "{}",
        String::from_utf8_lossy(&fourth.stdout)
    );
    assert!(!events(&fourth).iter().any(|event| matches!(
        event["type"].as_str(),
        Some("local_context" | "session_restarted")
    )));
    let prompts = std::fs::read_to_string(directory.path().join("received-prompts.jsonl")).unwrap();
    let prompts: Vec<Value> = prompts
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(prompts.len(), 3);
    assert_eq!(
        prompts[2]["prompt"][1]["text"],
        "日本語でデザインを作成してください"
    );
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
        .arg(helper())
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
        .arg(helper())
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
        opts.supervisor_executable = Some(env!("CARGO_BIN_EXE_lazypencil").into());
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

#[cfg(feature = "rendering")]
#[test]
#[ignore = "Requires sandboxed Chromium; exercises the complete supervised adapter/MCP process tree"]
fn supervised_agent_mcp_renders_script_browser_and_shader_pixels() {
    use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
    use skia_safe::{Color, Data, Image};
    let directory = TempDir::new().unwrap();
    let file = directory.path().join("design.pen");
    std::fs::write(directory.path().join("shape.js"), "/** @schema 2.21 */ return [{type:'rectangle',width:pencil.width,height:pencil.height,fill:'#ff0000'}];").unwrap();
    std::fs::write(
        directory.path().join("blue.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(0.0,0.0,1.0,1.0);}",
    )
    .unwrap();
    std::fs::write(&file, json!({"version":"2.21","children":[{"id":"canvas","type":"frame","width":300,"height":100,"layout":"none","fill":"#ffffff","children":[
        {"id":"page","type":"browser","url":"data:text/html,<style>html,body{margin:0;background:%2300ff00}</style>","width":100,"height":100},
        {"id":"shader","type":"rectangle","x":100,"width":100,"height":100,"fill":{"type":"shader","url":"blue.glsl"}},
        {"id":"script","type":"script","x":200,"width":100,"height":100,"scriptUri":"shape.js"}
    ]}]}).to_string()).unwrap();
    let original = std::fs::read(&file).unwrap();
    let store = lazypencil::store::Store::open(&file, Some(directory.path().join("data"))).unwrap();
    let chromium =
        std::env::var("LAZYPENCIL_TEST_CHROMIUM").expect("Pinned test Chromium executable");
    runtime().block_on(async {
        let mut opts = options("supervised-dynamic", directory.path());
        opts.setup_timeout = Duration::from_secs(20);
        opts.turn_timeout = Duration::from_secs(60);
        opts.mcp_servers = vec![McpServer::Stdio(
            McpServerStdio::new("lazypencil", env!("CARGO_BIN_EXE_lazypencil"))
                .args(vec![
                    "--data-dir".into(),
                    store.data_dir().to_str().unwrap().into(),
                    "mcp".into(),
                    "--file".into(),
                    file.to_str().unwrap().into(),
                ])
                .env(vec![EnvVariable::new("LAZYPENCIL_CHROMIUM", chromium)]),
        )];
        let mut host = AgentHost::spawn(opts).unwrap();
        ready(&mut host).await;
        host.prompt(prompt(), CancellationToken::new())
            .await
            .unwrap();
        let event = tokio::time::timeout(Duration::from_secs(70), host.next_event())
            .await
            .unwrap()
            .unwrap();
        let AgentEvent::Message(event) = event else {
            panic!("Unexpected permission event")
        };
        assert_eq!(
            event["type"],
            "update",
            "{event}\n{}",
            std::fs::read_to_string(directory.path().join("fixture-errors.log"))
                .unwrap_or_default()
        );
        assert_eq!(message(&mut host).await["type"], "turn_complete");
        let png = std::fs::read(directory.path().join("supervised-dynamic.png")).unwrap();
        let raster = Image::from_encoded(Data::new_copy(&png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!((raster.width(), raster.height()), (300, 100));
        let pixels = raster.peek_pixels().unwrap();
        assert_eq!(pixels.get_color((50, 50)), Color::GREEN);
        assert_eq!(pixels.get_color((150, 50)), Color::BLUE);
        assert_eq!(pixels.get_color((250, 50)), Color::RED);
        let report: Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("supervised-dynamic.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["diagnostics"], json!([]));
        let artifacts = report["dynamic_artifacts"].as_array().unwrap();
        assert_eq!(artifacts.len(), 2);
        for kind in ["browser", "shader"] {
            assert!(
                artifacts
                    .iter()
                    .any(|artifact| artifact["acquisition"]["kind"] == kind)
            );
        }
        host.close().await.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), original);
        if let Some(output) = std::env::var_os("LAZYPENCIL_TEST_SUPERVISION_DIR") {
            let output = PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(output.join("chromium-supervised.png"), &png).unwrap();
            std::fs::write(output.join("chromium-supervised.json"), report.to_string()).unwrap();
        }
    });
}
#[cfg(unix)]
#[path = "support/unix_agent_ownership.rs"]
mod agent_ownership;

#[cfg(unix)]
#[path = "support/unix_browser_ownership.rs"]
mod browser_ownership;
