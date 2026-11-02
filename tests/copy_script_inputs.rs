use lazypencil::{operations::EditRequest, service, store::Store};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

const HEADER: &str = "/**\n * @schema 2.21\n * @input selected: ref\n * @input label: string\n * @input external: ref\n * @input variable: ref\n */\n";

fn fixture(script: Value, body: &str) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::write(temp.path().join("draw.js"), format!("{HEADER}{body}")).unwrap();
    fs::write(
        temp.path().join("library.pen"),
        json!({"version":"2.21","children":[
            {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10}
        ]})
        .to_string(),
    )
    .unwrap();
    fs::write(temp.path().join("main.pen"), json!({"version":"2.21",
        "imports":{"lib":"library.pen"},
        "variables":{"choice":{"type":"string","value":"badge"}},
        "children":[
            {"id":"generator","type":"script","reusable":true,"enabled":false,"scriptUri":"draw.js"},
            {"id":"card","type":"frame","reusable":true,"enabled":false,
                "layout":"none","width":20,"height":10,"children":[
                {"id":"nested","type":"script","scriptUri":"draw.js","width":20,"height":10}
            ]},
            {"id":"group","type":"frame","layout":"none","width":30,"height":20,"children":[
                {"id":"badge","type":"rectangle","reusable":true,"enabled":false,
                    "width":10,"height":10,"fill":"#ff0000"}, script
            ]}
        ]}).to_string()).unwrap();
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn copy_and_remove(store: &Store) -> Value {
    let metadata = service::document_metadata(store).unwrap();
    let request: EditRequest = serde_json::from_value(json!({
        "request_id":"copy-script-inputs", "expected_revision":metadata["revision"],
        "expected_dependency_revision":metadata["dependency_revision"],
        "operations":[{"op":"copy","target":"group"},{"op":"delete","target":"group"}]
    }))
    .unwrap();
    service::apply_request(store, &request).unwrap()
}

#[test]
fn copy_remaps_only_declared_local_ref_inputs_without_executing_code() {
    for kind in ["script", "ref", "override"] {
        let inputs = json!({"selected":"badge","label":"badge","external":"lib:badge",
            "variable":"$choice","stale":"badge"});
        let script = match kind {
            "script" => json!({"id":"draw","type":"script","scriptUri":"draw.js","inputs":inputs}),
            "ref" => json!({"id":"draw","type":"ref","ref":"generator","inputs":inputs}),
            _ => {
                json!({"id":"draw","type":"ref","ref":"card","descendants":{"nested":{"inputs":inputs}}})
            }
        };
        let (temp, store) = fixture(
            script,
            "throw new Error('Copy must not execute JavaScript');",
        );
        let code = fs::read(temp.path().join("draw.js")).unwrap();
        let response = copy_and_remove(&store);
        let mapping = &response["generated_ids"]["0"]["id_map"];
        let document = store.read().unwrap();
        let draw = document.node(mapping["draw"].as_str().unwrap()).unwrap();
        let inputs = if kind == "override" {
            &draw["descendants"]["nested"]["inputs"]
        } else {
            &draw["inputs"]
        };
        assert_eq!(inputs["selected"], mapping["badge"], "{kind}");
        assert_eq!(inputs["label"], "badge");
        assert_eq!(inputs["external"], "lib:badge");
        assert_eq!(inputs["variable"], "$choice");
        assert_eq!(inputs["stale"], "badge");
        assert_eq!(fs::read(temp.path().join("draw.js")).unwrap(), code);
    }
}

#[test]
fn copied_captured_inputs_keep_their_local_or_external_namespace() {
    for external in [false, true] {
        let mut context = json!({"version":1,"values":{"selected":"original"},
            "variables":{},"components":{"original":"badge"}});
        if external {
            context["library"] = json!("lib");
        }
        let script = json!({"id":"draw","type":"script","scriptUri":"draw.js",
            "inputs":{"selected":"original"},
            "metadata":{"type":"lazypencil","lazypencil":{"script_input_context":context}}});
        let (_temp, store) = fixture(script, "return []; ");
        let response = copy_and_remove(&store);
        let mapping = &response["generated_ids"]["0"]["id_map"];
        let document = store.read().unwrap();
        let draw = document.node(mapping["draw"].as_str().unwrap()).unwrap();
        assert_eq!(draw["inputs"]["selected"], "original");
        assert_eq!(
            draw["metadata"]["lazypencil"]["script_input_context"]["components"]["original"],
            if external {
                json!("badge")
            } else {
                mapping["badge"].clone()
            }
        );
        lazypencil::project::Project::load(&store)
            .unwrap()
            .expanded_node(&lazypencil::project::NodeAddress {
                node_id: mapping["draw"].as_str().unwrap().into(),
                instance_path: vec![],
            })
            .unwrap();
    }
}

#[test]
fn copied_bound_inputs_keep_their_hash_and_retarget_captured_components() {
    let inputs = json!({"selected":"original"});
    let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
    let script = json!({"id":"draw","type":"script","scriptUri":"draw.js", "inputs":inputs,
    "metadata":{"type":"lazypencil","lazypencil":{"script_bindings":{
        "version":1,"script_uri":"draw.js","inputs_hash":hash,
        "components":{"original":"badge"}, "variables":{},"themes":{},"descendants":{}
    }}}});
    let (_temp, store) = fixture(script, "return []; ");
    let response = copy_and_remove(&store);
    let mapping = &response["generated_ids"]["0"]["id_map"];
    let document = store.read().unwrap();
    let draw = document.node(mapping["draw"].as_str().unwrap()).unwrap();
    assert_eq!(draw["inputs"], inputs);
    assert_eq!(
        draw["metadata"]["lazypencil"]["script_bindings"]["inputs_hash"],
        hash
    );
    assert_eq!(
        draw["metadata"]["lazypencil"]["script_bindings"]["components"]["original"],
        mapping["badge"]
    );
    lazypencil::project::Project::load(&store)
        .unwrap()
        .expanded_node(&lazypencil::project::NodeAddress {
            node_id: mapping["draw"].as_str().unwrap().into(),
            instance_path: vec![],
        })
        .unwrap();
}

#[cfg(feature = "rendering")]
#[test]
fn copied_script_renders_after_the_original_component_is_removed() {
    use lazypencil::{render, script::ScriptRuntime};
    use skia_safe::{Color, Data, Image};
    for kind in ["script", "ref", "override"] {
        let mut script = match kind {
            "script" => {
                json!({"type":"script","scriptUri":"draw.js","inputs":{"selected":"badge"}})
            }
            "ref" => json!({"type":"ref","ref":"generator","inputs":{"selected":"badge"}}),
            _ => {
                json!({"type":"ref","ref":"card","descendants":{"nested":{"inputs":{"selected":"badge"}}}})
            }
        };
        script["id"] = json!("draw");
        script["enabled"] = json!(true);
        script["width"] = json!(20);
        script["height"] = json!(10);
        let (_temp, store) = fixture(
            script,
            "return [{type:'ref',ref:pencil.input.selected,enabled:true}];",
        );
        let png = || {
            let (project, layout) = service::layout_snapshot_with_runtime(
                &store,
                &[],
                None,
                None,
                &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
            )
            .unwrap();
            render::png(&project, &layout, &Default::default())
                .unwrap()
                .png
        };
        let before = png();
        let image = Image::from_encoded(Data::new_copy(&before))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(
            image.peek_pixels().unwrap().get_color((5, 5)),
            Color::RED,
            "{kind}"
        );
        copy_and_remove(&store);
        assert_eq!(png(), before, "{kind}");
    }
}
