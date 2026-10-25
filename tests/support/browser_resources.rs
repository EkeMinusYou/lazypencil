use super::*;
use lazypencil::document::Document;
use std::sync::Arc;

const BASE: &str = "file:///original/site/";

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
