use lazypencil::{document::Document, operations, project::Project, service, store::Store};
use serde_json::{Value, json};
use std::{fs, process::Command, time::Duration};
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture_support;

fn invalid_nodes() -> Vec<Value> {
    vec![
        json!({"type":"prompt","content":"hello"}),
        json!({"type":"context","content":"hello"}),
        json!({"type":"future"}),
        json!({"type":"rectangle","fill":"#fff8"}),
        json!({"type":"rectangle","fill":{"type":"color","color":"#fff8"}}),
        json!({"type":"rectangle","fill":{"type":"color","color":"#fff","opacity":0.5}}),
        json!({"type":"rectangle","fill":{"type":"image","mode":"tile"}}),
        json!({"type":"rectangle","fill":{"type":"gradient","gradientType":"conic"}}),
        json!({"type":"rectangle","effect":{"type":"drop_shadow"}}),
        json!({"type":"rectangle","effect":{"type":"shadow","color":"#fff8"}}),
        json!({"type":"rectangle","strokeAlignment":"inside"}),
        json!({"type":"rectangle","strokeLinecap":"flat"}),
        json!({"type":"rectangle","strokeLinejoin":"rounded"}),
        json!({"type":"rectangle","futureFeature":true}),
        json!({"type":"icon","stroke":"#000"}),
        json!({"type":"icon","strokeWidth":2}),
        json!({"type":"group","fill":"#fff"}),
        json!({"type":"frame","layout":"free"}),
        json!({"type":"frame","layoutPosition":"relative"}),
        json!({"type":"frame","justifyContent":"space-between"}),
        json!({"type":"frame","alignItems":"stretch"}),
        json!({"type":"text","textAlignVertical":"center"}),
        json!({"type":"text","textAlign":"start"}),
        json!({"type":"text","textGrowth":"fixed"}),
        json!({"type":"text","fontWeight":400}),
        json!({"type":"path","fillRule":"even-odd"}),
        json!({"type":"script","inputs":{"label":null}}),
    ]
}

#[test]
fn public_schema_rejects_extensions_in_edits_validation_and_layout() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("schema.pen");
    for version in ["2.19", "2.21", "2.22"] {
        for mut node in invalid_nodes() {
            node["id"] = json!("invalid");
            let source = json!({"version":version,"children":[node]}).to_string();
            let document = Document::parse(source.clone()).unwrap();
            assert_eq!(
                operations::validate_local(&document).unwrap_err().code,
                "INVALID_DOCUMENT",
                "{node}"
            );
            fs::write(&path, &source).unwrap();
            let project = Project::from_document(&path, document.clone(), &[]).unwrap();
            assert_eq!(
                project.resolve().err().unwrap().code,
                "INVALID_DOCUMENT",
                "{node}"
            );
            assert_eq!(document.text(), source);
        }
    }
}

#[test]
fn invalid_edits_are_atomic_and_stored_extensions_can_be_repaired() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("schema.pen");
    let source = "// Keep this comment.\n{\"version\":\"2.22\",\"children\":[{\"id\":\"box\",\"type\":\"rectangle\",\"width\":16,\"height\":16,\"fill\":\"#fff\"}]}";
    fs::write(&path, source).unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    for (index, node) in invalid_nodes().into_iter().enumerate() {
        for dry_run in [true, false] {
            let mut node = node.clone();
            node["id"] = json!("invalid");
            let request = serde_json::from_value(json!({"request_id":format!("invalid-{index}"),
                "expected_revision":store.read().unwrap().revision(),"dry_run":dry_run,
                "operations":[{"op":"insert","node":node}]}))
            .unwrap();
            assert!(
                matches!(
                    service::apply_request(&store, &request).unwrap_err().code,
                    "INVALID_DOCUMENT" | "UNSUPPORTED_FEATURE"
                ),
                "{node}"
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            assert!(store.history(10, None).unwrap().is_empty());
        }
    }
    let invalid = source.replace("\"fill\":\"#fff\"", "\"fill\":\"#fff8\",\"fontSzie\":24");
    fs::write(&path, &invalid).unwrap();
    assert!(
        service::document_metadata(&store).unwrap()["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["severity"] == "error")
    );
    let request = serde_json::from_value(json!({"request_id":"repair",
        "expected_revision":store.read().unwrap().revision(),
        "operations":[{"op":"update","target":"box","set":{"fill":"#ffffff88"},"unset":["fontSzie"]}]})).unwrap();
    service::apply_request(&store, &request).unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .starts_with("// Keep this comment.")
    );
    assert_eq!(
        Project::load(&store).unwrap().resolve().unwrap().roots[0].properties["fill"],
        "#ffffff88"
    );
}

#[test]
fn cli_structure_and_layout_reject_the_same_invalid_values() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("schema.pen");
    let mut scopes = vec!["structure"];
    if cfg!(feature = "rendering") {
        scopes.push("layout");
    }
    for node in [
        json!({"type":"text","textAlignVertical":"center"}),
        json!({"type":"rectangle","fill":"#fff8"}),
        json!({"type":"icon","stroke":"#fff"}),
    ] {
        let mut node = node;
        node["id"] = json!("invalid");
        let source = json!({"version":"2.22","children":[node]}).to_string();
        fs::write(&path, &source).unwrap();
        for scope in &scopes {
            let output = fixture_support::output(
                Command::new(env!("CARGO_BIN_EXE_lazypencil"))
                    .args(["--json", "validate"])
                    .arg(&path)
                    .args(["--scope", scope])
                    .env("LAZYPENCIL_DATA_DIR", temp.path().join("data")),
                Duration::from_secs(15),
            );
            assert!(!output.status.success(), "{node}: {scope}");
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["error"]["code"], "INVALID_DOCUMENT", "{result}");
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
    }
}

#[test]
fn schema_accepts_canonical_values_and_open_metadata_after_reference_expansion() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("schema.pen");
    let document = Document::parse(json!({"version":"2.22","variables":{"ink":{"type":"color","value":"#ffffff88"}},"children":[
        {"id":"box","type":"frame","reusable":true,"layout":"vertical","layoutPosition":"auto","justifyContent":"space_between","alignItems":"center","metadata":{"type":"custom","anything":{"enabled":"a string","futureFeature":true}},"children":[
            {"id":"label","type":"text","content":"Hello","fontWeight":"bold","textAlign":"center","textAlignVertical":"middle","textGrowth":"fixed-width-height","width":80,"height":40,"fill":"$ink"}]},
        {"id":"use","type":"ref","ref":"box","descendants":{"label":{"textAlignVertical":"bottom"}}}
    ]}).to_string()).unwrap();
    fs::write(&path, document.text()).unwrap();
    let scene = Project::from_document(&path, document, &[])
        .unwrap()
        .resolve()
        .unwrap();
    assert_eq!(
        scene.roots[1].children[0].properties["textAlignVertical"],
        "bottom"
    );
    assert_eq!(scene.roots[0].children[0].properties["fill"], "#ffffff88");
    assert_eq!(
        scene.roots[1].properties["metadata"]["anything"]["enabled"],
        "a string"
    );
    let mut invalid: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    invalid["children"][1]["descendants"]["label"]["strokeAlignment"] = json!("inside");
    let document = Document::parse(invalid.to_string()).unwrap();
    assert_eq!(
        Project::from_document(&path, document, &[])
            .unwrap()
            .resolve()
            .err()
            .unwrap()
            .code,
        "INVALID_DOCUMENT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn script_output_and_direct_layout_cannot_bypass_schema_validation() {
    use lazypencil::{layout, script::ScriptRuntime, text::Fonts};
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("generated.pen");
    let script = temp.path().join("draw.js");
    fs::write(
        &path,
        json!({"version":"2.22","children":[
            {"id":"script","type":"script","scriptUri":"draw.js","width":20,"height":20}
        ]})
        .to_string(),
    )
    .unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    for output in [
        "{type:'prompt',content:'hello'}",
        "{type:'rectangle',fill:'#fff8'}",
        "{type:'icon',icon:'house',stroke:'#000'}",
        "{type:'frame',layout:'free'}",
    ] {
        fs::write(&script, format!("/** @schema 2.22 */ return [{output}];")).unwrap();
        let mut project = Project::load(&store).unwrap();
        let mut fonts = Fonts::new(&[]).unwrap();
        let mut runtime = ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil"));
        let error = lazypencil::script::compute(&mut project, &mut fonts, &mut runtime)
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            if output.starts_with("{type:'prompt'") {
                "SCRIPT_EXECUTION_FAILED"
            } else {
                "INVALID_DOCUMENT"
            },
            "{output}: {error}"
        );
    }
    fs::write(&script, "/** @schema 2.22 */ return []; ").unwrap();
    let mut scene = Project::load(&store).unwrap().resolve().unwrap();
    scene.roots[0]
        .properties
        .insert("layout".into(), json!("free"));
    assert_eq!(
        layout::compute(scene, &mut Fonts::new(&[]).unwrap())
            .err()
            .unwrap()
            .code,
        "INVALID_DOCUMENT"
    );
}
