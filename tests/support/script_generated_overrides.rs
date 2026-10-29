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
        mode if mode.starts_with("binding-combined-")
            || mode.starts_with("binding-native-parent-combined-") =>
        {
            let variant = mode
                .strip_prefix("binding-combined-")
                .or_else(|| mode.strip_prefix("binding-native-parent-combined-"))
                .unwrap();
            let inherited = variant.starts_with("inherited-");
            let inputs = if inherited {
                json!({"selected":"$selection","stale":"$missing"})
            } else {
                let inputs = json!({"selected":"badge"});
                changes["inputs"] = inputs.clone();
                inputs
            };
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            let stale_inputs = variant == "stale-inputs";
            let stale_binding = variant == "stale-binding-output";
            let stale_scope = variant == "stale-scope-output";
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{
                "script_scope":{"version":1,"script_uri":if stale_scope {"old.js"} else {"override.js"},"library":if stale_scope {"uncaptured"} else {"base"},
                    "inputs":{"hash":if stale_inputs {"a".repeat(64)} else {hash.clone()},"library":if variant == "inherited-scope-input" {""} else {"base"}}},
                "script_bindings":{"version":1,"script_uri":if stale_binding {"old.js"} else {"override.js"},"library":if stale_binding {"uncaptured"} else {""},
                    "components":{"badge":"badge"},"variables":{"tone":"tone"},
                    "inputs_hash":if stale_inputs || ["scope-input","stale-scope-output","inherited-scope-input"].contains(&variant) {"b".repeat(64)} else {hash},
                    "input_components":{"badge":"alternate"}}
            }});
        }
        mode if mode.starts_with("binding-native-parent-input-") => {
            let inherited = mode.contains("-inherited-");
            let stale = mode.contains("-stale");
            let selected = if inherited != stale {
                "$selection"
            } else {
                "$native_selection"
            };
            let guarded = if inherited {
                json!({"selected":selected,"stale":"$missing"})
            } else {
                changes["inputs"] = json!({"selected":"$selection"});
                json!({"selected":selected})
            };
            let hash = lazypencil::document::hash(guarded.to_string().as_bytes());
            let namespace = if mode.contains("-scope") {
                json!({"script_scope":{"version":1,"script_uri":if mode.contains("-only-") {"old.js"} else {"override.js"},"library":if mode.contains("-only-") {"uncaptured"} else {""},"inputs":{"hash":hash,"library":if inherited {""} else {"base"}}}})
            } else if mode.contains("-default") {
                json!({})
            } else {
                json!({"script_bindings":{"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"},"inputs_hash":hash,"input_components":if inherited {json!({"badge":"alternate"})} else {json!({"alternate":"badge"})}}})
            };
            changes["metadata"] =
                json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":namespace});
        }
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
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"base"}}})
        }
        "explicit-input-context" => {
            let inputs = json!({"selected":"$selection","stale":"$missing"});
            changes["inputs"] = inputs.clone();
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"base","inputs":{"hash":hash,"library":""}}}});
        }
        "unknown-context" => {
            changes["metadata"] = json!({"type":"test","lazypencil":{"script_scope":{"version":1,"script_uri":"override.js","library":"uncaptured"}}});
        }
        "explicit-bindings"
        | "binding-library"
        | "binding-mapped"
        | "binding-catalog"
        | "binding-inputs"
        | "binding-stale-inputs"
        | "binding-inherited-inputs"
        | "binding-metadata"
        | "binding-unknown-target"
        | "binding-unknown-library"
        | "binding-unmanifested-asset"
        | "binding-combined"
        | "binding-native-parent"
        | "binding-native-parent-scope"
        | "binding-native-parent-default"
        | "binding-native-parent-library"
        | "binding-native-parent-scope-library"
        | "binding-native-parent-default-library" => {
            let mut bindings = json!({"version":1,"script_uri":"override.js","components":{"badge":"badge"},"variables":{"tone":"tone"}});
            if ["binding-library", "binding-inputs", "binding-stale-inputs"].contains(&mode) {
                bindings["library"] = json!("base");
            }
            if mode == "binding-mapped" {
                bindings["components"]["badge"] = json!("alternate");
                bindings["variables"]["tone"] = json!("accent");
            }
            if ["binding-inputs", "binding-stale-inputs"].contains(&mode) {
                let inputs = json!({"selected":"badge","stale":"$missing"});
                changes["inputs"] = inputs.clone();
                bindings["inputs_hash"] = json!(if mode == "binding-inputs" {
                    lazypencil::document::hash(inputs.to_string().as_bytes())
                } else {
                    "a".repeat(64)
                });
                bindings["input_components"] = json!({"badge":"badge"});
            }
            if mode == "binding-inherited-inputs" {
                let inputs = json!({"selected":"$selection","stale":"$missing"});
                bindings["inputs_hash"] =
                    json!(lazypencil::document::hash(inputs.to_string().as_bytes()));
                bindings["input_components"] = json!({"badge":"alternate"});
            }
            if mode == "binding-metadata" {
                changes.as_object_mut().unwrap().remove("scriptUri");
                bindings["script_uri"] = json!("base.js");
                fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
            }
            if mode == "binding-catalog" {
                bindings["version"] = json!(2);
                bindings["components"] = json!({"component-slot":"badge"});
                bindings["variables"] = json!({"variable-slot":"tone"});
                bindings["assets"] = json!({});
                bindings["script_scopes"] = json!({"":{"components":{"badge":"component-slot"},"variables":{"tone":"variable-slot"},"themes":{},"assets":{},"libraries":{"":""},"browser_libraries":{}}});
            }
            if mode == "binding-unknown-target" {
                bindings["components"]["badge"] = json!("missing");
            }
            if mode == "binding-unknown-library" {
                bindings["library"] = json!("uncaptured");
            }
            if mode == "binding-unmanifested-asset" {
                bindings["assets"] = json!({"script:alias.js":"unmanifested.js"});
            }
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_bindings":bindings}});
            if mode == "binding-combined" {
                changes["metadata"]["lazypencil"]["script_scope"] =
                    json!({"version":1,"script_uri":"override.js","library":"base"});
            }
            if mode.starts_with("binding-native-parent-scope") {
                changes["metadata"]["lazypencil"] =
                    json!({"script_scope":{"version":1,"script_uri":"override.js","library":""}});
            } else if mode.starts_with("binding-native-parent-default") {
                changes["metadata"]
                    .as_object_mut()
                    .unwrap()
                    .remove("lazypencil");
            }
        }
        "stale-scope" | "stale-input-scope" => {
            let mut scope = json!({"version":1,"script_uri":"old.js","library":"uncaptured"});
            if mode == "stale-input-scope" {
                changes["inputs"] = json!({"selected":"badge"});
                scope["inputs"] = json!({"hash":"a".repeat(64),"library":"uncaptured"});
            }
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":scope}});
        }
        "active-input-scope" | "inherited-input-scope" => {
            let inputs = if mode == "active-input-scope" {
                let inputs = json!({"selected":"badge"});
                changes["inputs"] = inputs.clone();
                inputs
            } else {
                json!({"selected":"$selection","stale":"$missing"})
            };
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"old.js","library":"base","inputs":{"hash":hash,"library":""}}}});
        }
        "stale-inherited-input-scope" | "inherited-unknown-input-scope" => {
            if mode == "stale-inherited-input-scope" {
                fs::write(temp.path().join("library/base.js"), "/**\n * @schema 2.21\n * @input selected: ref = \"badge\"\n */\nreturn [{type:'rectangle',width:36,height:12,fill:'#0000ff'}];").unwrap();
            }
            let selected = if mode == "stale-inherited-input-scope" {
                "$base_selection"
            } else {
                "$selection"
            };
            let inputs = json!({"selected":selected,"stale":"$missing"});
            let hash = lazypencil::document::hash(inputs.to_string().as_bytes());
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":{"version":1,"script_uri":"old.js","library":"base","inputs":{"hash":hash,"library":"uncaptured"}}}});
        }
        "metadata-output" | "metadata-input" | "metadata-both" | "metadata-unknown-source" => {
            changes.as_object_mut().unwrap().remove("scriptUri");
            fs::write(temp.path().join("library/base.js"), REPLACEMENT).unwrap();
            let mut scope = json!({"version":1,"script_uri":"base.js","library":""});
            if mode == "metadata-input" {
                scope["script_uri"] = json!("old.js");
                scope["library"] = json!("uncaptured");
            }
            if ["metadata-input", "metadata-both"].contains(&mode) {
                let inputs = json!({"selected":"$selection","stale":"$missing"});
                scope["inputs"] = json!({"hash":lazypencil::document::hash(inputs.to_string().as_bytes()),"library":""});
            }
            if mode == "metadata-unknown-source" {
                scope["library"] = json!("uncaptured");
            }
            changes["metadata"] = json!({"type":"test","keep":"generated","literal":"$tone","lazypencil":{"script_scope":scope}});
        }
        "inherit" | "saved-input-owner" => {}
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
        "script" => {
            changes["type"] = json!("script");
            changes["width"] = json!(36);
            changes["height"] = json!(12);
            changes
        }
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
    if ["binding-mapped", "binding-inherited-inputs"].contains(&mode)
        || mode.starts_with("binding-combined-")
    {
        let path = temp.path().join("library/ui.pen");
        let mut ui: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        ui["variables"]["accent"] = json!({"type":"color","value":"#00ffff"});
        ui["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"UI alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}));
        write(&path, ui);
    }
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
    if mode == "saved-input-owner" {
        write(
            &temp.path().join("library/inputs.pen"),
            json!({"version":version,
                "variables":{"source_selection":{"type":"string","value":"badge"}},
                "children":[{"id":"badge","name":"Input badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}]
            }),
        );
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["imports"] = json!({"inputs":"inputs.pen"});
        let inputs = json!({"selected":"$source_selection","stale":"$missing"});
        base["children"][1]["inputs"] = inputs.clone();
        base["children"][1]["metadata"]["lazypencil"]["script_scope"] = json!({"version":1,"script_uri":"base.js","library":"","inputs":{"hash":lazypencil::document::hash(inputs.to_string().as_bytes()),"library":"inputs"}});
        write(&path, base);
    }
    if mode == "binding-combined-inherited-nonlexical" {
        write(
            &temp.path().join("library/inputs.pen"),
            json!({"version":version,
                "variables":{"selection":{"type":"string","value":"badge"}},
                "children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}]
            }),
        );
        let path = temp.path().join("library/base.pen");
        let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        base["imports"] = json!({"inputs":"inputs.pen"});
        base["children"][1]["metadata"]["lazypencil"]["script_input_context"] = json!({
            "version":1,"library":"inputs","values":{"selected":"$selection","stale":"$missing"},
            "variables":{"selection":"selection"},"components":{"badge":"badge"}
        });
        write(&path, base);
    }
    if mode.starts_with("binding-native-parent") {
        let library = mode.ends_with("-library");
        let path = temp.path().join(if library {
            "library/base.pen"
        } else {
            "library/ui.pen"
        });
        let mut owner: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        owner["variables"]["accent"] = json!({"type":"color","value":"#00ffff"});
        if mode.starts_with("binding-native-parent-input-") {
            owner["variables"]["native_selection"] = json!({"type":"string","value":"alternate"});
            if !library {
                owner["variables"]["selection"]["value"] = json!("alternate");
            }
            let path = temp.path().join("library/base.pen");
            let mut base: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            base["variables"]["native_selection"] = json!({"type":"string","value":"missing"});
            if !library {
                base["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"Base alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"}));
            }
            write(&path, base);
        }
        owner["children"].as_array_mut().unwrap().push(json!({"id":"alternate","name":"Native alternate","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}));
        if !library {
            owner["children"].as_array_mut().unwrap().extend([
                json!({"id":"worker-bridge","type":"ref","reusable":true,"ref":"base:worker"}),
                json!({"id":"card-bridge","type":"ref","reusable":true,"ref":"base:card"}),
            ]);
        }
        write(&path, owner);
        let path = temp.path().join("library/ui.pen");
        let mut ui: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut parent = json!({"version":1,"script_uri":"draw.js",
            "components":{"badge":"alternate","base:worker":if library {"worker"} else {"worker-bridge"},"base:card":if library {"card"} else {"card-bridge"}},
            "variables":{"tone":"accent"},
            "descendants":{"base:card":{"draw":"draw"}},
            "descendant_contexts":{"base:card":{"draw":{"parent":"","instance":true,"target_instance":true}}}
        });
        if mode.starts_with("binding-native-parent-input-") {
            parent["variables"]["selection"] = json!("native_selection");
        }
        if library {
            parent["library"] = json!("base");
            ui["imports"]["base"] = json!("base/base.pen");
        }
        let generator = ui["children"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["id"] == "generator")
            .unwrap();
        generator["metadata"] =
            json!({"type":"test","keep":"native-parent","lazypencil":{"script_bindings":parent}});
        write(&path, ui);
        if library {
            fs::create_dir(temp.path().join("library/base")).unwrap();
            fs::rename(
                temp.path().join("library/base.pen"),
                temp.path().join("library/base/base.pen"),
            )
            .unwrap();
            fs::rename(
                temp.path().join("library/base.js"),
                temp.path().join("library/base/base.js"),
            )
            .unwrap();
            fs::write(temp.path().join("library/base/override.js"), REPLACEMENT).unwrap();
            fs::write(
                temp.path().join("library/override.js"),
                "/* @schema 2.21 */ return [{type:'rectangle',width:36,height:12,fill:'#ff00ff'}];",
            )
            .unwrap();
        }
    }
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store, code)
}

fn copy(store: &Store, include_source: bool, mode: &str) -> Value {
    let mut variables = serde_json::Map::new();
    for (scope, prefix) in [("ui", "ui_"), ("ui:base", "base_")] {
        for name in ["selection", "tone"] {
            variables.insert(
                format!("{scope}:{name}"),
                json!({"action":"rename","name":format!("{prefix}{name}")}),
            );
        }
        if mode.starts_with("binding-native-parent-input-")
            && !(scope == "ui" && mode.ends_with("-library"))
        {
            variables.insert(
                format!("{scope}:native_selection"),
                json!({"action":"rename","name":format!("{prefix}native_selection")}),
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
    preview_colours(store, [first, Color::GREEN, Color::GREEN], metadata)
}

fn preview_colours(store: &Store, colours: [Color; 3], metadata: &str) -> Vec<u8> {
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
    for (x, color) in [5, 17, 29].into_iter().zip(colours) {
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
    let metadata = if [
        "metadata",
        "stale-scope",
        "stale-input-scope",
        "explicit-context",
        "explicit-input-context",
        "active-input-scope",
        "inherited-input-scope",
        "stale-inherited-input-scope",
        "metadata-output",
        "metadata-input",
        "metadata-both",
    ]
    .contains(&mode)
        || mode == "explicit-bindings"
        || mode.starts_with("binding-")
    {
        "generated"
    } else {
        "base"
    };
    let output = if (mode.starts_with("binding-native-parent") && mode.ends_with("-library"))
        || [
            "explicit-context",
            "explicit-input-context",
            "metadata-input",
            "binding-library",
            "binding-inputs",
            "binding-stale-inputs",
            "binding-combined-stale-binding-output",
        ]
        .contains(&mode)
    {
        Color::RED
    } else if mode == "binding-mapped" {
        Color::CYAN
    } else {
        Color::GREEN
    };
    let render = |store: &Store, stage: &str| {
        let png = preview_colours(store, [first, output, output], metadata);
        if ([
            "explicit-context",
            "explicit-input-context",
            "active-input-scope",
            "inherited-input-scope",
            "stale-inherited-input-scope",
            "metadata-output",
            "metadata-input",
            "metadata-both",
            "saved-input-owner",
        ]
        .contains(&mode)
            || mode == "explicit-bindings"
            || mode.starts_with("binding-"))
            && let Some(root) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS")
        {
            let root = std::path::PathBuf::from(root).join("generated-script-scopes");
            fs::create_dir_all(&root).unwrap();
            let name = format!("{version}-{style}-{mode}-{stage}");
            fs::write(root.join(format!("{name}.png")), &png).unwrap();
            fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(&json!({
                "source_commit":std::env::var("GITHUB_SHA").ok(),"png_sha256":lazypencil::document::hash(&png),
                "width":36,"height":12,"red_first":first==Color::RED,"red_output":output==Color::RED,
                "first_rgba":[first.r(),first.g(),first.b(),first.a()],
                "output_rgba":[output.r(),output.g(),output.b(),output.a()]
            })).unwrap()).unwrap();
        }
        png
    };
    let before = render(&store, "original");
    let response = copy(&store, true, mode);
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
        render(&store, "copied"),
        before,
        "{version}/{style}/{mode}/copy"
    );
    let (second, _) = fixture::recopy(&store, component, &temp.path().join("second"), "again_");
    fs::remove_file(store.path()).unwrap();
    fs::remove_dir_all(temp.path().join("assets")).unwrap();
    assert_eq!(
        render(&second, "recopied"),
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
fn inactive_generated_script_scopes_do_not_block_copy_or_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "script"] {
                    for mode in ["stale-scope", "stale-input-scope"] {
                        let first = if style == "script" || mode == "stale-input-scope" {
                            Color::GREEN
                        } else {
                            Color::RED
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_code_overrides_reject_unmanifested_sources_explicit_contexts_and_binding_growth() {
    for (include_source, mode, count, expected) in [
        (false, "inherit", 1, "ASSET_NOT_FOUND"),
        (true, "unknown-context", 1, "INVALID_DOCUMENT"),
        (true, "binding-unknown-target", 1, "INVALID_DOCUMENT"),
        (true, "binding-unknown-library", 1, "INVALID_DOCUMENT"),
        (true, "binding-unmanifested-asset", 1, "ASSET_NOT_FOUND"),
        (
            true,
            "binding-combined-inherited-nonlexical",
            1,
            "UNSUPPORTED_FEATURE",
        ),
        (true, "inherited-unknown-input-scope", 1, "INVALID_DOCUMENT"),
        (true, "metadata-unknown-source", 1, "INVALID_DOCUMENT"),
        (true, "bad-variable", 1, "INVALID_DOCUMENT"),
        (true, "inherit", 600, "INVALID_DOCUMENT"),
    ] {
        let (temp, store, _) = setup("2.21", "direct", mode, count);
        copy(&store, include_source, mode);
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

#[test]
fn generated_scope_and_binding_guards_compose_without_overwriting_each_other() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for (mode, first) in [
                        (
                            "binding-combined",
                            if style == "script" {
                                Color::GREEN
                            } else {
                                Color::RED
                            },
                        ),
                        ("binding-combined-scope-input", Color::RED),
                        ("binding-combined-binding-input", Color::CYAN),
                        ("binding-combined-stale-binding-output", Color::RED),
                        ("binding-combined-stale-scope-output", Color::RED),
                        ("binding-combined-stale-inputs", Color::GREEN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for style in ["direct", "descendant", "replacement", "children"] {
                    for (mode, first) in [
                        ("binding-combined-inherited-binding-input", Color::CYAN),
                        ("binding-combined-inherited-scope-input", Color::GREEN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for style in ["script", "direct"] {
                    for (mode, first) in [
                        ("binding-native-parent-combined-scope-input", Color::RED),
                        ("binding-native-parent-combined-binding-input", Color::CYAN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
            });
        }
    });
}

#[test]
fn generated_custom_bindings_compose_names_libraries_catalogs_and_saved_inputs() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children", "script"] {
                    for mode in [
                        "explicit-bindings",
                        "binding-library",
                        "binding-mapped",
                        "binding-catalog",
                        "binding-inputs",
                        "binding-stale-inputs",
                    ] {
                        let first = match mode {
                            "binding-inputs" => Color::RED,
                            "binding-stale-inputs" => Color::GREEN,
                            "binding-mapped" if style == "script" => Color::CYAN,
                            "binding-library" => Color::RED,
                            _ if style == "script" => Color::GREEN,
                            _ => Color::RED,
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for style in ["direct", "descendant", "replacement", "children"] {
                    let _fixture =
                        independent_case(version, style, "binding-inherited-inputs", Color::CYAN);
                    let _fixture = independent_case(version, style, "binding-metadata", Color::RED);
                }
            });
        }
    });
}

#[test]
fn native_binding_parents_keep_the_document_owner_of_nested_code_and_custom_contexts() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for mode in [
                        "binding-native-parent",
                        "binding-native-parent-scope",
                        "binding-native-parent-default",
                    ] {
                        let first = if style == "script" {
                            Color::GREEN
                        } else {
                            Color::RED
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for mode in [
                    "binding-native-parent-library",
                    "binding-native-parent-scope-library",
                    "binding-native-parent-default-library",
                ] {
                    let _fixture = independent_case(version, "script", mode, Color::RED);
                }
            });
        }
    });
}

#[test]
fn native_parent_input_guards_use_post_mapping_expressions_and_keep_inherited_controls_separate() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["script", "direct", "descendant", "replacement", "children"] {
                    for (mode, first) in [
                        ("binding-native-parent-input-scope", Color::RED),
                        ("binding-native-parent-input-scope-stale", Color::CYAN),
                        ("binding-native-parent-input-only-scope", Color::RED),
                        ("binding-native-parent-input-only-scope-stale", Color::CYAN),
                        ("binding-native-parent-input-custom", Color::GREEN),
                        ("binding-native-parent-input-custom-stale", Color::CYAN),
                        ("binding-native-parent-input-default", Color::CYAN),
                    ] {
                        let _fixture = independent_case(version, style, mode, first);
                    }
                    if style != "script" {
                        for (mode, first) in [
                            ("binding-native-parent-input-inherited-scope", Color::GREEN),
                            (
                                "binding-native-parent-input-inherited-only-scope",
                                Color::GREEN,
                            ),
                            (
                                "binding-native-parent-input-inherited-only-scope-stale",
                                Color::RED,
                            ),
                            (
                                "binding-native-parent-input-inherited-scope-stale",
                                Color::RED,
                            ),
                            ("binding-native-parent-input-inherited-custom", Color::CYAN),
                            (
                                "binding-native-parent-input-inherited-custom-stale",
                                Color::RED,
                            ),
                        ] {
                            let _fixture = independent_case(version, style, mode, first);
                        }
                    }
                }
                for (mode, first) in [
                    ("binding-native-parent-input-custom-library", Color::RED),
                    (
                        "binding-native-parent-input-custom-stale-library",
                        Color::CYAN,
                    ),
                    ("binding-native-parent-input-default-library", Color::CYAN),
                ] {
                    let _fixture = independent_case(version, "script", mode, first);
                }
            });
        }
    });
}

#[test]
fn generated_inherited_scope_guards_keep_original_spelling_after_copy_and_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children"] {
                    for mode in ["inherited-input-scope", "metadata-output", "metadata-both"] {
                        let first = if mode == "metadata-output" {
                            Color::RED
                        } else {
                            Color::GREEN
                        };
                        let _fixture = independent_case(version, style, mode, first);
                    }
                }
                for (mode, first) in [
                    ("metadata-input", Color::GREEN),
                    ("stale-inherited-input-scope", Color::RED),
                    ("saved-input-owner", Color::CYAN),
                ] {
                    let _fixture = independent_case(version, "direct", mode, first);
                }
            });
        }
    });
}

#[test]
fn generated_explicit_scopes_keep_output_and_input_namespaces_through_copy_and_recopy() {
    std::thread::scope(|threads| {
        for version in ["2.19", "2.21"] {
            threads.spawn(move || {
                for style in ["direct", "descendant", "replacement", "children", "script"] {
                    for mode in [
                        "explicit-context",
                        "explicit-input-context",
                        "active-input-scope",
                    ] {
                        let first = if mode == "explicit-context" {
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
