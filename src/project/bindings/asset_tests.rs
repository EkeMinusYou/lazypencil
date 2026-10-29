use super::*;

#[cfg(feature = "rendering")]
fn empty_project() -> Project {
    Project::local(Document::parse(r#"{"version":"2.21","children":[]}"#.into()).unwrap()).unwrap()
}

#[cfg(feature = "rendering")]
#[test]
fn remapping_a_generated_uri_cannot_reactivate_its_stale_scope() {
    let bound = Bound {
        origin: PathBuf::from("/captured/main.pen"),
        inputs_bound: false,
        values: serde_json::from_value(json!({
            "version":1,"script_uri":"generator.js",
            "assets":{"script:nested.js":"assets/nested.js"}
        }))
        .unwrap(),
    };
    let mut node = json!({"type":"script","scriptUri":"nested.js",
    "metadata":{"type":"test","keep":"unchanged","lazypencil":{
        "script_scope":{"version":1,"script_uri":"assets/nested.js","library":"uncaptured"}
    }}});
    bound
        .rewrite(
            &mut node,
            &BTreeMap::new(),
            &mut 0,
            &empty_project(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(node["scriptUri"], "assets/nested.js");
    assert_eq!(node["metadata"]["keep"], "unchanged");
    assert!(node.pointer("/metadata/lazypencil/script_scope").is_none());
    assert_eq!(
        node["metadata"]["lazypencil"]["script_bindings"]["script_uri"],
        "assets/nested.js"
    );
    // Metadata on a known non-script node is opaque, even if an unknown
    // property happens to have the same spelling as a script URI.
    let scope = json!({"version":1,"script_uri":"assets/nested.js","library":"uncaptured"});
    let mut shape = json!({"type":"rectangle","scriptUri":"nested.js",
        "metadata":{"type":"test","lazypencil":{"script_scope":scope}}});
    bound
        .rewrite(
            &mut shape,
            &BTreeMap::new(),
            &mut 0,
            &empty_project(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(shape["metadata"]["lazypencil"]["script_scope"], scope);
}

#[cfg(feature = "rendering")]
#[test]
fn remapping_a_generated_uri_cannot_reactivate_stale_bindings() {
    let bound = Bound {
        origin: PathBuf::from("/captured/main.pen"),
        inputs_bound: false,
        values: serde_json::from_value(json!({
            "version":1,"script_uri":"generator.js", "components":{"badge":"copied-badge"},
            "assets":{"script:nested.js":"assets/nested.js"}
        }))
        .unwrap(),
    };
    let stale = json!({"version":1,"script_uri":"assets/nested.js",
        "library":"uncaptured","components":{"badge":"unrelated-badge"}});
    let mut node = json!({"type":"script","scriptUri":"nested.js",
        "metadata":{"type":"test","keep":[1,2],"lazypencil":{"script_bindings":stale}}});
    bound
        .rewrite(
            &mut node,
            &BTreeMap::new(),
            &mut 0,
            &empty_project(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(node["scriptUri"], "assets/nested.js");
    assert_eq!(node["metadata"]["keep"], json!([1, 2]));
    let actual = &node["metadata"]["lazypencil"]["script_bindings"];
    assert_eq!(actual["components"]["badge"], "copied-badge");
    assert!(actual.get("library").is_none());

    // This change must neither reinterpret opaque metadata nor discard a
    // valid active context that still requires independent composition.
    let mut shape = json!({"type":"rectangle","scriptUri":"nested.js",
        "metadata":{"type":"test","lazypencil":{"script_bindings":stale}}});
    bound
        .rewrite(
            &mut shape,
            &BTreeMap::new(),
            &mut 0,
            &empty_project(),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(shape["metadata"]["lazypencil"]["script_bindings"], stale);
    let mut active = json!({"type":"script","scriptUri":"assets/nested.js",
        "metadata":{"lazypencil":{"script_bindings":stale}}});
    discard_inactive_generated_bindings(&mut active).unwrap();
    assert_eq!(active["metadata"]["lazypencil"]["script_bindings"], stale);
    for invalid in [
        json!({"version":2,"script_uri":"old.js"}),
        json!({"version":1,"script_uri":"old.js","unknown":true}),
    ] {
        let mut node = json!({"type":"script","scriptUri":"nested.js",
            "metadata":{"lazypencil":{"script_bindings":invalid}}});
        assert!(discard_inactive_generated_bindings(&mut node).is_err());
    }
}

#[test]
fn asset_keys_remove_dot_spelling_without_merging_parent_traversal_origins() {
    assert_eq!(
        asset_key("image", "./pixel.png").unwrap(),
        "image:pixel.png"
    );
    assert_eq!(
        asset_key("script", "../nested.js").unwrap(),
        "script:../nested.js"
    );
    assert_ne!(
        asset_key("image", "a/../pixel.png").unwrap(),
        asset_key("image", "pixel.png").unwrap()
    );
    assert_eq!(
        asset_key("image", "a/../pixel.png").unwrap(),
        "image:a/../pixel.png"
    );
}

#[test]
fn asset_keys_do_not_grant_schemes_unknown_kinds_or_control_paths() {
    for (kind, uri) in [
        ("image", "https://example.com/image.png"),
        ("script", "file:///outside.js"),
        ("binary", "asset.bin"),
        ("image", "bad\nname.png"),
        ("image", ""),
    ] {
        assert!(asset_key(kind, uri).is_err());
    }
}
