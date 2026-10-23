use lazypencil::config::{Snapshot, runtime::ScriptSettings};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    project: PathBuf,
    user: PathBuf,
    document: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let project = temp.path().join("project");
        let user = temp.path().join("user");
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::create_dir(&user).unwrap();
        let document = project.join("design.pen");
        fs::write(
            &document,
            json!({"version":"2.21","children":[]}).to_string(),
        )
        .unwrap();
        Self {
            temp,
            project,
            user,
            document,
        }
    }
    fn snapshot(&self) -> lazypencil::Result<Snapshot> {
        Snapshot::load_with_user_file(&self.document, Some(self.user.join("config.toml")))
    }
    fn cli(&self, arguments: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .env("LAZYPENCIL_CONFIG_DIR", &self.user)
            .args(["--json", "--data-dir"])
            .arg(self.temp.path().join("data"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn runtime_limits_inherit_per_field_and_explicit_defaults_and_zero_override_users() {
    let f = Fixture::new();
    assert_eq!(
        f.snapshot().unwrap().settings.runtime.javascript,
        ScriptSettings::default()
    );
    fs::write(
        f.user.join("config.toml"),
        "[runtime.javascript]\ntimeout_ms=55\nheap_mib=12\nidle_seconds=7\nmax_evaluations=24\n",
    )
    .unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.javascript]\ntimeout_ms=2000\nidle_seconds=0\nmax_passes=3\n",
    )
    .unwrap();
    let captured = f.snapshot().unwrap();
    assert_eq!(
        captured.settings.runtime.javascript,
        ScriptSettings {
            timeout_ms: 2000,
            heap_mib: 12,
            idle_seconds: 0,
            max_evaluations: 24,
            max_passes: 3,
            ..Default::default()
        }
    );
    let summary = captured.summary();
    for (field, origin) in [
        ("timeout_ms", "project"),
        ("heap_mib", "user"),
        ("idle_seconds", "project"),
        ("max_evaluations", "user"),
        ("max_passes", "project"),
        ("layout_timeout_ms", "builtin"),
    ] {
        assert_eq!(
            summary["runtime_origins"][format!("javascript.{field}")],
            origin
        );
    }
    captured.verify_unchanged().unwrap();
    fs::write(
        f.user.join("config.toml"),
        "[runtime.javascript]\nheap_mib=16\n",
    )
    .unwrap();
    assert_eq!(
        captured.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(captured.settings.runtime.javascript.heap_mib, 12);
    assert_eq!(
        f.snapshot().unwrap().settings.runtime.javascript.heap_mib,
        16
    );
}

#[test]
fn invalid_runtime_fields_are_rejected_even_when_another_scope_would_override_them() {
    let f = Fixture::new();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.javascript]\ntimeout_ms=2000\nheap_mib=64\n",
    )
    .unwrap();
    for entry in [
        "timeout_ms=0",
        "timeout_ms=60001",
        "heap_mib=3",
        "heap_mib=513",
        "idle_seconds=3601",
        "max_passes=0",
        "max_passes=65",
        "max_evaluations=4097",
        "layout_timeout_ms=0",
        "layout_timeout_ms=300001",
        "heap_mib='private-value'",
        "timeout_ms=1.5",
        "heap_mib=-1",
        "unknown=2",
    ] {
        fs::write(
            f.user.join("config.toml"),
            format!("[runtime.javascript]\n{entry}\n"),
        )
        .unwrap();
        let error = f
            .snapshot()
            .err()
            .expect("Invalid user policy must not be hidden");
        assert_eq!(error.code, "CONFIGURATION_ERROR", "{entry}");
        assert!(!error.message.contains("private-value"));
    }
    fs::write(
        f.user.join("config.toml"),
        "[runtime.javascript]\nheap_mib=16\n",
    )
    .unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.javascript]\nheap_mib=0\n",
    )
    .unwrap();
    assert_eq!(f.snapshot().err().unwrap().code, "CONFIGURATION_ERROR");
}

#[test]
fn runtime_schema_and_templates_expose_limits_without_overriding_inheritance() {
    let f = Fixture::new();
    for arguments in [vec!["config", "schema"], vec!["config", "schema", "--user"]] {
        let schema = f.cli(&arguments);
        assert!(schema["properties"]["runtime"].is_object());
        let fields = &schema["$defs"]["ScriptSettings"]["properties"];
        assert_eq!(fields["timeout_ms"]["minimum"].as_f64(), Some(1.0));
        assert_eq!(fields["timeout_ms"]["maximum"].as_f64(), Some(60000.0));
        assert_eq!(fields["heap_mib"]["minimum"].as_f64(), Some(4.0));
        assert_eq!(fields["heap_mib"]["maximum"].as_f64(), Some(512.0));
    }
    f.cli(&["config", "init", "--user"]);
    let template = fs::read_to_string(f.user.join("config.toml")).unwrap();
    assert!(template.contains("# [runtime.javascript]"));
    assert_eq!(
        f.snapshot().unwrap().settings.runtime.javascript,
        ScriptSettings::default()
    );
    assert_eq!(
        f.cli(&["config", "show", "--user"])["settings"]["runtime"]["javascript"]["idle_seconds"],
        60
    );
}

#[cfg(feature = "rendering")]
#[test]
fn cli_layout_uses_configured_evaluation_pass_and_aggregate_limits_without_saving() {
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    let source = f.project.join("draw.js");
    fs::write(
        &source,
        "/** @schema 2.21 */ return [{type:'rectangle',width:20,height:10,fill:'#FF0000'}];",
    )
    .unwrap();
    let script = |id| json!({"id":id,"type":"script","width":80,"height":40,"scriptUri":"draw.js"});
    let saved = json!({"version":"2.21","children":[script("one"),script("two")]}).to_string();
    fs::write(&f.document, &saved).unwrap();
    fs::write(
        f.user.join("config.toml"),
        "[runtime.javascript]\nheap_mib=12\ntimeout_ms=2000\nidle_seconds=0\n",
    )
    .unwrap();
    let execute = || {
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .env("LAZYPENCIL_CONFIG_DIR", &f.user)
            .args(["--json", "--data-dir"])
            .arg(f.temp.path().join("data"))
            .args(["layout"])
            .arg(&f.document)
            .output()
            .unwrap()
    };
    let output = execute();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let layout: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(layout["script_executions"].as_array().unwrap().len(), 2);
    assert_eq!(layout["script_executions"][0]["limits"]["heap_mib"], 12);
    assert_eq!(layout["script_executions"][0]["limits"]["idle_seconds"], 0);
    for (settings, expected) in [
        ("max_evaluations=1", "RENDER_TIMEOUT"),
        ("max_passes=1", "LAYOUT_UNRESOLVED"),
    ] {
        fs::write(
            f.project.join("lazypencil.toml"),
            format!("[runtime.javascript]\n{settings}\n"),
        )
        .unwrap();
        let output = execute();
        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["error"]["code"], expected);
    }
    fs::write(&source, "/** @schema 2.21 */ for (;;) {} ").unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.javascript]\ntimeout_ms=60000\nlayout_timeout_ms=300\n",
    )
    .unwrap();
    let started = Instant::now();
    let output = execute();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "RENDER_TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fs::read_to_string(&f.document).unwrap(), saved);
}
