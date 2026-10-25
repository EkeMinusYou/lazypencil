use super::*;

fn source(version: &str, inherited: bool) -> (TempDir, Store, Value) {
    let (temp, store) = fixture();
    for name in ["main.pen", "library/ui.lib.pen", "library/base.lib.pen"] {
        let path = temp.path().join(name);
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["version"] = json!(version);
        fixture::write(&path, document);
    }
    let mut edits = fixture::operations();
    if inherited {
        edits[0]["source"]["node_id"] = json!("card");
    }
    let response = fixture::apply(&store, "source-copy", edits);
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    (temp, store, response)
}

#[test]
fn imported_bound_detach_preserves_saved_default_and_live_variable_inputs() {
    for version in ["2.19", "2.21"] {
        for inherited in [false, true] {
            for explicit in [false, true] {
                let (temp, source, response) = source(version, inherited);
                let id = response["generated_ids"]["0"]["component_id"]
                    .as_str()
                    .unwrap();
                let target = fixture::linked(&source, id, &temp.path().join("linked"));
                let address = if inherited {
                    json!({"node_id":"use","instance_path":[response["generated_ids"]["0"]["id_map"]["ui:draw"]]})
                } else {
                    json!("use")
                };
                if explicit {
                    let inputs = json!({"selected":"$selection","unused":"$missing"});
                    let edits = json!([
                        {"op":"set_variable","name":"selection","definition":{"type":"string","value":"badge"}},
                        {"op":"update","target":address,"set":{"inputs":inputs}}
                    ]);
                    fixture::apply(&target, "explicit-variable-input", edits);
                }
                let before = png(&target, None);
                let first = if explicit { Color::MAGENTA } else { Color::RED };
                assert_eq!(pixel(&before, 5), first);
                let detached = fixture::apply(
                    &target,
                    "detach-bound",
                    json!([{"op":"detach_instance","target":address,"client_ref":"detached"}]),
                );
                let id = detached["bindings"]["detached"].as_str().unwrap();
                let document = target.read().unwrap();
                let node = document.node(id).unwrap();
                assert_eq!(node["metadata"]["keep"], json!([1, 2, 3]));
                assert_eq!(
                    node["metadata"]["lazypencil"]["script_bindings"]["library"],
                    "source"
                );
                let after = png(&target, None);
                assert_eq!(before, after);
                fixture::apply(
                    &source,
                    "live-source-token",
                    json!([{"op":"set_variable","name":"copy.tone","definition":{"type":"color","value":"#ffff00"}}]),
                );
                let edited = png(&target, None);
                for x in [25, 75, 85] {
                    assert_eq!(pixel(&edited, x), Color::YELLOW);
                }
                assert_eq!(pixel(&edited, 5), first);
                fixture::apply(
                    &target,
                    "default-after-detach",
                    json!([{"op":"update","target":address,"set":{"inputs":{"selected":null}}}]),
                );
                let reset = png(&target, None);
                assert_eq!(pixel(&reset, 5), Color::RED);
                record(
                    &format!("detach-{version}-{inherited}-{explicit}"),
                    &[
                        ("before", &before),
                        ("after", &after),
                        ("edited", &edited),
                        ("reset", &reset),
                    ],
                    json!({"inherited":inherited,"explicit_variable_input":explicit,"pixels_equal":true,"live_source_variables":true,"default_namespace":true,"metadata_preserved":true}),
                );
            }
        }
    }
}

#[test]
fn detached_bound_scopes_survive_another_import_and_independent_copy() {
    for version in ["2.19", "2.21"] {
        let (temp, source, response) = source(version, false);
        let source_id = response["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        let linked = fixture::linked(&source, source_id, &temp.path().join("linked"));
        let before = png(&linked, None);
        fixture::apply(
            &linked,
            "detach-first",
            json!([{"op":"detach_instance","target":"use"},{"op":"update","target":"use","set":{"reusable":true,"name":"Detached Generator"}}]),
        );
        let first = png(&linked, None);
        assert_eq!(before, first);
        let another = fixture::linked(&linked, "use", &temp.path().join("another"));
        fixture::apply(
            &another,
            "detach-again",
            json!([{"op":"detach_instance","target":"use"},{"op":"update","target":"use","set":{"reusable":true,"name":"Detached Twice"}}]),
        );
        assert_eq!(
            another.read().unwrap().node("use").unwrap()["metadata"]["lazypencil"]["script_bindings"]
                ["library"],
            "source:source"
        );
        let second = png(&another, None);
        assert_eq!(before, second);
        let (independent, id) =
            fixture::recopy(&another, "use", &temp.path().join("independent"), "last.");
        fs::remove_dir_all(temp.path().join("linked")).unwrap();
        fs::remove_dir_all(temp.path().join("another")).unwrap();
        fs::remove_file(source.path()).unwrap();
        fs::remove_dir_all(temp.path().join("assets")).unwrap();
        let copied = png(&independent, None);
        assert_eq!(before, copied);
        fixture::apply(
            &independent,
            "future-after-detach-copy",
            json!([{"op":"update","target":id,"set":{"inputs":{"alternate":true}}}]),
        );
        let future = png(&independent, None);
        for x in [5, 25, 45] {
            assert_eq!(pixel(&future, x), Color::CYAN);
        }
        record(
            &format!("detach-again-{version}"),
            &[
                ("before", &before),
                ("first", &first),
                ("second", &second),
                ("independent", &copied),
                ("future", &future),
            ],
            json!({"detached_twice":true,"sources_removed_after_independent_copy":true,"all_pixels_equal":true,"future_branch":true}),
        );
    }
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn detached_bound_shader_preserves_manifest_and_live_sampler_text_edits() {
    let (temp, source, response) = source("2.21", false);
    let source_id = response["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    fixture::apply(
        &source,
        "shader-definition",
        json!([{"op":"update","target":source_id,"set":{"inputs":{"shader":true}}}]),
    );
    let linked = fixture::linked(&source, source_id, &temp.path().join("linked"));
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
        );
    let before = png(&linked, Some(&mut runtime));
    fixture::apply(
        &linked,
        "detach-shader",
        json!([{"op":"detach_instance","target":"use"}]),
    );
    let after = png(&linked, Some(&mut runtime));
    assert_eq!(before, after);
    assert_eq!(pixel(&after, 45), Color::YELLOW);
    fixture::apply(
        &source,
        "edit-forwarded-texture",
        json!([{"op":"set_variable","name":"copy.texture","definition":{"type":"string","value":"consumer.png"}}]),
    );
    let changed = png(&linked, Some(&mut runtime));
    assert_eq!(pixel(&changed, 45), Color::GREEN);
    assert!(
        calculation(&linked)
            .1
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("consumer.png")))
    );
    record(
        "detach-shader",
        &[
            ("before", &before),
            ("after", &after),
            ("changed", &changed),
        ],
        json!({"pixels_equal":true,"live_sampler_text":true,"software_webgl":true}),
    );
}
