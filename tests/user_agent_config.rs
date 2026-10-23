use lazypencil::config::UserSnapshot;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

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
        let python = std::env::var("LAZYPENCIL_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
        let peer = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/acp_fixture.py");
        format!(
            "[agents.{agent}]\nadapter={}\nadapter_args=[{},{}]\n{extra}\n",
            quoted(&python),
            quoted(peer.to_str().unwrap()),
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
    let python = std::env::var("LAZYPENCIL_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let peer = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/acp_fixture.py");
    let events = f.events(&[
        "--adapter",
        &python,
        &format!("--adapter-arg={}", peer.display()),
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
fn config_schema_and_template_expose_only_user_owned_agent_preferences() {
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
    assert!(schema["properties"].get("agents").is_none());
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
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "CONFIGURATION_ERROR");
    assert!(!value.to_string().contains("SECRET"));
}

#[cfg(unix)]
#[test]
fn relative_adapter_uses_the_physical_user_file_directory_across_nested_working_directories() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let physical = f.temp.path().join("physical settings");
    fs::create_dir_all(physical.join("bin")).unwrap();
    let python = std::env::var("LAZYPENCIL_TEST_PYTHON").unwrap_or_else(|_| "python3".into());
    let output = Command::new(python)
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let executable = String::from_utf8(output.stdout).unwrap();
    symlink(executable.trim(), physical.join("bin/peer")).unwrap();
    let peer = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/acp_fixture.py");
    fs::write(
        physical.join("config.toml"),
        format!(
            "[agents.codex]\nadapter='./bin/peer'\nadapter_args=[{},'normal']\n",
            quoted(peer.to_str().unwrap())
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
