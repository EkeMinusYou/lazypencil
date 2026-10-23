use lazypencil::{
    config::{Snapshot, UserSnapshot},
    store::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;
#[path = "support/lock_wait.rs"]
mod lock_wait;

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
        fs::create_dir_all(project.join(".git")).unwrap();
        let user = temp.path().join("user settings");
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
    fn settings(&self, body: &str) {
        fs::create_dir_all(&self.user).unwrap();
        fs::write(self.user.join("config.toml"), body).unwrap();
    }
    fn snapshot(&self) -> lazypencil::Result<Snapshot> {
        Snapshot::load_with_user_file(&self.document, Some(self.user.join("config.toml")))
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        command
            .env("LAZYPENCIL_CONFIG_DIR", &self.user)
            .arg("--json")
            .arg("--data-dir")
            .arg(self.temp.path().join("data"))
            .args(args);
        command
    }
    fn cli(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn data(&self, args: &[&str]) -> Value {
        let output = self.cli(args);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
fn font(path: &Path, name: &str) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fonts")
            .join(name),
        path,
    )
    .unwrap();
    lazypencil::paths::canonical(path).unwrap()
}

#[test]
fn prompt_shortcuts_are_user_owned_and_visible_in_config_and_schema() {
    let f = Fixture::new();
    f.settings("[tui.keys]\nsubmit=['F5', 'Ctrl+Alt+s']\npaste=[]\n");
    let shown = f.data(&["config", "show", "--user"]);
    assert_eq!(
        shown["settings"]["tui"]["keys"],
        json!({"submit":["F5","Ctrl+Alt+s"],"paste":[]})
    );
    let snapshot = f.snapshot().unwrap();
    assert_eq!(
        snapshot.summary()["user"]["settings"]["tui"]["keys"],
        shown["settings"]["tui"]["keys"]
    );
    let schema = f.data(&["config", "schema", "--user"]);
    assert!(schema["properties"]["tui"].is_object());
    fs::write(
        f.project.join("lazypencil.toml"),
        "[tui.keys]\nsubmit=['F6']\n",
    )
    .unwrap();
    assert!(f.snapshot().is_err(), "Projects cannot remap a user's keys");
}

#[test]
fn invalid_shortcuts_fail_without_silently_restoring_defaults() {
    let f = Fixture::new();
    for body in [
        "[tui.keys]\nsubmit=['Enter']\n",
        "[tui.keys]\npaste=['Ctrl+c']\n",
        "[tui.keys]\nsubmit=['F5']\npaste=['f5']\n",
        "[tui.keys]\nsubmit=[]\n",
        "[tui.keys]\npastes=['F6']\n",
    ] {
        f.settings(body);
        let output = f.cli(&["config", "show", "--user"]);
        assert!(
            !output.status.success(),
            "Unexpected accepted settings: {body}"
        );
        assert!(f.snapshot().is_err());
    }
}

#[test]
fn project_fields_override_independently_and_empty_arrays_stop_inheritance() {
    let f = Fixture::new();
    let user_font = font(&f.user.join("fonts/regular.ttf"), "regular.ttf");
    let user_extra = font(&f.user.join("extra/cff.otf"), "cff.otf");
    f.settings("[fonts]\nfiles=['fonts/regular.ttf']\nsearch_paths=['extra']\ndefault_family='LP Test Sans'\nlocale='en-US'\nfallbacks=['Noto Sans JP']\n");
    fs::write(
        f.project.join("lazypencil.toml"),
        "[fonts]\ndefault_family='Noto Sans JP'\nlocale='ja'\n",
    )
    .unwrap();
    let s = f.snapshot().unwrap();
    assert_eq!(s.root(), f.project);
    assert_eq!(s.font_files, vec![user_font, user_extra]);
    assert_eq!(s.settings.fonts.default_family, "Noto Sans JP");
    assert_eq!(s.settings.fonts.locale, "ja");
    assert_eq!(
        s.settings.fonts.fallbacks,
        Some(vec!["Noto Sans JP".into()])
    );
    assert_eq!(
        s.summary()["font_origins"],
        json!({"files":"user","search_paths":"user","default_family":"project","locale":"project","fallbacks":"user"})
    );
    assert_eq!(s.sources().len(), 2);
    s.verify_unchanged().unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[fonts]\nfiles=[]\nsearch_paths=[]\nfallbacks=[]\ndefault_family='Noto Sans JP'\n",
    )
    .unwrap();
    assert_eq!(s.verify_unchanged().unwrap_err().code, "REVISION_CONFLICT");
    let s = f.snapshot().unwrap();
    assert!(s.font_files.is_empty());
    assert_eq!(s.settings.fonts.fallbacks, Some(vec![]));
    assert_eq!(s.settings.fonts.locale, "en-US");
    f.settings("[fonts]\nfiles=['missing.ttf']\nsearch_paths=['missing-directory']\n");
    assert!(
        f.snapshot().unwrap().font_files.is_empty(),
        "Overridden file lists must not be opened"
    );
}

#[test]
fn project_search_precedes_inherited_user_files_and_user_configuration_is_optional() {
    let f = Fixture::new();
    let absent = f.snapshot().unwrap();
    assert_eq!(absent.summary()["user"]["path"], Value::Null);
    assert!(!f.user.exists());
    assert!(absent.lock_paths().contains(&f.user.join("config.toml")));
    let user_font = font(&f.user.join("fonts/faces.ttc"), "faces.ttc");
    let project_font = font(&f.project.join("fonts/regular.ttf"), "regular.ttf");
    f.settings("[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\n");
    assert_eq!(
        absent.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    fs::write(
        f.project.join("lazypencil.toml"),
        "[fonts]\nsearch_paths=['fonts']\n",
    )
    .unwrap();
    let s = f.snapshot().unwrap();
    assert_eq!(s.font_files, vec![project_font, user_font]);
    assert_eq!(s.settings.fonts.default_family, "LP Test Sans");
    let isolated = Snapshot::load_with_user_file(&f.document, None).unwrap();
    assert_eq!(isolated.font_files.len(), 1);
    assert_eq!(isolated.settings.fonts.default_family, "Noto Sans JP");
}

#[test]
fn user_source_and_search_membership_are_part_of_snapshot_validation() {
    let f = Fixture::new();
    font(&f.user.join("fonts/regular.ttf"), "regular.ttf");
    f.settings("[fonts]\nsearch_paths=['fonts']\nlocale='en-US'\n");
    let first = f.snapshot().unwrap();
    font(&f.user.join("fonts/italic.ttf"), "italic.ttf");
    assert_eq!(
        first.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let second = f.snapshot().unwrap();
    let standalone = UserSnapshot::load_file(Some(f.user.join("config.toml"))).unwrap();
    f.settings("[fonts]\nsearch_paths=['fonts']\nlocale='en-US'\n# comment changed\n");
    assert_eq!(
        second.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        standalone.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let third = f.snapshot().unwrap();
    fs::remove_file(f.user.join("config.toml")).unwrap();
    assert_eq!(
        third.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(f.snapshot().unwrap().settings.fonts.locale, "ja");
}

#[test]
fn user_configuration_rejects_unsupported_scopes_and_reports_no_source_secrets() {
    let f = Fixture::new();
    for body in [
        "version=2",
        "[design]\nstyles=[]",
        "[agent]\ncommand='SECRET_NOT_TO_LOG'",
        "[fonts]\nlocale='ja_JP'",
        "[fonts]\nfiles=['https://example.invalid/SECRET_NOT_TO_LOG']",
        "[fonts]\nfiles='SECRET_NOT_TO_LOG'",
        "version=1\nversion=1",
    ] {
        f.settings(body);
        let error = f.snapshot().err().unwrap();
        assert_eq!(error.code, "CONFIGURATION_ERROR");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("SECRET_NOT_TO_LOG")
        );
    }
    fs::write(f.user.join("config.toml"), vec![b' '; 256 * 1024 + 1]).unwrap();
    assert_eq!(f.snapshot().err().unwrap().code, "CONFIGURATION_ERROR");
    fs::write(f.user.join("config.toml"), [0xff]).unwrap();
    assert_eq!(f.snapshot().err().unwrap().code, "CONFIGURATION_ERROR");
}

#[test]
fn native_user_configuration_commands_create_once_and_project_defaults_inherit() {
    let f = Fixture::new();
    let missing = f.data(&["config", "show", "--user"]);
    assert_eq!(missing["requested_path"], json!(f.user.join("config.toml")));
    assert!(missing["path"].is_null());
    assert!(!f.user.exists());
    let initialized = f.data(&["config", "init", "--user"]);
    assert_eq!(initialized["scope"], "user");
    let saved = fs::read(f.user.join("config.toml")).unwrap();
    assert!(!f.cli(&["config", "init", "--user"]).status.success());
    assert_eq!(fs::read(f.user.join("config.toml")).unwrap(), saved);
    let schema = f.data(&["config", "schema", "--user"]);
    assert!(schema["properties"]["fonts"].is_object());
    assert!(schema["properties"].get("design").is_none());
    font(&f.user.join("fonts/regular.ttf"), "regular.ttf");
    f.settings(
        "[fonts]\nfiles=['fonts/regular.ttf']\ndefault_family='LP Test Sans'\nlocale='en-US'\n",
    );
    let project_file = f.project.join("lazypencil.toml");
    f.data(&["config", "init", project_file.to_str().unwrap()]);
    let summary = f.data(&["config", "show", f.document.to_str().unwrap()]);
    assert_eq!(
        summary["settings"]["fonts"]["default_family"],
        "LP Test Sans"
    );
    assert_eq!(summary["font_origins"]["locale"], "user");
    assert_eq!(
        summary["user"]["path"],
        json!(lazypencil::paths::canonical(f.user.join("config.toml")).unwrap())
    );
    assert_eq!(
        f.cli(&["config", "init", project_file.to_str().unwrap(), "--user"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn user_bytes_change_dependencies_and_committed_retries_survive_bad_user_configuration() {
    let f = Fixture::new();
    f.settings("[fonts]\nlocale='en-US'\n");
    let before = f.data(&["inspect", f.document.to_str().unwrap()]);
    let canonical = lazypencil::paths::canonical(f.user.join("config.toml")).unwrap();
    assert!(
        before["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["path"] == json!(canonical) && d["kind"] == "configuration")
    );
    assert_eq!(
        before["configuration"]["root"],
        json!(lazypencil::paths::canonical(&f.project).unwrap())
    );
    f.settings("[fonts]\nlocale='en-US'\n# Changed\n");
    let newer = f.data(&["inspect", f.document.to_str().unwrap()]);
    assert_eq!(newer["revision"], before["revision"]);
    assert_ne!(newer["dependency_revision"], before["dependency_revision"]);
    let ops = f.temp.path().join("ops.json");
    fs::write(
        &ops,
        json!([{"op":"insert","node":{"type":"rectangle","width":30,"height":20}}]).to_string(),
    )
    .unwrap();
    let args = [
        "apply",
        f.document.to_str().unwrap(),
        "--operations",
        ops.to_str().unwrap(),
        "--expected-revision",
        newer["revision"].as_str().unwrap(),
        "--expected-dependency-revision",
        newer["dependency_revision"].as_str().unwrap(),
        "--request-id",
        "user-config-edit",
    ];
    let saved = f.data(&args);
    let document = fs::read(&f.document).unwrap();
    f.settings("not valid = TOML");
    let replay = f.data(&args);
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["revision"], saved["revision"]);
    let mut new_args = args;
    new_args[5] = saved["revision"].as_str().unwrap();
    new_args[9] = "fresh-user-config-edit";
    let output = f.cli(&new_args);
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "CONFIGURATION_ERROR"
    );
    assert_eq!(fs::read(&f.document).unwrap(), document);
}

#[test]
fn user_initialization_waits_for_configuration_readers() {
    let f = Fixture::new();
    let store = Store::open(&f.document, Some(f.temp.path().join("data"))).unwrap();
    let path = f.user.join("config.toml");
    let locks = store
        .lock_dependencies(std::slice::from_ref(&path))
        .unwrap();
    let mut command = f.command(&["config", "init", "--user"]);
    let child = lock_wait::WaitingProcess::spawn(&mut command);
    assert!(!f.user.exists());
    drop(locks);
    let output = child.wait_with_output();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(path.is_file());
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn user_font_changes_invalidate_data_without_revoking_project_instruction_trust() {
    let f = Fixture::new();
    fs::write(f.project.join("brand.txt"), "Keep the project color roles.").unwrap();
    fs::write(
        f.project.join("lazypencil.toml"),
        "[[design.guides]]\nid='brand'\npath='brand.txt'\n",
    )
    .unwrap();
    f.settings("[fonts]\nlocale='en-US'\n");
    let before = f.data(&["config", "show", f.document.to_str().unwrap()]);
    assert_eq!(before["design"]["trusted"], false);
    f.data(&[
        "config",
        "trust",
        f.document.to_str().unwrap(),
        "--expected-fingerprint",
        before["design"]["fingerprint"].as_str().unwrap(),
    ]);
    let trusted = f.data(&["inspect", f.document.to_str().unwrap()]);
    assert_eq!(trusted["configuration"]["design"]["trusted"], true);
    f.settings("[fonts]\nlocale='ja'\n");
    let changed = f.data(&["inspect", f.document.to_str().unwrap()]);
    assert_ne!(
        changed["dependency_revision"],
        trusted["dependency_revision"]
    );
    assert_eq!(changed["configuration"]["design"]["trusted"], true);
    assert_eq!(
        changed["configuration"]["design"]["trust_fingerprint"],
        trusted["configuration"]["design"]["trust_fingerprint"]
    );
    let guide = f.data(&[
        "guidance",
        "project:brand",
        "--file",
        f.document.to_str().unwrap(),
    ]);
    assert_eq!(guide["body"], "Keep the project color roles.");
    fs::write(f.project.join("brand.txt"), "Changed project instructions.").unwrap();
    let rejected = f.cli(&[
        "guidance",
        "project:brand",
        "--file",
        f.document.to_str().unwrap(),
    ]);
    assert_eq!(
        serde_json::from_slice::<Value>(&rejected.stdout).unwrap()["error"]["code"],
        "PROJECT_NOT_TRUSTED"
    );
}

#[cfg(unix)]
#[test]
fn user_initialization_can_be_cancelled_without_creating_its_directory() {
    let f = Fixture::new();
    let store = Store::open(&f.document, Some(f.temp.path().join("data"))).unwrap();
    let locks = store
        .lock_dependencies(&[f.user.join("config.toml")])
        .unwrap();
    let child = lock_wait::WaitingProcess::spawn(&mut f.command(&["config", "init", "--user"]));
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let output = child.wait_with_output();
    assert_eq!(output.status.code(), Some(130));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
        "CANCELLED"
    );
    assert!(!f.user.exists());
    drop(locks);
}

#[cfg(unix)]
#[test]
fn user_configuration_uses_its_physical_base_and_alias_retargeting_invalidates_it() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::create_dir(&f.user).unwrap();
    let physical = f.temp.path().join("physical");
    let actual = font(&physical.join("fonts/regular.ttf"), "regular.ttf");
    let body = "[fonts]\nfiles=['fonts/regular.ttf']\n";
    fs::write(physical.join("a.toml"), body).unwrap();
    fs::write(physical.join("b.toml"), body).unwrap();
    let alias = f.user.join("config.toml");
    symlink(physical.join("a.toml"), &alias).unwrap();
    let frozen = f.snapshot().unwrap();
    assert_eq!(frozen.font_files, vec![actual]);
    fs::remove_file(&alias).unwrap();
    symlink(physical.join("b.toml"), &alias).unwrap();
    assert_eq!(
        frozen.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn user_fonts_apply_to_standalone_inventory_and_layout_with_cli_precedence() {
    let f = Fixture::new();
    let user_font = font(&f.user.join("fonts/faces.ttc"), "faces.ttc");
    let project_font = font(&f.project.join("fonts/regular.ttf"), "regular.ttf");
    f.settings(
        "[fonts]\nfiles=['fonts/faces.ttc']\ndefault_family='LP Test Sans'\nlocale='en-US'\n",
    );
    fs::write(
        f.project.join("lazypencil.toml"),
        "[fonts]\nsearch_paths=['fonts']\n",
    )
    .unwrap();
    fs::write(&f.document, json!({"version":"2.21","children":[{"id":"label","type":"text","content":"A","fontSize":24,"fill":"#111111"}]}).to_string()).unwrap();
    let inventory = f.data(&["fonts", "list"]);
    assert_eq!(inventory["default_family"], "LP Test Sans");
    assert_eq!(inventory["locale"], "en-US");
    let layout = f.data(&["layout", f.document.to_str().unwrap()]);
    assert_eq!(
        layout["nodes"][0]["text"]["fonts"][0]["hash"],
        lazypencil::document::hash(&fs::read(&project_font).unwrap())
    );
    assert_eq!(layout["nodes"][0]["text"]["locale"], "en-US");
    let explicit = f.data(&[
        "layout",
        f.document.to_str().unwrap(),
        "--font",
        user_font.to_str().unwrap(),
    ]);
    assert_eq!(
        explicit["nodes"][0]["text"]["fonts"][0]["hash"],
        lazypencil::document::hash(&fs::read(&user_font).unwrap())
    );
    let png = f.temp.path().join("preview.png");
    let rendered = f.data(&[
        "render",
        f.document.to_str().unwrap(),
        "--output",
        png.to_str().unwrap(),
    ]);
    assert_eq!(rendered["diagnostics"], json!([]));
    assert!(fs::read(png).unwrap().starts_with(b"\x89PNG"));
}
