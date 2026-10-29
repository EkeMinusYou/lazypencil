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
    let mut unclosed = raw.clone();
    unclosed.as_object_mut().unwrap().remove("assets");
    let native = parent.custom_output(&unclosed, hash(b"nested.js")).unwrap();
    assert_eq!(native.original().version, 4);
    let (owner, original) = native
        .generated_inputs(&json!({"value":"$tone","nested":["$accent"],"unused":{"metadata":"$tone","ref":"$tone","children":["$tone"],"paint":"$tone"}}))
        .unwrap();
    assert_eq!(
        original,
        json!({"value":"$accent","nested":["$tone"],"unused":{"metadata":"$tone","ref":"$tone","children":["$tone"],"paint":"$accent"}})
    );
    assert_eq!(owner.values.variables["accent"], "copied-accent");
    let namespace = native.original().script_namespace.as_deref().unwrap();
    let mut legacy = serde_json::to_value(native.original()).unwrap();
    legacy["script_scopes"][namespace]
        .as_object_mut()
        .unwrap()
        .remove("source_variables");
    let legacy = Bound::from_values(native.origin.clone(), false, parse(&legacy).unwrap()).unwrap();
    assert_eq!(
        legacy
            .generated_inputs(&json!({"selected":"$tone"}))
            .unwrap_err()
            .code,
        "UNSUPPORTED_FEATURE"
    );
    assert!(
        legacy
            .generated_inputs(&json!({"selected":"badge"}))
            .is_ok()
    );
    let mut wrong = serde_json::to_value(native.original()).unwrap();
    wrong["script_scopes"][namespace]["source_variables"]["tone"] = json!("tone");
    assert!(parse(&wrong).is_err());
    assert_eq!(
        native.document_scope().unwrap().component("badge").unwrap(),
        "copied-badge"
    );
    assert_eq!(
        native.document_scope().unwrap().values.variables["tone"],
        "copied-tone"
    );
    let child = native.custom_output(&json!({"version":1,"script_uri":"nested.js","components":{"badge":"badge"},"variables":{"tone":"tone"}}), hash(b"nested.js")).unwrap();
    assert_eq!(child.component("badge").unwrap(), "copied-badge");
    assert_eq!(child.values.variables["tone"], "copied-tone");
    for invalid_target in [
        "uncaptured",
        native.original().script_namespace.as_deref().unwrap(),
    ] {
        let mut invalid = serde_json::to_value(native.original()).unwrap();
        let name = native.original().script_namespace.as_deref().unwrap();
        invalid["script_scopes"][name]["document_namespace"] = json!(invalid_target);
        assert!(parse(&invalid).is_err());
    }
    let mut old_version = serde_json::to_value(native.original()).unwrap();
    old_version["version"] = json!(3);
    assert!(parse(&old_version).is_err());
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
    // Native binding validation checks global declarations even when the
    // selected view does not use them. Copy composition must do the same.
    for field in ["browser_library", "browser_libraries"] {
        let mut invalid = raw.clone();
        invalid[field] = if field == "browser_library" {
            json!("uncaptured")
        } else {
            json!({"unused":"uncaptured"})
        };
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
fn captured_code_owner_stays_distinct_from_output_and_requires_valid_provenance() {
    let parent = custom_parent();
    assert_eq!(parent.code_scope().unwrap_err().code, "UNSUPPORTED_FEATURE");
    let mut values = parent.original().clone();
    values.version = 3;
    values.source_uri_hash = Some(hash(b"generator.js"));
    values.code_namespace = Some("base".into());
    let bound = Bound::from_values(parent.origin.clone(), false, values.clone()).unwrap();
    assert_eq!(bound.component("badge").unwrap(), "copied-badge");
    assert_eq!(
        bound.code_scope().unwrap().component("badge").unwrap(),
        "base-badge"
    );
    assert!(parse(&serde_json::to_value(&values).unwrap()).is_ok());
    let output = bound
        .inherited_output_with_hash("nested.js", None, Some(hash(b"nested.js")), &mut 0)
        .unwrap();
    assert!(output.get("code_namespace").is_none());
    values.code_namespace = Some("missing".into());
    assert!(parse(&serde_json::to_value(&values).unwrap()).is_err());
    values.code_namespace = Some("base".into());
    values.version = 2;
    values.source_uri_hash = None;
    assert!(parse(&serde_json::to_value(values).unwrap()).is_err());
}

#[cfg(feature = "rendering")]
#[test]
fn generated_code_ownership_merges_colliding_graphs_without_changing_output() {
    let parent = custom_parent();
    let mut source = parent.original().clone();
    source
        .components
        .insert("badge-slot".into(), "code-badge".into());
    source
        .variables
        .insert("tone-slot".into(), "code-tone".into());
    source
        .assets
        .as_mut()
        .unwrap()
        .insert("script:code.js".into(), "assets/code.js".into());
    source
        .script_scopes
        .get_mut("")
        .unwrap()
        .assets
        .insert("script:code.js".into(), "assets/code.js".into());
    let code = Bound::from_values(parent.origin.clone(), false, source).unwrap();
    let output = parent
        .output_with_code_owner(
            "assets/nested.js",
            None,
            Some(hash(b"nested.js")),
            Some(&code),
            &mut 0,
        )
        .unwrap();
    let parsed = Bound::from_values(parent.origin.clone(), false, parse(&output).unwrap()).unwrap();
    assert_eq!(parsed.component("badge").unwrap(), "copied-badge");
    assert_eq!(parsed.values.variables["tone"], "copied-tone");
    let owner = parsed.code_scope().unwrap();
    assert_eq!(owner.component("badge").unwrap(), "code-badge");
    assert_eq!(owner.values.variables["tone"], "code-tone");
    assert_eq!(owner.asset("script", "code.js").unwrap(), "assets/code.js");
    assert_eq!(
        owner.selected("base").unwrap().component("badge").unwrap(),
        "base-badge"
    );
    let again = parsed
        .output_with_code_owner(
            "assets/nested.js",
            None,
            Some(hash(b"nested.js")),
            Some(&owner),
            &mut 0,
        )
        .unwrap();
    assert_eq!(again["code_namespace"], output["code_namespace"]);
    assert_eq!(again["script_scopes"], output["script_scopes"]);
    let mut exhausted = crate::script::MAX_OUTPUT_BYTES;
    assert!(
        parent
            .output_with_code_owner(
                "assets/nested.js",
                None,
                Some(hash(b"nested.js")),
                Some(&code),
                &mut exhausted
            )
            .is_err()
    );
    let mut foreign = code;
    foreign.origin = PathBuf::from("/different/main.pen");
    assert!(
        parent
            .output_with_code_owner(
                "assets/nested.js",
                None,
                Some(hash(b"nested.js")),
                Some(&foreign),
                &mut 0
            )
            .is_err()
    );
}

#[cfg(feature = "rendering")]
#[test]
fn inherited_script_uri_guards_use_original_provenance_instead_of_relocated_spellings() {
    let project = Project::local(Document::parse(json!({"version":"2.21","children":[{
        "id":"worker","type":"script","width":36,"height":12,"scriptUri":"assets/nested.js",
        "metadata":{"type":"test","lazypencil":{"script_bindings":{
            "version":3,"script_uri":"assets/nested.js","source_uri_hash":hash(b"nested.js"),"assets":{},"code_namespace":"",
            "script_scopes":{"":{"components":{},"variables":{},"themes":{},"assets":{},"libraries":{"":""},"browser_libraries":{}}}
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
    let scope = changes.pointer("/metadata/lazypencil").unwrap().clone();
    let (output, input) = parent
        .compose_generated_scopes(
            &scope,
            None,
            &mut changes,
            &target,
            &GeneratedState::default(),
            &mut 0,
        )
        .unwrap();
    assert!(output.is_some() && input.is_none());
    assert_eq!(output.unwrap()["source_uri_hash"], hash(b"nested.js"));
    changes["metadata"]["lazypencil"]["script_scope"]["script_uri"] = json!("nested.js");
    let scope = changes.pointer("/metadata/lazypencil").unwrap().clone();
    assert_eq!(
        parent
            .compose_generated_scopes(
                &scope,
                None,
                &mut changes,
                &target,
                &GeneratedState::default(),
                &mut 0
            )
            .unwrap_err()
            .code,
        "INVALID_DOCUMENT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn combined_scope_guards_preserve_independent_output_input_and_variable_owners() {
    let parent = custom_parent();
    let input = json!({"selected":"$tone"});
    let input_hash = super::super::scope::value_hash(&input);
    for scope_output in [false, true] {
        for binding_output in [false, true] {
            for scope_input in [false, true] {
                for binding_input in [false, true] {
                    let mut node = json!({"type":"script","scriptUri":"nested.js","inputs":input,"metadata":{"lazypencil":{
                        "script_scope":{"version":1,"script_uri":if scope_output {"nested.js"} else {"old.js"},"library":if scope_output {"base"} else {"unreachable"},"inputs":{"hash":if scope_input {input_hash.clone()} else {"a".repeat(64)},"library":if scope_input {"base"} else {"unreachable"}}},
                        "script_bindings":{"version":1,"script_uri":if binding_output {"nested.js"} else {"old.js"},"library":if binding_output {""} else {"unreachable"},"components":{"badge":if binding_output {"alternate"} else {"missing"}},"variables":{"tone":"accent"},"inputs_hash":if binding_input {input_hash.clone()} else {"b".repeat(64)},"input_components":{"badge":"alternate"}}
                    }}});
                    let selected = generated::selection(&parent, &node).unwrap().unwrap();
                    assert_eq!(
                        selected.output.component("badge").unwrap(),
                        if binding_output {
                            "copied-alternate"
                        } else if scope_output {
                            "base-badge"
                        } else {
                            "copied-badge"
                        }
                    );
                    selected.apply(&mut node, &mut 0).unwrap();
                    let context = &node["metadata"]["lazypencil"]["script_input_context"];
                    assert_eq!(context["variables"]["tone"], "copied-tone");
                    assert_eq!(node["inputs"]["selected"], "$copied-tone");
                    assert_eq!(
                        context["components"]["badge"],
                        if binding_output && binding_input {
                            "copied-alternate"
                        } else if scope_input {
                            "base-badge"
                        } else {
                            "copied-badge"
                        }
                    );
                }
            }
        }
    }
    let node = json!({"type":"script","scriptUri":"nested.js","metadata":{"lazypencil":{
        "script_scope":{"version":1,"script_uri":"nested.js","library":"unreachable"},
        "script_bindings":{"version":1,"script_uri":"nested.js","components":{"badge":"badge"}}
    }}});
    assert_eq!(
        generated::selection(&parent, &node).err().unwrap().code,
        "INVALID_DOCUMENT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn explicit_generated_input_context_owns_expressions_and_components_independently() {
    let parent = custom_parent();
    for (library, variable, component) in [
        ("", "copied-accent", "copied-alternate"),
        ("base", "copied-tone", "base-badge"),
    ] {
        let mut node = json!({"type":"script","scriptUri":"nested.js","inputs":{"selected":"$selection"},"metadata":{"lazypencil":{
            "script_scope":{"version":1,"script_uri":"nested.js","library":"base"},
            "script_bindings":{"version":1,"script_uri":"nested.js","components":{"badge":"badge"}},
            "script_input_context":{"version":1,"library":library,"values":{"selected":"$tone"},"variables":{"selection":"tone"},"components":{"badge":if library.is_empty() {"alternate"} else {"badge"}}}
        }}});
        if library.is_empty() {
            node["metadata"]["lazypencil"]["script_input_context"]["values"]["selected"] =
                json!("$accent");
            node["metadata"]["lazypencil"]["script_input_context"]["variables"]["selection"] =
                json!("accent");
            node["metadata"]["lazypencil"]["script_input_context"]["variables"]["accent"] =
                json!("tone");
        }
        let selected = generated::selection(&parent, &node).unwrap().unwrap();
        assert_eq!(selected.output.component("badge").unwrap(), "copied-badge");
        selected.apply(&mut node, &mut 0).unwrap();
        let context = &node["metadata"]["lazypencil"]["script_input_context"];
        assert_eq!(node["inputs"]["selected"], format!("${variable}"));
        assert_eq!(context["components"]["badge"], component);
        assert_eq!(context["source_values"]["selected"], "$selection");
    }
    let context = json!({"version":1,"library":"missing","values":{"selected":"different"},"variables":{"tone":"missing"},"components":{"badge":"missing"}});
    let mut node = json!({"type":"script","scriptUri":"nested.js","inputs":{"selected":"badge"},"metadata":{"lazypencil":{"script_input_context":context}}});
    let selected = generated::selection(&parent, &node).unwrap().unwrap();
    selected.apply(&mut node, &mut 0).unwrap();
    assert_eq!(
        node["metadata"]["lazypencil"]["script_input_context"]["components"]["badge"],
        "copied-badge"
    );
    node["metadata"]["lazypencil"]["script_input_context"] = context;
    node["inputs"]["selected"] = json!("different");
    assert_eq!(
        generated::selection(&parent, &node).err().unwrap().code,
        "INVALID_DOCUMENT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn generated_override_carriers_replace_only_the_selected_context_side() {
    let parent = custom_parent();
    let project = Project::local(Document::parse(json!({"version":"2.21","children":[
        {"id":"worker","type":"script","scriptUri":"nested.js","width":36,"height":12,"inputs":{"selected":"badge"}}
    ]}).to_string()).unwrap()).unwrap();
    let target = project.resolve().unwrap().roots.remove(0);
    let namespace = json!({
        "script_scope":{"version":1,"script_uri":"nested.js","library":"base","inputs":{"hash":super::super::scope::value_hash(&json!({"selected":"badge"})),"library":"base"}},
        "script_bindings":{"version":1,"script_uri":"nested.js","components":{"badge":"badge"},"inputs_hash":super::super::scope::value_hash(&json!({"selected":"badge"})),"input_components":{"badge":"badge"}},
        "script_input_context":{"version":1,"values":{"selected":"badge"},"variables":{},"components":{"badge":"alternate"}}
    });
    for (variant, output, input) in [
        ("output", "base-badge", "copied-alternate"),
        ("input", "copied-badge", "base-badge"),
        ("both", "base-badge", "base-badge"),
        ("stale-output", "copied-badge", "copied-alternate"),
        ("stale-input", "copied-badge", "copied-badge"),
    ] {
        let carried_output = ["output", "both", "stale-output"].contains(&variant).then(|| json!({
            "version":1,"script_uri":if variant == "stale-output" {"old.js"} else {"nested.js"},
            "library":if variant == "stale-output" {"unknown"} else {"base"},"components":{"badge":"badge"}
        }));
        let carried_input = ["input", "both", "stale-input"]
            .contains(&variant)
            .then(|| {
                json!({
                    "version":1,"library":if variant == "stale-input" {"unknown"} else {"base"},
                    "values":{"selected":if variant == "stale-input" {"different"} else {"badge"}},
                    "variables":{},"components":{"badge":"badge"}
                })
            });
        let mut changes = json!({"scriptUri":"nested.js","inputs":{"selected":"badge"},
            "metadata":{"type":"lazypencil","lazypencil":{"script_override":{
                "version":1,"metadata":{"type":"test","lazypencil":namespace},"output":carried_output,"input":carried_input
            }}}
        });
        let carrier = super::super::script_context::override_value(changes.as_object().unwrap())
            .unwrap()
            .unwrap();
        let (actual_output, actual_input) = parent
            .compose_generated_scopes(
                &namespace,
                Some(&carrier),
                &mut changes,
                &target,
                &GeneratedState::default(),
                &mut 0,
            )
            .unwrap();
        let bound = Bound::from_values(
            parent.origin.clone(),
            false,
            parse(&actual_output.unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(bound.component("badge").unwrap(), output, "{variant}");
        assert_eq!(
            actual_input.unwrap()["components"]["badge"],
            input,
            "{variant}"
        );
    }
    let script = json!({"type":"script","scriptUri":"nested.js","metadata":{"type":"lazypencil","lazypencil":{"script_override":{"version":1}}}});
    assert_eq!(
        generated::selection(&parent, &script).err().unwrap().code,
        "INVALID_DOCUMENT"
    );
}

#[cfg(feature = "rendering")]
#[test]
fn combined_inherited_guards_require_known_lexical_input_ownership() {
    let input = json!({"selected":"badge"});
    let context = super::super::script_context::capture(
        Some(&input),
        json!({
            "version":1,"values":input,"variables":{},"components":{"badge":"badge"}
        }),
    )
    .unwrap();
    let project = Project::local(Document::parse(json!({"version":"2.21","children":[
        {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10},
        {"id":"worker","type":"script","scriptUri":"nested.js","width":10,"height":10,"inputs":input,
         "metadata":{"type":"test","lazypencil":{"script_input_context":context}}}
    ]}).to_string()).unwrap()).unwrap();
    let mut target = project.resolve().unwrap().roots.remove(1);
    let parent = custom_parent();
    let mut captured = parent.original().clone();
    captured.version = 3;
    captured.source_uri_hash = Some(hash(b"generator.js"));
    captured.input_epoch = Some("1".repeat(32));
    let parent = Bound::from_values(parent.origin.clone(), false, captured).unwrap();
    for invalid in ["short".into(), "G".repeat(32), "f".repeat(33)] {
        let mut value = serde_json::to_value(parent.original()).unwrap();
        value["input_epoch"] = json!(invalid);
        assert!(parse(&value).is_err());
    }
    let legacy = parent
        .inherited_output_with_hash("nested.js", None, None, &mut 0)
        .unwrap();
    assert!(legacy.get("input_epoch").is_none());
    assert!(parse(&legacy).is_ok());
    let metadata = json!({
        "script_scope":{"version":1,"script_uri":"nested.js","library":"base","inputs":{"hash":super::super::scope::value_hash(&input),"library":"base"}},
        "script_bindings":{"version":1,"script_uri":"nested.js","components":{"badge":"badge"}}
    });
    for (lexical, epoch, accepted) in [
        (None, None, false),
        (Some(false), Some("1".repeat(32)), false),
        (Some(true), None, false),
        (Some(true), Some("2".repeat(32)), false),
        (Some(true), Some("1".repeat(32)), true),
    ] {
        target.input_context.as_mut().unwrap().values.lexical_inputs = lexical;
        target.input_context.as_mut().unwrap().values.input_epoch = epoch;
        let mut changes = json!({"metadata":{"lazypencil":metadata}});
        let result = parent.compose_generated_scopes(
            &metadata,
            None,
            &mut changes,
            &target,
            &GeneratedState::default(),
            &mut 0,
        );
        if accepted {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().code, "UNSUPPORTED_FEATURE");
        }
    }
    let context = target.input_context.as_mut().unwrap();
    context.origin = parent.origin.clone();
    context.values.version = 3;
    context.values.lexical_inputs = Some(false);
    context.values.input_epoch = Some("2".repeat(32));
    context.values.generations.insert(
        "1".repeat(32),
        serde_json::from_value(json!({
            "hash":super::super::scope::value_hash(&input),"values":input,
            "variables":{},"components":{"badge":"copied-badge"}
        }))
        .unwrap(),
    );
    for active in [true, false] {
        let mut metadata = metadata.clone();
        if !active {
            metadata["script_scope"]["inputs"]["hash"] = json!("0".repeat(64));
        }
        let mut changes = json!({"metadata":{"lazypencil":metadata}});
        let (_, result) = parent
            .compose_generated_scopes(
                &metadata,
                None,
                &mut changes,
                &target,
                &GeneratedState::default(),
                &mut 0,
            )
            .unwrap();
        let result = result.unwrap();
        assert_eq!(result["version"], 3);
        assert_eq!(
            result["components"]["badge"],
            if active { "base-badge" } else { "copied-badge" }
        );
        assert_eq!(result["input_epoch"], "1".repeat(32));
        assert_eq!(result["generations"].as_object().unwrap().len(), 1);
    }
    target
        .input_context
        .as_mut()
        .unwrap()
        .values
        .generations
        .clear();
    let mut changes = json!({"metadata":{"lazypencil":metadata}});
    assert_eq!(
        parent
            .compose_generated_scopes(
                &metadata,
                None,
                &mut changes,
                &target,
                &GeneratedState::default(),
                &mut 0
            )
            .unwrap_err()
            .code,
        "UNSUPPORTED_FEATURE"
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
