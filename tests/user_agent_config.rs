use lazypencil::config::UserSnapshot;
use serde_json::Value;
#[cfg(unix)]
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture_support;

struct Fixture {
    temp: TempDir,
    user: PathBuf,
    document: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let user = temp.path().join("user settings");
        let document = temp.path().join("project/nested/design.pen");
        fs::create_dir_all(document.parent().unwrap()).unwrap();
        fs::create_dir_all(temp.path().join("project/.git")).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::write(&document, r#"{"version":"2.21","children":[]}"#).unwrap();
        Self {
            temp,
            user,
            document,
        }
    }
    fn settings(&self, body: &str) {
        fs::write(self.user.join("config.toml"), body).unwrap();
    }
    fn peer(&self, agent: &str, mode: &str, extra: &str) -> String {
        let helper = fixture_support::helper();
        format!(
            "[agents.{agent}]\nadapter={}\nadapter_args=[{},{}]\n{extra}\n",
            quoted(helper.to_str().unwrap()),
            quoted("acp"),
            quoted(mode)
        )
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        command
            .env("LAZYPENCIL_CONFIG_DIR", &self.user)
            .current_dir(self.temp.path())
            .arg("--json")
            .arg("--data-dir")
            .arg(self.temp.path().join("data"));
        command
    }
    fn prompt(&self, arguments: &[&str]) -> Output {
        self.command()
            .arg("prompt")
            .arg(&self.document)
            .arg("設定を確認してください")
            .args(["--non-interactive", "--timeout", "5"])
            .args(arguments)
            .output()
            .unwrap()
    }
    fn events(&self, arguments: &[&str]) -> Vec<Value> {
        let output = self.prompt(arguments);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
fn quoted(value: &str) -> String {
    // JSON basic strings are valid TOML for these text/path fixtures.
    serde_json::to_string(value).unwrap()
}
fn event<'a>(events: &'a [Value], kind: &str) -> &'a Value {
    events.iter().find(|event| event["type"] == kind).unwrap()
}

#[test]
fn bound_mcp_memory_policy_is_platform_scoped_preserves_overrides_and_never_changes_the_adapter() {
    for agent in ["codex", "claude"] {
        for explicit in [None, Some("0"), Some("1")] {
            let f = Fixture::new();
            f.settings(&f.peer(agent, "mcp-environment", ""));
            let original = fs::read(&f.document).unwrap();
            let mut command = f.command();
            command
                .arg("prompt")
                .arg(&f.document)
                .args([
                    "Inspect the MCP configuration",
                    "--agent",
                    agent,
                    "--non-interactive",
                    "--timeout",
                    "5",
                ])
                .env_remove("MallocSpaceEfficient");
            if let Some(value) = explicit {
                command.env("MallocSpaceEfficient", value);
            }
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let received: Value = serde_json::from_slice(
                &fs::read(
                    f.document
                        .parent()
                        .unwrap()
                        .join("received-mcp-environment.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                received["adapter_malloc_space_efficient"].as_str(),
                explicit
            );
            let servers = received["servers"].as_array().unwrap();
            assert_eq!(servers.len(), 1);
            let environment = servers[0]["env"].as_array().unwrap();
            assert!(
                environment
                    .iter()
                    .any(|entry| entry["name"] == "LAZYPENCIL_CONFIG_DIR")
            );
            let allocator = environment
                .iter()
                .find(|entry| entry["name"] == "MallocSpaceEfficient");
            if cfg!(target_os = "macos") {
                assert_eq!(allocator.unwrap()["value"], explicit.unwrap_or("1"));
            } else {
                assert!(allocator.is_none());
            }
            assert_eq!(fs::read(&f.document).unwrap(), original);
        }
    }
}

#[test]
fn cli_uses_the_selected_user_profile_and_negotiates_its_model_mode_and_authentication_method() {
    let f = Fixture::new();
    f.settings(&format!(
        "[agents]\ndefault='claude'\n{}",
        f.peer(
            "claude",
            "settings-auth-check",
            "model='fixture-model-2'\nmode='workspace-write'\nauth_method='fixture-auth'\ntimeout=7"
        )
    ));
    let original = fs::read(&f.document).unwrap();
    let events = f.events(&[]);
    assert_eq!(event(&events, "start")["agent"], "claude");
    assert_eq!(event(&events, "start")["requested_auth"], "subscription");
    let ready = event(&events, "ready");
    assert_eq!(
        ready["response"]["modes"]["currentModeId"],
        "workspace-write"
    );
    assert_eq!(
        ready["response"]["configOptions"][0]["currentValue"],
        "fixture-model-2"
    );
    assert_eq!(
        event(&events, "turn_complete")["response"]["stopReason"],
        "end_turn"
    );
    assert_eq!(fs::read(&f.document).unwrap(), original);
}

#[test]
fn explicit_builtin_agent_and_auth_values_override_user_defaults_without_opening_other_adapters() {
    let f = Fixture::new();
    f.settings(&format!(
        "[agents]\ndefault='claude'\n[agents.claude]\nadapter='missing/never-launch-this'\n{}",
        f.peer(
            "codex",
            "auth-account",
            "auth='api'\nrequire_auth_status=true"
        )
    ));
    let events = f.events(&["--agent", "codex", "--auth", "subscription"]);
    assert_eq!(event(&events, "start")["agent"], "codex");
    assert_eq!(
        event(&events, "complete")["authentication"]["source"],
        "subscription"
    );
}

#[test]
fn user_api_route_is_checked_against_the_peer_report() {
    let f = Fixture::new();
    f.settings(&f.peer("codex", "auth-api", "auth='api'\nrequire_auth_status=true"));
    let events = f.events(&[]);
    assert_eq!(event(&events, "start")["requested_auth"], "api");
    assert_eq!(
        event(&events, "complete")["authentication"]["source"],
        "api_key"
    );
}

#[test]
fn configured_timeout_cancels_a_live_request_without_changing_the_document() {
    let f = Fixture::new();
    f.settings(&f.peer("codex", "cancel", "timeout=1"));
    let original = fs::read(&f.document).unwrap();
    let output = f
        .command()
        .arg("prompt")
        .arg(&f.document)
        .arg("Wait for cancellation")
        .arg("--non-interactive")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let events: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(event(&events, "failed")["error"]["code"], "AGENT_ERROR");
    assert!(
        event(&events, "failed")["error"]["message"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    assert!(f.document.parent().unwrap().join("sent-turn.txt").exists());
    assert_eq!(fs::read(&f.document).unwrap(), original);
}

#[test]
fn reload_keeps_the_saved_agent_identity_and_rejects_changed_profile_arguments() {
    let f = Fixture::new();
    f.settings(&format!(
        "[agents]\ndefault='claude'\n{}",
        f.peer("claude", "normal", "")
    ));
    let first = f.events(&[]);
    let session = event(&first, "start")["session"].as_str().unwrap();
    let reloaded = f.events(&["--session", session]);
    assert_eq!(event(&reloaded, "start")["session"], session);
    assert_eq!(
        event(&reloaded, "start")["native_session_id"],
        "fixture-native"
    );
    f.settings(&format!(
        "[agents]\ndefault='claude'\n{}",
        f.peer("claude", "auth-account", "")
    ));
    fs::remove_file(f.document.parent().unwrap().join("sent-turn.txt")).unwrap();
    let output = f.prompt(&["--session", session]);
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
    assert!(!f.document.parent().unwrap().join("sent-turn.txt").exists());
}

#[test]
fn explicit_adapter_replaces_the_configured_executable_and_its_entire_argument_list() {
    let f = Fixture::new();
    f.settings("[agents.codex]\nadapter='missing/never-launch-this'\nadapter_args=['DO_NOT_INHERIT']\nrequire_auth_status=true\n");
    let helper = fixture_support::helper();
    let events = f.events(&[
        "--adapter",
        helper.to_str().unwrap(),
        "--adapter-arg=acp",
        "--adapter-arg=auth-account",
    ]);
    assert_eq!(
        event(&events, "complete")["authentication"]["source"],
        "subscription"
    );
}

#[test]
fn strict_authentication_default_can_be_explicitly_disabled_without_consuming_positional_text() {
    let f = Fixture::new();
    f.settings(&f.peer("codex", "normal", "require_auth_status=true"));
    let output = f.prompt(&[]);
    assert!(!output.status.success());
    assert!(!f.document.parent().unwrap().join("sent-turn.txt").exists());
    let events = f.events(&["--require-auth-status=false"]);
    assert!(event(&events, "complete")["authentication"].is_null());
}

#[test]
fn configured_document_tool_policy_controls_actual_correlated_permission_requests() {
    let f = Fixture::new();
    f.settings(&f.peer("codex", "configured-document-ask", "document_tools='ask'"));
    let output = f.prompt(&[]);
    assert!(!output.status.success());
    let events: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        event(&events, "permission_response")["document_tool_preauthorized"],
        false
    );
    assert_eq!(event(&events, "failed")["error"]["code"], "AGENT_ERROR");
    f.settings(&f.peer("codex", "correlated-document", "document_tools='ask'"));
    let events = f.events(&["--document-tools", "allow"]);
    assert_eq!(
        event(&events, "permission_response")["document_tool_preauthorized"],
        true
    );
}

#[test]
fn invalid_profiles_are_rejected_even_when_unselected_and_errors_do_not_quote_values() {
    let f = Fixture::new();
    for body in [
        "[agents]\ndefault='SECRET_UNKNOWN_AGENT'".into(),
        "[agents.claude]\napi_key='SECRET_KEY'".into(),
        "[agents.claude]\nadapter_args=['SECRET_ARG']".into(),
        "[agents.claude]\nadapter='https://SECRET_HOST/peer'".into(),
        "[agents.claude]\nmodel=\"SECRET\\nVALUE\"".into(),
        "[agents.claude]\ntimeout=0".into(),
        "[agents.claude]\ntimeout=86401".into(),
        format!("[agents.claude]\nmode={}", quoted(&"SECRET".repeat(44))),
        format!(
            "[agents.claude]\nadapter='peer'\nadapter_args=[{}]",
            quoted(&"SECRET".repeat(690))
        ),
    ] {
        f.settings(&body);
        let output = f.prompt(&["--agent", "codex"]);
        assert!(!output.status.success(), "{body}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["error"]["code"],
            "CONFIGURATION_ERROR"
        );
        assert!(!text.contains("SECRET"), "{text}");
    }
}

#[test]
fn config_schema_and_template_separate_user_preferences_from_project_execution() {
    let f = Fixture::new();
    let output = f
        .command()
        .args(["config", "schema", "--user"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let schema: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(schema["properties"]["agents"].is_object());
    assert_eq!(
        schema["$defs"]["AgentProfile"]["properties"]["timeout"]["minimum"],
        1
    );
    fs::remove_file(f.user.join("config.toml")).ok();
    let output = f
        .command()
        .args(["config", "init", "--user"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshot = UserSnapshot::load_file(Some(f.user.join("config.toml"))).unwrap();
    assert_eq!(snapshot.settings.agents.default, None);
    let text = fs::read_to_string(f.user.join("config.toml")).unwrap();
    assert!(text.contains("# [agents.codex]"));
    let output = f.command().args(["config", "schema"]).output().unwrap();
    let schema: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(schema["properties"]["agents"].is_object());
    fs::write(
        f.temp.path().join("project/lazypencil.toml"),
        "[agents.codex]\nadapter='SECRET_PROJECT_EXECUTABLE'\n",
    )
    .unwrap();
    let output = f
        .command()
        .args(["config", "show"])
        .arg(&f.document)
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["agent_execution"]["trusted"], false);
    assert_eq!(
        value["settings"]["agents"]["codex"]["adapter"],
        "SECRET_PROJECT_EXECUTABLE"
    );
    assert!(
        !f.temp.path().join("data").exists(),
        "Reading project settings must not create grants"
    );
    let output = f.prompt(&[]);
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "PROJECT_NOT_TRUSTED");
    assert!(!value.to_string().contains("SECRET"));
}

#[test]
fn project_profiles_require_a_separate_grant_and_preserve_user_and_cli_precedence() {
    let f = Fixture::new();
    f.settings(&format!(
        "[agents]\ndefault='codex'\n[agents.codex]\nadapter='missing/never-launch-this'\n{}",
        f.peer(
            "claude",
            "settings-auth-check",
            "mode='workspace-write'\nauth_method='fixture-auth'"
        )
    ));
    let configuration = f.temp.path().join("project/lazypencil.toml");
    fs::write(
        &configuration,
        "[agents]\ndefault='claude'\n[agents.claude]\nmodel='fixture-model-2'\n",
    )
    .unwrap();
    let show = f
        .command()
        .args(["config", "show"])
        .arg(&f.document)
        .arg("--executables")
        .output()
        .unwrap();
    assert!(show.status.success());
    let shown: Value = serde_json::from_slice(&show.stdout).unwrap();
    let fingerprint = shown["agent_execution"]["fingerprint"].as_str().unwrap();
    assert_eq!(shown["agent_execution"]["trusted"], false);
    let design_grant = f
        .command()
        .args(["config", "trust"])
        .arg(&f.document)
        .output()
        .unwrap();
    assert!(design_grant.status.success());
    let denied = f.prompt(&[]);
    assert_eq!(
        serde_json::from_slice::<Value>(&denied.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    let stale = f
        .command()
        .args(["config", "trust"])
        .arg(&f.document)
        .args(["--executables", "--expected-fingerprint", &"0".repeat(64)])
        .output()
        .unwrap();
    assert!(!stale.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&stale.stdout).unwrap()["error"]["code"],
        "REVISION_CONFLICT"
    );
    let granted = f
        .command()
        .args(["config", "trust"])
        .arg(&f.document)
        .args(["--executables", "--expected-fingerprint", fingerprint])
        .output()
        .unwrap();
    assert!(granted.status.success());
    let original = fs::read(&f.document).unwrap();
    let events = f.events(&[]);
    assert_eq!(event(&events, "start")["agent"], "claude");
    assert_eq!(
        event(&events, "start")["project_agent_execution"]["fingerprint"],
        fingerprint
    );
    assert_eq!(
        event(&events, "start")["project_agent_execution"]["scope"],
        "agent_execution"
    );
    assert_eq!(
        event(&events, "ready")["response"]["configOptions"][0]["currentValue"],
        "fixture-model-2"
    );
    assert_eq!(
        event(&events, "ready")["response"]["modes"]["currentModeId"],
        "workspace-write"
    );
    let revoked = f
        .command()
        .args(["config", "untrust"])
        .arg(&f.document)
        .output()
        .unwrap();
    assert!(revoked.status.success());
    assert!(
        serde_json::from_slice::<Value>(&revoked.stdout).unwrap()["removed_grants"]
            .as_u64()
            .unwrap()
            >= 2
    );
    let denied = f.prompt(&[]);
    assert_eq!(
        serde_json::from_slice::<Value>(&denied.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    let helper = fixture_support::helper();
    let explicit = f.events(&[
        "--agent",
        "codex",
        "--adapter",
        helper.to_str().unwrap(),
        "--adapter-arg=acp",
        "--adapter-arg=normal",
    ]);
    assert_eq!(event(&explicit, "start")["agent"], "codex");
    assert!(event(&explicit, "start")["project_agent_execution"].is_null());
    assert_eq!(fs::read(&f.document).unwrap(), original);

    fs::write(&configuration, "[agents]\n").unwrap();
    let output = f
        .command()
        .args(["config", "show"])
        .arg(&f.document)
        .arg("--executables")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["agent_execution"]["configured"], false);
    assert_eq!(value["agent_execution"]["trusted"], true);
    assert!(value["agent_execution"]["fingerprint"].is_null());

    fs::write(
        &configuration,
        "[fonts]\nfiles=['missing-font.ttf']\n[agents.codex]\nadapter='never-execute'\n",
    )
    .unwrap();
    let output = f
        .command()
        .args(["config", "show"])
        .arg(&f.document)
        .arg("--executables")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Agent review must not require font loading: {output:?}"
    );
}

#[cfg(unix)]
#[test]
fn project_execution_grants_bind_physical_configuration_and_do_not_cross_project_roots() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let physical = f.temp.path().join("physical project settings");
    fs::create_dir_all(physical.join("bin")).unwrap();
    symlink(
        fixture_support::helper().canonicalize().unwrap(),
        physical.join("bin/peer"),
    )
    .unwrap();
    let body = "[agents]\ndefault='claude'\n[agents.claude]\nadapter='bin/peer'\nadapter_args=['acp','settings-auth-check']\nmodel='fixture-model-2'\nmode='workspace-write'\nauth_method='fixture-auth'\n";
    let source = physical.join("actual.toml");
    fs::write(&source, body).unwrap();
    symlink(&source, f.temp.path().join("project/lazypencil.toml")).unwrap();
    let output = f
        .command()
        .args(["config", "trust"])
        .arg(&f.document)
        .arg("--executables")
        .output()
        .unwrap();
    assert!(output.status.success());
    let granted: Value = serde_json::from_slice(&output.stdout).unwrap();
    let grant_path = PathBuf::from(granted["grant_path"].as_str().unwrap());
    // Unrelated data settings do not broaden or invalidate the execution grant.
    fs::write(&source, format!("{body}[fonts]\nlocale='en'\n")).unwrap();
    let events = f.events(&[]);
    assert_eq!(event(&events, "start")["agent"], "claude");
    assert_eq!(
        event(&events, "ready")["response"]["configOptions"][0]["currentValue"],
        "fixture-model-2"
    );
    let second = f.temp.path().join("second");
    fs::create_dir_all(second.join(".git")).unwrap();
    let document = second.join("design.pen");
    fs::copy(&f.document, &document).unwrap();
    symlink(&source, second.join("lazypencil.toml")).unwrap();
    let denied = f
        .command()
        .arg("prompt")
        .arg(&document)
        .args(["do not execute", "--non-interactive"])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&denied.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    let mut wrong_scope: Value = serde_json::from_slice(&fs::read(&grant_path).unwrap()).unwrap();
    wrong_scope["scope"] = Value::String("design_instructions".into());
    fs::write(&grant_path, wrong_scope.to_string()).unwrap();
    let denied = f.prompt(&[]);
    assert_eq!(
        serde_json::from_slice::<Value>(&denied.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    let output = f
        .command()
        .args(["config", "trust"])
        .arg(&f.document)
        .arg("--executables")
        .output()
        .unwrap();
    assert!(output.status.success());
    fs::write(
        &source,
        body.replace("model='fixture-model-2'", "model='fixture-model'"),
    )
    .unwrap();
    let denied = f.prompt(&[]);
    assert_eq!(
        serde_json::from_slice::<Value>(&denied.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
    fs::write(&source, "[invalid configuration").unwrap();
    let output = f
        .command()
        .args(["config", "untrust"])
        .arg(&f.document)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Revocation must work with broken project settings"
    );
    assert!(!grant_path.exists());
}

#[cfg(unix)]
#[test]
fn relative_adapter_uses_the_physical_user_file_directory_across_nested_working_directories() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let physical = f.temp.path().join("physical settings");
    fs::create_dir_all(physical.join("bin")).unwrap();
    symlink(
        fixture_support::helper().canonicalize().unwrap(),
        physical.join("bin/peer"),
    )
    .unwrap();
    fs::write(
        physical.join("config.toml"),
        format!(
            "[agents.codex]\nadapter='./bin/peer'\nadapter_args=[{},'normal']\n",
            quoted("acp")
        ),
    )
    .unwrap();
    symlink(physical.join("config.toml"), f.user.join("config.toml")).unwrap();
    let events = f.events(&[]);
    assert_eq!(event(&events, "start")["agent"], "codex");
    assert!(f.document.parent().unwrap().join("sent-turn.txt").exists());
    assert!(!physical.join("sent-turn.txt").exists());
    let output = f
        .command()
        .args(["config", "show", "--user"])
        .output()
        .unwrap();
    let summary: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        summary["path"],
        json!(lazypencil::paths::canonical(physical.join("config.toml")).unwrap())
    );
}
