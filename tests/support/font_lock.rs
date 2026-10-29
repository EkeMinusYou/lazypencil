use super::*;
use lazypencil::{
    font_lock,
    render::{self, RenderOptions},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

fn configured() -> (TempDir, Store, PathBuf) {
    let (temp, store) = fixture(json!([
        {"id":"label","type":"text","content":"A","fontFamily":"LP Test Sans","fontSize":20,"width":40,"height":30,"fill":"#112233"}
    ]));
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/regular.ttf"),
        temp.path().join("regular.ttf"),
    )
    .unwrap();
    fs::write(
        temp.path().join("lazypencil.toml"),
        "font_lock='fonts.lock.json'\n[fonts]\nfiles=['regular.ttf']\nfallbacks=[]\n",
    )
    .unwrap();
    let path = temp.path().join("fonts.lock.json");
    (temp, store, path)
}

fn cli(store: &Store, args: &[&str]) -> Value {
    let output = command_output::output(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .arg("--json")
            .arg("--data-dir")
            .arg(store.data_dir())
            .args(["fonts"])
            .args(args),
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
    .unwrap();
    assert!(
        !output.timed_out,
        "CLI timed out after {:?}",
        output.elapsed
    );
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn font_locks_capture_portable_data_allow_relocation_and_require_reviewed_updates() {
    let (temp, store, path) = configured();
    let original = fs::read(store.path()).unwrap();
    let missing = Project::load(&store).unwrap();
    let created = font_lock::create(&store, &[], None, None).unwrap();
    assert_eq!(created["active"], true);
    assert_eq!(created["reused"], false);
    assert_eq!(
        missing.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let baseline = fs::read(&path).unwrap();
    let manifest: Value = serde_json::from_slice(&baseline).unwrap();
    assert_eq!(manifest["version"], 1);
    assert_eq!(
        manifest["input_hashes"][0],
        lazypencil::document::hash(&fs::read(temp.path().join("regular.ttf")).unwrap())
    );
    assert!(
        manifest["faces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|face| face["source"] == "file" && face["family"] == "LP Test Sans")
    );
    assert!(manifest["faces"].as_array().unwrap().iter().any(|face| face["source"] == "bundled" && !face["axes"].as_array().unwrap().is_empty()));
    assert!(
        !String::from_utf8(baseline.clone())
            .unwrap()
            .contains(temp.path().to_str().unwrap())
    );
    assert_eq!(
        font_lock::create(&store, &[], None, None).unwrap()["reused"],
        true
    );
    let (project, layout) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let before = render::png(&project, &layout, &RenderOptions::default()).unwrap();
    assert!(before.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(font_lock::check(&store, &[]).unwrap()["valid"], true);
    fs::rename(
        temp.path().join("regular.ttf"),
        temp.path().join("moved.ttf"),
    )
    .unwrap();
    fs::write(
        temp.path().join("lazypencil.toml"),
        "font_lock='fonts.lock.json'\n[fonts]\nfiles=['moved.ttf']\nfallbacks=[]\n",
    )
    .unwrap();
    assert_eq!(font_lock::check(&store, &[]).unwrap()["valid"], true);
    let frozen = Project::load(&store).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join("moved.ttf"))
        .unwrap()
        .write_all(&[0])
        .unwrap();
    assert_eq!(
        frozen.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::layout_snapshot(&store, &[], None, None)
            .err()
            .unwrap()
            .code,
        "FONT_LOCK_MISMATCH"
    );
    assert_eq!(
        font_lock::create(&store, &[], None, None).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(&path).unwrap(), baseline);
    assert_eq!(
        font_lock::create(&store, &[], None, Some(&"0".repeat(64)))
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    font_lock::create(&store, &[], None, created["hash"].as_str()).unwrap();
    let (project, layout) = service::layout_snapshot(&store, &[], None, None).unwrap();
    let after = render::png(&project, &layout, &RenderOptions::default()).unwrap();
    assert_eq!(
        before.png, after.png,
        "Trailing padding changes source identity, not glyph output"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn font_locks_guard_deferred_faces_settings_engine_and_schema_without_blocking_edits() {
    let (temp, store, path) = configured();
    font_lock::create(&store, &[], None, None).unwrap();
    let manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut removed = manifest.clone();
    removed["faces"]
        .as_array_mut()
        .unwrap()
        .retain(|face| face["source"] != "file");
    fs::write(&path, serde_json::to_vec(&removed).unwrap()).unwrap();
    let project = Project::load(&store).unwrap();
    project.load_fonts().unwrap();
    let error = service::layout_snapshot(&store, &[], None, None)
        .err()
        .unwrap();
    assert_eq!(error.code, "FONT_LOCK_MISMATCH");
    assert_eq!(error.details.unwrap()["reason"], "unlocked_font_face");
    service::apply_request(
        &store,
        &request(
            &store,
            "edit-with-mismatched-lock",
            json!([
                {"op":"update","target":"label","set":{"name":"Kept edit"}}
            ]),
            false,
        ),
    )
    .unwrap();
    for (field, value, code) in [
        ("version", json!(2), "FONT_LOCK_INVALID"),
        ("unexpected", json!(true), "FONT_LOCK_INVALID"),
        ("engine", json!("different-engine"), "FONT_LOCK_MISMATCH"),
        (
            "native_sources",
            json!("0".repeat(64)),
            "FONT_LOCK_MISMATCH",
        ),
        ("locale", json!("en-US"), "FONT_LOCK_MISMATCH"),
        (
            "default_family",
            json!("different-family"),
            "FONT_LOCK_MISMATCH",
        ),
        (
            "fallback_families",
            json!(["Noto Sans JP"]),
            "FONT_LOCK_MISMATCH",
        ),
    ] {
        let mut changed = manifest.clone();
        changed[field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert_eq!(
            Project::load(&store)
                .unwrap()
                .load_fonts()
                .err()
                .unwrap()
                .code,
            code,
            "{field}"
        );
    }
    fs::write(&path, b"{ secret-private-file-content }").unwrap();
    let error = Project::load(&store).unwrap().load_fonts().err().unwrap();
    assert_eq!(error.code, "FONT_LOCK_INVALID");
    assert!(!error.to_string().contains("secret-private"));
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert_eq!(font_lock::check(&store, &[]).unwrap()["valid"], true);
    assert_eq!(
        fs::read(temp.path().join("regular.ttf")).unwrap(),
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/regular.ttf"))
            .unwrap()
    );
}

#[test]
fn font_lock_cli_captures_generated_text_and_checks_the_active_project_baseline() {
    let (temp, store, path) = configured();
    fs::write(temp.path().join("generated.js"), "/** @schema 2.21 */\nreturn [{id:'generated-label',type:'text',content:'A',fontFamily:'LP Test Sans',fontSize:20,width:40,height:30}];\n").unwrap();
    fs::write(store.path(), json!({"version":"2.21","children":[{"id":"generator","type":"script","scriptUri":"generated.js","width":40,"height":30}]}).to_string()).unwrap();
    let created = cli(&store, &["lock", store.path().to_str().unwrap()]);
    assert_eq!(created["active"], true);
    assert!(path.is_file());
    let checked = cli(&store, &["check", store.path().to_str().unwrap()]);
    assert_eq!(checked["valid"], true);
    let listed = cli(&store, &["list", "--file", store.path().to_str().unwrap()]);
    assert!(
        listed["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|font| font["family"] == "LP Test Sans")
    );
}

#[cfg(unix)]
#[test]
fn font_lock_publication_refuses_linked_outputs_and_existing_project_inputs() {
    let (temp, store, path) = configured();
    font_lock::create(&store, &[], None, None).unwrap();
    let original = fs::read(&path).unwrap();
    let linked = temp.path().join("linked.json");
    std::os::unix::fs::symlink(&path, &linked).unwrap();
    assert_eq!(
        font_lock::create(&store, &[], Some(&linked), None)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    // The source itself is protected even when its spelling has a .json suffix.
    let json_document = temp.path().join("document.json");
    fs::write(&json_document, fs::read(store.path()).unwrap()).unwrap();
    let json_store = Store::open(&json_document, Some(store.data_dir().into())).unwrap();
    assert_eq!(
        font_lock::create(&json_store, &[], Some(&json_document), None)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
}
