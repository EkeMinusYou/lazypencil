use base64::Engine;
use lazypencil::assets::{self, AssetKind};
use lazypencil::{operations::EditRequest, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, process::Command, time::Duration};
use tempfile::TempDir;

#[path = "../xtask/src/test_support/command_output.rs"]
mod command_output;

#[cfg(feature = "rendering")]
#[path = "support/font_lock.rs"]
mod font_lock;

#[test]
fn missing_font_lock_preserves_edits_and_rechecks_its_absence_and_raw_contents() {
    let (temp, store) = fixture(json!([]));
    fs::write(
        temp.path().join("lazypencil.toml"),
        "font_lock='fonts.lock.json'\n",
    )
    .unwrap();
    let frozen = Project::load(&store).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    assert_eq!(metadata["configuration"]["font_lock"]["available"], false);
    assert_eq!(metadata["asset_diagnostics"][0]["kind"], "font_lock");
    service::apply_request(
        &store,
        &request(
            &store,
            "edit-with-missing-lock",
            json!([
                {"op":"insert","node":{"id":"rectangle","type":"rectangle","width":10,"height":10}}
            ]),
            false,
        ),
    )
    .unwrap();
    fs::write(temp.path().join("fonts.lock.json"), b"{}").unwrap();
    assert_eq!(
        frozen.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let malformed = Project::load(&store).unwrap();
    assert!(malformed.assets_complete());
    #[cfg(feature = "rendering")]
    assert_eq!(
        malformed.load_fonts().err().unwrap().code,
        "FONT_LOCK_INVALID"
    );
    fs::write(temp.path().join("fonts.lock.json"), b"[]").unwrap();
    assert_eq!(
        malformed.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

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
fn missing_configured_fonts_allow_structural_edits_and_conflict_when_the_file_appears() {
    let (temp, store) = fixture(json!([
        {"id":"label","type":"text","content":"A","fontFamily":"LP Test Sans","fontSize":20,"width":40,"height":30}
    ]));
    let configuration = temp.path().join("lazypencil.toml");
    let settings = "[fonts]\nfiles=['fonts/regular.ttf']\n";
    fs::write(&configuration, settings).unwrap();
    let project = Project::load(&store).unwrap();
    assert!(!project.assets_complete());
    let metadata = service::document_metadata(&store).unwrap();
    assert_eq!(metadata["assets_complete"], false);
    assert_eq!(metadata["configuration"]["fonts_complete"], false);
    assert_eq!(metadata["asset_diagnostics"][0]["kind"], "font");
    assert_eq!(
        metadata["asset_diagnostics"][0]["origin"],
        json!(configuration.canonicalize().unwrap())
    );
    assert!(
        metadata["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|dependency| dependency["kind"] == "font"
                && dependency["availability"] == "missing")
    );
    #[cfg(feature = "rendering")]
    assert_eq!(project.load_fonts().err().unwrap().code, "ASSET_NOT_FOUND");
    let edit = request(
        &store,
        "edit-with-missing-font",
        json!([
            {"op":"update","target":"label","set":{"name":"Kept draft"}}
        ]),
        false,
    );
    let saved = service::apply_request(&store, &edit).unwrap();
    assert_eq!(saved["assets_complete"], false);
    assert_eq!(
        store.read().unwrap().node("label").unwrap()["fontFamily"],
        "LP Test Sans"
    );
    let stale = request(
        &store,
        "stale-missing-font",
        json!([
            {"op":"update","target":"label","set":{"name":"Must not save"}}
        ]),
        false,
    );
    let frozen = Project::load(&store).unwrap();
    fs::create_dir(temp.path().join("fonts")).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/regular.ttf"),
        temp.path().join("fonts/regular.ttf"),
    )
    .unwrap();
    assert_eq!(
        frozen.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::apply_request(&store, &stale).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        store.read().unwrap().node("label").unwrap()["name"],
        "Kept draft"
    );
    assert_eq!(fs::read_to_string(&configuration).unwrap(), settings);
    let repaired = service::document_metadata(&store).unwrap();
    assert_eq!(repaired["assets_complete"], true);
    assert_ne!(
        repaired["dependency_revision"],
        saved["dependency_revision"]
    );
    #[cfg(feature = "rendering")]
    {
        let (project, layout) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let text = layout
            .items
            .iter()
            .find(|item| item.record.id == "label")
            .unwrap()
            .record
            .text
            .as_ref()
            .unwrap();
        assert_eq!(text.fonts[0].family, "LP Test Sans");
        assert_eq!(
            text.fonts[0].hash,
            lazypencil::document::hash(&fs::read(temp.path().join("fonts/regular.ttf")).unwrap())
        );
        let output = lazypencil::render::png(&project, &layout, &Default::default()).unwrap();
        assert!(!output.png.is_empty());
        let mut fonts = project.load_fonts().unwrap();
        assert!(
            fonts
                .inventory()
                .unwrap()
                .iter()
                .any(|face| face.family == "LP Test Sans")
        );
    }
}

#[cfg(unix)]
#[test]
fn a_missing_configured_font_retains_parent_link_identity_during_inspection() {
    use std::os::unix::fs::symlink;
    let (temp, store) = fixture(json!([]));
    fs::create_dir(temp.path().join("one")).unwrap();
    fs::create_dir(temp.path().join("two")).unwrap();
    symlink("one", temp.path().join("fonts")).unwrap();
    fs::write(
        temp.path().join("lazypencil.toml"),
        "[fonts]\nfiles=['fonts/missing.ttf']\n",
    )
    .unwrap();
    let project = Project::load(&store).unwrap();
    let stale = request(
        &store,
        "redirect-missing-font",
        json!([
            {"op":"set_variable","name":"tone","definition":{"type":"color","value":"#2563eb"}}
        ]),
        false,
    );
    fs::remove_file(temp.path().join("fonts")).unwrap();
    symlink("two", temp.path().join("fonts")).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        service::apply_request(&store, &stale).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn a_missing_font_search_directory_is_diagnosed_and_rechecked_after_restoration() {
    let (temp, store) = fixture(json!([]));
    fs::write(
        temp.path().join("lazypencil.toml"),
        "[fonts]\nsearch_paths=['fonts']\n",
    )
    .unwrap();
    let project = Project::load(&store).unwrap();
    assert_eq!(project.asset_diagnostics()[0]["kind"], "font_directory");
    let frozen = lazypencil::config::Snapshot::load(store.path()).unwrap();
    assert_eq!(frozen.require_fonts().unwrap_err().code, "ASSET_NOT_FOUND");
    fs::create_dir(temp.path().join("fonts")).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts/regular.ttf"),
        temp.path().join("fonts/regular.ttf"),
    )
    .unwrap();
    assert_eq!(
        frozen.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let restored = lazypencil::config::Snapshot::load(store.path()).unwrap();
    restored.require_fonts().unwrap();
    assert_eq!(restored.font_files.len(), 1);
    assert!(Project::load(&store).unwrap().assets_complete());
}

#[test]
fn user_font_configuration_is_inspectable_while_inventory_rejects_missing_inputs() {
    let (temp, _store) = fixture(json!([]));
    let user = temp.path().join("user");
    fs::create_dir(&user).unwrap();
    fs::write(
        user.join("config.toml"),
        "[fonts]\nfiles=['missing.ttf','missing.ttf']\nsearch_paths=['missing-directory']\n",
    )
    .unwrap();
    for (arguments, inspect) in [
        (&["config", "show", "--user"][..], true),
        (&["fonts", "list"][..], false),
    ] {
        if !inspect && !cfg!(feature = "rendering") {
            continue;
        }
        let output = command_output::output(
            Command::new(env!("CARGO_BIN_EXE_lazypencil"))
                .env("LAZYPENCIL_CONFIG_DIR", &user)
                .arg("--json")
                .args(arguments),
            Duration::from_secs(30),
            1024 * 1024,
        )
        .unwrap();
        assert!(
            !output.timed_out,
            "Font command timed out after {:?}",
            output.elapsed
        );
        assert_eq!(
            output.status.success(),
            inspect,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        if inspect {
            assert_eq!(value["fonts_complete"], false);
            assert_eq!(value["font_diagnostics"].as_array().unwrap().len(), 2);
            assert_eq!(value["font_files"], json!([]));
        } else {
            assert_eq!(value["error"]["code"], "ASSET_NOT_FOUND");
        }
    }
    assert!(!user.join("missing.ttf").exists());
    assert!(!user.join("missing-directory").exists());
}

#[test]
fn existing_wrong_font_input_types_are_not_reclassified_as_missing() {
    let (temp, store) = fixture(json!([]));
    fs::create_dir(temp.path().join("directory.ttf")).unwrap();
    fs::write(temp.path().join("file"), "not a directory").unwrap();
    for settings in [
        "[fonts]\nfiles=['directory.ttf']\n",
        "[fonts]\nsearch_paths=['file']\n",
    ] {
        fs::write(temp.path().join("lazypencil.toml"), settings).unwrap();
        assert_eq!(
            Project::load(&store).err().unwrap().code,
            "CONFIGURATION_ERROR"
        );
    }
}

#[test]
fn missing_library_can_be_rebound_with_revision_checks_without_inventing_its_contents() {
    let (temp, store) = fixture(json!([
        {"id":"instance","type":"ref","ref":"ui:tile"}
    ]));
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"ui":"lost/ui.lib.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let original = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    assert!(metadata.get("dependency_error").is_none(), "{metadata}");
    assert_eq!(metadata["assets_complete"], Value::Null);
    assert!(metadata.get("resolution_error").is_some());
    assert_eq!(metadata["asset_diagnostics"][0]["kind"], "library");
    assert_eq!(metadata["asset_diagnostics"][0]["uri"], "lost/ui.lib.pen");
    assert_eq!(metadata["dependencies"][0]["availability"], "missing");
    let project = Project::load(&store).unwrap();
    assert!(project.components().is_empty());
    assert_eq!(project.resolve().unwrap_err().code, "ASSET_NOT_FOUND");
    fs::write(
        temp.path().join("replacement.pen"),
        json!({"version":"2.21","children":[
            {"id":"tile","type":"rectangle","reusable":true,"width":20,"height":20,"fill":"#2563eb"}
        ]})
        .to_string(),
    )
    .unwrap();
    let operations = json!([
        {"op":"remove_import","alias":"ui"},
        {"op":"add_import","alias":"ui","uri":"replacement.pen"}
    ]);
    let edit = request(&store, "rebind-library", operations, true);
    let dry = service::apply_request(&store, &edit).unwrap();
    assert_eq!(dry["assets_complete"], true);
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
    let mut edit = edit;
    edit.dry_run = false;
    let result = service::apply_request(&store, &edit).unwrap();
    assert_eq!(result["assets_complete"], true);
    assert_eq!(
        service::apply_request(&store, &edit).unwrap()["replayed"],
        true
    );
    let repaired = Project::load(&store).unwrap().resolve().unwrap();
    assert_eq!(repaired.roots.len(), 1);
    assert_eq!(repaired.roots[0].properties["fill"], "#2563eb");
    assert!(
        store
            .read()
            .unwrap()
            .text()
            .contains("// Preserve this comment.")
    );
    assert_eq!(store.read().unwrap().value()["future"]["keep"], 17);
    assert!(!temp.path().join("lost").exists());
}

#[test]
fn unused_missing_libraries_support_partial_removal_but_not_new_missing_imports() {
    let (_temp, store) = fixture(json!([]));
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"one":"one.pen","two":"two.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    assert_eq!(metadata["assets_complete"], false);
    assert_eq!(metadata["asset_diagnostics"].as_array().unwrap().len(), 2);
    let result = service::apply_request(
        &store,
        &request(
            &store,
            "remove-one",
            json!([
                {"op":"remove_import","alias":"one"}
            ]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(result["assets_complete"], false);
    assert_eq!(result["asset_diagnostics"].as_array().unwrap().len(), 1);
    let original = fs::read(store.path()).unwrap();
    let edit = request(
        &store,
        "new-missing-library",
        json!([
            {"op":"add_import","alias":"three","uri":"three.pen"}
        ]),
        false,
    );
    assert_eq!(
        service::apply_request(&store, &edit).unwrap_err().code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn an_appearing_library_invalidates_its_absence_revision_before_saving() {
    let (temp, store) = fixture(json!([]));
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"ui":"absent.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let project = Project::load(&store).unwrap();
    let original = fs::read(store.path()).unwrap();
    let edit = request(
        &store,
        "stale-library",
        json!([
            {"op":"remove_import","alias":"ui"}
        ]),
        false,
    );
    fs::write(
        temp.path().join("absent.pen"),
        r#"{"version":"2.21","children":[]}"#,
    )
    .unwrap();
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
fn missing_library_detection_does_not_accept_urls_directories_or_invalid_extensions() {
    for uri in [
        "https://example.invalid/absent.pen",
        "file:///absent.pen",
        "directory.pen",
        "absent.png",
        "../outside.pen",
    ] {
        let (temp, store) = fixture(json!([]));
        fs::create_dir(temp.path().join("directory.pen")).unwrap();
        let mut document = store.read().unwrap();
        document
            .set_property("", "imports", &json!({"ui":uri}))
            .unwrap();
        fs::write(store.path(), document.text()).unwrap();
        let metadata = service::document_metadata(&store).unwrap();
        assert!(
            metadata.get("dependency_error").is_some(),
            "{uri}: {metadata}"
        );
        assert!(
            metadata.get("assets_complete").is_none(),
            "{uri}: {metadata}"
        );
        assert!(Project::load(&store).is_err(), "{uri}");
    }
}

#[test]
fn a_missing_library_alias_cannot_fall_back_to_a_different_librarys_literal_names() {
    for variable in [false, true] {
        let (temp, store) = fixture(if variable {
            json!([{"id":"instance","type":"rectangle","fill":"$ui:color"}])
        } else {
            json!([{"id":"instance","type":"ref","ref":"ui:tile"}])
        });
        fs::write(temp.path().join("other.pen"), json!({"version":"2.21",
            "variables":{"color":{"type":"color","value":"#2563eb"}},
            "children":[{"id":"ui:tile","type":"rectangle","reusable":true,"width":20,"height":20}]
        }).to_string()).unwrap();
        let mut document = store.read().unwrap();
        document
            .set_property(
                "",
                "imports",
                &json!({"ui":"absent.pen","other":"other.pen"}),
            )
            .unwrap();
        fs::write(store.path(), document.text()).unwrap();
        let metadata = service::document_metadata(&store).unwrap();
        assert!(metadata.get("dependency_error").is_none(), "{metadata}");
        assert_eq!(
            metadata["resolution_error"]["code"], "ASSET_NOT_FOUND",
            "{metadata}"
        );
        assert_eq!(
            Project::load(&store)
                .unwrap()
                .library_document(Some("ui"))
                .unwrap_err()
                .code,
            "ASSET_NOT_FOUND"
        );
    }
}

#[test]
fn a_nested_missing_library_reports_its_owner_and_can_be_removed_with_its_instances() {
    let (temp, store) = fixture(json!([
        {"id":"instance","type":"ref","ref":"outer:ui:tile"}
    ]));
    let outer = temp.path().join("outer.pen");
    let library =
        json!({"version":"2.21","imports":{"ui":"nested/absent.pen"},"children":[]}).to_string();
    fs::write(&outer, &library).unwrap();
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"outer":"outer.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    assert!(metadata.get("dependency_error").is_none(), "{metadata}");
    assert_eq!(
        metadata["asset_diagnostics"][0]["origin"],
        json!(outer.canonicalize().unwrap())
    );
    assert_eq!(metadata["asset_diagnostics"][0]["uri"], "nested/absent.pen");
    let result = service::apply_request(
        &store,
        &request(
            &store,
            "remove-nested-library",
            json!([
                {"op":"delete","target":"instance"},
                {"op":"remove_import","alias":"outer"}
            ]),
            false,
        ),
    )
    .unwrap();
    assert_eq!(result["assets_complete"], true);
    assert_eq!(fs::read_to_string(&outer).unwrap(), library);
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
        "ftp://example.invalid/lost.png",
        "https://user:private@host.invalid/lost.png",
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
    for missing_library in [false, true] {
        let (temp, store) = fixture(if missing_library {
            json!([])
        } else {
            json!([image("images/lost.png")])
        });
        if missing_library {
            let mut document = store.read().unwrap();
            document
                .set_property("", "imports", &json!({"ui":"absent.pen"}))
                .unwrap();
            fs::write(store.path(), document.text()).unwrap();
        }
        let output = command_output::output(
            Command::new(env!("CARGO_BIN_EXE_lazypencil"))
                .arg("--data-dir")
                .arg(store.data_dir())
                .args(["--json", "inspect"])
                .arg(store.path()),
            Duration::from_secs(30),
            1024 * 1024,
        )
        .unwrap();
        assert!(
            !output.timed_out,
            "CLI inspection timed out after {:?}",
            output.elapsed
        );
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
            if missing_library {
                json!([{"op":"remove_import","alias":"ui"}])
            } else {
                json!([{"op":"delete","target":"image"}])
            }
            .to_string(),
        )
        .unwrap();
        let output = command_output::output(
            Command::new(env!("CARGO_BIN_EXE_lazypencil"))
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
                ]),
            Duration::from_secs(30),
            1024 * 1024,
        )
        .unwrap();
        assert!(
            !output.timed_out,
            "CLI repair timed out after {:?}",
            output.elapsed
        );
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["assets_complete"], true);
    }
}
