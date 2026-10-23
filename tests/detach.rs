use lazypencil::{
    document::Document,
    operations::{self, EditRequest, Operation},
    project::{NodeAddress, Project},
    service,
    store::Store,
};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

fn write(path: &std::path::Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir_all(temp.path().join("library/nested")).unwrap();
    #[cfg(feature = "rendering")]
    let image = {
        use skia_safe::{Color, EncodedImageFormat};
        let mut surface = skia_safe::surfaces::raster_n32_premul((2, 2)).unwrap();
        surface.canvas().clear(Color::YELLOW);
        surface
            .image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .unwrap()
            .as_bytes()
            .to_vec()
    };
    #[cfg(not(feature = "rendering"))]
    let image = b"image bytes are not decoded by structural operations".to_vec();
    fs::write(temp.path().join("library/pixel.png"), image).unwrap();
    write(
        &temp.path().join("library/nested/base.lib.pen"),
        json!({
            "version":"2.21","themes":{"density":["roomy","compact"]},
            "variables":{"ink":{"type":"color","value":"#111111"}},
            "children":[{"id":"shared","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"$ink"}]
        }),
    );
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({
            "version":"2.21","imports":{"nested":"nested/base.lib.pen"},
            "themes":{"mode":["dark","light"],"density":["compact","roomy"]},
            "variables":{"color":{"type":"color","value":[{"value":"#ff0000"},{"theme":{"mode":"dark"},"value":"#0000ff"}]}},
            "children":[
                {"id":"card","type":"frame","reusable":true,"layout":"none","width":80,"height":80,"fill":"$color","children":[
                    {"id":"shared","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"$color"},
                    {"id":"own-use","type":"ref","ref":"shared","x":12},
                    {"id":"action","type":"ref","ref":"nested:shared","x":24},
                    {"id":"picture","type":"rectangle","x":40,"width":10,"height":10,"fill":{"type":"image","url":"pixel.png"}},
                    {"id":"slot","type":"frame","y":20,"width":10,"height":10,"slot":["shared"],"children":[]}
                ]},
                {"id":"variant","type":"ref","ref":"card","reusable":true,"descendants":{"action":{"name":"Nested action"}}}
            ]
        }),
    );
    write(
        &temp.path().join("main.pen"),
        json!({
            "version":"2.21","imports":{"ui":"library/ui.lib.pen"},
            "themes":{"mode":["light","dark"]},
            "variables":{"color":{"type":"color","value":"#00ff00"}},
            "children":[
                {"id":"shared","type":"rectangle","x":180,"width":8,"height":8,"fill":"$color"},
                {"id":"use","type":"ref","ref":"ui:variant","descendants":{"action":{"width":12}}},
                {"id":"other","type":"ref","ref":"ui:variant","x":90}
            ]
        }),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn detach(store: &Store, target: Value, request_id: &str) -> Value {
    let metadata = service::document_metadata(store).unwrap();
    let request: EditRequest = serde_json::from_value(json!({
        "request_id":request_id,"expected_revision":metadata["revision"],
        "expected_dependency_revision":metadata["dependency_revision"],
        "operations":[{"op":"detach_instance","target":target,"client_ref":"detached"}]
    }))
    .unwrap();
    service::apply_request(store, &request).unwrap()
}

#[test]
fn external_detach_keeps_variable_and_asset_origins_and_disambiguates_owned_ids() {
    let (temp, store) = fixture();
    let library = temp.path().join("library/ui.lib.pen");
    let original = fs::read(&library).unwrap();
    let result = detach(&store, json!("use"), "external-detach");
    let document = store.read().unwrap();
    let node = document.node("use").unwrap();
    assert_eq!(node["type"], "frame");
    assert_eq!(node["fill"], "$ui:color");
    assert_eq!(node["theme"]["density"], "compact");
    let children = node["children"].as_array().unwrap();
    assert_eq!(children[1]["ref"], children[0]["id"]);
    assert_ne!(children[0]["id"], "shared");
    assert_eq!(children[2]["ref"], "ui:nested:shared");
    assert_eq!(children[2]["name"], "Nested action");
    assert_eq!(children[2]["width"], 12);
    assert_eq!(children[3]["fill"]["url"], "library/pixel.png");
    assert_eq!(children[4]["slot"][0], children[0]["id"]);
    assert_eq!(document.node("shared").unwrap()["fill"], "$color");
    assert_eq!(document.node("other").unwrap()["type"], "ref");
    assert_eq!(result["bindings"]["detached"], "use");
    assert_eq!(fs::read(&library).unwrap(), original);
    let scene = Project::load(&store).unwrap().resolve().unwrap();
    let materialized = scene
        .roots
        .iter()
        .find(|n| n.address.node_id == "use")
        .unwrap();
    assert_eq!(materialized.properties["fill"], "#ff0000");
    assert_eq!(materialized.theme["density"], "compact");
    assert_eq!(materialized.children[2].properties["fill"], "#111111");
    assert_eq!(materialized.children[2].theme["density"], "compact");
}

#[test]
fn an_inherited_external_instance_detaches_without_mutating_either_library() {
    let (temp, store) = fixture();
    let originals = ["library/ui.lib.pen", "library/nested/base.lib.pen"]
        .map(|p| (temp.path().join(p), fs::read(temp.path().join(p)).unwrap()));
    let result = detach(
        &store,
        json!({"node_id":"use","instance_path":["action"]}),
        "inherited-external",
    );
    let document = store.read().unwrap();
    assert_eq!(document.node("use").unwrap()["type"], "ref");
    let id = result["bindings"]["detached"].as_str().unwrap();
    let node = document.node(id).unwrap();
    assert_eq!(node["type"], "rectangle");
    assert_eq!(node["fill"], "$ui:nested:ink");
    assert_eq!(node["width"], 12);
    assert_eq!(node["name"], "Nested action");
    assert!(node.get("ref").is_none());
    let effective = Project::load(&store)
        .unwrap()
        .expanded_node(&NodeAddress {
            node_id: "use".into(),
            instance_path: vec!["action".into()],
        })
        .unwrap();
    assert_eq!(effective.properties["width"], 12);
    for (path, bytes) in originals {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn external_detach_is_guarded_by_dependency_revisions_and_atomic_on_failure() {
    let (temp, store) = fixture();
    let metadata = service::document_metadata(&store).unwrap();
    let bytes = fs::read(store.path()).unwrap();
    let library = temp.path().join("library/ui.lib.pen");
    fs::write(
        &library,
        fs::read_to_string(&library)
            .unwrap()
            .replace("Nested action", "Changed"),
    )
    .unwrap();
    let mut request:EditRequest = serde_json::from_value(json!({
        "request_id":"stale-external","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],
        "operations":[{"op":"detach_instance","target":"use"}]
    })).unwrap();
    assert_eq!(
        service::apply_request(&store, &request).unwrap_err().code,
        "REVISION_CONFLICT"
    );
    assert_eq!(fs::read(store.path()).unwrap(), bytes);
    let fresh = service::document_metadata(&store).unwrap();
    request.expected_dependency_revision =
        Some(fresh["dependency_revision"].as_str().unwrap().into());
    request.operations.push(
        serde_json::from_value::<Operation>(
            json!({"op":"update","target":"missing","set":{"x":1}}),
        )
        .unwrap(),
    );
    assert!(service::apply_request(&store, &request).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), bytes);
}

#[cfg(feature = "rendering")]
#[test]
fn external_and_inherited_detachment_preserve_rendered_pixels_and_image_dependencies() {
    use lazypencil::render::{self, RenderOptions};
    for target in [
        json!("use"),
        json!({"node_id":"use","instance_path":["action"]}),
    ] {
        let (_temp, store) = fixture();
        let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let before = render::png(&project, &snapshot, &RenderOptions::default()).unwrap();
        detach(&store, target, "pixel-detach");
        let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let after = render::png(&project, &snapshot, &RenderOptions::default()).unwrap();
        assert_eq!((after.width, after.height), (before.width, before.height));
        assert_eq!(after.png, before.png);
        assert_eq!(
            serde_json::to_value(after.assets).unwrap(),
            serde_json::to_value(before.assets).unwrap()
        );
    }
}

#[test]
fn detaching_an_external_script_retains_its_execution_scope_without_changing_code() {
    let (temp, store) = fixture();
    fs::write(
        temp.path().join("library/code.js"),
        "/* @schema 2.21 */\nreturn [];",
    )
    .unwrap();
    let library = temp.path().join("library/ui.lib.pen");
    let mut document = Document::parse(fs::read_to_string(&library).unwrap()).unwrap();
    document
        .set_property("/children/0/children/0", "type", &json!("script"))
        .unwrap();
    document
        .set_property("/children/0/children/0", "scriptUri", &json!("code.js"))
        .unwrap();
    fs::write(library, document.text()).unwrap();
    let bytes = fs::read(store.path()).unwrap();
    let project = Project::load(&store).unwrap();
    let operations: Vec<Operation> =
        serde_json::from_value(json!([{"op":"detach_instance","target":"use"}])).unwrap();
    let (after, _) =
        operations::apply_in_project(project.root_document(), &operations, store.path()).unwrap();
    let copied_script = after.value()["children"][1]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["type"] == "script")
        .unwrap();
    assert_eq!(
        copied_script["metadata"]["lazypencil"]["script_scope"]["library"],
        "ui"
    );
    assert_eq!(copied_script["scriptUri"], "library/code.js");
    assert_eq!(
        fs::read_to_string(temp.path().join("library/code.js")).unwrap(),
        "/* @schema 2.21 */\nreturn [];"
    );
    assert_eq!(fs::read(store.path()).unwrap(), bytes);
}

#[test]
fn external_nested_overrides_remain_in_the_correct_namespace_after_detachment() {
    for target in [
        json!("use"),
        json!({"node_id":"use","instance_path":["action"]}),
    ] {
        let (temp, store) = fixture();
        let path = temp.path().join("library/nested/base.lib.pen");
        let mut base = Document::parse(fs::read_to_string(&path).unwrap()).unwrap();
        base.set_property("/children/0", "type", &json!("frame"))
            .unwrap();
        base.set_property(
            "/children/0",
            "children",
            &json!([
                {"id":"label","type":"text","content":"Original"}
            ]),
        )
        .unwrap();
        fs::write(path, base.text()).unwrap();
        let mut document = store.read().unwrap();
        document
            .set_property(
                "/children/1/descendants",
                "action/label",
                &json!({"content":"Outer override"}),
            )
            .unwrap();
        fs::write(store.path(), document.text()).unwrap();
        detach(&store, target, "deep-override");
        let document = store.read().unwrap();
        if document.node("use").unwrap()["type"] == "frame" {
            let action = &document.node("use").unwrap()["children"][2];
            assert_eq!(action["type"], "ref");
            assert_eq!(action["descendants"]["label"]["content"], "Outer override");
        } else {
            let action = &document.node("use").unwrap()["descendants"]["action"];
            assert_eq!(action["children"][0]["content"], "Outer override");
            assert!(
                document.node("use").unwrap()["descendants"]
                    .get("action/label")
                    .is_none()
            );
        }
    }
}

#[test]
fn a_shader_sampler_variable_cannot_silently_switch_to_another_file() {
    let (temp, store) = fixture();
    fs::copy(
        temp.path().join("library/pixel.png"),
        temp.path().join("pixel.png"),
    )
    .unwrap();
    fs::write(temp.path().join("library/paint.glsl"), "void main() {}").unwrap();
    let library = temp.path().join("library/ui.lib.pen");
    let mut document = Document::parse(fs::read_to_string(&library).unwrap()).unwrap();
    document
        .set_property(
            "/variables",
            "picture",
            &json!({"type":"string","value":"pixel.png"}),
        )
        .unwrap();
    document
        .set_property(
            "/children/0",
            "fill",
            &json!({"type":"shader","url":"paint.glsl","uniforms":{"texture":"$picture"}}),
        )
        .unwrap();
    fs::write(library, document.text()).unwrap();
    let bytes = fs::read(store.path()).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let request:EditRequest=serde_json::from_value(json!({
        "request_id":"sampler-scope","expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],
        "operations":[{"op":"detach_instance","target":"use"}]
    })).unwrap();
    service::apply_request(&store, &request).unwrap();
    assert_ne!(fs::read(store.path()).unwrap(), bytes);
    let project = Project::load(&store).unwrap();
    let node = project
        .resolve()
        .unwrap()
        .roots
        .into_iter()
        .find(|node| node.address.node_id == "use")
        .unwrap();
    assert_eq!(
        node.properties["fill"]["uniforms"]["texture"],
        json!(
            temp.path()
                .join("library/pixel.png")
                .canonicalize()
                .unwrap()
        )
    );
}

#[test]
fn actual_cli_detaches_an_inherited_external_instance() {
    let (_temp, store) = fixture();
    let metadata = service::document_metadata(&store).unwrap();
    let request = store.path().parent().unwrap().join("request.json");
    write(
        &request,
        json!([{"op":"detach_instance","target":{"node_id":"use","instance_path":["action"]}}]),
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--data-dir")
        .arg(store.data_dir())
        .arg("--json")
        .arg("apply")
        .arg(store.path())
        .arg("--operations")
        .arg(request)
        .arg("--expected-revision")
        .arg(metadata["revision"].as_str().unwrap())
        .arg("--expected-dependency-revision")
        .arg(metadata["dependency_revision"].as_str().unwrap())
        .args(["--request-id", "cli-inherited-detach"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result["generated_ids"]["0"]["id"].is_string(), "{result}");
    assert_eq!(
        store.read().unwrap().node("use").unwrap()["descendants"]["action"]["type"],
        "rectangle"
    );
}

#[test]
fn equal_owned_ids_from_library_and_consumer_keep_separate_internal_links() {
    let (_temp, store) = fixture();
    let mut document = store.read().unwrap();
    document.remove_element("/children/0").unwrap();
    document.set_property("/children/0/descendants","slot",&json!({"children":[
        {"id":"shared","type":"rectangle","reusable":true,"width":3,"height":3,"fill":"$color"},
        {"id":"slot-use","type":"ref","ref":"shared"}
    ]})).unwrap();
    fs::write(store.path(), document.text()).unwrap();
    detach(&store, json!("use"), "same-owned-id");
    let document = store.read().unwrap();
    let children = &document.node("use").unwrap()["children"];
    let external = &children[0];
    let local = &children[4]["children"][0];
    assert_ne!(external["id"], local["id"]);
    assert_eq!(children[1]["ref"], external["id"]);
    assert_eq!(children[4]["children"][1]["ref"], local["id"]);
    assert_eq!(external["fill"], "$ui:color");
    assert_eq!(local["fill"], "$color");
}

#[test]
fn unavailable_slot_suggestions_do_not_become_detachment_constraints() {
    let (temp, store) = fixture();
    let library = temp.path().join("library/ui.lib.pen");
    let mut document = Document::parse(fs::read_to_string(&library).unwrap()).unwrap();
    document
        .set_property(
            "/children/0/children/4",
            "slot",
            &json!(["shared", "not-yet-provided"]),
        )
        .unwrap();
    fs::write(library, document.text()).unwrap();
    detach(&store, json!("use"), "open-suggestions");
    let document = store.read().unwrap();
    let children = &document.node("use").unwrap()["children"];
    assert_eq!(children[4]["slot"][0], children[0]["id"]);
    assert_eq!(children[4]["slot"][1], "ui:not-yet-provided");
}
