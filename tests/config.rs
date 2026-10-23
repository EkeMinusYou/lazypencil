use lazypencil::{config::Snapshot, project::Project, service, store::Store};
use serde_json::{Value, json};
#[cfg(unix)]
use std::time::{Duration, Instant};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
#[path = "support/lock_wait.rs"]
mod lock_wait;

fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("designs")).unwrap();
    let store = Store::init(
        temp.path().join("designs/design.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    (temp, store)
}
fn font(root: &Path, name: &str, source: &str) -> PathBuf {
    let target = root.join(name);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fonts")
            .join(source),
        &target,
    )
    .unwrap();
    target
}
fn configuration(root: &Path, body: &str) {
    fs::write(root.join("lazypencil.toml"), body).unwrap();
}
fn cli(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn configuration_discovery_respects_git_boundaries_and_nearer_projects() {
    let (temp, store) = fixture();
    let original = Project::load(&store).unwrap();
    assert!(original.configuration()["path"].is_null());
    configuration(temp.path(), "version=1\n[fonts]\nlocale='en-US'\n");
    assert_eq!(
        original.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let ancestor = Project::load(&store).unwrap();
    assert_eq!(ancestor.font_settings().locale, "en-US");
    configuration(store.path().parent().unwrap(), "[fonts]\nlocale='ja'\n");
    assert_eq!(
        ancestor.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(Project::load(&store).unwrap().font_settings().locale, "ja");
    fs::remove_file(store.path().parent().unwrap().join("lazypencil.toml")).unwrap();
    fs::create_dir(store.path().parent().unwrap().join(".git")).unwrap();
    let isolated = Project::load(&store).unwrap();
    assert!(isolated.configuration()["path"].is_null());
    assert_eq!(isolated.font_settings().locale, "ja");
}

#[test]
fn configured_files_and_recursive_search_are_ordered_deduplicated_and_captured() {
    let (temp, store) = fixture();
    let regular = font(temp.path(), "fonts/A-regular.TTF", "regular.ttf");
    let bold = font(temp.path(), "fonts/bold.ttf", "bold.ttf");
    let italic = font(temp.path(), "fonts/sub/italic.ttf", "italic.ttf");
    font(temp.path(), "fonts/.hidden.ttf", "regular.ttf");
    fs::write(temp.path().join("fonts/LICENSE"), "unrelated text").unwrap();
    configuration(
        temp.path(),
        "version=1\n[fonts]\nfiles=['fonts/bold.ttf']\nsearch_paths=['fonts']\nfallbacks=[]\nlocale='zh-Hant'\n",
    );
    let project = Project::load(&store).unwrap();
    let actual = project.configuration()["effective_font_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| PathBuf::from(path.as_str().unwrap()))
        .collect::<Vec<_>>();
    let expected = [bold, regular, italic]
        .into_iter()
        .map(|path| lazypencil::paths::canonical(path).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(
        project
            .dependencies()
            .iter()
            .filter(|dependency| dependency.kind == "font")
            .count(),
        3
    );
    assert_eq!(
        project
            .dependencies()
            .iter()
            .filter(|dependency| dependency.kind == "configuration")
            .count(),
        1
    );
    assert_eq!(project.font_settings().fallbacks, Some(vec![]));
    assert_eq!(project.font_settings().locale, "zh-Hant");
    project.verify_unchanged().unwrap();
}

#[test]
fn search_membership_and_configuration_contents_invalidate_frozen_projects() {
    let (temp, store) = fixture();
    font(temp.path(), "fonts/a.ttf", "regular.ttf");
    let settings = "[fonts]\nsearch_paths=['fonts']\n";
    configuration(temp.path(), settings);
    let first = Project::load(&store).unwrap();
    font(temp.path(), "fonts/new.ttf", "italic.ttf");
    assert_eq!(
        first.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let second = Project::load(&store).unwrap();
    assert_ne!(first.dependency_revision(), second.dependency_revision());
    fs::remove_file(temp.path().join("fonts/new.ttf")).unwrap();
    assert_eq!(
        second.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let third = Project::load(&store).unwrap();
    configuration(temp.path(), &format!("{settings}# changed comment\n"));
    assert_eq!(
        third.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let frozen = Snapshot::load(store.path()).unwrap();
    configuration(temp.path(), "[fonts]\nlocale='en'\n");
    assert_eq!(
        frozen.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn malformed_unknown_and_oversized_settings_are_bounded_data_errors() {
    let (temp, store) = fixture();
    for text in [
        "version=2",
        "version='one'",
        "[agents]\napi_key='SECRET_SHOULD_NOT_APPEAR'",
        "[fonts]\nfiles='SECRET_SHOULD_NOT_APPEAR'",
        "[fonts]\nfilez=[]",
        "[fonts]\nlocale='ja_JP'",
        "[fonts]\ndefault_family=''",
        "[fonts]\nfiles=['https://example.invalid/font.ttf']",
        "version=1\nversion=1",
    ] {
        configuration(temp.path(), text);
        let error = Snapshot::load(store.path()).err().unwrap();
        assert_eq!(error.code, "CONFIGURATION_ERROR", "{text}");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("SECRET_SHOULD_NOT_APPEAR")
        );
    }
    fs::write(
        temp.path().join("lazypencil.toml"),
        vec![b' '; 256 * 1024 + 1],
    )
    .unwrap();
    assert_eq!(
        Snapshot::load(store.path()).err().unwrap().code,
        "CONFIGURATION_ERROR"
    );
    fs::write(temp.path().join("lazypencil.toml"), [0xff, 0xfe]).unwrap();
    assert_eq!(
        Snapshot::load(store.path()).err().unwrap().code,
        "CONFIGURATION_ERROR"
    );
    configuration(
        temp.path(),
        &format!("[fonts]\nfiles=[{}]", vec!["'a.ttf'"; 257].join(",")),
    );
    assert_eq!(
        Snapshot::load(store.path()).err().unwrap().code,
        "CONFIGURATION_ERROR"
    );
}

#[test]
fn configuration_cli_initializes_once_shows_from_another_cwd_and_returns_schema() {
    let (temp, store) = fixture();
    let path = temp.path().join("lazypencil.toml");
    let output = cli(&["config", "init", path.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let saved = fs::read(&path).unwrap();
    let output = cli(&["config", "init", path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert_eq!(fs::read(&path).unwrap(), saved);
    let other = temp.path().join("another-cwd");
    fs::create_dir(&other).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(other)
        .arg("--json")
        .args(["config", "show"])
        .arg(store.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["path"],
        json!(lazypencil::paths::canonical(&path).unwrap())
    );
    assert_eq!(value["settings"]["fonts"]["locale"], "ja");
    let schema = cli(&["config", "schema"]);
    assert!(schema.status.success());
    let value: Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(value["additionalProperties"], false);
    assert!(value["properties"].get("fonts").is_some());
    assert_eq!(value["properties"]["version"]["minimum"], 1);
    assert_eq!(value["properties"]["version"]["maximum"], 1);
}

#[test]
fn committed_edit_retries_survive_later_bad_configuration_and_new_edits_check_it() {
    let (temp, store) = fixture();
    configuration(temp.path(), "[fonts]\nlocale='ja'\n");
    let project = Project::load(&store).unwrap();
    let request: lazypencil::operations::EditRequest=serde_json::from_value(json!({
        "request_id":"configuration-edit","expected_revision":store.read().unwrap().revision(),"expected_dependency_revision":project.dependency_revision(),
        "operations":[{"op":"insert","node":{"type":"rectangle","width":10,"height":10}}]
    })).unwrap();
    let first = service::apply_request(&store, &request).unwrap();
    configuration(temp.path(), "not valid toml");
    let replay = service::apply_request(&store, &request).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["revision"], first["revision"]);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
    let mut next = request.clone();
    next.request_id = "new-config-edit".into();
    next.expected_revision = store.read().unwrap().revision();
    assert_eq!(
        service::apply_request(&store, &next).unwrap_err().code,
        "CONFIGURATION_ERROR"
    );
    configuration(temp.path(), "[fonts]\nlocale='en'\n");
    assert_eq!(
        service::apply_request(&store, &next).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn configuration_creation_waits_for_dependency_read_locks() {
    let (temp, store) = fixture();
    let project = Project::load(&store).unwrap();
    let locks = store
        .lock_dependencies(&project.configuration_lock_paths())
        .unwrap();
    let target = temp.path().join("lazypencil.toml");
    let child = lock_wait::WaitingProcess::spawn(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "config", "init"])
            .arg(&target),
    );
    assert!(!target.exists());
    drop(locks);
    let output = child.wait_with_output();
    assert!(output.status.success());
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[cfg(unix)]
#[test]
fn configuration_lock_wait_cancels_without_creating_a_file() {
    let (temp, store) = fixture();
    let project = Project::load(&store).unwrap();
    let _locks = store
        .lock_dependencies(&project.configuration_lock_paths())
        .unwrap();
    let target = temp.path().join("lazypencil.toml");
    let child = lock_wait::WaitingProcess::spawn(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "config", "init"])
            .arg(&target),
    );
    let started = Instant::now();
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let output = child.wait_with_output();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(output.status.code(), Some(130));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "CANCELLED");
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn symlink_retargeting_and_directory_cycles_do_not_hide_configuration_changes() {
    use std::os::unix::fs::symlink;
    let (temp, store) = fixture();
    let one = font(temp.path(), "fonts/one.ttf", "regular.ttf");
    let two = font(temp.path(), "other/two.ttf", "regular.ttf");
    let alias = temp.path().join("chosen.ttf");
    symlink(&one, &alias).unwrap();
    symlink(temp.path().join("fonts"), temp.path().join("fonts/cycle")).unwrap();
    configuration(
        temp.path(),
        "[fonts]\nfiles=['chosen.ttf']\nsearch_paths=['fonts']\n",
    );
    let first = Project::load(&store).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(&two, &alias).unwrap();
    assert_eq!(
        first.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let settings = temp.path().join("lazypencil.toml");
    fs::remove_file(&settings).unwrap();
    fs::write(temp.path().join("one.toml"), "[fonts]\nlocale='en'\n").unwrap();
    fs::write(temp.path().join("two.toml"), "[fonts]\nlocale='en'\n").unwrap();
    symlink(temp.path().join("one.toml"), &settings).unwrap();
    let second = Project::load(&store).unwrap();
    fs::remove_file(&settings).unwrap();
    symlink(temp.path().join("two.toml"), settings).unwrap();
    assert_eq!(
        second.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn common_layout_uses_configured_defaults_locale_fallbacks_and_cli_precedence() {
    let (temp, store) = fixture();
    font(temp.path(), "fonts/faces.ttc", "faces.ttc");
    let regular = font(temp.path(), "regular.ttf", "regular.ttf");
    configuration(
        temp.path(),
        "version=1\n[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\nlocale='en-US'\nfallbacks=['Noto Sans JP','LP Test Supplement']\n",
    );
    fs::write(store.path(),json!({"version":"2.21","children":[
        {"id":"auto","type":"text","content":"A日\u{f0000}","fontSize":24,"fill":"#000000"},
        {"id":"explicit","type":"text","content":"日","fontFamily":"Noto Sans JP","y":50,"fontSize":24,"fill":"#000000"}
    ]}).to_string()).unwrap();
    let saved = store.read().unwrap().text().to_string();
    let inventory = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "fonts", "list", "--file"])
        .arg(store.path())
        .output()
        .unwrap();
    assert!(inventory.status.success());
    let inventory: Value = serde_json::from_slice(&inventory.stdout).unwrap();
    assert_eq!(inventory["default_family"], "LP Test Sans");
    assert_eq!(inventory["locale"], "en-US");
    assert_eq!(inventory["fonts"].as_array().unwrap().len(), 5);
    let (project, layout) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let auto = layout
        .items
        .iter()
        .find(|item| item.record.id == "auto")
        .unwrap()
        .record
        .text
        .as_ref()
        .unwrap();
    assert_eq!(auto.requested_family, "LP Test Sans");
    assert_eq!(auto.locale, "en-US");
    assert_eq!(auto.fonts.len(), 3);
    let explicit = layout
        .items
        .iter()
        .find(|item| item.record.id == "explicit")
        .unwrap()
        .record
        .text
        .as_ref()
        .unwrap();
    assert_eq!(explicit.requested_family, "Noto Sans JP");
    let (_, overridden) =
        service::layout_snapshot(&store, std::slice::from_ref(&regular), None, None).unwrap();
    let auto = overridden
        .items
        .iter()
        .find(|item| item.record.id == "auto")
        .unwrap()
        .record
        .text
        .as_ref()
        .unwrap();
    assert_eq!(
        auto.fonts[0].hash,
        lazypencil::document::hash(&fs::read(regular).unwrap())
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "layout"])
        .arg(store.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["snapshot_id"], layout.id);
    configuration(
        temp.path(),
        "[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\nfallbacks=[]\n",
    );
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::layout_snapshot(&store, &[], None, None)
            .err()
            .unwrap()
            .code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(store.read().unwrap().text(), saved);
}
