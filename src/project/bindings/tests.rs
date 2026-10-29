use super::*;

fn project(bindings: Value, uri: &str) -> Project {
    Project::local(Document::parse(json!({"version":"2.21", "variables":{"copied.tone":{"type":"color","value":"#0000ff"}},
        "themes":{"copied.mode":["light","dark"]},"children":[
            {"id":"copy","type":"rectangle","reusable":true,"width":10,"height":10},
            {"id":"not-reusable","type":"rectangle","width":10,"height":10},
            {"id":"generator","type":"script","scriptUri":uri,"width":10,"height":10,
             "inputs":{"selected":"old"},"metadata":{"type":"example","lazypencil":{"script_bindings":bindings}}}
        ]}).to_string()).unwrap()).unwrap()
}

fn bindings() -> Value {
    json!({"version":1,"script_uri":"draw.js","components":{"old":"copy"},
        "variables":{"old.tone":"copied.tone"},"themes":{"mode":"copied.mode"}})
}

#[test]
fn binding_validation_requires_local_targets_and_a_known_bounded_format() {
    for (value, code) in [
        (
            json!({"version":4,"script_uri":"draw.js"}),
            "UNSUPPORTED_VERSION",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","unexpected":true}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","library":12}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","library":"missing"}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","browser_library":"missing"}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","browser_library":"../outside.pen"}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","browser_library":""}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","browser_libraries":{"base":"missing"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","browser_libraries":{"":""}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","browser_libraries":{"\u{0001}":"copy"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","library":"../outside.pen"}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","components":{"old":"missing"}}),
            "INVALID_OPERATION",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","components":{"old":"not-reusable"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","variables":{"old":"missing"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","themes":{"old":"missing"}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","inputs_hash":"bad"}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","descendants":{"unknown":{"old":"new"}}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","components":{"old":"copy"},"descendant_contexts":{"old":{"child":{"parent":"child","instance":false,"target_instance":false}}}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","components":{"old":"copy"},"descendant_contexts":{"old":{"child":{"parent":"absent","instance":false,"target_instance":true}}}}),
            "INVALID_DOCUMENT",
        ),
        (
            json!({"version":1,"script_uri":"draw.js","variables":{"":"copied.tone"}}),
            "INVALID_DOCUMENT",
        ),
    ] {
        let result = project(value.clone(), "draw.js").resolve();
        assert_eq!(result.unwrap_err().code, code, "{value}");
    }
}

#[test]
fn changed_code_disables_stale_targets_but_does_not_bypass_format_validation() {
    let mut value = bindings();
    value["components"]["old"] = json!("removed");
    let scene = project(value, "replacement.js").resolve().unwrap();
    assert!(scene.roots[2].script_bindings.is_none());
    let mut value = bindings();
    value["version"] = json!(9);
    assert_eq!(
        project(value, "replacement.js").resolve().unwrap_err().code,
        "UNSUPPORTED_VERSION"
    );
    let mut value = bindings();
    value["library"] = json!("bad\nalias");
    assert_eq!(
        project(value, "replacement.js").resolve().unwrap_err().code,
        "INVALID_DOCUMENT"
    );
}

#[test]
fn saved_input_bindings_are_bound_to_canonical_json_and_cannot_grant_paths() {
    let mut value = bindings();
    value["inputs_hash"] = json!(hash(br#"{"selected":"old"}"#));
    let scene = project(value.clone(), "draw.js").resolve().unwrap();
    assert!(
        scene.roots[2]
            .script_bindings
            .as_ref()
            .unwrap()
            .inputs_bound
    );
    value["inputs_hash"] = json!("0".repeat(64));
    let scene = project(value, "draw.js").resolve().unwrap();
    assert!(
        !scene.roots[2]
            .script_bindings
            .as_ref()
            .unwrap()
            .inputs_bound
    );
    let mut value = bindings();
    value["components"]["old"] = json!("../outside.pen:component");
    assert!(project(value, "draw.js").resolve().is_err());
}
