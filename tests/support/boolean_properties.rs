//! Boolean validation must agree across edits, structural checks and layout checks.
use super::*;
use lazypencil::{project::Project, store::Store};

#[test]
fn boolean_literals_reject_invalid_types_without_changing_the_document() {
    for version in ["2.19", "2.21", "2.22"] {
        let source = json!({"version":version,"children":[{
            "id":"box","type":"frame","width":20,"height":20,
            "metadata":{"enabled":"false"},"futureFeature":{"enabled":"false"}
        }]})
        .to_string();
        let document = Document::parse(source.clone()).unwrap();
        for key in [
            "enabled",
            "flipX",
            "flipY",
            "clip",
            "underline",
            "strikethrough",
            "reusable",
            "layoutIncludeStroke",
            "placeholder",
        ] {
            for value in [
                json!("false"),
                json!("true"),
                json!(0),
                json!([]),
                json!({}),
                Value::Null,
                json!("$"),
            ] {
                let error = operations::apply(
                    &document,
                    &operations(json!([
                        {"op":"update","target":"box","set":{key:value}}
                    ])),
                )
                .unwrap_err();
                assert_eq!(error.code, "INVALID_DOCUMENT", "{version}: {key}");
                assert!(error.message.contains(key), "{error}");
            }
            for value in [false, true] {
                let (after, _) = operations::apply(
                    &document,
                    &operations(json!([
                        {"op":"update","target":"box","set":{key:value}}
                    ])),
                )
                .unwrap();
                assert_eq!(after.node("box").unwrap()[key], value);
                assert_eq!(after.node("box").unwrap()["metadata"]["enabled"], "false");
                assert_eq!(
                    after.node("box").unwrap()["futureFeature"]["enabled"],
                    "false"
                );
            }
        }
        assert_eq!(document.text(), source);
    }
}

#[test]
fn boolean_validation_checks_paints_effects_and_descendant_overrides() {
    for changes in [
        json!({"enabled":"false"}),
        json!({"flipX":"true"}),
        json!({"fill":{"type":"color","color":"#fff","enabled":"false"}}),
        json!({"stroke":["#fff",{"type":"color","color":"#000","enabled":0}]}),
        json!({"effect":[{"type":"blur","radius":1,"enabled":"false"}]}),
    ] {
        let document = Document::parse(json!({"version":"2.22","children":[
            {"id":"component","type":"frame","reusable":true,"children":[{"id":"child","type":"rectangle"}]},
            {"id":"instance","type":"ref","ref":"component","descendants":{"child":changes}}
        ]}).to_string()).unwrap();
        let error = operations::validate_local(&document).unwrap_err();
        assert_eq!(error.code, "INVALID_DOCUMENT");
        assert!(error.message.contains("instance/child"), "{error}");
    }
}

#[test]
fn boolean_variables_validate_the_resolved_type_and_failed_edits_leave_no_history() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("booleans.pen");
    let source = json!({"version":"2.22","variables":{
        "flag":{"type":"boolean","value":false},
        "alias":{"type":"boolean","value":"$flag"},
        "text":{"type":"string","value":"false"},
        "number":{"type":"number","value":1}
    },"children":[{"id":"box","type":"rectangle","width":20,"height":20,
        "enabled":"$alias","flipX":"$flag","fill":{"type":"color","color":"#fff","enabled":"$flag"}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
    let scene = Project::load(&store).unwrap().resolve().unwrap();
    assert_eq!(scene.roots[0].properties["enabled"], false);
    assert_eq!(scene.roots[0].properties["flipX"], false);
    for value in [json!("false"), json!("$text"), json!("$number")] {
        let request = serde_json::from_value(json!({"request_id":"invalid-boolean",
            "expected_revision":store.read().unwrap().revision(),
            "operations":[{"op":"update","target":"box","set":{"enabled":value}}]}))
        .unwrap();
        assert_eq!(
            service::apply_request(&store, &request).unwrap_err().code,
            "INVALID_DOCUMENT"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert!(store.history(10, None).unwrap().is_empty());
    }
    let mut scopes = vec!["structure"];
    if cfg!(feature = "rendering") {
        scopes.push("layout");
    }
    for value in [json!("false"), json!("$text"), json!("$number")] {
        let mut document: Value = serde_json::from_str(&source).unwrap();
        document["children"][0]["enabled"] = value;
        fs::write(&path, document.to_string()).unwrap();
        for scope in &scopes {
            let output = fixture_support::output(
                Command::new(env!("CARGO_BIN_EXE_lazypencil"))
                    .args(["--json", "validate"])
                    .arg(&path)
                    .args(["--scope", scope])
                    .env("LAZYPENCIL_DATA_DIR", temp.path().join("data")),
                std::time::Duration::from_secs(10),
            );
            assert!(!output.status.success(), "{scope}: {:?}", output.stdout);
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["error"]["code"], "INVALID_DOCUMENT");
        }
    }
}
