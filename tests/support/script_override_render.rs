use super::*;
#[path = "script_override_fixture.rs"]
mod fixture;

fn rendered(store: &Store) -> Vec<u8> {
    let (project, layout) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    assert_eq!(layout.script_executions.len(), 1);
    render::png(
        &project,
        &layout,
        &RenderOptions {
            region: Some(Rect {
                x: 0.,
                y: 0.,
                width: 36.,
                height: 12.,
            }),
            ..Default::default()
        },
    )
    .unwrap()
    .png
}

fn stored_override(store: &Store) -> Value {
    let document = store.read().unwrap();
    let mut targets = Vec::new();
    for id in document.locations().keys() {
        let node = document.node(id).unwrap();
        if let Some(overrides) = node.get("descendants").and_then(Value::as_object) {
            for (path, changes) in overrides {
                if changes
                    .pointer("/metadata/lazypencil/script_override")
                    .is_some()
                    || changes
                        .pointer("/metadata/lazypencil/script_input_context")
                        .is_some()
                {
                    targets.push(
                        json!({"node_id":id,"instance_path":path.split('/').collect::<Vec<_>>()}),
                    );
                }
            }
        }
    }
    assert_eq!(targets.len(), 1);
    targets.pop().unwrap()
}

#[test]
fn inherited_overrides_preserve_live_namespaces_metadata_and_pixels_after_two_independent_copies() {
    let mut reports = Vec::new();
    for version in ["2.19", "2.21"] {
        for mode in fixture::MODES {
            let (temp, store) = fixture::fixture(version, mode);
            let before = rendered(&store);
            let selected = if ["inputs", "combined", "literal", "nullable"].contains(mode) {
                Color::GREEN
            } else {
                Color::RED
            };
            let tone = if ["code", "combined", "nullable"].contains(mode) {
                Color::GREEN
            } else {
                Color::RED
            };
            assert_eq!(pixel(&before, 5), selected, "{version}/{mode}/reference");
            assert_eq!(pixel(&before, 17), tone, "{version}/{mode}/output");
            let result = fixture::apply(&store, "copy", fixture::operations());
            let component = result["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            fs::remove_dir_all(temp.path().join("library")).unwrap();
            let copied = rendered(&store);
            assert_eq!(copied, before, "{version}/{mode}/copy");
            let (second, second_id) =
                fixture::recopy(&store, component, &temp.path().join("second"));
            fs::remove_file(store.path()).unwrap();
            fs::remove_dir_all(temp.path().join("assets")).unwrap();
            let recopied = rendered(&second);
            assert_eq!(recopied, before, "{version}/{mode}/recopy");

            // Unoverridden user metadata continues to follow the copied base definition.
            let document = second.read().unwrap();
            let base = document
                .locations()
                .keys()
                .find(|id| {
                    document.node(id).unwrap()["name"]
                        .as_str()
                        .is_some_and(|name| name.ends_with("Base generator"))
                })
                .unwrap()
                .clone();
            let mut metadata = document.node(&base).unwrap()["metadata"].clone();
            metadata["keep"] = json!("future");
            fixture::apply(
                &second,
                "future-metadata",
                json!([{"op":"update","target":base,"set":{"metadata":metadata}}]),
            );
            assert_eq!(
                fixture::active(&second, &second_id).properties["metadata"]["keep"],
                if ["metadata", "combined"].contains(mode) {
                    "override"
                } else {
                    "future"
                }
            );
            let mut inherited_inputs =
                second.read().unwrap().node(&base).unwrap()["inputs"].clone();
            inherited_inputs["amount"] = json!(4);
            fixture::apply(
                &second,
                "future-base-input",
                json!([{"op":"update","target":base,"set":{"inputs":inherited_inputs}}]),
            );
            if ["code", "metadata"].contains(mode) {
                assert_eq!(
                    fixture::active(&second, &second_id).properties["inputs"]["amount"],
                    4
                );
            } else {
                assert_ne!(
                    fixture::active(&second, &second_id).properties["inputs"]["amount"],
                    4
                );
            }

            let target = stored_override(&second);
            let current = fixture::active(&second, &second_id);
            let inputs = current.properties["inputs"].clone();
            fixture::apply(
                &second,
                "same-input",
                json!([{"op":"update","target":target,"set":{"inputs":inputs}}]),
            );
            let explicit = rendered(&second);
            assert_eq!(
                pixel(&explicit, 5),
                if ["literal", "nullable"].contains(mode) {
                    Color::GREEN
                } else {
                    Color::MAGENTA
                },
                "{version}/{mode}/explicit input"
            );
            assert_eq!(pixel(&explicit, 17), tone);

            // Replacing code and inputs together must revoke both private contexts,
            // including an explicit same-JSON replacement stored on the inner ref.
            fixture::apply(
                &second,
                "same-code-and-input",
                json!([{"op":"update","target":target,"set":{"inputs":inputs,"scriptUri":current.properties["scriptUri"]}}]),
            );
            let caller = rendered(&second);
            assert_eq!(
                pixel(&caller, 17),
                Color::MAGENTA,
                "{version}/{mode}/explicit code"
            );
            assert_eq!(
                pixel(&caller, 5),
                if *mode == "literal" {
                    Color::GREEN
                } else {
                    Color::MAGENTA
                }
            );

            let prefix = format!("codec-script-overrides-{version}-{mode}");
            let names = ["before", "copied", "recopied", "explicit"]
                .map(|stage| format!("{prefix}-{stage}.png"));
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let directory = std::path::PathBuf::from(directory);
                fs::create_dir_all(&directory).unwrap();
                for (name, png) in names.iter().zip([&before, &copied, &recopied, &explicit]) {
                    fs::write(directory.join(name), png).unwrap();
                }
            }
            reports.push(json!({"document_version":version,"mode":mode,"before":names[0],"copied":names[1],"recopied":names[2],"explicit":names[3],"width":36,"height":12,"png_hash":hash(&before),"explicit_png_hash":hash(&explicit)}));
        }
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(
            std::path::PathBuf::from(directory).join("codec-script-overrides.json"),
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":reports}).to_string(),
        )
        .unwrap();
    }
}

#[test]
fn copied_input_context_survives_nested_detach_and_another_independent_copy() {
    for mode in ["inputs", "code", "metadata"] {
        let (temp, store) = fixture::fixture("2.21", mode);
        let result = fixture::apply(&store, "copy", fixture::operations());
        let component = result["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        let before = rendered(&store);
        let target = fixture::active(&store, component).address;
        fixture::apply(
            &store,
            &format!("detach-{mode}"),
            json!([{"op":"detach_instance","target":target}]),
        );
        assert_eq!(rendered(&store), before);
        let (copied, _) = fixture::recopy(&store, component, &temp.path().join("detached"));
        fs::remove_file(store.path()).unwrap();
        fs::remove_dir_all(temp.path().join("assets")).unwrap();
        assert_eq!(rendered(&copied), before, "{mode}");
    }
}
