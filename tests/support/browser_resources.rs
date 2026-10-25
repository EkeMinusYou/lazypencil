use super::*;
use lazypencil::document::Document;
use std::sync::Arc;

const BASE: &str = "file:///original/site/";

fn edit(store: &Store, id: &str, operations: Value, dry_run: bool) -> EditRequest {
    let project = Project::load(store).unwrap();
    serde_json::from_value(json!({"request_id":id,
        "expected_revision":project.root_document().revision(),
        "expected_dependency_revision":project.dependency_revision(),
        "operations":operations,"dry_run":dry_run}))
    .unwrap()
}

#[test]
fn packed_web_resources_keep_original_urls_empty_files_aliases_and_revision_after_relocation() {
    for version in ["2.19", "2.21"] {
        let (temp, store) = fixture();
        fs::write(temp.path().join("web/alias.HTML"), "<html></html>").unwrap();
        fs::write(temp.path().join("web/日本語 #?.css"), "body{}").unwrap();
        let binary = (0..=255u8).collect::<Vec<_>>();
        fs::write(temp.path().join("web/payload.bin"), &binary).unwrap();
        let mut document: Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        document["version"] = json!(version);
        write(store.path(), document);
        let original = Project::load(&store).unwrap();
        let revision = original
            .browser_page_resources(store.path(), &format!("{BASE}index.html"))
            .unwrap()
            .revision;
        let operations = json!([{"op":"pack_browser_resources"}]);
        let before = fs::read(store.path()).unwrap();
        service::apply_request(
            &store,
            &edit(&store, "pack-preview", operations.clone(), true),
        )
        .unwrap();
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(!temp.path().join("assets").exists());
        let result = service::apply_request(
            &store,
            &edit(&store, "pack-save", operations.clone(), false),
        )
        .unwrap();
        let assets = result["generated_ids"]["0"]["assets"].as_array().unwrap();
        // Four distinct bodies, one web catalogue, and the shared bundle index.
        assert_eq!(assets.len(), 6);
        assert!(assets.iter().any(|entry| entry["bytes"] == 0));
        assert_eq!(store.read().unwrap().version(), version);
        assert!(
            store
                .read()
                .unwrap()
                .value()
                .pointer("/metadata/lazypencil/browser_resources/roots")
                .is_none()
        );
        fs::remove_dir_all(temp.path().join("web")).unwrap();
        let packed = Project::load(&store).unwrap();
        for name in [
            "index.html?query=1#hash",
            "empty.css",
            "alias.HTML",
            "payload.bin",
            "%E6%97%A5%E6%9C%AC%E8%AA%9E%20%23%3F.css",
        ] {
            assert_eq!(
                packed
                    .browser_page_resources(store.path(), &format!("{BASE}{name}"))
                    .unwrap()
                    .revision,
                revision
            );
        }
        packed.verify_unchanged().unwrap();
        let payload = packed
            .dependencies()
            .into_iter()
            .find(|entry| entry.revision == lazypencil::document::hash(&binary))
            .unwrap();
        assert_eq!(packed.asset(&payload.path).unwrap().bytes.as_ref(), binary);
        let declaration =
            store.read().unwrap().value()["metadata"]["lazypencil"]["browser_resources"].clone();
        service::apply_request(
            &store,
            &edit(&store, "pack-repeat", operations.clone(), false),
        )
        .unwrap();
        assert_eq!(
            store.read().unwrap().value()["metadata"]["lazypencil"]["browser_resources"],
            declaration
        );
        let moved = temp.path().join("relocated");
        fs::create_dir(&moved).unwrap();
        fs::rename(temp.path().join("assets"), moved.join("assets")).unwrap();
        fs::rename(store.path(), moved.join("scene.pen")).unwrap();
        let moved = Store::open(
            moved.join("scene.pen"),
            Some(temp.path().join("relocated-private")),
        )
        .unwrap();
        let relocated = Project::load(&moved).unwrap();
        assert_eq!(
            relocated
                .browser_page_resources(moved.path(), &format!("{BASE}index.html"))
                .unwrap()
                .revision,
            revision
        );
        service::apply_request(&moved, &edit(&moved, "pack-relocated", operations, false)).unwrap();
        Project::load(&moved).unwrap().verify_unchanged().unwrap();
    }
}

#[test]
fn packing_rolls_back_later_errors_and_rejects_missing_input_without_publishing() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let request = edit(
        &store,
        "pack-rollback",
        json!([
            {"op":"pack_browser_resources"},
            {"op":"update","target":"missing","set":{"x":1}}
        ]),
        false,
    );
    assert!(service::apply_request(&store, &request).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
    fs::remove_dir_all(temp.path().join("web")).unwrap();
    let request = edit(
        &store,
        "pack-missing",
        json!([{"op":"pack_browser_resources"}]),
        false,
    );
    assert_eq!(
        service::apply_request(&store, &request).unwrap_err().code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
}

#[test]
fn packing_never_publishes_into_its_own_captured_source_directory() {
    let temp = TempDir::new().unwrap();
    let private = TempDir::new().unwrap();
    fs::write(temp.path().join("index.html"), "<html></html>").unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(private.path().join("state")),
    )
    .unwrap();
    write(store.path(), scene(json!([{"url":BASE,"path":"."}])));
    let before = fs::read(store.path()).unwrap();
    let request = edit(
        &store,
        "pack-source-overlap",
        json!([{"op":"pack_browser_resources"}]),
        false,
    );
    assert_eq!(
        service::apply_request(&store, &request).unwrap_err().code,
        "ASSET_CONFLICT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(!temp.path().join("assets").exists());
}

#[test]
fn packed_catalogues_reject_invalid_names_versions_and_response_metadata() {
    for case in [
        "duplicate",
        "remote",
        "fragment",
        "header",
        "size",
        "digest",
        "version",
        "field",
    ] {
        let (temp, store) = fixture();
        service::apply_request(
            &store,
            &edit(
                &store,
                "pack-invalid-catalogue",
                json!([{"op":"pack_browser_resources"}]),
                false,
            ),
        )
        .unwrap();
        let mut document: Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
        let declaration = &mut document["metadata"]["lazypencil"]["browser_resources"]["bundle"];
        let path = temp.path().join(declaration["uri"].as_str().unwrap());
        let mut catalog: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match case {
            "duplicate" => {
                let first = catalog["files"][0].clone();
                catalog["files"].as_array_mut().unwrap().push(first);
            }
            "remote" => catalog["files"][0]["url"] = json!("https://example.com/index.html"),
            "fragment" => catalog["files"][0]["url"] = json!(format!("{BASE}index.html#alias")),
            "header" => catalog["files"][0]["mime"] = json!("text/html\r\nInjected: yes"),
            "size" => catalog["files"][0]["bytes"] = json!(32 * 1024 * 1024 + 1),
            "digest" => catalog["files"][0]["sha256"] = json!("../../private"),
            "version" => catalog["version"] = json!(99),
            "field" => catalog["files"][0]["path"] = json!("../../private"),
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&catalog).unwrap();
        declaration["sha256"] = json!(lazypencil::document::hash(&bytes));
        fs::write(path, bytes).unwrap();
        write(store.path(), document);
        assert!(Project::load(&store).is_err(), "accepted {case}");
    }
}

#[test]
fn packed_catalogue_and_payload_changes_are_rejected_and_missing_payloads_remain_repairable() {
    for catalogue in [true, false] {
        let (temp, store) = fixture();
        let result = service::apply_request(
            &store,
            &edit(
                &store,
                "pack-immutable",
                json!([{"op":"pack_browser_resources"}]),
                false,
            ),
        )
        .unwrap();
        let assets = result["generated_ids"]["0"]["assets"].as_array().unwrap();
        let entry = assets
            .iter()
            .find(|entry| {
                let uri = entry["uri"].as_str().unwrap();
                if catalogue {
                    uri.ends_with("/resources.json")
                } else {
                    uri.ends_with(".blob") && entry["bytes"].as_u64().unwrap() > 0
                }
            })
            .unwrap();
        let path = temp.path().join(entry["uri"].as_str().unwrap());
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, vec![b'x'; bytes.len()]).unwrap();
        assert_eq!(Project::load(&store).err().unwrap().code, "ASSET_CONFLICT");
        fs::write(&path, &bytes).unwrap();
        let captured = Project::load(&store).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(
            captured.verify_unchanged().unwrap_err().code,
            "REVISION_CONFLICT"
        );
        let missing = Project::load(&store).unwrap();
        assert!(!missing.assets_complete());
        service::apply_request(
            &store,
            &edit(
                &store,
                "pack-repair-edit",
                json!([
                    {"op":"update","target":"page","set":{"x":3}}
                ]),
                false,
            ),
        )
        .unwrap();
        assert_eq!(
            service::apply_request(
                &store,
                &edit(
                    &store,
                    "pack-incomplete",
                    json!([{"op":"pack_browser_resources"}]),
                    false
                )
            )
            .unwrap_err()
            .code,
            "ASSET_NOT_FOUND"
        );
        fs::write(&path, &bytes).unwrap();
        let repaired = Project::load(&store).unwrap();
        assert!(repaired.assets_complete());
        repaired.verify_unchanged().unwrap();
    }
}

fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("web")).unwrap();
    fs::write(temp.path().join("web/index.html"), "<html></html>").unwrap();
    fs::write(temp.path().join("web/empty.css"), "").unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(temp.path().join("private")),
    )
    .unwrap();
    write(store.path(), scene(json!([{"url":BASE,"path":"web"}])));
    (temp, store)
}

fn scene(roots: Value) -> Value {
    json!({"version":"2.21","metadata":{"lazypencil":{"browser_resources":{"version":1,"roots":roots}}},
        "children":[{"id":"page","type":"browser","url":format!("{BASE}index.html"),"width":32,"height":32}]})
}

#[test]
fn resource_declaration_edits_create_namespaces_preview_pack_and_clear_atomically() {
    for metadata in [
        None,
        Some(json!({"label":"keep","lazypencil":{"custom":7}})),
    ] {
        let (_temp, store) = fixture();
        let mut document = scene(json!([]));
        document.as_object_mut().unwrap().remove("metadata");
        if let Some(metadata) = &metadata {
            document["metadata"] = metadata.clone();
        }
        write(store.path(), document);
        let before = fs::read(store.path()).unwrap();
        let set = json!({"op":"set_browser_resources","roots":[{"url":BASE,"path":"web"}]});
        service::apply_request(&store, &edit(&store, "roots-preview", json!([set]), true)).unwrap();
        assert_eq!(fs::read(store.path()).unwrap(), before);
        service::apply_request(
            &store,
            &edit(
                &store,
                "roots-pack",
                json!([set,{"op":"pack_browser_resources"}]),
                false,
            ),
        )
        .unwrap();
        let packed = Project::load(&store).unwrap();
        packed
            .browser_page_resources(store.path(), &format!("{BASE}index.html"))
            .unwrap();
        assert!(
            packed
                .root_document()
                .value()
                .pointer("/metadata/lazypencil/browser_resources/bundle")
                .is_some()
        );
        if metadata.is_some() {
            assert_eq!(packed.root_document().value()["metadata"]["label"], "keep");
            assert_eq!(
                packed.root_document().value()["metadata"]["lazypencil"]["custom"],
                7
            );
        }
        service::apply_request(
            &store,
            &edit(
                &store,
                "roots-clear",
                json!([{"op":"set_browser_resources","roots":[]}]),
                false,
            ),
        )
        .unwrap();
        let cleared = Project::load(&store).unwrap();
        assert_eq!(
            cleared
                .browser_page_resources(store.path(), &format!("{BASE}index.html"))
                .err()
                .unwrap()
                .code,
            "ASSET_NOT_FOUND"
        );
        cleared.verify_unchanged().unwrap();
    }
}

#[test]
fn declaration_edits_cannot_expand_allowed_roots_or_publish_invalid_batches() {
    let (temp, store) = fixture();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), b"private").unwrap();
    for path in [
        outside.path().to_string_lossy().into_owned(),
        "private".into(),
        "missing".into(),
    ] {
        let before = fs::read(store.path()).unwrap();
        let request = edit(
            &store,
            &format!("roots-reject-{}", uuid::Uuid::new_v4()),
            json!([{"op":"set_browser_resources","roots":[{"url":BASE,"path":path}]},{"op":"pack_browser_resources"}]),
            false,
        );
        assert!(
            service::apply_request(&store, &request).is_err(),
            "accepted {path}"
        );
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(!temp.path().join("assets").exists());
    }
    for roots in [
        json!([{"url":"https://example.com/","path":"web"}]),
        json!([{"url":BASE,"path":"web","grant":true}]),
        json!(vec![json!({"url":BASE,"path":"web"}); 129]),
    ] {
        let before = fs::read(store.path()).unwrap();
        let request: std::result::Result<EditRequest, _> = serde_json::from_value(
            json!({"request_id":"invalid-roots","expected_revision":Project::load(&store).unwrap().root_document().revision(),"expected_dependency_revision":Project::load(&store).unwrap().dependency_revision(),"operations":[{"op":"set_browser_resources","roots":roots}]}),
        );
        if let Ok(request) = request {
            assert!(service::apply_request(&store, &request).is_err());
        }
        assert_eq!(fs::read(store.path()).unwrap(), before);
    }
}

#[test]
fn declared_roots_capture_empty_files_and_share_one_immutable_namespace_per_owner() {
    let (temp, store) = fixture();
    let project = Project::load(&store).unwrap();
    let first = project
        .browser_page_resources(store.path(), &format!("{BASE}index.html?theme=dark#top"))
        .unwrap();
    let second = project
        .browser_page_resources(store.path(), &format!("{BASE}empty.css"))
        .unwrap();
    assert!(Arc::ptr_eq(&first.files, &second.files));
    assert_eq!(first.revision, second.revision);
    assert_eq!(first.revision.len(), 64);
    assert_eq!(
        project
            .dependencies()
            .iter()
            .filter(|entry| entry.kind == "browser_resource")
            .count(),
        2
    );
    fs::remove_dir_all(temp.path().join("web")).unwrap();
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    // Captured data remains owned; serving it never reopens the original tree.
    let retained = project
        .browser_page_resources(store.path(), &format!("{BASE}index.html"))
        .unwrap();
    assert!(Arc::ptr_eq(&first.files, &retained.files));
    assert!(
        project
            .browser_page_resources(store.path(), "file:///original/private/secret")
            .is_err()
    );
}

#[test]
fn membership_and_same_length_content_changes_invalidate_resource_revisions() {
    let (temp, store) = fixture();
    let initial = Project::load(&store).unwrap();
    let revision = initial
        .browser_page_resources(store.path(), &format!("{BASE}index.html"))
        .unwrap()
        .revision;
    fs::write(temp.path().join("web/later.js"), "").unwrap();
    assert_eq!(
        initial.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let added = Project::load(&store).unwrap();
    assert_ne!(added.dependency_revision(), initial.dependency_revision());
    fs::write(temp.path().join("web/index.html"), "<HTML></HTML>").unwrap();
    assert_eq!(
        added.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let changed = Project::load(&store).unwrap();
    assert_ne!(
        changed
            .browser_page_resources(store.path(), &format!("{BASE}index.html"))
            .unwrap()
            .revision,
        revision
    );
    fs::remove_file(temp.path().join("web/later.js")).unwrap();
    assert_eq!(
        changed.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn new_symbolic_aliases_change_the_namespace_even_when_unique_file_dependencies_do_not() {
    let (temp, store) = fixture();
    let initial = Project::load(&store).unwrap();
    std::os::unix::fs::symlink("index.html", temp.path().join("web/alias.html")).unwrap();
    assert_eq!(
        initial.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let aliased = Project::load(&store).unwrap();
    assert_eq!(
        serde_json::to_value(initial.dependencies()).unwrap(),
        serde_json::to_value(aliased.dependencies()).unwrap()
    );
    assert_ne!(initial.dependency_revision(), aliased.dependency_revision());
    aliased
        .browser_page_resources(store.path(), &format!("{BASE}alias.html"))
        .unwrap();
    fs::remove_file(temp.path().join("web/alias.html")).unwrap();
    std::os::unix::fs::symlink("empty.css", temp.path().join("web/alias.html")).unwrap();
    assert_eq!(
        aliased.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn imported_namespaces_with_identical_urls_keep_their_own_resource_revisions() {
    let (temp, store) = fixture();
    fs::create_dir(temp.path().join("other-web")).unwrap();
    fs::write(temp.path().join("other-web/index.html"), "another library").unwrap();
    write(
        &temp.path().join("library.pen"),
        scene(json!([{"url":BASE,"path":"other-web"}])),
    );
    let mut root = scene(json!([{"url":BASE,"path":"web"}]));
    root["imports"] = json!({"library":"library.pen"});
    write(store.path(), root);
    let project = Project::load(&store).unwrap();
    let library = fs::canonicalize(temp.path().join("library.pen")).unwrap();
    let root = project
        .browser_page_resources(store.path(), &format!("{BASE}index.html"))
        .unwrap();
    let imported = project
        .browser_page_resources(&library, &format!("{BASE}index.html"))
        .unwrap();
    assert_ne!(root.revision, imported.revision);
    assert!(!Arc::ptr_eq(&root.files, &imported.files));
    assert!(
        project
            .browser_page_resources(&library, &format!("{BASE}empty.css"))
            .is_err()
    );
}

#[test]
fn a_colocated_design_can_be_edited_when_private_state_is_outside_the_web_root() {
    let temp = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::write(temp.path().join("index.html"), "<!doctype html>").unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(state.path().join("private")),
    )
    .unwrap();
    write(store.path(), scene(json!([{"url":BASE,"path":"."}])));
    let project = Project::load(&store).unwrap();
    let before = project
        .browser_page_resources(store.path(), &format!("{BASE}scene.pen"))
        .unwrap()
        .revision;
    let request: EditRequest = serde_json::from_value(json!({
        "request_id":"colocated-design-resource-edit","expected_revision":store.read().unwrap().revision(),
        "expected_dependency_revision":project.dependency_revision(),
        "operations":[{"op":"update","target":"page","set":{"x":12}}]
    })).unwrap();
    service::apply_request(&store, &request).unwrap();
    let after = Project::load(&store).unwrap();
    after.verify_unchanged().unwrap();
    assert_ne!(
        before,
        after
            .browser_page_resources(store.path(), &format!("{BASE}scene.pen"))
            .unwrap()
            .revision
    );
    assert_eq!(store.read().unwrap().node("page").unwrap()["x"], 12);
}

#[test]
fn roots_reject_directory_cycles_and_external_links() {
    use std::os::unix::fs::symlink;
    let (temp, store) = fixture();
    symlink(".", temp.path().join("web/loop")).unwrap();
    let cycle = Project::load(&store).err().unwrap();
    assert_eq!(cycle.code, "INVALID_DOCUMENT", "{cycle:?}");
    assert!(cycle.message.contains("symbolic-link cycle"), "{cycle:?}");
    fs::remove_file(temp.path().join("web/loop")).unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret.js"), "private").unwrap();
    symlink(
        outside.path().join("secret.js"),
        temp.path().join("web/escape.js"),
    )
    .unwrap();
    assert_eq!(Project::load(&store).err().unwrap().code, "ASSET_NOT_FOUND");
    fs::remove_file(temp.path().join("web/escape.js")).unwrap();
}

#[test]
fn roots_reject_private_directories_descendants_and_symbolic_file_links() {
    use std::os::unix::fs::symlink;
    let (temp, store) = fixture();
    fs::create_dir_all(store.data_dir().join("tokens")).unwrap();
    fs::write(store.data_dir().join("tokens/private.js"), "private").unwrap();
    symlink(
        store.data_dir().join("tokens/private.js"),
        temp.path().join("web/private.js"),
    )
    .unwrap();
    assert_eq!(
        Project::load(&store).err().unwrap().code,
        "INVALID_DOCUMENT"
    );
    fs::remove_file(temp.path().join("web/private.js")).unwrap();
    write(
        store.path(),
        scene(json!([{"url":BASE,"path":"private/tokens"}])),
    );
    assert_eq!(
        Project::load(&store).err().unwrap().code,
        "INVALID_DOCUMENT"
    );
    write(store.path(), scene(json!([{"url":BASE,"path":"."}])));
    assert_eq!(
        Project::load(&store).err().unwrap().code,
        "INVALID_DOCUMENT"
    );
}

#[test]
fn roots_reject_non_regular_files_without_opening_them() {
    let (temp, store) = fixture();
    write(store.path(), scene(json!([{"url":BASE,"path":"web"}])));
    let fifo = std::ffi::CString::new(temp.path().join("web/pipe").as_os_str().as_encoded_bytes())
        .unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_eq!(
        Project::load(&store).err().unwrap().code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn missing_roots_and_dangling_members_remain_repairable_without_host_fallback() {
    let (temp, store) = fixture();
    fs::remove_dir_all(temp.path().join("web")).unwrap();
    let missing = Project::load(&store).unwrap();
    assert!(!missing.assets_complete());
    assert!(
        missing
            .asset_diagnostics()
            .iter()
            .any(|entry| entry["kind"] == "browser_root")
    );
    missing.verify_unchanged().unwrap();
    let request: EditRequest = serde_json::from_value(json!({
        "request_id":"repairable-missing-browser-root","expected_revision":store.read().unwrap().revision(),
        "expected_dependency_revision":missing.dependency_revision(),
        "operations":[{"op":"update","target":"page","set":{"x":7}}]
    })).unwrap();
    service::apply_request(&store, &request).unwrap();
    fs::create_dir(temp.path().join("web")).unwrap();
    fs::write(temp.path().join("web/index.html"), "<!doctype html>").unwrap();
    assert_eq!(
        missing.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    std::os::unix::fs::symlink("later.js", temp.path().join("web/alias.js")).unwrap();
    let dangling = Project::load(&store).unwrap();
    assert!(!dangling.assets_complete());
    fs::write(temp.path().join("web/later.js"), "").unwrap();
    assert_eq!(
        dangling.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let repaired = Project::load(&store).unwrap();
    assert!(repaired.assets_complete());
    repaired
        .browser_page_resources(store.path(), &format!("{BASE}alias.js"))
        .unwrap();
    repaired.verify_unchanged().unwrap();
}

#[test]
fn explicit_additional_allowed_roots_can_supply_a_separate_web_tree() {
    let (_temp, store) = fixture();
    let outside = TempDir::new().unwrap();
    fs::write(
        outside.path().join("index.html"),
        "outside but explicitly allowed",
    )
    .unwrap();
    let value = scene(json!([{"url":BASE,"path":outside.path()}]));
    write(store.path(), value.clone());
    assert_eq!(Project::load(&store).err().unwrap().code, "ASSET_NOT_FOUND");
    let project = Project::from_document_with_data(
        store.path(),
        Document::parse(value.to_string()).unwrap(),
        &[outside.path().into()],
        Some(store.data_dir()),
    )
    .unwrap();
    project
        .browser_page_resources(store.path(), &format!("{BASE}index.html"))
        .unwrap();
    project.verify_unchanged().unwrap();
}

#[test]
fn captured_shader_files_keep_their_code_kind_when_they_are_also_web_resources() {
    let (temp, store) = fixture();
    let shader = temp.path().join("web/paint.glsl");
    fs::write(
        &shader,
        "void mainImage(out vec4 color,in vec2 p){color=vec4(1);}",
    )
    .unwrap();
    let mut value = scene(json!([{"url":BASE,"path":"web"}]));
    value["children"].as_array_mut().unwrap().push(json!({"id":"paint","type":"rectangle","width":1,"height":1,"fill":{"type":"shader","url":"web/paint.glsl"}}));
    write(store.path(), value);
    let project = Project::load(&store).unwrap();
    let shader = fs::canonicalize(shader).unwrap();
    assert_eq!(
        project
            .dependencies()
            .iter()
            .find(|entry| entry.path == shader)
            .unwrap()
            .kind,
        "shader"
    );
    project.verify_unchanged().unwrap();
}

#[test]
fn invalid_declarations_and_oversized_files_fail_before_page_navigation() {
    let (temp, store) = fixture();
    for roots in [
        json!(null),
        json!([{"url":BASE}]),
        json!([{"url":"https://example.com/","path":"web"}]),
        json!([{"url":"file:///original/site/?q=1","path":"web"}]),
        json!([{"url":BASE,"path":"web"},{"url":format!("{BASE}nested/"),"path":"web"}]),
    ] {
        write(store.path(), scene(roots));
        assert!(Project::load(&store).is_err());
    }
    let mut value = scene(json!([{"url":BASE,"path":"web"}]));
    value["metadata"]["lazypencil"]["browser_resources"]["version"] = json!(99);
    write(store.path(), value);
    assert_eq!(
        Project::load(&store).err().unwrap().code,
        "UNSUPPORTED_VERSION"
    );
    write(store.path(), scene(json!([{"url":BASE,"path":"web"}])));
    fs::File::create(temp.path().join("web/large.bin"))
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    assert_eq!(
        Project::load(&store).err().unwrap().code,
        "INVALID_ARGUMENT"
    );
}
