use super::*;
use lazypencil::{
    render::{self, RenderOptions},
    script::ScriptRuntime,
};

fn prepare(version: &str, case: Case, replacement: &str) -> (TempDir, Store, Value, String) {
    let (temp, store, copy, _) = setup(version, case, true);
    let file = temp.path().join("library.pen");
    let mut source: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    source["children"].as_array_mut().unwrap().extend([
        json!({"id":"runtime-badge","type":"frame","reusable":true,"width":10,"height":10,"children":[{"id":"runtime-badge-label","type":"text","content":"$replacement","fontSize":7}]}),
        json!({"id":"runtime-branch","type":"frame","reusable":true,"children":[]}),
    ]);
    let parent = if case == Case::RefToGroup {
        "sibling"
    } else {
        "nested"
    };
    let parent = format!("parent-use/{parent}");
    let instance = if replacement == "children" {
        matches!(case, Case::Reference | Case::RefToGroup)
    } else {
        replacement == "ref"
    };
    let prefix = if instance {
        format!("{parent}/")
    } else {
        "parent-use/".into()
    };
    let body = json!({"id":"live-wrap","type":"frame","layout":"none","width":120,"height":30,"fill":"#123456","metadata":{"type":"runtime","literal":"$replacement"},"children":[
        {"id":"live-text","type":"text","content":"$replacement","fontSize":8,"x":2,"y":2},
        {"id":"live-ref","type":"ref","ref":"runtime-badge","x":100,"descendants":{"runtime-badge-label":{"fontSize":7}}}
    ]});
    let change = match replacement {
        "children" => json!({"children":[body]}),
        "frame" => body,
        "ref" => {
            let mut body = body;
            body["type"] = json!("ref");
            body["ref"] = json!("runtime-branch");
            body
        }
        _ => unreachable!(),
    };
    let output = json!([{"id":"canvas","type":"frame","layout":"none","width":120,"height":60,"fill":"#123456","children":[
        {"id":"chosen","type":"ref","ref":"wrapper","descendants":{
            parent:change,
            format!("{prefix}live-text"):{"content":"Runtime replacement"},
            format!("{prefix}live-ref/runtime-badge-label"):{"content":"Nested replacement"}
        }}
    ]}]);
    let code = format!(
        "/** @schema 2.21 */\nreturn {};\n",
        serde_json::to_string(&output).unwrap()
    );
    fs::write(temp.path().join("replacement.js"), &code).unwrap();
    write(&file, source);
    (temp, store, copy, code)
}

fn capture(store: &Store, label: &str) -> Vec<u8> {
    let (project, snapshot) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    for expected in ["Runtime replacement", "Nested replacement"] {
        assert!(
            snapshot
                .items
                .iter()
                .any(|item| item.node.properties.get("content") == Some(&json!(expected))),
            "{label}: missing {expected}"
        );
    }
    let output = render::png(
        &project,
        &snapshot,
        &RenderOptions {
            region: Some(lazypencil::geometry::Rect {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 60.0,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
        let directory = std::path::PathBuf::from(directory).join("component-reuse/runtime");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{label}.png")), &output.png).unwrap();
        fs::write(directory.join(format!("{label}.json")), serde_json::to_vec_pretty(&json!({
            "source_commit":option_env!("GITHUB_SHA"), "png_sha256":lazypencil::document::hash(&output.png),
            "width":output.width,"height":output.height,"items":snapshot.items.iter().map(|item|&item.record).collect::<Vec<_>>()
        })).unwrap()).unwrap();
    }
    // The independent canvas remains below all component placements.
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&output.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        image.peek_pixels().unwrap().get_color((5, 50)),
        skia_safe::Color::from_rgb(0x12, 0x34, 0x56)
    );
    output.png
}

#[test]
fn runtime_children_reject_an_override_to_a_removed_body_without_changing_the_document() {
    let (temp, store, copy, code) = prepare("2.21", Case::Contained, "children");
    let code = code.replace("\"parent-use/live-text\"", "\"parent-use/authored-text\"");
    fs::write(temp.path().join("replacement.js"), &code).unwrap();
    apply(&store, copy, "removed-runtime-child");
    fs::remove_file(temp.path().join("library.pen")).unwrap();
    fs::remove_file(temp.path().join("replacement.js")).unwrap();
    let document = fs::read(store.path()).unwrap();
    let history = store.history(10, None).unwrap().len();
    let error = service::layout_snapshot_with_runtime(
        &store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .err()
    .expect("An override cannot revive a removed child");
    assert!(error.message.contains("replaced child"), "{error}");
    assert_eq!(fs::read(store.path()).unwrap(), document);
    assert_eq!(store.history(10, None).unwrap().len(), history);
}

#[test]
fn runtime_children_keep_original_addresses_across_reuse_source_removal_and_recopy() {
    let cases = [
        (Case::Contained, "children"),
        (Case::Frame, "children"),
        (Case::Reference, "children"),
        (Case::RefToGroup, "children"),
        (Case::GroupToRef, "children"),
        (Case::Contained, "frame"),
        (Case::Contained, "ref"),
    ];
    for version in ["2.19", "2.21"] {
        for (case, replacement) in cases {
            let (temp, store, copy, code) = prepare(version, case, replacement);
            let original = store.read().unwrap();
            let mut document = original.clone();
            document
                .insert_element(
                    "/children",
                    1,
                    &json!({"id":"original-probe","type":"ref","ref":"ui:replacement-generator"})
                        .to_string(),
                )
                .unwrap();
            fs::write(store.path(), document.text()).unwrap();
            capture(
                &store,
                &format!("{version}-{case:?}-{replacement}-original"),
            );
            fs::write(store.path(), original.text()).unwrap();
            let result = apply(&store, copy, "runtime-children-copy");
            let component = result["generated_ids"]["0"]["component_id"]
                .as_str()
                .unwrap();
            assert!(
                result["generated_ids"]["0"]["assets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|asset| asset["hash"] == lazypencil::document::hash(code.as_bytes()))
            );
            assert_eq!(
                fs::read(temp.path().join("replacement.js")).unwrap(),
                code.as_bytes()
            );
            fs::remove_file(temp.path().join("library.pen")).unwrap();
            fs::remove_file(temp.path().join("replacement.js")).unwrap();
            let first = capture(&store, &format!("{version}-{case:?}-{replacement}-copied"));
            let (second, _) = recopy(&temp, &store, component, version);
            assert_eq!(
                capture(
                    &second,
                    &format!("{version}-{case:?}-{replacement}-recopied")
                ),
                first
            );
        }
    }
}
