#[path = "script_copy_fixture.rs"]
mod fixture;
use fixture::*;
use lazypencil::{project::Project, service};
use serde_json::json;
use std::fs;

#[test]
fn script_copy_previews_without_publication_and_keeps_a_complete_namespace_after_source_removal() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    let source = fs::read(temp.path().join("library/ui.lib.pen")).unwrap();
    let edits = operations();
    let preview = service::apply_request(
        &store,
        &request(&store, "script-preview", edits.clone(), true),
    )
    .unwrap();
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(store.history(10, None).unwrap().is_empty());
    assert!(!temp.path().join("assets").exists());
    assert!(
        preview["generated_ids"]["0"]["assets"]
            .as_array()
            .unwrap()
            .len()
            >= 3
    );
    let result = apply(&store, "script-copy", edits);
    assert_eq!(
        fs::read(temp.path().join("library/ui.lib.pen")).unwrap(),
        source
    );
    let ids = &result["generated_ids"]["0"]["id_map"];
    let document = store.read().unwrap();
    let generator = document
        .node(ids["ui:generator"].as_str().unwrap())
        .unwrap();
    assert_eq!(generator["type"], "script");
    assert_eq!(generator["inputs"]["stale"], "$missing");
    assert_eq!(generator["metadata"]["keep"], json!([1, 2, 3]));
    let bindings = &generator["metadata"]["lazypencil"]["script_bindings"];
    assert_eq!(bindings["components"]["badge"], ids["ui:badge"]);
    assert_eq!(bindings["components"]["base:other"], ids["ui:base:other"]);
    assert_eq!(bindings["components"]["base:future"], ids["ui:base:future"]);
    assert_eq!(bindings["variables"]["tone"], "copy.tone");
    assert_eq!(bindings["variables"]["futureTone"], "copy.futureTone");
    assert_eq!(bindings["themes"]["mode"], "copy.mode");
    assert!(
        bindings["assets"]
            .as_object()
            .unwrap()
            .contains_key("image:pixel.png")
    );
    let uri = generator["scriptUri"].as_str().unwrap();
    assert_eq!(fs::read(temp.path().join(uri)).unwrap(), CODE.as_bytes());
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    let project = Project::load(&store).unwrap();
    project.resolve().unwrap();
    assert!(
        project
            .dependencies()
            .iter()
            .filter(|entry| entry.kind == "document")
            .all(|entry| entry
                .path
                .starts_with(store.path().parent().unwrap().join("assets/lazypencil")))
    );
    assert!(store.history(10, None).unwrap().len() == 1);
}

#[test]
fn script_manifest_failures_and_link_options_leave_the_document_assets_and_history_unchanged() {
    let (temp, store) = fixture();
    let before = fs::read(store.path()).unwrap();
    for (index, mode, key, uri) in [
        (0, "copy", "ui:generator", "missing.png"),
        (1, "copy", "ui:unknown", "pixel.png"),
        (2, "link", "ui:generator", "pixel.png"),
    ] {
        let mut edits = operations();
        edits[0]["mode"] = json!(mode);
        if mode == "link" {
            edits[0]["options"] = json!({"script_assets":{key:[{"kind":"image","uri":uri}]}});
        } else {
            edits[0]["options"]["script_assets"] = json!({key:[{"kind":"image","uri":uri}]});
        }
        let error = service::apply_request(
            &store,
            &request(&store, &format!("bad-script-{index}"), edits, false),
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            if index == 0 {
                "IO_ERROR"
            } else {
                "INVALID_OPERATION"
            }
        );
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(!temp.path().join("assets").exists());
    }
}

#[test]
fn script_copy_does_not_evaluate_javascript_during_preview_or_commit() {
    let (temp, store) = fixture();
    let code = "/* @schema 2.21 */ while (true) {}";
    fs::write(temp.path().join("library/draw.js"), code).unwrap();
    let before = fs::read(store.path()).unwrap();
    service::apply_request(
        &store,
        &request(&store, "nonexecuting-preview", operations(), true),
    )
    .unwrap();
    assert_eq!(fs::read(store.path()).unwrap(), before);
    let result = apply(&store, "nonexecuting-copy", operations());
    let document = store.read().unwrap();
    let id = result["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    let uri = document.node(id).unwrap()["scriptUri"].as_str().unwrap();
    assert_eq!(fs::read_to_string(temp.path().join(uri)).unwrap(), code);
}
