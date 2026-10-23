use lazypencil::{Error, operations::EditRequest, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{cell::Cell, fs, path::Path};
use tempfile::TempDir;

fn write(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(temp.path().join("library/asset.png"), b"image fixture").unwrap();
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({
            "version":"2.21", "themes":{"mode":["light","dark"]},
            "variables":{
                "label":{"type":"string","value":"Library label"},
                "surface":{"type":"color","value":[
                    {"value":"#ffffff"},{"theme":{"mode":"dark"},"value":"#111111"}
                ]},
                "space":{"type":"number","value":12},
                "alias":{"type":"number","value":"$space"}
            },
            "children":[
                {"id":"button","type":"frame","reusable":true,"gap":"$alias","children":[
                    {"id":"wrapper","type":"frame","children":[
                        {"id":"label","type":"text","content":"$label"}
                    ]}
                ]},
                {"id":"card","type":"frame","reusable":true,"fill":"$surface","children":[
                    {"id":"action","type":"ref","ref":"button","descendants":{"label":{"content":"Inner"}}},
                    {"id":"body","type":"frame","slot":["button"],"children":[
                        {"id":"image","type":"rectangle","fill":{"type":"image","url":"asset.png"}}
                    ]}
                ]}
            ]
        }),
    );
    write(
        &temp.path().join("main.pen"),
        json!({
            "version":"2.21", "imports":{"ui":"library/ui.lib.pen"},
            "variables":{"label":{"type":"string","value":"Consumer label"}},
            "children":[
                {"id":"screen","type":"frame","theme":{"mode":"dark"},"children":[
                    {"id":"card-1","type":"ref","ref":"ui:card","descendants":{
                        "action/label":{"content":"$label"}
                    }}
                ]},
                {"id":"card-2","type":"ref","ref":"ui:card"}
            ]
        }),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

#[test]
fn imported_instances_resolve_nested_overrides_themes_and_property_origins() {
    let (temp, store) = fixture();
    let project = Project::load(&store).unwrap();
    let scene = project.resolve().unwrap();
    let card = &scene.roots[0].children[0];
    assert_eq!(card.id(), "card-1");
    assert_eq!(card.properties["fill"], "#111111");
    let label = &card.children[0].children[0].children[0];
    assert_eq!(label.id(), "card-1/action/label");
    assert_eq!(label.properties["content"], "Consumer label");
    assert_eq!(label.property_source("content"), store.path());
    assert_eq!(card.children[0].properties["gap"], 12);
    let other = &scene.roots[1];
    assert_eq!(other.properties["fill"], "#ffffff");
    assert_eq!(
        other.children[0].children[0].children[0].properties["content"],
        "Inner"
    );
    let image = &other.children[1].children[0];
    assert_eq!(
        image.property_source("fill"),
        temp.path()
            .join("library/ui.lib.pen")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(scene.dependencies.len(), 2);
    assert_eq!(
        fs::read_to_string(store.path()).unwrap(),
        project.root_document().text()
    );
}

#[test]
fn retained_assets_detect_changes_across_streaming_chunks_lengths_and_size_limits() {
    let (temp, store) = fixture();
    let asset = temp.path().join("library/asset.png");
    let original = (0..(3 * 65536 + 17))
        .map(|n| (n % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&asset, &original).unwrap();
    let project = Project::load(&store).unwrap();
    let revision = project.dependency_revision();
    project.verify_unchanged().unwrap();
    for offset in [0, 65535, 65536, original.len() - 1] {
        let mut changed = original.clone();
        changed[offset] ^= 1;
        fs::write(&asset, changed).unwrap();
        assert_eq!(
            project.verify_unchanged().unwrap_err().code,
            "REVISION_CONFLICT"
        );
        fs::write(&asset, &original).unwrap();
        project.verify_unchanged().unwrap();
    }
    for length in [0, original.len() - 1, original.len() + 1] {
        fs::OpenOptions::new()
            .write(true)
            .open(&asset)
            .unwrap()
            .set_len(length as u64)
            .unwrap();
        assert_eq!(
            project.verify_unchanged().unwrap_err().code,
            "REVISION_CONFLICT"
        );
        fs::write(&asset, &original).unwrap();
    }
    fs::OpenOptions::new()
        .write(true)
        .open(&asset)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "INVALID_DOCUMENT"
    );
    fs::write(&asset, &original).unwrap();
    project.verify_unchanged().unwrap();
    assert_eq!(project.dependency_revision(), revision);
    assert_eq!(
        project
            .asset(&asset.canonicalize().unwrap())
            .unwrap()
            .bytes
            .as_ref(),
        original
    );
}

#[test]
fn slots_replace_children_and_full_replacements_remove_inherited_properties() {
    let (_temp, store) = fixture();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "/children/1",
            "descendants",
            &json!({
                "body":{"children":[{"id":"custom","type":"text","content":"Slot"}]},
                "action/label":{"id":"replacement","type":"rectangle","width":24,"height":24}
            }),
        )
        .unwrap();
    let scene = Project::from_document(store.path(), document, &[])
        .unwrap()
        .resolve()
        .unwrap();
    let card = &scene.roots[1];
    assert_eq!(card.children[1].children.len(), 1);
    assert_eq!(card.children[1].children[0].id(), "card-2/custom");
    let replacement = &card.children[0].children[0].children[0];
    assert_eq!(replacement.kind(), "rectangle");
    assert!(!replacement.properties.contains_key("content"));
    assert_eq!(replacement.id(), "card-2/action/label");
    assert_eq!(replacement.source_id, "replacement");
}

#[test]
fn unresolved_overrides_and_invalid_imported_nodes_are_rejected() {
    let (temp, store) = fixture();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "/children/1",
            "descendants",
            &json!({"missing":{"opacity":0.5}}),
        )
        .unwrap();
    assert!(
        Project::from_document(store.path(), document, &[])
            .unwrap()
            .resolve()
            .is_err()
    );
    let path = temp.path().join("library/ui.lib.pen");
    let mut library: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    library["children"][0]["opacity"] = json!(2);
    write(&path, library);
    assert!(service::validate_document(&store).is_err());
}

#[test]
fn imported_ids_and_variables_must_be_unambiguous() {
    let (temp, store) = fixture();
    fs::write(temp.path().join("asset.png"), b"image fixture").unwrap();
    let mut document = store.read().unwrap();
    fs::copy(
        temp.path().join("library/ui.lib.pen"),
        temp.path().join("other.pen"),
    )
    .unwrap();
    document
        .set_property(
            "",
            "imports",
            &json!({"a":"library/ui.lib.pen","b":"other.pen"}),
        )
        .unwrap();
    document
        .set_property("/children/1", "ref", &json!("card"))
        .unwrap();
    assert!(
        Project::from_document(store.path(), document, &[])
            .unwrap()
            .resolve()
            .is_err()
    );
}

#[test]
fn import_and_variable_cycles_and_project_escapes_fail_without_modifying_files() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let library_path = temp.path().join("library/ui.lib.pen");
    let library_original = fs::read(&library_path).unwrap();
    let mut library: Value = serde_json::from_slice(&library_original).unwrap();
    library["imports"] = json!({"cycle":"../main.pen"});
    write(&library_path, library);
    assert_eq!(Project::load(&store).err().unwrap().code, "REFERENCE_CYCLE");
    fs::write(&library_path, &library_original).unwrap();
    let mut library: Value = serde_json::from_slice(&library_original).unwrap();
    library["variables"]["space"]["value"] = json!("$alias");
    write(&library_path, library);
    assert!(Project::load(&store).unwrap().resolve().is_err());
    let outside = TempDir::new().unwrap();
    write(
        &outside.path().join("outside.pen"),
        json!({"version":"2.21","children":[]}),
    );
    let mut document = store.read().unwrap();
    document
        .set_property(
            "",
            "imports",
            &json!({"outside":outside.path().join("outside.pen")}),
        )
        .unwrap();
    assert_eq!(
        Project::from_document(store.path(), document, &[])
            .err()
            .unwrap()
            .code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

fn request(store: &Store, dependency_revision: Option<String>, id: &str) -> EditRequest {
    serde_json::from_value(json!({
        "request_id":id,"expected_revision":store.read().unwrap().revision(),
        "expected_dependency_revision":dependency_revision,
        "operations":[{"op":"update","target":"card-2","set":{"width":320}}]
    }))
    .unwrap()
}

#[test]
fn edits_require_current_library_revisions_and_replay_without_reapplying() {
    let (temp, store) = fixture();
    let original = fs::read(store.path()).unwrap();
    let missing = request(&store, None, "missing");
    assert_eq!(
        service::apply_request(&store, &missing).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
    let revision = Project::load(&store).unwrap().dependency_revision();
    let stale = request(&store, Some(revision), "stale");
    let path = temp.path().join("library/ui.lib.pen");
    let mut library = fs::read_to_string(&path).unwrap();
    library.push('\n');
    fs::write(&path, &library).unwrap();
    assert_eq!(
        service::apply_request(&store, &stale).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    assert!(store.history(10, None).unwrap().is_empty());
    let fresh = request(
        &store,
        Some(Project::load(&store).unwrap().dependency_revision()),
        "fresh",
    );
    let result = service::apply_request(&store, &fresh).unwrap();
    assert_eq!(result["replayed"], false);
    fs::remove_file(path).unwrap();
    let replay = service::apply_request(&store, &fresh).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(store.history(10, None).unwrap().len(), 1);
}

#[test]
fn a_dependency_changed_between_preparation_and_commit_aborts_the_save() {
    let (temp, store) = fixture();
    let before = store.read().unwrap();
    let project = Project::load(&store).unwrap();
    let library_path = temp.path().join("library/ui.lib.pen");
    let ran = Cell::new(false);
    let error = store
        .transact_guarded(
            "race",
            &before.revision(),
            "test",
            |doc| {
                let mut after = doc.clone();
                after.set_property("/children/1", "width", &json!(300))?;
                fs::write(&library_path, "changed").unwrap();
                Ok((after, json!({})))
            },
            || {
                ran.set(true);
                project.verify_unchanged()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "REVISION_CONFLICT");
    assert!(ran.get());
    assert_eq!(store.read().unwrap().text(), before.text());
    assert!(store.history(10, None).unwrap().is_empty());
    assert!(store.recover().unwrap().is_empty());
}

#[test]
fn shared_dependency_locks_use_the_writer_lock_namespace() {
    let (temp, store) = fixture();
    let library_path = temp
        .path()
        .join("library/ui.lib.pen")
        .canonicalize()
        .unwrap();
    let locks = store
        .lock_dependencies(std::slice::from_ref(&library_path))
        .unwrap();
    let library = Store::open(&library_path, Some(store.data_dir().into())).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(library.lock_file_path())
        .unwrap();
    assert!(fs2::FileExt::try_lock_exclusive(&lock).is_err());
    drop(locks);
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    drop(lock);
    let before = library.read().unwrap();
    library
        .transact("library-edit", &before.revision(), "test", |doc| {
            Ok((doc.clone(), json!({})))
        })
        .unwrap();
}

#[test]
fn dry_runs_validate_imported_content_without_creating_history() {
    let (_temp, store) = fixture();
    let original = store.read().unwrap();
    let mut request = request(
        &store,
        Some(Project::load(&store).unwrap().dependency_revision()),
        "dry",
    );
    request.dry_run = true;
    let result = service::apply_request(&store, &request).unwrap();
    assert_eq!(result["dry_run"], true);
    assert_ne!(result["revision"], result["proposed_revision"]);
    assert_eq!(store.read().unwrap().text(), original.text());
    assert!(store.history(10, None).unwrap().is_empty());
}

#[test]
fn guarded_transaction_deduplication_does_not_repeat_side_effects() {
    let temp = TempDir::new().unwrap();
    let store = Store::init(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    let revision = store.read().unwrap().revision();
    store
        .transact("id", &revision, "hash", |doc| Ok((doc.clone(), json!({}))))
        .unwrap();
    let result = store
        .transact_guarded(
            "id",
            &revision,
            "hash",
            |_| Err(Error::new("TEST", "must not run")),
            || Err(Error::new("TEST", "must not run")),
        )
        .unwrap();
    assert_eq!(result["replayed"], true);
}

#[test]
fn image_script_and_shader_bytes_are_part_of_the_dependency_snapshot() {
    let (temp, store) = fixture();
    fs::write(temp.path().join("draw.js"), "return [];").unwrap();
    fs::write(
        temp.path().join("effect.glsl"),
        "void main(){gl_FragColor=vec4(1.0);}",
    )
    .unwrap();
    let mut document = store.read().unwrap();
    document
        .insert_element(
            "/children",
            2,
            &json!({"id":"script","type":"script","scriptUri":"draw.js","width":100,"height":100})
                .to_string(),
        )
        .unwrap();
    document
        .set_property(
            "/children/0",
            "effect",
            &json!({"type":"shader","url":"effect.glsl"}),
        )
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let project = Project::load(&store).unwrap();
    let kinds = project
        .dependencies()
        .iter()
        .map(|d| d.kind.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        kinds,
        ["document", "image", "script", "shader"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    let revision = project.dependency_revision();
    let asset = temp.path().join("library/asset.png");
    assert_eq!(
        project
            .asset(&asset.canonicalize().unwrap())
            .unwrap()
            .bytes
            .as_ref(),
        b"image fixture"
    );
    fs::write(&asset, b"replacement image").unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_ne!(
        Project::load(&store).unwrap().dependency_revision(),
        revision
    );
    assert_eq!(
        project
            .asset(&asset.canonicalize().unwrap())
            .unwrap()
            .bytes
            .as_ref(),
        b"image fixture"
    );
}

#[test]
fn restores_validate_current_dependencies_and_are_idempotent() {
    let (temp, store) = fixture();
    let initial = request(
        &store,
        Some(Project::load(&store).unwrap().dependency_revision()),
        "initial",
    );
    let saved = service::apply_request(&store, &initial).unwrap();
    let checkpoint = saved["checkpoint_id"].as_str().unwrap();
    let mut second = request(
        &store,
        Some(Project::load(&store).unwrap().dependency_revision()),
        "second",
    );
    second.operations =
        serde_json::from_value(json!([{"op":"update","target":"card-2","set":{"height":200}}]))
            .unwrap();
    service::apply_request(&store, &second).unwrap();
    let expected = store.read().unwrap().revision();
    let old_dependencies = Project::load(&store).unwrap().dependency_revision();
    let asset = temp.path().join("library/asset.png");
    fs::write(&asset, b"updated image").unwrap();
    let mut restore = service::RestoreRequest {
        request_id: "restore",
        expected_revision: &expected,
        expected_dependency_revision: Some(&old_dependencies),
        checkpoint,
    };
    assert_eq!(
        service::restore_checkpoint(&store, &restore, &[])
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    assert_eq!(store.read().unwrap().node("card-2").unwrap()["height"], 200);
    let current_dependencies = Project::load(&store).unwrap().dependency_revision();
    restore.expected_dependency_revision = Some(&current_dependencies);
    service::restore_checkpoint(&store, &restore, &[]).unwrap();
    assert!(
        store
            .read()
            .unwrap()
            .node("card-2")
            .unwrap()
            .get("height")
            .is_none()
    );
    fs::remove_file(&asset).unwrap();
    assert_eq!(
        service::restore_checkpoint(&store, &restore, &[]).unwrap()["replayed"],
        true
    );
    assert_eq!(store.history(10, None).unwrap().len(), 3);
}

#[test]
fn explicitly_selected_font_files_use_the_same_dependency_revision_in_reads_and_edits() {
    let (temp, store) = fixture();
    let font = temp.path().join("font.ttf");
    fs::write(&font, b"font contents").unwrap();
    let fonts = vec![font.clone()];
    let metadata = service::document_metadata_with_fonts(&store, &fonts).unwrap();
    assert!(
        metadata["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["kind"] == "font")
    );
    let expected = metadata["dependency_revision"]
        .as_str()
        .unwrap()
        .to_string();
    let stale = request(&store, Some(expected), "font-edit");
    fs::write(&font, b"changed font").unwrap();
    assert_eq!(
        service::apply_request_with_fonts(&store, &stale, &fonts)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
    let metadata = service::document_metadata_with_fonts(&store, &fonts).unwrap();
    let fresh = request(
        &store,
        Some(metadata["dependency_revision"].as_str().unwrap().into()),
        "font-edit",
    );
    service::apply_request_with_fonts(&store, &fresh, &fonts).unwrap();
    fs::remove_file(&font).unwrap();
    assert_eq!(
        service::apply_request_with_fonts(&store, &fresh, &fonts).unwrap()["replayed"],
        true
    );
}
