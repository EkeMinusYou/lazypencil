use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
    time::{Duration, Instant},
};
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture_support;

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .current_dir(root)
        .args(["--data-dir"])
        .arg(root.join("absent-data"))
        .env("LAZYPENCIL_CONFIG_DIR", root.join("settings"))
        .env("LAZYPENCIL_LOCK_DIR", root.join("absent-locks"))
        .env_remove("LAZYPENCIL_CHROMIUM")
        .env("PATH", root.join("absent-bin"));
    command
}

fn settings(root: &Path, mode: &str, additional: &str) {
    fs::create_dir_all(root.join("settings")).unwrap();
    fs::write(root.join("settings/config.toml"), format!(
        "[agents]\ndefault='codex'\n[agents.codex]\nadapter={}\nadapter_args=['acp', '{}', 'PRIVATE_ARGUMENT']\n{}\n",
        serde_json::to_string(&fixture_support::helper()).unwrap(), mode, additional
    )).unwrap();
}

fn doctor(root: &Path, args: &[&str]) -> (Output, Value) {
    let output = fixture_support::output(
        command(root).args(["--json", "doctor"]).args(args),
        Duration::from_secs(12),
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output, value)
}

fn check<'a>(value: &'a Value, id: &str) -> &'a Value {
    value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == id)
        .unwrap_or_else(|| panic!("Missing check {id}: {value}"))
}

#[test]
fn doctor_missing_adapters_explain_installation_and_strict_mode_without_creating_state() {
    let root = TempDir::new().unwrap();
    let (output, value) = doctor(root.path(), &[]);
    assert!(output.status.success());
    assert_eq!(check(&value, "agents.codex.executable")["status"], "error");
    assert!(
        check(&value, "agents.codex.executable")["remediation"]
            .as_str()
            .unwrap()
            .contains("npm install -g @agentclientprotocol/codex-acp@latest")
    );
    assert_eq!(
        check(&value, "agents.claude.executable")["status"],
        "not_checked"
    );
    assert_eq!(value["agents"][0]["status"], "missing");
    assert_eq!(value["summary"]["status"], "error");
    assert!(check(&value, "platform")["details"]["minimum_os"].is_string());
    #[cfg(feature = "rendering")]
    assert_eq!(check(&value, "fonts.loading")["status"], "ok");
    #[cfg(not(feature = "rendering"))]
    assert_eq!(check(&value, "fonts.loading")["status"], "not_checked");
    for key in [
        "version",
        "release",
        "release_ready",
        "skia_compiled",
        "chromium",
        "document_version",
        "supported_document_versions",
        "user_configuration",
        "data_directory",
        "lock_directory",
    ] {
        assert!(!value[key].is_null(), "Missing legacy field {key}");
    }
    #[cfg(feature = "rendering")]
    assert_eq!(check(&value, "preview.renderer")["status"], "ok");
    #[cfg(feature = "tui")]
    assert_eq!(check(&value, "preview.terminal")["status"], "not_checked");
    let (output, strict) = doctor(root.path(), &["--strict"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(strict["checks"].is_array());
    let output = command(root.path())
        .args(["doctor", "--strict"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("[FAIL] agents.codex.executable"));
    assert!(text.contains("npm install -g"));
    for name in ["absent-data", "absent-locks", "settings"] {
        assert!(!root.path().join(name).exists());
    }
}

#[cfg(all(unix, feature = "tui"))]
#[path = "support/doctor_terminal.rs"]
mod terminal_tests;

#[test]
fn doctor_explicit_agent_selection_does_not_start_another_configured_adapter() {
    let root = TempDir::new().unwrap();
    settings(root.path(), "auth-inspect-account", "");
    let (_, value) = doctor(root.path(), &["--agent", "claude"]);
    assert_eq!(value["agents"].as_array().unwrap().len(), 1);
    assert_eq!(value["agents"][0]["agent"], "claude");
    assert_eq!(value["agents"][0]["selected"], true);
    assert_eq!(value["agents"][0]["status"], "missing");
    assert!(!root.path().join("received-account-methods.jsonl").exists());
}

#[test]
fn doctor_human_output_escapes_terminal_control_sequences_in_external_facts() {
    let report = serde_json::json!({"version":"test","checks":[{"id":"agent","status":"ok","message":"adapter\u{1b}]52;clipboard\nFORGED","details":{"version":"1.2.3\u{202e}"}}],"summary":{"errors":0,"warnings":0}});
    let output = lazypencil::doctor::format_report(&report);
    assert!(!output.contains('\u{1b}') && !output.contains('\u{202e}'));
    assert!(!output.lines().any(|line| line == "FORGED"));
    assert!(output.contains("FORGED"));
}

#[test]
fn doctor_checks_acp_and_login_by_default_without_sessions_prompts_logins_or_credentials() {
    let root = TempDir::new().unwrap();
    settings(root.path(), "auth-inspect-account", "");
    let (output, value) = doctor(root.path(), &["--strict"]);
    let platform_errors = u64::from(check(&value, "platform")["status"] == "error");
    assert_eq!(value["summary"]["errors"], platform_errors, "{value}");
    assert_eq!(output.status.success(), platform_errors == 0, "{value}");
    assert_eq!(check(&value, "agents.codex.communication")["status"], "ok");
    assert_eq!(check(&value, "agents.codex.authentication")["status"], "ok");
    assert_eq!(value["agents"][0]["status"], "ready");
    assert_eq!(
        value["agents"][0]["authentication"]["source"],
        "subscription"
    );
    let methods = fs::read_to_string(root.path().join("received-account-methods.jsonl")).unwrap();
    let methods: Vec<Value> = methods
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0]["method"], "initialize");
    let serialized = value.to_string();
    for secret in [
        "PRIVATE_ARGUMENT",
        "fixture-not-a-token",
        "person@example",
        "fixture@example",
        "must-not-log",
        "accessToken",
    ] {
        assert!(!serialized.contains(secret), "Leaked {secret}");
    }
    for name in ["absent-data", "absent-locks", "sent-turn.txt"] {
        assert!(!root.path().join(name).exists());
    }
    let output = command(root.path()).args(["doctor"]).output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("[OK] agents.codex.communication")
            && text.contains("[OK] agents.codex.authentication")
    );
    assert!(!text.contains("PRIVATE_ARGUMENT") && !text.contains("fixture@example"));
}

#[test]
fn doctor_distinguishes_not_signed_in_unknown_auth_and_authentication_route_mismatch() {
    for (mode, extra, expected, status) in [
        ("auth-inspect-none", "", "not_authenticated", "error"),
        ("echo", "", "authentication_unknown", "warning"),
        (
            "echo",
            "require_auth_status=true",
            "authentication_unknown",
            "error",
        ),
        (
            "auth-inspect-account",
            "auth='api'",
            "authentication_mismatch",
            "error",
        ),
    ] {
        let root = TempDir::new().unwrap();
        settings(root.path(), mode, extra);
        let (_, value) = doctor(root.path(), &[]);
        assert_eq!(value["agents"][0]["status"], expected, "{value}");
        assert_eq!(
            check(&value, "agents.codex.authentication")["status"],
            status,
            "{value}"
        );
        assert!(!root.path().join("sent-turn.txt").exists());
    }
}

#[test]
fn doctor_bounds_agent_startup_and_keeps_partial_protocol_results_on_auth_timeout() {
    for mode in ["auth-inspect-hang", "setup-hang"] {
        let root = TempDir::new().unwrap();
        settings(root.path(), mode, "");
        let started = Instant::now();
        let (output, value) = doctor(root.path(), &["--timeout-seconds", "1", "--strict"]);
        assert_eq!(output.status.code(), Some(1), "{value}");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "Unbounded doctor startup"
        );
        assert_eq!(
            check(&value, "agents.codex.communication")["status"],
            if mode == "setup-hang" { "error" } else { "ok" }
        );
        if mode == "auth-inspect-hang" {
            assert_eq!(
                check(&value, "agents.codex.authentication")["status"],
                "error"
            );
        }
        assert!(!root.path().join("sent-turn.txt").exists());
    }
}

#[test]
fn doctor_invalid_configuration_is_reported_without_executing_adapters_or_echoing_contents() {
    let root = TempDir::new().unwrap();
    settings(root.path(), "auth-inspect-account", "");
    fs::write(
        root.path().join("settings/config.toml"),
        "PRIVATE_CREDENTIAL = [invalid TOML",
    )
    .unwrap();
    let (output, value) = doctor(root.path(), &["--strict"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(check(&value, "user_configuration")["status"], "error");
    assert_eq!(value["agents"][0]["status"], "configuration_invalid");
    assert!(!value.to_string().contains("PRIVATE_CREDENTIAL"));
    assert!(!root.path().join("received-account-methods.jsonl").exists());
    assert!(value["chromium"].is_object());
}

#[test]
fn doctor_resolves_relative_adapter_profiles_against_the_physical_user_config() {
    let root = TempDir::new().unwrap();
    settings(root.path(), "auth-inspect-account", "");
    let text = fs::read_to_string(root.path().join("settings/config.toml")).unwrap();
    let helper = fixture_support::helper();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&helper, root.path().join("settings/relative-adapter")).unwrap();
    #[cfg(windows)]
    fs::copy(&helper, root.path().join("settings/relative-adapter.exe")).unwrap();
    let relative = if cfg!(windows) {
        "./relative-adapter.exe"
    } else {
        "./relative-adapter"
    };
    fs::write(
        root.path().join("settings/config.toml"),
        text.replace(
            &serde_json::to_string(&helper).unwrap(),
            &serde_json::to_string(relative).unwrap(),
        ),
    )
    .unwrap();
    let (_, value) = doctor(root.path(), &["--agent", "codex"]);
    assert_eq!(value["agents"].as_array().unwrap().len(), 1);
    assert_eq!(value["agents"][0]["status"], "ready", "{value}");
    assert!(
        value["agents"][0]["path"]
            .as_str()
            .unwrap()
            .contains("settings/./relative-adapter")
    );
}

#[test]
fn doctor_checks_configured_fonts_and_rejects_a_file_as_a_data_directory() {
    let root = TempDir::new().unwrap();
    settings(root.path(), "auth-inspect-account", "");
    let path = root.path().join("settings/config.toml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n[fonts]\nfiles=['missing.ttf']\n");
    fs::write(path, text).unwrap();
    fs::write(root.path().join("absent-data"), b"not a directory").unwrap();
    let (_, value) = doctor(root.path(), &[]);
    assert_eq!(check(&value, "fonts")["status"], "error");
    assert_eq!(check(&value, "fonts.loading")["status"], "not_checked");
    assert_eq!(check(&value, "data_directory")["status"], "error");
    assert_eq!(check(&value, "agents.codex.authentication")["status"], "ok");
}

#[cfg(feature = "rendering")]
#[test]
fn doctor_loads_all_configured_font_faces_and_reports_corrupt_files_or_unavailable_families() {
    for scenario in ["valid", "corrupt", "missing-family"] {
        let root = TempDir::new().unwrap();
        settings(root.path(), "auth-inspect-account", "");
        let font = root.path().join("settings/custom.ttf");
        if scenario == "corrupt" {
            fs::write(&font, b"This is not a font file.").unwrap();
        } else {
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/regular.ttf"),
                &font,
            )
            .unwrap();
        }
        let configuration = root.path().join("settings/config.toml");
        let mut text = fs::read_to_string(&configuration).unwrap();
        text.push_str("\n[fonts]\nfiles=['custom.ttf']\n");
        if scenario == "missing-family" {
            text.push_str("default_family='Doctor Missing Font Family 7fdd57e2'\n");
        }
        fs::write(configuration, text).unwrap();
        let (output, value) = doctor(root.path(), &["--strict"]);
        assert_eq!(
            check(&value, "fonts")["status"],
            "ok",
            "Availability does not prove a font is loadable"
        );
        assert_eq!(
            check(&value, "fonts.loading")["status"],
            if scenario == "valid" { "ok" } else { "error" },
            "{value}"
        );
        let expected_errors = u64::from(scenario != "valid")
            + u64::from(check(&value, "platform")["status"] == "error");
        assert_eq!(value["summary"]["errors"], expected_errors, "{value}");
        assert_eq!(output.status.success(), expected_errors == 0, "{value}");
        if scenario == "valid" {
            assert!(
                check(&value, "fonts.loading")["details"]["loaded_faces"]
                    .as_u64()
                    .unwrap()
                    >= 2
            );
            assert!(
                check(&value, "fonts.loading")["details"]["families"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|family| family == "LP Test Sans")
            );
        }
        assert_eq!(check(&value, "preview.renderer")["status"], "ok");
        assert!(
            !root.path().join("absent-data").exists() && !root.path().join("absent-locks").exists()
        );
    }
}

#[cfg(unix)]
#[test]
fn doctor_detects_missing_node_and_shadowed_path_installations_without_running_them() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    for directory in [&first, &second] {
        fs::create_dir(directory).unwrap();
        let adapter = directory.join("codex-acp");
        fs::write(
            &adapter,
            "#!/usr/bin/env node\nthrow new Error('DO_NOT_RUN');\n",
        )
        .unwrap();
        fs::set_permissions(adapter, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = command(root.path())
        .args(["--json", "doctor"])
        .env("PATH", std::env::join_paths([&first, &second]).unwrap())
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        check(&value, "agents.codex.executable")["status"],
        "warning"
    );
    assert_eq!(
        value["agents"][0]["path"],
        first.join("codex-acp").to_string_lossy().as_ref()
    );
    assert_eq!(value["agents"][0]["status"], "node_missing");
    assert_eq!(check(&value, "agents.codex.node")["status"], "error");
    assert!(!value.to_string().contains("DO_NOT_RUN"));
}
