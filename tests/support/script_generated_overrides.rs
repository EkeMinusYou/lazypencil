use super::*;
use crate::script_copy::fixture;

const REPLACEMENT: &str = r##"/**
 * @schema 2.21
 * @input selected: ref = "badge"
 */
if ('stale' in pencil.input) throw new Error('Stale control leaked');
return [{type:'ref',ref:pencil.input.selected},
 {type:'rectangle',x:12,width:10,height:10,fill:'$tone'},
 {type:'ref',x:24,ref:'badge'}];
"##;

fn setup(version: &str, style: &str, mode: &str, count: usize) -> (TempDir, Store, String) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(
        temp.path().join("library/base.js"),
        "/* @schema 2.21 */ return [{type:'rectangle',width:36,height:12,fill:'#0000ff'}];",
    )
    .unwrap();
    fs::write(temp.path().join("library/override.js"), REPLACEMENT).unwrap();
    // A tempting caller-side filename must never replace an unmanifested source.
    fs::write(
        temp.path().join("override.js"),
        "/* @schema 2.21 */ return [{type:'rectangle',width:36,height:12,fill:'#ff00ff'}];",
    )
    .unwrap();
    let mut changes = json!({"scriptUri":"override.js"});
    match mode {
        "literal" => changes["inputs"] = json!({"selected":"badge","stale":"$missing"}),
        "variable" | "themed-variable" => {
            changes["inputs"] = json!({"selected":"$selection","stale":"$missing"})
        }
        "bad-variable" => changes["inputs"] = json!({"selected":"$missing"}),
        "null" => changes["inputs"] = json!({"selected":null,"stale":"$missing"}),
        "metadata" => {
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone"})
        }
        "explicit-context" => {
            changes["metadata"] = json!({"type":"test","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"base"}}})
        }
        "inherit" => {}
        _ => panic!("Unknown generated override case"),
    }
    let direct = |mut changes: Value| {
        changes["type"] = json!("ref");
        changes["ref"] = json!("base:worker");
        changes
    };
    let template = match style {
        "direct" => direct(changes),
        "descendant" => json!({"type":"ref","ref":"base:card","descendants":{"draw":changes}}),
        "replacement" => {
            json!({"type":"ref","ref":"base:card","descendants":{"draw":direct(changes)}})
        }
        "children" => json!({"type":"ref","ref":"base:card","children":[direct(changes)]}),
        _ => panic!("Unknown generated override address"),
    };
    let code = format!(
        "/* @schema 2.21 */\nconst template={template};return Array.from({{length:{count}}},(_,i)=>({{...template,id:'child-'+i}}));\n"
    );
    fs::write(temp.path().join("library/draw.js"), &code).unwrap();
    write(
        &temp.path().join("library/base.pen"),
        json!({"version":version,
        "variables":{"selection":{"type":"string","value":"badge"},"tone":{"type":"color","value":"#ff0000"}},
        "children":[
            {"id":"badge","name":"Base badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"},
            {"id":"worker","name":"Base worker","type":"script","reusable":true,"width":36,"height":12,"scriptUri":"base.js","inputs":{"selected":"$selection","stale":"$missing"},"metadata":{"type":"test","keep":"base"}},
            {"id":"card","type":"frame","reusable":true,"layout":"none","width":36,"height":12,"children":[{"id":"draw","type":"ref","ref":"worker"}]}
        ]}),
    );
    write(
        &temp.path().join("library/ui.pen"),
        json!({"version":version,"imports":{"base":"base.pen"},
        "variables":{"selection":{"type":"string","value":"badge"},"tone":{"type":"color","value":"#00ff00"}},
        "children":[
            {"id":"badge","name":"UI badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ff00"},
            {"id":"generator","name":"Generator","type":"script","reusable":true,"scriptUri":"draw.js","width":36,"height":12}
        ]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":version,"imports":{"ui":"library/ui.pen"},
        "variables":{"tone":{"type":"color","value":"#0000ff"},"selection":{"type":"string","value":"badge"},"missing":{"type":"string","value":"badge"}},
        "children":[
            {"id":"definitions","type":"frame","enabled":false,"children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#0000ff"}]},
            {"id":"use","type":"ref","ref":"ui:generator"}
        ]}),
    );
    if mode == "themed-variable" {
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["themes"] = json!({"mode":["light","dark"]});
        base["children"][2]["theme"] = json!({"mode":"dark"});
        write(&path, base);
        let path = temp.path().join("library/ui.pen");
        let mut ui: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        ui["themes"] = json!({"mode":["light","dark"]});
        ui["variables"]["selection"]["value"] = json!([
            {"value":"badge"},{"theme":{"mode":"dark"},"value":"alternate"}
        ]);
        ui["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"UI alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}));
        write(&path, ui);
        let path = temp.path().join("main.pen");
        let mut main: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        main["themes"] = json!({"copy_mode":["light","dark"]});
        write(&path, main);
    }
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store, code)
}

fn copy(store: &Store, include_source: bool) -> Value {
    let mut variables = serde_json::Map::new();
    for (scope, prefix) in [("ui", "ui_"), ("ui:base", "base_")] {
        for name in ["selection", "tone"] {
            variables.insert(
                format!("{scope}:{name}"),
                json!({"action":"rename","name":format!("{prefix}{name}")}),
            );
        }
    }
    let mut options = json!({"component_prefix":"Copy ","variables":variables});
    if store.read().unwrap().value()["themes"]
        .get("copy_mode")
        .is_some()
    {
        options["themes"] = json!({
            "ui:mode":{"action":"reuse","target":"copy_mode"},
            "ui:base:mode":{"action":"reuse","target":"copy_mode"}
        });
    }
    if include_source {
        options["script_assets"] = json!({"ui:generator":[{"kind":"script","uri":"override.js"}]});
    }
    fixture::apply(
        store,
        "copy",
        json!([
            {"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy","options":options},
            {"op":"delete","target":"use"},{"op":"remove_import","alias":"ui"}
        ]),
    )
}

fn preview(store: &Store, first: Color, metadata: &str) -> Vec<u8> {
    let bytes = fs::read(store.path()).unwrap();
    let (project, layout) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    assert_eq!(layout.script_executions.len(), 2);
    assert!(
        layout.summary()["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let child = layout
        .items
        .iter()
        .find(|item| {
            item.node.kind() == "script"
                && item.node.generated_by.is_some()
                && item
                    .node
                    .properties
                    .get("metadata")
                    .is_some_and(|m| m["keep"] == metadata)
        })
        .expect("Generated script metadata");
    assert_eq!(child.node.properties["metadata"]["keep"], metadata);
    if metadata == "generated" {
        assert_eq!(child.node.properties["metadata"]["literal"], "$tone");
    }
    let image = render::png(
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
    .unwrap();
    assert_eq!((image.width, image.height), (36, 12));
    for (x, color) in [(5, first), (17, Color::GREEN), (29, Color::GREEN)] {
        assert_eq!(pixel(&image.png, x), color, "x={x}");
    }
    assert_eq!(fs::read(store.path()).unwrap(), bytes);
    image.png
}

fn independent_case(
    version: &str,
    style: &str,
    mode: &str,
    first: Color,
) -> (TempDir, Store, Vec<u8>) {
    eprintln!("Generated source: schema={version}, style={style}, mode={mode}");
    let (temp, store, code) = setup(version, style, mode, 1);
    let metadata = if mode == "metadata" {
        "generated"
    } else {
        "base"
    };
    let before = preview(&store, first, metadata);
    let response = copy(&store, true);
    let component = response["generated_ids"]["0"]["component_id"]
        .as_str()
        .unwrap();
    let document = store.read().unwrap();
    let node = document.node(component).unwrap();
    assert_eq!(
        fs::read(temp.path().join(node["scriptUri"].as_str().unwrap())).unwrap(),
        code.as_bytes()
    );
    let replacement =
        node["metadata"]["lazypencil"]["script_bindings"]["assets"]["script:override.js"]
            .as_str()
            .unwrap();
    assert_eq!(
        fs::read(temp.path().join(replacement)).unwrap(),
        REPLACEMENT.as_bytes()
    );
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    assert_eq!(
        preview(&store, first, metadata),
        before,
        "{version}/{style}/{mode}/copy"
    );
    let (second, _) = fixture::recopy(&store, component, &temp.path().join("second"), "again_");
    fs::remove_file(store.path()).unwrap();
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    assert_eq!(
        preview(&second, first, metadata),
        before,
        "{version}/{style}/{mode}/recopy"
    );

    (temp, second, before)
}

#[test]
fn generated_script_code_overrides_keep_saved_input_ownership_after_copy_and_recopy() {
    // Each schema owns independent temporary projects, assets and JS processes.
    // Keep both complete matrices while bounding their concurrency to two.
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement"] {
                    for mode in ["inherit", "literal", "variable", "null", "metadata"] {
                        let first = if ["inherit", "metadata"].contains(&mode) {
                            Color::RED
                        } else {
                            Color::GREEN
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_ref_inputs_follow_inherited_container_themes_after_copy_recopy_and_live_changes() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["descendant", "replacement", "children"] {
                    let (_temp, store, original) = independent_case(version, style, "themed-variable", Color::CYAN);
                    let document = store.read().unwrap();
                    let cards = document.locations().keys().filter(|id| {
                        let node = document.node(id).unwrap();
                        node["type"] == "frame" && node["reusable"] == true
                    }).cloned().collect::<Vec<_>>();
                    assert_eq!(cards.len(), 1, "Expected one reusable theme container");
                    fixture::apply(&store, "container-light", json!([{"op":"update","target":cards[0],"set":{"theme":{"copy_mode":"light"}}}]));
                    assert_ne!(preview(&store, Color::GREEN, "base"), original);
                    fixture::apply(&store, "container-dark", json!([{"op":"update","target":cards[0],"set":{"theme":{"copy_mode":"dark"}}}]));
                    assert_eq!(preview(&store, Color::CYAN, "base"), original);
                }
            });
        }
    });
}

#[test]
fn generated_code_overrides_reject_unmanifested_sources_explicit_contexts_and_binding_growth() {
    for (include_source, mode, count, expected) in [
        (false, "inherit", 1, "ASSET_NOT_FOUND"),
        (true, "explicit-context", 1, "UNSUPPORTED_FEATURE"),
        (true, "bad-variable", 1, "INVALID_DOCUMENT"),
        (true, "inherit", 600, "INVALID_DOCUMENT"),
    ] {
        let (temp, store, _) = setup("2.21", "direct", mode, count);
        copy(&store, include_source);
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        let bytes = fs::read(store.path()).unwrap();
        let error = service::layout_snapshot_with_runtime(
            &store,
            &[],
            None,
            None,
            &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
        )
        .err()
        .expect("Invalid generated source must fail");
        assert_eq!(
            error.code, expected,
            "{mode}/{include_source}/{count}: {error:?}"
        );
        if count == 600 {
            assert!(
                error.message.contains("generated output budget"),
                "{error:?}"
            );
        }
        assert_eq!(fs::read(store.path()).unwrap(), bytes);
    }
}
