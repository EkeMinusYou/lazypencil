use lazypencil::config::{
    Snapshot,
    runtime::{PageWaitUntil, ScriptSettings},
};
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
fn dynamic_limits_merge_independently_with_explicit_defaults_and_cache_disabling() {
    let f = Fixture::new();
    fs::write(
        f.user.join("config.toml"),
        "[runtime.browser]\ntimeout_ms=1200\nlaunch_timeout_ms=4000\nidle_seconds=7\n[runtime.shader]\ntimeout_ms=2200\nlaunch_timeout_ms=1000\nidle_seconds=9\n[runtime.dynamic]\nrender_timeout_ms=1500\ncache_mib=3\ncache_entries=5\n",
    )
    .unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.browser]\ntimeout_ms=30000\nidle_seconds=0\n[runtime.shader]\nlaunch_timeout_ms=15000\n[runtime.dynamic]\ncache_entries=32\ncache_mib=0\n",
    )
    .unwrap();
    let snapshot = f.snapshot().unwrap();
    let effective = snapshot.settings.runtime;
    assert_eq!(effective.browser.timeout_ms, 30000);
    assert_eq!(effective.browser.launch_timeout_ms, 4000);
    assert_eq!(effective.browser.idle_seconds, 0);
    assert_eq!(effective.shader.timeout_ms, 2200);
    assert_eq!(effective.shader.launch_timeout_ms, 15000);
    assert_eq!(effective.shader.idle_seconds, 9);
    assert_eq!(effective.dynamic.render_timeout_ms, 1500);
    assert_eq!(effective.dynamic.cache_mib, 0);
    assert_eq!(effective.dynamic.cache_entries, 32);
    let summary = snapshot.summary();
    for (field, origin) in [
        ("browser.timeout_ms", "project"),
        ("browser.launch_timeout_ms", "user"),
        ("browser.idle_seconds", "project"),
        ("shader.timeout_ms", "user"),
        ("shader.launch_timeout_ms", "project"),
        ("shader.idle_seconds", "user"),
        ("dynamic.render_timeout_ms", "user"),
        ("dynamic.cache_mib", "project"),
        ("dynamic.cache_entries", "project"),
    ] {
        assert_eq!(summary["runtime_origins"][field], origin);
    }
    fs::write(f.user.join("config.toml"), "").unwrap();
    assert_eq!(
        snapshot.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        f.snapshot().unwrap().settings.runtime.shader.idle_seconds,
        60
    );
}

#[test]
fn invalid_dynamic_limits_are_not_hidden_by_higher_priority_values() {
    let f = Fixture::new();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.browser]\ntimeout_ms=30000\nlaunch_timeout_ms=15000\nidle_seconds=60\n[runtime.shader]\ntimeout_ms=15000\nlaunch_timeout_ms=15000\nidle_seconds=60\n[runtime.dynamic]\nrender_timeout_ms=45000\ncache_mib=64\ncache_entries=32\n",
    )
    .unwrap();
    for scope in ["browser", "shader"] {
        for field in [
            "timeout_ms=0",
            "timeout_ms=300001",
            "launch_timeout_ms=0",
            "launch_timeout_ms=60001",
            "idle_seconds=3601",
            "timeout_ms=1.5",
            "timeout_ms='private-value'",
            "idle_seconds=-1",
            "unknown=1",
        ] {
            fs::write(
                f.user.join("config.toml"),
                format!("[runtime.{scope}]\n{field}\n"),
            )
            .unwrap();
            let error = f.snapshot().err().unwrap();
            assert_eq!(error.code, "CONFIGURATION_ERROR", "{scope}.{field}");
            assert!(!error.message.contains("private-value"));
        }
    }
    for field in [
        "render_timeout_ms=0",
        "render_timeout_ms=300001",
        "cache_mib=257",
        "cache_entries=257",
        "cache_entries=-1",
        "cache_mib=1.5",
        "unknown=1",
    ] {
        fs::write(
            f.user.join("config.toml"),
            format!("[runtime.dynamic]\n{field}\n"),
        )
        .unwrap();
        assert_eq!(
            f.snapshot().err().unwrap().code,
            "CONFIGURATION_ERROR",
            "{field}"
        );
    }
}

#[test]
fn page_wait_policy_inherits_invalidates_and_rejects_unknown_or_shader_values() {
    let f = Fixture::new();
    assert_eq!(
        f.snapshot().unwrap().settings.runtime.browser.wait_until,
        PageWaitUntil::Load
    );
    fs::write(
        f.user.join("config.toml"),
        "[runtime.browser]\nwait_until='network_idle'\n",
    )
    .unwrap();
    let captured = f.snapshot().unwrap();
    assert_eq!(
        captured.settings.runtime.browser.wait_until,
        PageWaitUntil::NetworkIdle
    );
    assert_eq!(
        captured.summary()["runtime_origins"]["browser.wait_until"],
        "user"
    );
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.browser]\nwait_until='load'\n",
    )
    .unwrap();
    assert_eq!(
        captured.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let current = f.snapshot().unwrap();
    assert_eq!(
        current.settings.runtime.browser.wait_until,
        PageWaitUntil::Load
    );
    assert_eq!(
        current.summary()["runtime_origins"]["browser.wait_until"],
        "project"
    );
    for invalid in [
        "[runtime.browser]\nwait_until='private-value'\n",
        "[runtime.browser]\nwait_until=1\n",
        "[runtime.shader]\nwait_until='load'\n",
    ] {
        fs::write(f.user.join("config.toml"), invalid).unwrap();
        let error = f.snapshot().err().unwrap();
        assert_eq!(error.code, "CONFIGURATION_ERROR");
        assert!(!error.message.contains("private-value"));
    }
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
        for name in ["BrowserSettings", "ShaderSettings"] {
            let fields = &schema["$defs"][name]["properties"];
            assert_eq!(fields["timeout_ms"]["maximum"].as_f64(), Some(300000.0));
            assert_eq!(fields["launch_timeout_ms"]["minimum"].as_f64(), Some(1.0));
            assert_eq!(fields["idle_seconds"]["minimum"].as_f64(), Some(0.0));
        }
        let fields = &schema["$defs"]["DynamicSettings"]["properties"];
        assert_eq!(fields["cache_mib"]["minimum"].as_f64(), Some(0.0));
        assert_eq!(fields["cache_entries"]["maximum"].as_f64(), Some(256.0));
        assert_eq!(
            schema["$defs"]["PageWaitUntil"]["enum"],
            json!(["load", "network_idle"])
        );
        assert!(
            schema["$defs"]["ShaderSettings"]["properties"]
                .get("wait_until")
                .is_none()
        );
    }
    f.cli(&["config", "init", "--user"]);
    let template = fs::read_to_string(f.user.join("config.toml")).unwrap();
    assert!(template.contains("# [runtime.javascript]"));
    assert!(template.contains("# [runtime.browser]"));
    assert!(template.contains("# wait_until = \"load\""));
    assert!(template.contains("# [runtime.shader]"));
    assert!(template.contains("# [runtime.dynamic]"));
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

#[cfg(all(feature = "rendering", unix))]
#[test]
fn cli_render_applies_project_launch_policy_before_starting_a_page_worker() {
    use std::{os::unix::fs::PermissionsExt, time::Instant};
    let f = Fixture::new();
    let executable = f.project.join("waiting-browser");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"${0%/*}/pid\"\nexec sleep 60\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let saved = json!({"version":"2.21","children":[{"id":"page","type":"browser","url":"about:blank","width":8,"height":8}]}).to_string();
    fs::write(&f.document, &saved).unwrap();
    fs::write(
        f.user.join("config.toml"),
        "[runtime.browser]\ntimeout_ms=5000\nlaunch_timeout_ms=5000\n",
    )
    .unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[runtime.browser]\nlaunch_timeout_ms=300\n",
    )
    .unwrap();
    let image = f.project.join("unpublished.png");
    let start = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .env("LAZYPENCIL_CONFIG_DIR", &f.user)
        .env("LAZYPENCIL_CHROMIUM", &executable)
        .args(["--json", "--data-dir"])
        .arg(f.temp.path().join("data"))
        .arg("render")
        .arg(&f.document)
        .arg("--output")
        .arg(&image)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "RENDER_TIMEOUT", "{error}");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(!image.exists());
    assert_eq!(fs::read_to_string(&f.document).unwrap(), saved);
    let pid: i32 = fs::read_to_string(f.project.join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}
