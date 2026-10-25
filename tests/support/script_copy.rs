use super::*;
use lazypencil::{document::hash, dynamic::DynamicRuntime};
#[path = "script_copy_detach.rs"]
mod detach;
#[path = "script_copy_fixture.rs"]
pub(super) mod fixture;

fn image(path: &Path, color: Color) {
    let mut surface = skia_safe::surfaces::raster_n32_premul((2, 2)).unwrap();
    surface.canvas().clear(color);
    fs::write(
        path,
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
}

fn fixture() -> (TempDir, Store) {
    let (temp, store) = fixture::fixture();
    image(&temp.path().join("library/pixel.png"), Color::YELLOW);
    image(&temp.path().join("library/future.png"), Color::CYAN);
    image(&temp.path().join("pixel.png"), Color::MAGENTA);
    image(&temp.path().join("consumer.png"), Color::GREEN);
    (temp, store)
}

fn calculation(store: &Store) -> (Project, LayoutSnapshot) {
    service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap()
}

fn png(store: &Store, runtime: Option<&mut DynamicRuntime>) -> Vec<u8> {
    let (project, layout) = calculation(store);
    let options = RenderOptions {
        region: Some(Rect {
            x: 0.,
            y: 0.,
            width: 110.,
            height: 36.,
        }),
        ..Default::default()
    };
    if let Some(runtime) = runtime {
        render::png_with_runtime(&project, &layout, &options, runtime, None)
            .unwrap()
            .png
    } else {
        render::png(&project, &layout, &options).unwrap().png
    }
}

fn pixel(png: &[u8], x: i32) -> Color {
    Image::from_encoded(Data::new_copy(png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
        .peek_pixels()
        .unwrap()
        .get_color((x, 5))
}

fn copied(store: &Store) -> (Value, String) {
    let response = fixture::apply(store, "independent-script-copy", fixture::operations());
    let id = response["bindings"]["copied"].as_str().unwrap().to_owned();
    (response, id)
}

fn record(name: &str, images: &[(&str, &[u8])], extra: Value) {
    if let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
        let directory = std::path::PathBuf::from(root).join("script-copy");
        fs::create_dir_all(&directory).unwrap();
        let mut hashes = serde_json::Map::new();
        for (label, png) in images {
            fs::write(directory.join(format!("{name}-{label}.png")), png).unwrap();
            hashes.insert((*label).into(), json!(hash(png)));
        }
        fs::write(directory.join(format!("{name}-report.json")),json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"code_hash":hash(fixture::CODE.as_bytes()),"png_hashes":hashes,"checks":extra}).to_string()).unwrap();
    }
}

#[test]
fn independent_copy_keeps_namespace_nested_workers_and_live_choices_after_source_removal() {
    for version in ["2.19", "2.21"] {
        let (temp, store) = fixture();
        for name in ["main.pen", "library/ui.lib.pen", "library/base.lib.pen"] {
            let path = temp.path().join(name);
            let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            document["version"] = json!(version);
            fixture::write(&path, document);
        }
        let before = png(&store, None);
        let (response, id) = copied(&store);
        let after = png(&store, None);
        assert_eq!(before, after);
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        let independent = png(&store, None);
        assert_eq!(before, independent);
        for (x, color) in [
            (5, Color::RED),
            (25, Color::RED),
            (45, Color::YELLOW),
            (65, Color::BLUE),
            (75, Color::RED),
            (85, Color::BLUE),
        ] {
            assert_eq!(pixel(&independent, x), color, "initial x={x}");
        }
        let document = store.read().unwrap();
        let generator = response["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        let uri = document.node(generator).unwrap()["scriptUri"]
            .as_str()
            .unwrap();
        assert_eq!(
            fs::read(temp.path().join(uri)).unwrap(),
            fixture::CODE.as_bytes()
        );
        assert!(
            Project::load(&store)
                .unwrap()
                .dependencies()
                .iter()
                .all(|entry| !entry
                    .path
                    .starts_with(store.path().parent().unwrap().join("library")))
        );
        fixture::apply(
            &store,
            "future-choice",
            json!([{"op":"update","target":id,"set":{"inputs":{"alternate":true}}}]),
        );
        let future = png(&store, None);
        for x in [5, 25, 45] {
            assert_eq!(pixel(&future, x), Color::CYAN);
        }
        fixture::apply(
            &store,
            "consumer-choice",
            json!([{"op":"update","target":id,"set":{"inputs":{"selected":"badge"}}}]),
        );
        let consumer = png(&store, None);
        assert_eq!(pixel(&consumer, 5), Color::GREEN);
        fixture::apply(
            &store,
            "edit-copied-token",
            json!([{"op":"set_variable","name":"copy.tone","definition":{"type":"color","value":"#ffff00"}}]),
        );
        let changed = png(&store, None);
        for x in [25, 75, 85] {
            assert_eq!(pixel(&changed, x), Color::YELLOW);
        }
        record(
            &format!("namespace-{version}"),
            &[
                ("before", &before),
                ("independent", &independent),
                ("future", &future),
                ("consumer", &consumer),
                ("changed", &changed),
            ],
            json!({"original_removed":true,"initial_pixels_equal":true,"future_namespace_choice":true,"explicit_consumer_choice":true,"live_variable_nested_and_theme":true,"source_code_unchanged":true,"copied_id":id}),
        );
    }
}

#[test]
fn repeated_copy_keeps_original_and_saved_input_namespaces_after_each_source_is_removed() {
    for version in ["2.19", "2.21"] {
        let (temp, store) = fixture();
        for name in ["main.pen", "library/ui.lib.pen", "library/base.lib.pen"] {
            let path = temp.path().join(name);
            let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            document["version"] = json!(version);
            fixture::write(&path, document);
        }
        let (response, id) = copied(&store);
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        fixture::apply(
            &store,
            "saved-consumer-input",
            json!([{"op":"update","target":id,"set":{"inputs":{"selected":"badge"}}}]),
        );
        let before = png(&store, None);
        assert_eq!(pixel(&before, 5), Color::GREEN);
        let source_id = response["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap();
        // Copy the instance's current input into its reusable definition.
        fixture::apply(
            &store,
            "saved-definition-input",
            json!([{"op":"update","target":source_id,"set":{"inputs":{"selected":"badge"}}}]),
        );
        let (second, second_instance) =
            fixture::recopy(&store, source_id, &temp.path().join("second"), "second.");
        fs::remove_file(store.path()).unwrap();
        fs::remove_dir_all(temp.path().join("assets")).unwrap();
        let after = png(&second, None);
        assert_eq!(before, after);
        let second_id = second.read().unwrap().node(&second_instance).unwrap()["ref"]
            .as_str()
            .unwrap()
            .to_owned();
        let (third, third_instance) =
            fixture::recopy(&second, &second_id, &temp.path().join("third"), "third.");
        fs::remove_dir_all(temp.path().join("second")).unwrap();
        let independent = png(&third, None);
        assert_eq!(before, independent);
        fixture::apply(
            &third,
            "uncalled-branch-after-third-copy",
            json!([{"op":"update","target":third_instance,"set":{"inputs":{"alternate":true}}}]),
        );
        let future = png(&third, None);
        for x in [5, 25, 45] {
            assert_eq!(pixel(&future, x), Color::CYAN);
        }
        fixture::apply(
            &third,
            "default-after-third-copy",
            json!([{"op":"update","target":third_instance,"set":{"inputs":{"selected":null}}}]),
        );
        let default = png(&third, None);
        assert_eq!(pixel(&default, 5), Color::RED);
        fixture::apply(
            &third,
            "edited-token-after-third-copy",
            json!([{"op":"set_variable","name":"third.second.copy.tone","definition":{"type":"color","value":"#ffff00"}}]),
        );
        let changed = png(&third, None);
        for x in [25, 75, 85] {
            assert_eq!(pixel(&changed, x), Color::YELLOW);
        }
        record(
            &format!("recopy-{version}"),
            &[
                ("before", &before),
                ("second", &after),
                ("independent", &independent),
                ("future", &future),
                ("default", &default),
                ("changed", &changed),
            ],
            json!({"copies":3,"sources_removed":true,"saved_consumer_input":true,"default_body_namespace":true,"unexecuted_branch":true,"live_nested_theme_variable":true}),
        );
    }
}

#[test]
fn copy_rejects_unmanifested_generated_paths_without_using_consumer_files() {
    let (temp, store) = fixture();
    let (_, id) = copied(&store);
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    fixture::apply(
        &store,
        "unmanifested",
        json!([{"op":"update","target":id,"set":{"inputs":{"bad":true}}}]),
    );
    let before = fs::read(store.path()).unwrap();
    let history = store.history(100, None).unwrap().len();
    let error = service::layout_snapshot_with_runtime(
        &store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "ASSET_NOT_FOUND");
    assert!(error.message.contains("script_assets"));
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert_eq!(store.history(100, None).unwrap().len(), history);
    fixture::apply(
        &store,
        "recover-after-manifest-error",
        json!([{"op":"update","target":id,"set":{"inputs":{"bad":false}}}]),
    );
    let recovered = png(&store, None);
    assert_eq!(pixel(&recovered, 45), Color::YELLOW);
    record(
        "guard",
        &[("recovered", &recovered)],
        json!({"unmanifested_consumer_file_rejected":true,"failed_render_preserved_document_and_history":true,"recovered":true}),
    );
}

#[test]
#[ignore = "Requires sandboxed software WebGL Chromium"]
fn copied_script_shader_keeps_string_sampler_coupling_and_consumer_edits() {
    let (temp, store) = fixture();
    let (_, id) = copied(&store);
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    fixture::apply(
        &store,
        "copied-shader",
        json!([{"op":"update","target":id,"set":{"inputs":{"shader":true}}}]),
    );
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
        );
    let initial = png(&store, Some(&mut runtime));
    assert_eq!(pixel(&initial, 45), Color::YELLOW);
    let (_, layout) = calculation(&store);
    assert!(
        layout
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("pixel.png")))
    );
    fixture::apply(
        &store,
        "consumer-sampler-edit",
        json!([{"op":"set_variable","name":"copy.texture","definition":{"type":"string","value":"consumer.png"}}]),
    );
    let changed = png(&store, Some(&mut runtime));
    assert_eq!(pixel(&changed, 45), Color::GREEN);
    let (_, layout) = calculation(&store);
    assert!(
        layout
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("consumer.png")))
    );
    record(
        "shader",
        &[("initial", &initial), ("changed", &changed)],
        json!({"original_removed":true,"copied_sampler_origin":true,"consumer_image_and_text_edit_coupled":true,"software_webgl":true}),
    );
    let source_id = store.read().unwrap().node(&id).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture::apply(
        &store,
        "shader-definition",
        json!([{"op":"update","target":source_id,"set":{"inputs":{"shader":true}}}]),
    );
    let (second, _) = fixture::recopy(
        &store,
        &source_id,
        &temp.path().join("shader-copy"),
        "shader.",
    );
    fs::remove_file(store.path()).unwrap();
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    fs::remove_file(temp.path().join("consumer.png")).unwrap();
    let independent = png(&second, Some(&mut runtime));
    assert_eq!(changed, independent);
    image(
        &second.path().parent().unwrap().join("fresh.png"),
        Color::BLUE,
    );
    fixture::apply(
        &second,
        "recopied-sampler-edit",
        json!([{"op":"set_variable","name":"shader.copy.texture","definition":{"type":"string","value":"fresh.png"}}]),
    );
    let edited = png(&second, Some(&mut runtime));
    assert_eq!(pixel(&edited, 45), Color::BLUE);
    let (_, layout) = calculation(&second);
    assert!(
        layout
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("fresh.png")))
    );
    record(
        "recopy-shader",
        &[("independent", &independent), ("edited", &edited)],
        json!({"source_removed":true,"pixels_equal":true,"live_sampler_and_text":true,"software_webgl":true}),
    );
}
