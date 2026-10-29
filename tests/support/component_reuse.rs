use super::{request, write};
use lazypencil::{
    project::{Project, ResolvedNode},
    service,
    store::Store,
};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

fn fixture(version: &str) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let nested = json!({"id":"nested","type":"frame","name":"Source","reusable":true,"x":"$offset","y":3,"width":20,"height":20,"fill":"$unused","metadata":{"type":"source"},"children":[
        {"id":"source-text","type":"text","content":"Source"},
        {"id":"badge","type":"rectangle","reusable":true,"width":2,"height":2,"fill":"#ff0000"}
    ]});
    let group = json!({"id":"group","type":"frame","layout":"none","width":120,"height":30,"children":[
        nested,
        {"id":"sibling","type":"ref","ref":"nested","x":55,"y":3,"descendants":{"source-text":{"content":"Inner"}}},
        {"id":"external","type":"ref","ref":"badge","x":100,"y":3}
    ]});
    let parent = json!({"id":"parent","type":"frame","name":"Parent","reusable":true,"layout":"none","width":120,"height":30,"children":[group]});
    let wrapper = json!({"id":"wrapper","type":"frame","name":"Wrapper","reusable":true,"layout":"none","width":120,"height":30,"metadata":{"type":"test","literal":"$unused"},"children":[
        {"id":"parent-use","type":"ref","ref":"parent","descendants":{
            "nested":{"x":18},"source-text":{"content":"Outer"},"sibling/source-text":{"content":"Reference"}
        }}
    ]});
    write(
        &temp.path().join("library.pen"),
        json!({"version":version,
        "variables":{"offset":{"type":"number","value":12},"unused":{"type":"color","value":"#ff0000"}},
        "children":[parent,wrapper]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":version,"imports":{"ui":"library.pen"},"children":[
            {"id":"existing","name":"Local","type":"frame","reusable":true,"enabled":false,"layout":"none","width":24,"height":20,"fill":"#00ff00","metadata":{"type":"local","keep":[1,2]},"children":[
            {"id":"local-label","type":"text","content":"Local","fontSize":7,"fill":"#000000"},
            {"id":"local-badge","type":"rectangle","width":2,"height":2,"fill":"#00ff00"}
            ]}
        ]}),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn operation() -> Value {
    json!({"op":"import_component","source":{"library_alias":"ui","node_id":"wrapper"},"mode":"copy",
        "options":{"variable_prefix":"from.","components":{"ui:nested":{"action":"reuse","target":"existing"}},
            "component_paths":{"ui:nested":{"source-text":"local-label","badge":"local-badge"}}}})
}

fn find<'a>(node: &'a ResolvedNode, key: &str) -> &'a ResolvedNode {
    fn visit<'a>(node: &'a ResolvedNode, key: &str) -> Option<&'a ResolvedNode> {
        if node.component_key == key {
            return Some(node);
        }
        node.children.iter().find_map(|child| visit(child, key))
    }
    visit(node, key).unwrap_or_else(|| panic!("Missing component key {key}"))
}

#[cfg(feature = "rendering")]
fn pixels(store: &Store, id: &str) -> Vec<u8> {
    use lazypencil::render::{self, RenderOptions};
    let (project, snapshot) = service::layout_snapshot(store, &[], None, None).unwrap();
    render::png(
        &project,
        &snapshot,
        &RenderOptions {
            node: Some(id.into()),
            ..Default::default()
        },
    )
    .unwrap()
    .png
}

#[test]
fn contained_reuse_keeps_placement_overrides_and_live_target_through_source_removal_and_recopy() {
    for version in ["2.19", "2.21"] {
        let (temp, store) = fixture(version);
        let edits = json!([operation(), {"op":"remove_import","alias":"ui"}]);
        let before = fs::read(store.path()).unwrap();
        service::apply_request(
            &store,
            &request(&store, "reuse-preview", edits.clone(), true),
        )
        .unwrap();
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(!temp.path().join("assets").exists());
        let saved =
            service::apply_request(&store, &request(&store, "reuse-save", edits, false)).unwrap();
        let report = &saved["generated_ids"]["0"];
        let id = report["id"].as_str().unwrap();
        let component = report["component_id"].as_str().unwrap();
        let placed = report["id_map"]["ui:nested"].as_str().unwrap();
        let sibling = report["id_map"]["ui:sibling"].as_str().unwrap();
        assert_ne!(placed, "existing");
        assert_eq!(report["reused_components"]["ui:nested"], "existing");
        assert!(report["id_map"].get("ui:source-text").is_none());
        let document = store.read().unwrap();
        let badge = report["id_map"]["ui:badge"].as_str().unwrap();
        assert_eq!(document.node(badge).unwrap()["fill"], "#ff0000");
        assert_eq!(document.node(placed).unwrap()["type"], "ref");
        assert_eq!(document.node(placed).unwrap()["ref"], "existing");
        assert_eq!(document.node(placed).unwrap()["x"], "$from.offset");
        assert!(document.node(placed).unwrap().get("children").is_none());
        assert!(document.value()["variables"].get("unused").is_none());
        assert_eq!(
            document.node(component).unwrap()["metadata"]["literal"],
            "$unused"
        );
        fs::remove_file(temp.path().join("library.pen")).unwrap();
        let project = Project::load(&store).unwrap();
        let scene = project.resolve().unwrap();
        let root = scene.roots.iter().find(|node| node.id() == id).unwrap();
        let placement = find(root, placed);
        assert_eq!(placement.properties["x"], 18);
        assert_eq!(placement.properties["y"], 3);
        assert_eq!(placement.properties["width"], 20);
        assert_eq!(placement.properties["fill"], "#00ff00");
        assert_eq!(
            placement.properties["metadata"],
            json!({"type":"local","keep":[1,2]})
        );
        assert_eq!(
            find(placement, "local-label").properties["content"],
            "Outer"
        );
        assert_eq!(
            find(find(root, sibling), "local-label").properties["content"],
            "Reference"
        );
        // The target definition stays live; no copied source body masks it.
        service::apply_request(
            &store,
            &request(
                &store,
                "edit-target",
                json!([
                    {"op":"update","target":"existing","set":{"fill":"#0000ff","enabled":true}},
                    {"op":"update","target":"local-label","set":{"fontSize":8}}
                ]),
                false,
            ),
        )
        .unwrap();
        let scene = Project::load(&store).unwrap().resolve().unwrap();
        let root = scene.roots.iter().find(|node| node.id() == id).unwrap();
        assert_eq!(find(root, placed).properties["fill"], "#0000ff");
        assert_eq!(
            find(find(root, placed), "local-label").properties["fontSize"],
            8
        );
        #[cfg(feature = "rendering")]
        let first_png = pixels(&store, id);

        write(
            &temp.path().join("recopy.pen"),
            json!({"version":version,"imports":{"first":"main.pen"},"children":[]}),
        );
        let second = Store::open(
            temp.path().join("recopy.pen"),
            Some(temp.path().join("second-data")),
        )
        .unwrap();
        let again = service::apply_request(&second, &request(&second, "reuse-recopy", json!([
            {"op":"import_component","source":{"library_alias":"first","node_id":component},"mode":"copy"},
            {"op":"remove_import","alias":"first"}
        ]), false)).unwrap();
        fs::remove_file(store.path()).unwrap();
        let second_id = again["generated_ids"]["0"]["id"].as_str().unwrap();
        let scene = Project::load(&second).unwrap().resolve().unwrap();
        let second_root = scene
            .roots
            .iter()
            .find(|node| node.id() == second_id)
            .unwrap();
        let second_placed = again["generated_ids"]["0"]["id_map"][format!("first:{placed}")]
            .as_str()
            .unwrap();
        assert_eq!(
            find(second_root, second_placed).properties["fill"],
            "#0000ff"
        );
        assert!(
            Project::load(&second)
                .unwrap()
                .dependencies()
                .iter()
                .all(|entry| entry.kind != "document")
        );
        #[cfg(feature = "rendering")]
        assert_eq!(pixels(&second, second_id), first_png);
    }
}

#[test]
fn incompatible_or_unused_reuse_paths_fail_without_publication_and_link_rejects_them() {
    let (temp, store) = fixture("2.21");
    let before = fs::read(store.path()).unwrap();
    let mut cases = Vec::new();
    let mut missing = operation();
    missing["options"]
        .as_object_mut()
        .unwrap()
        .remove("component_paths");
    cases.push(missing);
    for paths in [
        json!({"ui:nested":{"source-text":"absent"}}),
        json!({"ui:nested":{"typo":"local-label"}}),
        json!({"ui:absent":{"source-text":"local-label"}}),
        json!({"ui:nested":{"source-text":"/local-label"}}),
    ] {
        let mut copy = operation();
        copy["options"]["component_paths"] = paths;
        cases.push(copy);
    }
    let mut link = operation();
    link["mode"] = json!("link");
    link["options"] = json!({"component_paths":{"ui:nested":{"source-text":"local-label"}}});
    cases.push(link);
    for (index, operation) in cases.into_iter().enumerate() {
        let error = service::apply_request(
            &store,
            &request(
                &store,
                &format!("bad-reuse-{index}"),
                json!([operation]),
                false,
            ),
        )
        .unwrap_err();
        assert_eq!(error.code, "INVALID_OPERATION", "{error}");
        assert_eq!(fs::read(store.path()).unwrap(), before);
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(!temp.path().join("assets").exists());
    }
}

#[test]
fn matching_descendant_ids_need_no_explicit_reuse_path_mapping() {
    let (_temp, store) = fixture("2.21");
    let mut document = store.read().unwrap();
    document
        .set_property("/children/0/children/0", "id", &json!("source-text"))
        .unwrap();
    document
        .set_property("/children/0/children/1", "id", &json!("badge"))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let mut copy = operation();
    copy["options"]
        .as_object_mut()
        .unwrap()
        .remove("component_paths");
    let result = service::apply_request(
        &store,
        &request(
            &store,
            "matching-reuse",
            json!([copy,{"op":"remove_import","alias":"ui"}]),
            false,
        ),
    )
    .unwrap();
    let id = result["generated_ids"]["0"]["id"].as_str().unwrap();
    let placed = result["generated_ids"]["0"]["id_map"]["ui:nested"]
        .as_str()
        .unwrap();
    let scene = Project::load(&store).unwrap().resolve().unwrap();
    let root = scene.roots.iter().find(|node| node.id() == id).unwrap();
    assert_eq!(
        find(find(root, placed), "source-text").properties["content"],
        "Outer"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn generated_refs_keep_reused_definitions_and_override_paths_after_independent_recopy() {
    use lazypencil::{
        document::hash,
        render::{self, RenderOptions},
        script::ScriptRuntime,
    };
    const CODE: &str = r#"/** @schema 2.21 */
return [
 {id:"chosen",type:"ref",ref:"parent",descendants:{"source-text":{content:"Generated"},"sibling/source-text":{content:"Generated ref"}}},
 {id:"direct",type:"ref",ref:"nested",y:30}
];"#;
    fn render(store: &Store, id: &str) -> Vec<u8> {
        let (project, snapshot) = service::layout_snapshot_with_runtime(
            store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        )
        .unwrap();
        render::png(
            &project,
            &snapshot,
            &RenderOptions {
                node: Some(id.into()),
                ..Default::default()
            },
        )
        .unwrap()
        .png
    }
    for version in ["2.19", "2.21"] {
        let (temp, store) = fixture(version);
        let library = temp.path().join("library.pen");
        let mut source: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
        source["children"].as_array_mut().unwrap().push(json!({"id":"generator","type":"script","reusable":true,"name":"Generator","scriptUri":"generate.js","width":120,"height":60}));
        write(&library, source);
        fs::write(temp.path().join("generate.js"), CODE).unwrap();
        let mut document = store.read().unwrap();
        document
            .set_property("/children/0", "enabled", &json!(true))
            .unwrap();
        fs::write(store.path(), document.text()).unwrap();
        let mut copy = operation();
        copy["source"]["node_id"] = json!("generator");
        let result = service::apply_request(
            &store,
            &request(
                &store,
                "generated-reuse",
                json!([copy,{"op":"remove_import","alias":"ui"}]),
                false,
            ),
        )
        .unwrap();
        let report = &result["generated_ids"]["0"];
        let id = report["id"].as_str().unwrap();
        let component = report["component_id"].as_str().unwrap();
        let placed = report["id_map"]["ui:nested"].as_str().unwrap();
        let document = store.read().unwrap();
        let bindings =
            &document.node(component).unwrap()["metadata"]["lazypencil"]["script_bindings"];
        assert_eq!(bindings["components"]["nested"], "existing");
        assert_eq!(bindings["descendants"]["parent"]["nested"], placed);
        assert_eq!(
            bindings["descendants"]["parent"]["source-text"],
            format!("{placed}/local-label")
        );
        assert_eq!(
            bindings["descendants"]["nested"]["source-text"],
            "local-label"
        );
        assert_eq!(
            fs::read(temp.path().join("generate.js")).unwrap(),
            CODE.as_bytes()
        );
        assert!(
            report["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|asset| asset["hash"] == hash(CODE.as_bytes()))
        );
        fs::remove_file(library).unwrap();
        fs::remove_file(temp.path().join("generate.js")).unwrap();
        let independent = render(&store, id);
        let raster = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&independent))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(
            raster.peek_pixels().unwrap().get_color((15, 18)),
            skia_safe::Color::GREEN
        );
        assert_eq!(
            raster.peek_pixels().unwrap().get_color((1, 49)),
            skia_safe::Color::GREEN
        );
        fs::create_dir(temp.path().join("second")).unwrap();
        write(
            &temp.path().join("second/recopy.pen"),
            json!({"version":version,"imports":{"first":"../main.pen"},"children":[]}),
        );
        let second = Store::open(
            temp.path().join("second/recopy.pen"),
            Some(temp.path().join("second-data")),
        )
        .unwrap();
        let again = service::apply_request(&second, &request(&second, "generated-recopy", json!([
            {"op":"import_component","source":{"library_alias":"first","node_id":component},"mode":"copy"},
            {"op":"remove_import","alias":"first"}
        ]), false)).unwrap();
        fs::remove_file(store.path()).unwrap();
        fs::remove_dir_all(temp.path().join("assets")).unwrap();
        let second_id = again["generated_ids"]["0"]["id"].as_str().unwrap();
        let recopied = render(&second, second_id);
        assert_eq!(recopied, independent);
        if let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
            let root = std::path::PathBuf::from(root).join("component-reuse");
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join(format!("generated-{version}-independent.png")),
                &independent,
            )
            .unwrap();
            fs::write(
                root.join(format!("generated-{version}-recopied.png")),
                &recopied,
            )
            .unwrap();
            write(
                &root.join(format!("generated-{version}-report.json")),
                json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"code_hash":hash(CODE.as_bytes()),"independent":hash(&independent),"recopied":hash(&recopied),"width":120,"height":60}),
            );
        }
    }
}
