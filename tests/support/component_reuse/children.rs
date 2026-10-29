use super::*;

#[cfg(feature = "rendering")]
#[path = "runtime.rs"]
mod runtime;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Case {
    Contained,
    Frame,
    Reference,
    ReusedReplacement,
    RefToGroup,
    GroupToRef,
}

const CASES: [Case; 6] = [
    Case::Contained,
    Case::Frame,
    Case::Reference,
    Case::ReusedReplacement,
    Case::RefToGroup,
    Case::GroupToRef,
];

fn setup(version: &str, case: Case, generated: bool) -> (TempDir, Store, Value, String) {
    let (temp, store) = fixture(version);
    let library = temp.path().join("library.pen");
    let mut source: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    source["variables"]["replacement"] = json!({"type":"string","value":"Authored"});
    source["children"][1]["height"] = json!(60);
    let label = json!({"id":"authored-text","type":"text","content":"$replacement","fontSize":8,"x":2,"y":2});
    let children = json!([label,{"id":"authored-ref","type":"ref","ref":"badge","x":100,"y":1}]);
    let body = json!({"id":"authored-wrap","type":"frame","layout":"none","width":120,"height":30,"fill":"#123456","metadata":{"type":"authored","literal":"$replacement"},"children":children});
    let mut copy = operation();
    let (parent, path, change) = match case {
        Case::Contained => ("nested", "authored-text", json!({"children":[body]})),
        Case::Frame | Case::ReusedReplacement => {
            let mut replacement = body;
            if case == Case::ReusedReplacement {
                replacement["reusable"] = json!(true);
                copy["options"]["components"]["ui:authored-wrap"] =
                    json!({"action":"reuse","target":"existing"});
                copy["options"]["component_paths"]["ui:authored-wrap"] =
                    json!({"authored-text":"local-label","authored-ref":"local-badge"});
            }
            ("nested", "authored-text", replacement)
        }
        Case::Reference => {
            let mut replacement = body;
            replacement["type"] = json!("ref");
            replacement["ref"] = json!("branch");
            source["children"].as_array_mut().unwrap().push(json!({"id":"branch","type":"frame","reusable":true,"children":[{"id":"branch-label","type":"text","content":"Base"}]}));
            copy["options"]["components"]["ui:branch"] =
                json!({"action":"reuse","target":"existing"});
            ("nested", "nested/authored-text", replacement)
        }
        Case::RefToGroup | Case::GroupToRef => {
            copy["options"]["components"] =
                json!({"ui:parent":{"action":"reuse","target":"chosen-parent"}});
            let (parent, target, path) = if case == Case::RefToGroup {
                ("sibling", "plain-target", "sibling/authored-text")
            } else {
                ("nested", "target-ref", "authored-text")
            };
            copy["options"]["component_paths"] = json!({"ui:parent":{parent:target}});
            let chosen = if case == Case::RefToGroup {
                json!({"id":"plain-target","type":"frame","layout":"none","width":120,"height":30,"children":[{"id":"target-label","type":"text","content":"Target"}]})
            } else {
                json!({"id":"target-ref","type":"ref","ref":"existing"})
            };
            let target = json!({"id":"chosen-parent","name":"Chosen parent","type":"frame","reusable":true,"layout":"none","width":120,"height":60,"children":[chosen]});
            let mut document = store.read().unwrap();
            document
                .insert_element("/children/0/children", 1, &target.to_string())
                .unwrap();
            fs::write(store.path(), document.text()).unwrap();
            (parent, path, json!({"children":[body]}))
        }
    };
    // Child-first authoring is valid when the source's instance depth orders
    // the parent first. The copied form may collapse those depths to a tie.
    source["children"][1]["children"][0]["descendants"] = if path.contains('/') {
        json!({path:{"content":"After replacement"},parent:change})
    } else {
        json!({parent:change,path:{"content":"After replacement"}})
    };
    let original_path = format!("parent-use/{path}");
    let code = r##"/** @schema 2.21 */
return [{id:"chosen",type:"ref",ref:"wrapper",descendants:{[__PATH__]:{content:"Generated replacement"}}}];"##
        .replace("__PATH__", &serde_json::to_string(&original_path).unwrap());
    fs::write(temp.path().join("replacement.js"), &code).unwrap();
    source["children"].as_array_mut().unwrap().push(json!({"id":"replacement-generator","name":"Replacement generator","type":"script","reusable":true,"scriptUri":"replacement.js","width":120,"height":60}));
    write(&library, source);
    let mut document = store.read().unwrap();
    let pointer = document.location("existing").unwrap().pointer.clone();
    document
        .set_property(&pointer, "enabled", &json!(true))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    if generated {
        copy["source"]["node_id"] = json!("replacement-generator");
    }
    (temp, store, copy, code)
}

fn apply(store: &Store, operation: Value, name: &str) -> Value {
    service::apply_request(
        store,
        &request(
            store,
            name,
            json!([operation,{"op":"remove_import","alias":"ui"}]),
            false,
        ),
    )
    .unwrap()
}

fn recopy(temp: &TempDir, source: &Store, component: &str, version: &str) -> (Store, Value) {
    fs::create_dir(temp.path().join("second")).unwrap();
    write(
        &temp.path().join("second/main.pen"),
        json!({"version":version,"imports":{"first":"../main.pen"},"children":[]}),
    );
    let store = Store::open(
        temp.path().join("second/main.pen"),
        Some(temp.path().join("second-data")),
    )
    .unwrap();
    let result = service::apply_request(&store,&request(&store,"children-recopy",json!([
        {"op":"import_component","source":{"library_alias":"first","node_id":component},"mode":"copy","options":{"variable_prefix":"again.","component_prefix":"Again "}},
        {"op":"remove_import","alias":"first"}
    ]),false)).unwrap();
    fs::remove_file(source.path()).unwrap();
    if temp.path().join("assets").exists() {
        fs::remove_dir_all(temp.path().join("assets")).unwrap();
    }
    (store, result)
}

#[test]
fn replacement_children_keep_authorship_and_override_order_across_reuse_and_recopy() {
    for version in ["2.19", "2.21"] {
        for case in CASES {
            let (temp, store, copy, _) = setup(version, case, false);
            let before = fs::read(store.path()).unwrap();
            service::apply_request(
                &store,
                &request(&store, "children-preview", json!([copy.clone()]), true),
            )
            .unwrap();
            assert_eq!(fs::read(store.path()).unwrap(), before);
            let result = apply(&store, copy, "children-copy");
            let report = &result["generated_ids"]["0"];
            let id = report["id"].as_str().unwrap();
            let component = report["component_id"].as_str().unwrap();
            let label = if case == Case::ReusedReplacement {
                "local-label"
            } else {
                report["id_map"]["ui:authored-text"].as_str().unwrap()
            };
            fs::remove_file(temp.path().join("library.pen")).unwrap();
            fs::remove_file(temp.path().join("replacement.js")).unwrap();
            let scene = Project::load(&store).unwrap().resolve().unwrap();
            let root = scene.roots.iter().find(|node| node.id() == id).unwrap();
            assert_eq!(
                find(root, label).properties["content"],
                "After replacement",
                "{version} {case:?}"
            );
            if case != Case::ReusedReplacement {
                let text = report["id_map"]["ui:authored-text"].as_str().unwrap();
                assert_eq!(
                    store.read().unwrap().node(text).unwrap()["content"],
                    "$from.replacement"
                );
                assert_eq!(
                    store
                        .read()
                        .unwrap()
                        .node(report["id_map"]["ui:authored-ref"].as_str().unwrap())
                        .unwrap()["ref"],
                    report["id_map"]["ui:badge"]
                );
            }
            #[cfg(feature = "rendering")]
            let original = render(&store, version, case, "static-before", "After replacement");
            let (second, again) = recopy(&temp, &store, component, version);
            let second_id = again["generated_ids"]["0"]["id"].as_str().unwrap();
            let label_id = again["generated_ids"]["0"]["id_map"][format!("first:{label}")]
                .as_str()
                .unwrap();
            let scene = Project::load(&second).unwrap().resolve().unwrap();
            let root = scene
                .roots
                .iter()
                .find(|node| node.id() == second_id)
                .unwrap();
            assert_eq!(
                find(root, label_id).properties["content"],
                "After replacement"
            );
            #[cfg(feature = "rendering")]
            assert_eq!(
                render(
                    &second,
                    version,
                    case,
                    "static-recopied",
                    "After replacement"
                ),
                original
            );
            service::apply_request(
                &second,
                &request(
                    &second,
                    "children-edit",
                    json!([
                        {"op":"update","target":label_id,"set":{"fontSize":9}}
                    ]),
                    false,
                ),
            )
            .unwrap();
            let scene = Project::load(&second).unwrap().resolve().unwrap();
            let root = scene
                .roots
                .iter()
                .find(|node| node.id() == second_id)
                .unwrap();
            assert_eq!(find(root, label_id).properties["fontSize"], 9);
        }
    }
}

#[cfg(feature = "rendering")]
fn render(store: &Store, version: &str, case: Case, label: &str, expected: &str) -> Vec<u8> {
    use lazypencil::{
        render::{self, RenderOptions},
        script::ScriptRuntime,
    };
    let (project, snapshot) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    assert!(
        snapshot.items.iter().any(|item| item
            .node
            .properties
            .get("content")
            .and_then(Value::as_str)
            == Some(expected)),
        "{case:?}: missing {expected}"
    );
    let output = render::png(
        &project,
        &snapshot,
        &RenderOptions {
            region: Some(lazypencil::geometry::Rect {
                x: 0.,
                y: 0.,
                width: 120.,
                height: 60.,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS") {
        let directory = std::path::PathBuf::from(directory).join("component-reuse/children");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("{version}-{case:?}-{label}.png")),
            &output.png,
        )
        .unwrap();
        write(
            &directory.join(format!("{version}-{case:?}-{label}.json")),
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"png_sha256":lazypencil::document::hash(&output.png),"width":output.width,"height":output.height,"items":snapshot.items.iter().map(|item|&item.record).collect::<Vec<_>>() }),
        );
    }
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&output.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let expected = if case == Case::ReusedReplacement {
        skia_safe::Color::GREEN
    } else {
        skia_safe::Color::from_rgb(0x12, 0x34, 0x56)
    };
    assert_eq!(
        image.peek_pixels().unwrap().get_color((15, 18)),
        expected,
        "{case:?} {label}"
    );
    output.png
}

#[cfg(feature = "rendering")]
#[test]
fn generated_overrides_address_authored_children_after_substitution_and_independent_recopy() {
    for version in ["2.19", "2.21"] {
        for case in CASES {
            let (temp, store, copy, code) = setup(version, case, true);
            let result = apply(&store, copy, "generated-children-copy");
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
            let first = render(
                &store,
                version,
                case,
                "generated-before",
                "Generated replacement",
            );
            let (second, _) = recopy(&temp, &store, component, version);
            assert_eq!(
                render(
                    &second,
                    version,
                    case,
                    "generated-recopied",
                    "Generated replacement"
                ),
                first
            );
        }
    }
}

#[test]
fn colliding_stored_override_paths_are_rejected_without_losing_either_edit() {
    let (temp, store) = fixture("2.21");
    let path = temp.path().join("library.pen");
    let mut source: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    source["children"][1]["children"][0]["descendants"]["badge"] = json!({"opacity":0.5});
    write(&path, source);
    let mut copy = operation();
    copy["options"]["component_paths"]["ui:nested"]["badge"] = json!("local-label");
    let before = fs::read(store.path()).unwrap();
    let error = service::apply_request(
        &store,
        &request(&store, "colliding-paths", json!([copy]), false),
    )
    .unwrap_err();
    assert!(error.message.contains("collide"), "{error}");
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert!(store.history(10, None).unwrap().is_empty());
    assert!(!temp.path().join("assets").exists());
}
