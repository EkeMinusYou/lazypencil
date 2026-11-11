#[path = "../xtask/src/test_support/command_output.rs"]
#[allow(dead_code)]
mod command_output;

use lazypencil::document::Document;
use serde_json::{Value, json};
use std::{fs, process::Command, time::Duration};
use tempfile::TempDir;

fn cli(temp: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .current_dir(temp.path())
        .env_remove("LAZYPENCIL_COMPLETE")
        .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("configuration"))
        .env("LAZYPENCIL_DATA_DIR", temp.path().join("data"))
        .env("LAZYPENCIL_LOCK_DIR", temp.path().join("locks"));
    command
}

fn output(command: &mut Command, success: bool) -> Value {
    let output = command_output::output(command, Duration::from_secs(30), 4 * 1024 * 1024)
        .expect("CLI must finish");
    assert!(!output.timed_out, "CLI timed out");
    assert_eq!(
        output.status.success(),
        success,
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn demo_works_without_agents_or_valid_user_configuration() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("configuration")).unwrap();
    fs::write(temp.path().join("configuration/config.toml"), "invalid = [").unwrap();
    let result = output(cli(&temp).env("PATH", "").args(["--json", "demo"]), true);
    let path = temp.path().join("demo.pen").canonicalize().unwrap();
    assert_eq!(result["path"], json!(path));
    let document = Document::parse(fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(result["revision"], document.revision());
    assert!(document.node_count() > 100);
    assert_eq!(document.value()["children"][0]["width"], 1080);
    assert_eq!(document.value()["children"][0]["height"], 800);
    assert!(document.diagnostics().is_empty());
    assert!(!temp.path().join("assets").exists());
    assert!(!temp.path().join("data").exists());
}

#[test]
fn demo_preserves_existing_files_and_requires_an_existing_parent() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("日本語 demo.pen");
    output(cli(&temp).args(["--json", "demo"]).arg(&path), true);
    let original = fs::read(&path).unwrap();
    let error = output(cli(&temp).args(["--json", "demo"]).arg(&path), false);
    assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(fs::read(&path).unwrap(), original);

    fs::write(&path, b"precious bytes that are not a design").unwrap();
    output(cli(&temp).args(["--json", "demo"]).arg(&path), false);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"precious bytes that are not a design"
    );
    output(
        cli(&temp).args(["--json", "demo", "missing/demo.pen"]),
        false,
    );
    assert!(!temp.path().join("missing").exists());
}

#[cfg(unix)]
#[test]
fn demo_does_not_replace_or_follow_destination_symlinks() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("original.pen");
    let path = temp.path().join("demo.pen");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    output(cli(&temp).args(["--json", "demo"]), false);
    assert!(!target.exists());
    fs::write(&target, b"original design").unwrap();
    output(cli(&temp).args(["--json", "demo"]), false);
    assert_eq!(fs::read_link(&path).unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), b"original design");
}

#[test]
fn demo_is_editable_through_the_normal_cli() {
    let temp = TempDir::new().unwrap();
    let created = output(cli(&temp).args(["--json", "demo"]), true);
    let path = temp.path().join("demo.pen");
    let document = Document::parse(fs::read_to_string(&path).unwrap()).unwrap();
    let root = document.value()["children"][0]["id"].as_str().unwrap();
    fs::write(
        temp.path().join("operations.json"),
        json!([{"op":"update","target":root,"set":{"name":"My project dashboard"}}]).to_string(),
    )
    .unwrap();
    output(
        cli(&temp).args([
            "--json",
            "apply",
            "demo.pen",
            "--operations",
            "operations.json",
            "--expected-revision",
            created["revision"].as_str().unwrap(),
        ]),
        true,
    );
    let edited = Document::parse(fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        edited.value()["children"][0]["name"],
        "My project dashboard"
    );
    assert_ne!(edited.revision(), document.revision());
    assert_eq!(edited.node_count(), document.node_count());
}

#[cfg(feature = "rendering")]
#[test]
fn demo_renders_a_complete_png_without_external_assets_or_ai() {
    let temp = TempDir::new().unwrap();
    output(cli(&temp).args(["--json", "demo"]), true);
    output(
        cli(&temp).args(["--json", "validate", "demo.pen", "--scope", "layout"]),
        true,
    );
    output(
        cli(&temp)
            .env("PATH", "")
            .args(["--json", "render", "demo.pen", "--output", "demo.png"]),
        true,
    );
    let png = image::open(temp.path().join("demo.png")).unwrap();
    assert_eq!((png.width(), png.height()), (1080, 800));
}
