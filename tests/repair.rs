use base64::Engine;
use lazypencil::assets::{self, AssetKind};
use lazypencil::{operations::EditRequest, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, process::Command};
use tempfile::TempDir;

fn image(uri: &str) -> Value {
    json!({"id":"image","type":"rectangle","width":20,"height":20,"fill":{"type":"image","url":uri}})
}
fn fixture(children: Value) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let path = temp.path().join("design.pen");
    fs::write(
        &path,
        format!(
            "// Preserve this comment.\n{}",
            json!({"version":"2.21","future":{"keep":17},"children":children})
        ),
    )
    .unwrap();
    let store = Store::open(path, Some(temp.path().join("state"))).unwrap();
    (temp, store)
}
fn request(store: &Store, id: &str, operations: Value, dry_run: bool) -> EditRequest {
    let metadata = service::document_metadata(store).unwrap();
    assert!(metadata.get("dependency_error").is_none(), "{metadata}");
    serde_json::from_value(json!({"request_id":id,"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":operations,"dry_run":dry_run})).unwrap()
}
fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4/5/hPwgzwBgAbaQL9REU6k8AAAAASUVORK5CYII=").unwrap()
}

#[test]
fn declared_missing_assets_have_revisions_and_support_partial_dry_runs_and_saved_repairs() {
    let (temp, store) = fixture(json!([
        image("images/lost.png"),
        {"id":"script","type":"script","x":20,"width":20,"height":20,"scriptUri":"scripts/lost.js"},
        {"id":"shader","type":"rectangle","x":40,"width":20,"height":20,"fill":{"type":"shader","url":"shaders/lost.glsl","uniforms":{"image":"textures/lost.png"}}}
    ]));
    let before = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    assert_eq!(metadata["assets_complete"], false);
    assert_eq!(metadata["asset_diagnostics"].as_array().unwrap().len(), 4);
    assert!(
        metadata["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|dependency| dependency["availability"] == "missing")
    );
    assert_eq!(
        service::validate_document(&store).unwrap_err().code,
        "ASSET_NOT_FOUND"
    );
    let result = service::apply_request(
        &store,
        &request(
            &store,
            "dry",
            json!([{"op":"delete","target":"image"}]),
            true,
        ),
    )
    .unwrap();
    assert_eq!(result["assets_complete"], false);
    assert_eq!(result["asset_diagnostics"].as_array().unwrap().len(), 3);
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(store.history(10, None).unwrap().is_empty());
    for directory in ["images", "scripts", "shaders", "textures"] {
        assert!(!temp.path().join(directory).exists());
    }
    let saved_request = request(
        &store,
        "remove-image",
        json!([{"op":"delete","target":"image"}]),
        false,
    );
    let saved = service::apply_request(&store, &saved_request).unwrap();
    assert_eq!(saved["assets_complete"], false);
    assert_eq!(
        saved["dependency_revision"],
        service::document_metadata(&store).unwrap()["dependency_revision"]
    );
    assert_eq!(
        service::apply_request(&store, &saved_request).unwrap()["replayed"],
        true
    );
    let repaired = service::apply_request(
        &store,
        &request(
            &store,
            "remove-code",
            json!([{"op":"delete","target":"script"},{"op":"delete","target":"shader"}]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(repaired["assets_complete"], true);
    assert!(repaired["asset_diagnostics"].as_array().unwrap().is_empty());
    let document = store.read().unwrap();
    assert!(document.text().contains("// Preserve this comment."));
    assert_eq!(document.value()["future"]["keep"], 17);
    assert_eq!(store.history(10, None).unwrap().len(), 2);
}

#[test]
#[cfg(feature = "rendering")]
fn uploading_a_replacement_preserves_missing_revision_until_the_reference_changes() {
    let (_temp, store) = fixture(json!([image("assets/unrelated/lost.png")]));
    let before = service::document_metadata(&store).unwrap();
    let asset = assets::put_bytes(&store, AssetKind::Image, &png(), Some("image/png")).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    assert_eq!(
        before["dependency_revision"],
        metadata["dependency_revision"]
    );
    let result=service::apply_request(&store,&request(&store,"replace",json!([{"op":"update","target":"image","set":{"fill":{"type":"image","url":asset.uri}}}]),false)).unwrap();
    assert_eq!(result["assets_complete"], true);
    #[cfg(feature = "rendering")]
    {
        let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let output = lazypencil::render::png(&project, &snapshot, &Default::default()).unwrap();
        assert_eq!((output.width, output.height), (20, 20));
    }
}

#[test]
fn missing_script_and_shader_sources_can_be_replaced_in_separate_saved_batches() {
    let (_temp, store) = fixture(json!([
        {"id":"code","type":"script","width":20,"height":20,"scriptUri":"lost.js"},
        {"id":"shader","type":"rectangle","width":20,"height":20,"fill":{"type":"shader","url":"lost.glsl"}}
    ]));
    let script = assets::put_bytes(&store, AssetKind::Script, b"/**\n * @schema 2.21\n */\nreturn [{type:'rectangle',width:20,height:20,fill:'#2563EB'}];", None).unwrap();
    let shader = assets::put_bytes(
        &store,
        AssetKind::Shader,
        b"void main(){gl_FragColor=vec4(1.0);}",
        None,
    )
    .unwrap();
    let partial = service::apply_request(
        &store,
        &request(
            &store,
            "replace-script",
            json!([
                {"op":"update","target":"code","set":{"scriptUri":script.uri}}
            ]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(partial["assets_complete"], false);
    assert_eq!(partial["asset_diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(partial["asset_diagnostics"][0]["kind"], "shader");
    let repaired = service::apply_request(
        &store,
        &request(
            &store,
            "replace-shader",
            json!([
                {"op":"update","target":"shader","set":{"fill":{"type":"shader","url":shader.uri}}}
            ]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(repaired["assets_complete"], true);
    assert_eq!(
        repaired["dependency_revision"],
        service::document_metadata(&store).unwrap()["dependency_revision"]
    );
    service::validate_document(&store).unwrap();
    #[cfg(feature = "rendering")]
    {
        service::apply_request(
            &store,
            &request(
                &store,
                "script-preview",
                json!([
                    {"op":"delete","target":"shader"}
                ]),
                false,
            ),
        )
        .unwrap();
        let (project, snapshot) = service::layout_snapshot_with_runtime(
            &store,
            &[],
            None,
            None,
            &mut lazypencil::script::ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        )
        .unwrap();
        let output = lazypencil::render::png(&project, &snapshot, &Default::default()).unwrap();
        let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&output.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(
            image.peek_pixels().unwrap().get_color((10, 10)),
            skia_safe::Color::from_rgb(0x25, 0x63, 0xeb)
        );
    }
}

#[test]
fn a_missing_reference_still_requires_an_explicit_dependency_precondition() {
    let (_temp, store) = fixture(json!([image("lost.png")]));
    let mut edit = request(
        &store,
        "no-precondition",
        json!([{"op":"delete","target":"image"}]),
        false,
    );
    edit.expected_dependency_revision = None;
    let original = fs::read(store.path()).unwrap();
    assert_eq!(
        service::apply_request(&store, &edit).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn unsafe_urls_and_existing_directories_are_not_treated_as_missing_files() {
    for uri in [
        "https://example.invalid/lost.png",
        "file:///lost.png",
        "directory.png",
    ] {
        let (temp, store) = fixture(json!([image(uri)]));
        fs::create_dir(temp.path().join("directory.png")).unwrap();
        let metadata = service::document_metadata(&store).unwrap();
        assert!(
            metadata.get("dependency_error").is_some(),
            "{uri}: {metadata}"
        );
        assert!(
            metadata.get("assets_complete").is_none(),
            "{uri}: {metadata}"
        );
        assert!(Project::load(&store).is_err());
    }
}

#[test]
fn ordinary_edits_cannot_introduce_new_missing_paths_or_resource_kinds() {
    let (_temp, store) = fixture(json!([image("old.png")]));
    let before = fs::read(store.path()).unwrap();
    for operation in [
        json!({"op":"update","target":"image","set":{"fill":{"type":"image","url":"another.png"}}}),
        json!({"op":"update","target":"image","set":{"fill":{"type":"shader","url":"old.png"}}}),
    ] {
        let error = service::apply_request(
            &store,
            &request(&store, "new-missing", json!([operation]), false),
        )
        .unwrap_err();
        assert_eq!(error.code, "ASSET_NOT_FOUND");
        assert_eq!(fs::read(store.path()).unwrap(), before);
    }
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn one_missing_path_retains_each_resource_kind_during_partial_repairs() {
    let (_temp, store) = fixture(json!([
        image("shared.asset"),
        {"id":"code","type":"script","width":20,"height":20,"scriptUri":"shared.asset"}
    ]));
    let metadata = service::document_metadata(&store).unwrap();
    let diagnostics = metadata["asset_diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0]["path"], diagnostics[1]["path"]);
    assert_eq!(diagnostics[0]["kind"], "image");
    assert_eq!(diagnostics[1]["kind"], "script");
    let repaired = service::apply_request(
        &store,
        &request(
            &store,
            "retain-code",
            json!([{"op":"delete","target":"image"}]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(repaired["assets_complete"], false);
    assert_eq!(repaired["asset_diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(repaired["asset_diagnostics"][0]["kind"], "script");
    assert_eq!(
        repaired["dependency_revision"],
        service::document_metadata(&store).unwrap()["dependency_revision"]
    );
    let original = fs::read(store.path()).unwrap();
    let error = service::apply_request(&store, &request(&store, "introduce-shader", json!([
        {"op":"insert","node":{"type":"rectangle","width":20,"height":20,"fill":{"type":"shader","url":"shared.asset"}}}
    ]), false)).unwrap_err();
    assert_eq!(error.code, "ASSET_NOT_FOUND");
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn ordinary_parent_directory_creation_preserves_the_missing_file_revision() {
    for uri in [
        "new/nested/lost.png",
        "new/./nested/lost.png",
        "./new/nested/lost.png",
        "new/nested/./lost.png",
    ] {
        let (temp, store) = fixture(json!([image(uri)]));
        let metadata = service::document_metadata(&store).unwrap();
        let project = Project::load(&store).unwrap();
        let edit = request(
            &store,
            "after-directories",
            json!([{"op":"delete","target":"image"}]),
            false,
        );
        fs::create_dir_all(temp.path().join("new/nested")).unwrap();
        project.verify_unchanged().unwrap();
        assert_eq!(
            metadata["dependency_revision"],
            service::document_metadata(&store).unwrap()["dependency_revision"],
            "{uri}"
        );
        assert_eq!(
            service::apply_request(&store, &edit).unwrap()["assets_complete"],
            true
        );
    }
}

#[cfg(unix)]
#[test]
fn a_new_parent_symlink_cannot_redirect_a_missing_resource() {
    use std::os::unix::fs::symlink;
    let (temp, store) = fixture(json!([image("new/nested/lost.png")]));
    let project = Project::load(&store).unwrap();
    let edit = request(
        &store,
        "redirected",
        json!([{"op":"delete","target":"image"}]),
        false,
    );
    let original = fs::read(store.path()).unwrap();
    fs::create_dir(temp.path().join("other")).unwrap();
    symlink("other", temp.path().join("new")).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::apply_request(&store, &edit).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn an_asset_appearing_after_inspection_invalidates_the_edit() {
    let (temp, store) = fixture(json!([image("images/lost.png")]));
    let stale_request = request(
        &store,
        "stale",
        json!([{"op":"delete","target":"image"}]),
        false,
    );
    let before = fs::read(store.path()).unwrap();
    let project = Project::load(&store).unwrap();
    fs::create_dir(temp.path().join("images")).unwrap();
    fs::write(temp.path().join("images/lost.png"), png()).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::apply_request(&store, &stale_request)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn staged_imports_can_create_asset_directories_without_changing_an_absent_sibling() {
    let (_temp, store) = fixture(json!([image("assets/unrelated/lost.png")]));
    let saved=service::apply_request(&store,&request(&store,"import",json!([{"op":"import_component","mode":"link","source":{"style_id":"standard-ui","node_id":"button"}}]),false)).unwrap();
    assert_eq!(saved["assets_complete"], false);
    assert_eq!(saved["asset_diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(
        saved["dependency_revision"],
        service::document_metadata(&store).unwrap()["dependency_revision"]
    );
}

#[test]
fn restoring_a_checkpoint_reports_assets_that_are_still_missing() {
    let (_temp, store) = fixture(json!([image("lost.png")]));
    let first = service::apply_request(
        &store,
        &request(
            &store,
            "name",
            json!([{"op":"update","target":"image","set":{"name":"Working"}}]),
            false,
        ),
    )
    .unwrap();
    service::apply_request(
        &store,
        &request(
            &store,
            "remove",
            json!([{"op":"delete","target":"image"}]),
            false,
        ),
    )
    .unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let result = service::restore_checkpoint(
        &store,
        &service::RestoreRequest {
            request_id: "restore-missing",
            expected_revision: metadata["revision"].as_str().unwrap(),
            expected_dependency_revision: metadata["dependency_revision"].as_str(),
            checkpoint: first["checkpoint_id"].as_str().unwrap(),
        },
        &[],
    )
    .unwrap();
    assert_eq!(result["assets_complete"], false);
    assert_eq!(
        store.read().unwrap().node("image").unwrap()["name"],
        "Working"
    );
    assert_eq!(
        service::validate_document(&store).unwrap_err().code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(store.history(10, None).unwrap().len(), 3);
}

#[test]
fn external_instances_with_missing_images_can_be_detached_and_repaired() {
    let (temp, store) = fixture(json!([{"id":"use","type":"ref","ref":"ui:tile"}]));
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(temp.path().join("library/ui.lib.pen"),json!({"version":"2.21","children":[{"id":"tile","type":"rectangle","reusable":true,"width":20,"height":20,"fill":{"type":"image","url":"data/lost.png"}}]}).to_string()).unwrap();
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"ui":"library/ui.lib.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let result = service::apply_request(
        &store,
        &request(
            &store,
            "detach",
            json!([{"op":"detach_instance","target":"use"}]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(result["assets_complete"], false);
    assert_eq!(
        store.read().unwrap().node("use").unwrap()["fill"]["url"],
        "library/data/lost.png"
    );
    assert!(
        result["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|dependency| dependency["availability"] == "missing")
    );
}

#[cfg(unix)]
#[test]
fn missing_links_are_tracked_and_retargeting_an_absent_parent_conflicts() {
    use std::os::unix::fs::symlink;
    let (temp, store) = fixture(json!([image("alias/lost.png")]));
    fs::create_dir(temp.path().join("one")).unwrap();
    fs::create_dir(temp.path().join("two")).unwrap();
    symlink("one", temp.path().join("alias")).unwrap();
    let project = Project::load(&store).unwrap();
    let stale_request = request(
        &store,
        "stale-link",
        json!([{"op":"delete","target":"image"}]),
        false,
    );
    fs::remove_file(temp.path().join("alias")).unwrap();
    symlink("two", temp.path().join("alias")).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::apply_request(&store, &stale_request)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let (temp, store) = fixture(json!([image("link.png")]));
    symlink("nested/lost.png", temp.path().join("link.png")).unwrap();
    assert_eq!(
        service::document_metadata(&store).unwrap()["assets_complete"],
        false
    );
    let result = service::apply_request(
        &store,
        &request(
            &store,
            "remove-link",
            json!([{"op":"delete","target":"image"}]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(result["assets_complete"], true);
    assert!(
        fs::symlink_metadata(temp.path().join("link.png"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn cli_inspection_and_repair_share_the_missing_dependency_revision() {
    let (temp, store) = fixture(json!([image("images/lost.png")]));
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["--json", "inspect"])
        .arg(store.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(metadata["assets_complete"], false);
    let operations = temp.path().join("repair.json");
    fs::write(
        &operations,
        json!([{"op":"delete","target":"image"}]).to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["--json", "apply"])
        .arg(store.path())
        .arg("--operations")
        .arg(operations)
        .args([
            "--expected-revision",
            metadata["revision"].as_str().unwrap(),
            "--expected-dependency-revision",
            metadata["dependency_revision"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["assets_complete"], true);
}
