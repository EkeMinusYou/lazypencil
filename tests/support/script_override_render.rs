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
            let target = fixture::stored_override(&second);
            let absent = match *mode {
                "inputs" | "literal" => vec!["scriptUri"],
                "code" => vec!["inputs"],
                "metadata" => vec!["inputs", "scriptUri"],
                _ => vec![],
            };
            if !absent.is_empty() {
                fixture::apply(
                    &second,
                    "absent-reset",
                    json!([
                        {"op":"update","target":target,"unset":absent},
                        {"op":"reset_override","target":target,"properties":absent}
                    ]),
                );
                assert_eq!(rendered(&second), recopied, "{version}/{mode}/absent reset");
            }
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

            fixture::apply(
                &second,
                "reset-contexts",
                json!([{"op":"reset_override","target":target,"properties":["inputs","scriptUri"]}]),
            );
            let reset = rendered(&second);
            assert_eq!(pixel(&reset, 5), Color::RED, "{version}/{mode}/reset input");
            assert_eq!(pixel(&reset, 17), Color::RED, "{version}/{mode}/reset code");
            let mut base_inputs = second.read().unwrap().node(&base).unwrap()["inputs"].clone();
            base_inputs["amount"] = json!(4);
            fixture::apply(
                &second,
                "future-base-input",
                json!([{"op":"update","target":base,"set":{"inputs":base_inputs}}]),
            );
            assert_eq!(
                fixture::active(&second, &second_id).properties["inputs"]["amount"],
                4
            );
            let inherited_input = rendered(&second);
            assert_eq!(
                pixel(&inherited_input, 5),
                Color::MAGENTA,
                "{version}/{mode}/new base input"
            );
            assert_eq!(pixel(&inherited_input, 17), Color::RED);
            let base_uri = second.read().unwrap().node(&base).unwrap()["scriptUri"].clone();
            fixture::apply(
                &second,
                "future-base-code",
                json!([{"op":"update","target":base,"set":{"scriptUri":base_uri}}]),
            );
            let inherited_code = rendered(&second);
            assert_eq!(pixel(&inherited_code, 5), Color::MAGENTA);
            assert_eq!(
                pixel(&inherited_code, 17),
                Color::MAGENTA,
                "{version}/{mode}/new base code"
            );

            let prefix = format!("codec-script-overrides-{version}-{mode}");
            let names = [
                "before",
                "copied",
                "recopied",
                "explicit",
                "reset",
                "inherited_input",
                "inherited_code",
            ]
            .map(|stage| format!("{prefix}-{stage}.png"));
            if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
                let directory = std::path::PathBuf::from(directory);
                fs::create_dir_all(&directory).unwrap();
                for (name, png) in names.iter().zip([
                    &before,
                    &copied,
                    &recopied,
                    &explicit,
                    &reset,
                    &inherited_input,
                    &inherited_code,
                ]) {
                    fs::write(directory.join(name), png).unwrap();
                }
            }
            reports.push(json!({"document_version":version,"mode":mode,"before":names[0],"copied":names[1],"recopied":names[2],"explicit":names[3],"reset":names[4],"inherited_input":names[5],"inherited_code":names[6],"width":36,"height":12,"png_hash":hash(&before),"explicit_png_hash":hash(&explicit),"reset_png_hash":hash(&reset),"inherited_input_png_hash":hash(&inherited_input),"inherited_code_png_hash":hash(&inherited_code)}));
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
