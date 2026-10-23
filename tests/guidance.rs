use lazypencil::{document::Document, guidance};
use serde_json::Value;
#[cfg(feature = "rendering")]
use serde_json::json;
use std::process::Command;

#[test]
fn guides_styles_and_editable_examples_form_a_resolvable_catalog() {
    let entry = guidance::get_guidance(None).unwrap();
    assert_eq!(entry["id"], "start");
    assert_eq!(entry["language"], "en");
    assert!(
        entry["body"]
            .as_str()
            .unwrap()
            .contains("requested language")
    );
    for guide in entry["available_guides"].as_array().unwrap() {
        let body = guidance::get_guidance(guide["id"].as_str()).unwrap();
        assert!(!body["body"].as_str().unwrap().is_empty());
        assert_eq!(body["hash"].as_str().unwrap().len(), 64);
        assert_eq!(body, guidance::get_guidance(guide["id"].as_str()).unwrap());
    }
    let catalog = guidance::list_styles(None, 100, None).unwrap();
    assert_eq!(catalog["styles"].as_array().unwrap().len(), 3);
    for summary in catalog["styles"].as_array().unwrap() {
        let style = guidance::get_style(summary["id"].as_str().unwrap()).unwrap();
        assert_eq!(style["language"], "en");
        assert_eq!(style["license"], "MIT");
        for guide in style["guidance"].as_array().unwrap() {
            guidance::get_guidance(guide.as_str()).unwrap();
        }
        let example = Document::parse(style["example"]["document"].to_string()).unwrap();
        lazypencil::operations::validate_local(&example).unwrap();
        for id in example.locations().keys() {
            if let Some(size) = example.node(id).unwrap().get("fontSize") {
                assert!(
                    style["tokens"]["fontSizes"]
                        .as_array()
                        .unwrap()
                        .contains(size),
                    "{id}: {size}"
                );
            }
        }
        assert!(example.node_count() > 5);
        assert_eq!(style["required_fonts"][0]["family"], "Noto Sans JP");
    }
}

#[test]
fn style_pagination_binds_cursors_to_the_filter_and_catalog() {
    let first = guidance::list_styles(None, 1, None).unwrap();
    let second = guidance::list_styles(None, 1, first["next_cursor"].as_str()).unwrap();
    assert_ne!(first["styles"][0]["id"], second["styles"][0]["id"]);
    let dashboard = guidance::list_styles(Some("dashboard"), 100, None).unwrap();
    assert_eq!(dashboard["styles"].as_array().unwrap().len(), 1);
    assert_eq!(dashboard["styles"][0]["id"], "dense-workspace");
    assert!(guidance::list_styles(Some("dashboard"), 1, first["next_cursor"].as_str()).is_err());
    assert!(guidance::get_guidance(Some("does-not-exist")).is_err());
    assert!(guidance::get_style("does-not-exist").is_err());
    assert!(guidance::list_styles(None, 0, None).is_err());
}

#[test]
fn the_guide_operation_example_runs_against_the_real_editing_api() {
    let guide = guidance::get_guidance(Some("operations")).unwrap();
    let operations: Vec<lazypencil::operations::Operation> =
        serde_json::from_value(guide["example_operations"].clone()).unwrap();
    let (document, result) =
        lazypencil::operations::apply(&Document::empty(), &operations).unwrap();
    let button = result["bindings"]["button"].as_str().unwrap();
    let instance = result["bindings"]["instance"].as_str().unwrap();
    assert_eq!(document.node(instance).unwrap()["ref"], button);
    assert_eq!(document.node(button).unwrap()["reusable"], true);
    assert_eq!(document.value()["variables"]["accent"]["value"], "#2563EB");
    let example = include_str!("../resources/examples/button-operations.json");
    assert!(guide["body"].as_str().unwrap().contains(example.trim()));
}

#[test]
fn the_component_guide_import_batch_and_every_style_component_use_the_real_service() {
    use lazypencil::{operations::EditRequest, service, store::Store};
    let temp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let store = Store::init(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    let guide = guidance::get_guidance(Some("components")).unwrap();
    let body = guide["body"].as_str().unwrap();
    let example = body
        .split_once("```json\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    let revision = store.read().unwrap().revision();
    let request:EditRequest=serde_json::from_value(serde_json::json!({"request_id":"guide-import","expected_revision":revision,"operations":serde_json::from_str::<Value>(example).unwrap()})).unwrap();
    let result = service::apply_request(&store, &request).unwrap();
    let document = store.read().unwrap();
    let instance = document
        .node(result["bindings"]["primary"].as_str().unwrap())
        .unwrap();
    assert_eq!(instance["x"], 40);
    assert_eq!(instance["y"], 40);
    assert_eq!(
        document.node(instance["ref"].as_str().unwrap()).unwrap()["name"],
        "Kit Primary action"
    );
    for id in ["standard-ui", "dense-workspace", "content-page"] {
        let style = guidance::get_style(id).unwrap();
        assert!(
            !style["example"]["components"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for (index, component) in style["example"]["components"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let metadata = service::document_metadata(&store).unwrap();
            let request:EditRequest=serde_json::from_value(serde_json::json!({"request_id":format!("style-{id}-{index}"),"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":[
                {"op":"import_component","mode":"copy","source":{"style_id":id,"node_id":component["node_id"]},"options":{"variable_prefix":format!("{id}.{index}."),"component_prefix":format!("{id}.{index} ")}}
            ]})).unwrap();
            service::apply_request(&store, &request).unwrap();
        }
    }
}

#[test]
fn cli_uses_the_same_english_guides_and_style_hashes_as_the_shared_catalog() {
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "guidance", "typography"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, guidance::get_guidance(Some("typography")).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["guidance"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("# Design workflow")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "style", "standard-ui"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value, guidance::get_style("standard-ui").unwrap());
}

#[cfg(feature = "rendering")]
#[test]
fn all_style_examples_have_resolved_layouts_and_render_without_overflow() {
    use lazypencil::{layout, project::Project, render, store::Store, text::Fonts};
    let temp = tempfile::TempDir::new().unwrap();
    let mut fonts = Fonts::new(&[]).unwrap();
    for id in ["standard-ui", "dense-workspace", "content-page"] {
        let style = guidance::get_style(id).unwrap();
        let path = temp.path().join(format!("{id}.pen"));
        std::fs::write(&path, style["example"]["document"].to_string()).unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        let project = Project::load(&store).unwrap();
        let snapshot = layout::compute(project.resolve().unwrap(), &mut fonts).unwrap();
        let diagnostics = snapshot
            .items
            .iter()
            .flat_map(|i| &i.record.diagnostics)
            .collect::<Vec<_>>();
        assert!(diagnostics.is_empty(), "{id}: {}", json!(diagnostics));
        let png = render::png(&project, &snapshot, &render::RenderOptions::default()).unwrap();
        assert!(png.width >= 960, "{id}");
        assert!(png.height >= 600, "{id}");
    }
}
