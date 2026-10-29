use super::*;

#[cfg(feature = "rendering")]
fn custom_parent() -> Bound {
    let scope = |badge: &str, libraries: Value| {
        json!({
            "components":{"badge":badge,"alternate":"alternate-slot","card":"card-slot"},
            "variables":{"tone":"tone-slot","accent":"accent-slot"},"themes":{"mode":"mode-slot"},
            "assets":{"script:nested.js":"assets/nested.js"},"libraries":libraries,"browser_libraries":{}
        })
    };
    Bound::from_values(PathBuf::from("/captured/main.pen"), false, parse(&json!({
        "version":2,"script_uri":"generator.js","assets":{"script:nested.js":"assets/nested.js"},
        "components":{"badge-slot":"copied-badge","alternate-slot":"copied-alternate","card-slot":"copied-card","base-slot":"base-badge"},
        "variables":{"tone-slot":"copied-tone","accent-slot":"copied-accent"},"themes":{"mode-slot":"copied-mode"},
        "descendants":{"card-slot":{"label":"copied-label"}},
        "descendant_contexts":{"card-slot":{"label":{"parent":"","instance":false,"target_instance":false}}},
        "script_scopes":{"":scope("badge-slot", json!({"":"","base":"base"})),"base":scope("base-slot",json!({"":"base"}))}
    })).unwrap()).unwrap()
}

#[cfg(feature = "rendering")]
#[test]
fn custom_bindings_keep_key_shadowing_identity_fallbacks_paths_and_closed_assets() {
    let parent = custom_parent();
    let raw = json!({"version":1,"script_uri":"nested.js",
        "components":{"badge":"alternate","old":"card"},
        "input_components":{"selection":"badge"},
        "variables":{"tone":"accent","accent":"tone"},"themes":{"old-mode":"mode"},
        "descendants":{"old":{"old-label":"label"}},
        "descendant_contexts":{"old":{"old-label":{"parent":"","instance":false,"target_instance":true}}},
        "assets":{"script:alias.js":"nested.js"}
    });
    let output = parent.custom_output(&raw, hash(b"nested.js")).unwrap();
    assert_eq!(output.component("badge").unwrap(), "copied-alternate");
    assert_eq!(output.component("alternate").unwrap(), "copied-alternate");
    assert_eq!(output.component("old").unwrap(), "copied-card");
    assert_eq!(output.component("card").unwrap(), "copied-card");
    assert_eq!(
        output.values.descendants["old"]["old-label"],
        "copied-label"
    );
    assert_eq!(output.values.descendants["card"]["label"], "copied-label");
    assert!(!output.values.descendant_contexts["old"]["old-label"].target_instance);
    assert_eq!(output.values.variables["tone"], "copied-accent");
    assert_eq!(output.values.variables["accent"], "copied-tone");
    assert_eq!(output.values.themes["old-mode"], "copied-mode");
    assert_eq!(output.values.themes["mode"], "copied-mode");
    assert_eq!(
        output.values.input_components.as_ref().unwrap()["selection"],
        "copied-badge"
    );
    assert_eq!(
        output.asset("script", "alias.js").unwrap(),
        "assets/nested.js"
    );
    assert_eq!(
        output.asset("script", "nested.js").unwrap(),
        "assets/nested.js"
    );
    assert_eq!(
        output.asset("script", "missing.js").unwrap_err().code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(
        output.selected("").unwrap().component("badge").unwrap(),
        "copied-alternate"
    );
    assert_eq!(
        output.selected("base").unwrap().component("badge").unwrap(),
        "base-badge"
    );
    for (field, value) in [
        ("library", json!("missing")),
        ("components", json!({"badge":"missing"})),
        ("descendants", json!({"old":{"old-label":"missing"}})),
    ] {
        let mut invalid = raw.clone();
        invalid[field] = value;
        assert_eq!(
            parent
                .custom_output(&invalid, hash(b"nested.js"))
                .unwrap_err()
                .code,
            "INVALID_DOCUMENT"
        );
    }
}

#[cfg(feature = "rendering")]
#[test]
fn custom_captured_views_remain_separate_and_do_not_acquire_parent_aliases() {
    let parent = custom_parent();
    let raw = json!({"version":2,"script_uri":"nested.js","assets":{},
        "components":{"first":"badge","second":"alternate"},
        "script_scopes":{
            "":{"components":{"badge":"first"},"variables":{},"themes":{},"assets":{},"libraries":{"":"","view":"second"},"browser_libraries":{}},
            "second":{"components":{"badge":"second"},"variables":{},"themes":{},"assets":{},"libraries":{"":"second"},"browser_libraries":{}}
        }
    });
    let output = parent.custom_output(&raw, hash(b"nested.js")).unwrap();
    assert_eq!(output.component("badge").unwrap(), "copied-badge");
    let view = output.selected("view").unwrap();
    assert_eq!(view.component("badge").unwrap(), "copied-alternate");
    assert!(view.selected("base").is_err());
    assert!(view.selected("view").is_err());
    assert!(output.selected("base").is_err());
    assert_eq!(
        output.asset("script", "nested.js").unwrap_err().code,
        "ASSET_NOT_FOUND"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn inherited_script_uri_guards_use_original_provenance_instead_of_relocated_spellings() {
    let project = Project::local(Document::parse(json!({"version":"2.21","children":[{
        "id":"worker","type":"script","width":36,"height":12,"scriptUri":"assets/nested.js",
        "metadata":{"type":"test","lazypencil":{"script_bindings":{
            "version":3,"script_uri":"assets/nested.js","source_uri_hash":hash(b"nested.js"),"assets":{}
        }}}
    }]}).to_string()).unwrap()).unwrap();
    let target = project.resolve().unwrap().roots.remove(0);
    let parent = Bound::from_values(PathBuf::from("/captured/main.pen"), false, parse(&json!({
        "version":2,"script_uri":"generator.js","assets":{},
        "script_scopes":{"":{"components":{},"variables":{},"themes":{},"assets":{},"libraries":{"":""},"browser_libraries":{}}}
    })).unwrap()).unwrap();
    let mut changes = json!({"metadata":{"type":"test","lazypencil":{"script_scope":{
        "version":1,"script_uri":"assets/nested.js","library":"uncaptured"
    }}}});
    let scope = changes
        .pointer("/metadata/lazypencil/script_scope")
        .unwrap()
        .clone();
    let (output, input) = parent
        .compose_generated_scope(
            &scope,
            &mut changes,
            &target,
            &GeneratedState::default(),
            "assets/nested.js",
            &mut 0,
        )
        .unwrap();
    assert!(output.is_none() && input.is_none());
    changes["metadata"]["lazypencil"]["script_scope"]["script_uri"] = json!("nested.js");
    let scope = changes
        .pointer("/metadata/lazypencil/script_scope")
        .unwrap()
        .clone();
    assert_eq!(
        parent
            .compose_generated_scope(
                &scope,
                &mut changes,
                &target,
                &GeneratedState::default(),
                "assets/nested.js",
                &mut 0
            )
            .unwrap_err()
            .code,
        "INVALID_DOCUMENT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn generated_explicit_scope_selection_keeps_shared_tables_and_rejects_unreachable_namespaces() {
    let scope = |component: &str, variable: &str, libraries: Value| {
        json!({
            "components":{"badge":component},"variables":{"tone":variable},"themes":{},"assets":{},
            "libraries":libraries,"browser_libraries":{}
        })
    };
    let values = parse(&json!({"version":2,"script_uri":"draw.js","assets":{},
        "components":{"ui-member":"ui-target","base-member":"base-target"},
        "variables":{"ui-tone":"ui_tone","base-tone":"base_tone"},
        "descendants":{"ui-member":{"label":"ui-label"},"base-member":{"label":"base-label"}},
        "script_scopes":{
            "":scope("ui-member","ui-tone",json!({"":"","base":"base"})),
            "base":scope("base-member","base-tone",json!({"":"base"}))
        }
    }))
    .unwrap();
    let parent = Bound::from_values(PathBuf::from("/captured/main.pen"), false, values).unwrap();
    assert_eq!(parent.component("badge").unwrap(), "ui-target");
    let base = parent.selected("base").unwrap();
    assert_eq!(base.component("badge").unwrap(), "base-target");
    assert_eq!(base.values.descendants["badge"]["label"], "base-label");
    let mut tone = json!("$tone");
    base.declared_input(&mut tone).unwrap();
    assert_eq!(tone, "$base_tone");
    assert!(base.selected("base").is_err());
    assert_eq!(
        base.selected("").unwrap().component("badge").unwrap(),
        "base-target"
    );
    let output = base
        .inherited_output_with_hash("nested.js", None, None, &mut 0)
        .unwrap();
    assert!(parent.generated_output(&output).unwrap());
    let mut escaped = output.clone();
    escaped["script_namespace"] = json!("");
    assert!(!base.generated_output(&escaped).unwrap());
    assert_eq!(
        serde_json::to_value(&base).unwrap()["values"]["components"]["ui-member"],
        "ui-target"
    );
}

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
        captured: None,
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
        captured: None,
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
