#![cfg(feature = "rendering")]

use lazypencil::{
    geometry::Rect,
    layout::LayoutSnapshot,
    operations::EditRequest,
    project::Project,
    render::{self, RenderOptions},
    script::ScriptRuntime,
    service,
    store::Store,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{fs, path::Path};
use tempfile::TempDir;

fn write(path: &Path, value: Value) {
    fs::write(path, value.to_string()).unwrap();
}

fn fixture(inherited: bool, explicit: bool) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    let mut surface = skia_safe::surfaces::raster_n32_premul((2, 2)).unwrap();
    surface.canvas().clear(Color::YELLOW);
    fs::write(
        temp.path().join("library/pixel.png"),
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    surface.canvas().clear(Color::MAGENTA);
    fs::write(
        temp.path().join("pixel.png"),
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    write(
        &temp.path().join("library/base.lib.pen"),
        json!({"version":"2.21","children":[
            {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#0000ff"}
        ]}),
    );
    fs::write(temp.path().join("library/draw.js"), r##"/**
 * @schema 2.21
 * @input selected: ref = "nested:badge"
 * @input label: string = "Default"
 */
return [
 {id:"variable",type:"rectangle",width:10,height:10,fill:"$tone"},
 {id:"own",type:"ref",ref:"badge",x:10},
 {id:"nested-ref",type:"ref",ref:"nested:badge",x:20},
 {id:"input-ref",type:"ref",ref:pencil.input.selected,x:30},
 {id:"picture",type:"rectangle",x:40,width:10,height:10,fill:{type:"image",url:"pixel.png"}},
 {id:"nested-script",type:"script",scriptUri:"nested.js",y:10,width:20,height:10},
 {id:"label",type:"text",content:pencil.input.label,y:20,fontFamily:"Noto Sans JP",fontSize:10,fill:"#000000"}
];"##).unwrap();
    fs::write(temp.path().join("library/nested.js"), "/* @schema 2.21 */ return [{type:'rectangle',width:10,height:10,fill:'$tone'},{type:'ref',ref:'nested:badge',x:10}];").unwrap();
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({"version":"2.21",
        "imports":{"nested":"base.lib.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"tone":{"type":"color","value":[{"value":"#ff0000"},{"theme":{"mode":"dark"},"value":"#0000ff"}]},"caption":{"type":"string","value":"Library"}},
        "children":[
            {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"},
            {"id":"generator","type":"script","reusable":true,"width":60,"height":36,"scriptUri":"draw.js","inputs":{"label":"$caption"},"metadata":{"type":"example","keep":[1,2,3]}},
            {"id":"card","type":"frame","reusable":true,"layout":"none","width":60,"height":36,"children":[{"id":"draw","type":"ref","ref":"generator"}]}
        ]}),
    );
    let mut instance = json!({"id":"use","type":"ref","ref":if inherited {"ui:card"} else {"ui:generator"},"theme":{"mode":"dark"}});
    if explicit {
        if inherited {
            instance["descendants"] =
                json!({"draw":{"inputs":{"selected":"badge","label":"Consumer"}}});
        } else {
            instance["inputs"] = json!({"selected":"badge","label":"Consumer"});
        }
    }
    let mut other = instance.clone();
    other["id"] = json!("other");
    other["x"] = json!(70);
    write(
        &temp.path().join("main.pen"),
        json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"tone":{"type":"color","value":"#00ff00"},"caption":{"type":"string","value":"Consumer"}},
        "children":[{"id":"definitions","type":"frame","enabled":false,"width":0,"height":0,"children":[{"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ff00"}]}, instance, other]}),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn edit(store: &Store, id: &str, operations: Value) -> Value {
    let metadata = service::document_metadata(store).unwrap();
    let request: EditRequest = serde_json::from_value(json!({"request_id":id,"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":operations})).unwrap();
    service::apply_request(store, &request).unwrap()
}

fn snapshot(store: &Store) -> (Project, LayoutSnapshot, Vec<u8>) {
    let (project, snapshot) = service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    let image = render::png(
        &project,
        &snapshot,
        &RenderOptions {
            region: Some(Rect {
                x: 0.,
                y: 0.,
                width: 130.,
                height: 36.,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    (project, snapshot, image.png)
}

#[test]
fn external_and_inherited_script_detach_preserves_actual_code_refs_themes_inputs_and_pixels() {
    for inherited in [false, true] {
        for explicit in [false, true] {
            let (temp, store) = fixture(inherited, explicit);
            let source = fs::read(temp.path().join("library/draw.js")).unwrap();
            let (before_project, before, before_png) = snapshot(&store);
            assert_eq!(before.script_executions.len(), 4);
            let result = edit(
                &store,
                "detach-script",
                json!([{"op":"detach_instance","target":if inherited {json!({"node_id":"use","instance_path":["draw"]})} else {json!("use")},"client_ref":"detached"}]),
            );
            let document = store.read().unwrap();
            let detached = document
                .node(result["bindings"]["detached"].as_str().unwrap())
                .unwrap();
            assert_eq!(detached["metadata"]["keep"], json!([1, 2, 3]));
            assert_eq!(detached["metadata"]["type"], "example");
            assert_eq!(
                detached["metadata"]["lazypencil"]["script_scope"]["library"],
                "ui"
            );
            let (after_project, after, after_png) = snapshot(&store);
            assert_eq!(
                before_png, after_png,
                "inherited={inherited}, explicit={explicit}"
            );
            assert_eq!(
                before.script_executions.len(),
                after.script_executions.len()
            );
            assert_eq!(
                serde_json::to_value(before_project.runtime_dependencies()).unwrap(),
                serde_json::to_value(after_project.runtime_dependencies()).unwrap()
            );
            assert_eq!(
                fs::read(temp.path().join("library/draw.js")).unwrap(),
                source
            );
            let image = Image::from_encoded(Data::new_copy(&after_png))
                .unwrap()
                .make_raster_image(None, None)
                .unwrap();
            let pixels = image.peek_pixels().unwrap();
            for (x, color) in [
                (5, Color::BLUE),
                (15, Color::RED),
                (25, Color::BLUE),
                (35, if explicit { Color::GREEN } else { Color::BLUE }),
                (45, Color::YELLOW),
            ] {
                assert_eq!(
                    pixels.get_color((x, 5)),
                    color,
                    "x={x}, inherited={inherited}, explicit={explicit}"
                );
            }
            assert_eq!(pixels.get_color((5, 15)), Color::BLUE);
        }
    }
}

#[test]
fn editing_detached_inputs_and_source_starts_the_requested_context_without_stale_bindings() {
    let (temp, store) = fixture(false, false);
    let result = edit(
        &store,
        "detach-script",
        json!([{"op":"detach_instance","target":"use","client_ref":"detached"}]),
    );
    let id = result["bindings"]["detached"].as_str().unwrap();
    edit(
        &store,
        "new-input",
        json!([{"op":"update","target":id,"set":{"inputs":{"selected":"badge","label":"Edited"}}}]),
    );
    let (_, calculation, png) = snapshot(&store);
    let image = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        image.peek_pixels().unwrap().get_color((35, 5)),
        Color::GREEN
    );
    assert!(
        calculation
            .items
            .iter()
            .any(|item| item.node.properties.get("content") == Some(&json!("Edited")))
    );
    fs::write(
        temp.path().join("replacement.js"),
        "/* @schema 2.21 */ return [{type:'rectangle',width:10,height:10,fill:'$tone'}];",
    )
    .unwrap();
    edit(
        &store,
        "new-source",
        json!([{"op":"update","target":id,"set":{"scriptUri":"replacement.js"},"unset":["inputs"]}]),
    );
    let (_, _, png) = snapshot(&store);
    let image = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(image.peek_pixels().unwrap().get_color((5, 5)), Color::GREEN);
}

#[test]
fn a_detached_script_can_be_imported_and_detached_again_from_another_document() {
    let (temp, store) = fixture(false, false);
    let saved = edit(
        &store,
        "detach-script",
        json!([{"op":"detach_instance","target":"use","client_ref":"detached"}]),
    );
    let id = saved["bindings"]["detached"].as_str().unwrap();
    edit(
        &store,
        "reusable-script",
        json!([{"op":"update","target":id,"set":{"reusable":true}}]),
    );
    fs::create_dir(temp.path().join("consumer")).unwrap();
    let path = temp.path().join("consumer/design.pen");
    write(
        &path,
        json!({"version":"2.21","imports":{"main":"../main.pen"},"children":[{"id":"use","type":"ref","ref":format!("main:{id}")}]}),
    );
    let consumer = Store::open(path, Some(store.data_dir().into())).unwrap();
    let (_, before, before_png) = snapshot(&consumer);
    assert_eq!(before.script_executions.len(), 2);
    let saved = edit(
        &consumer,
        "detach-again",
        json!([{"op":"detach_instance","target":"use","client_ref":"detached"}]),
    );
    let document = consumer.read().unwrap();
    let node = document
        .node(saved["bindings"]["detached"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        node["metadata"]["lazypencil"]["script_scope"]["library"],
        "main:ui"
    );
    assert_eq!(node["scriptUri"], "../library/draw.js");
    let (_, after, after_png) = snapshot(&consumer);
    assert_eq!(after.script_executions.len(), 2);
    assert_eq!(before_png, after_png);
}

#[test]
fn equal_json_instance_overrides_use_the_overriding_documents_input_and_code_scopes() {
    let (temp, store) = fixture(false, false);
    let library = temp.path().join("library/ui.lib.pen");
    let mut document: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    document["children"][1]["inputs"] = json!({"selected":"badge","label":"Library"});
    write(&library, document);
    fs::write(
        temp.path().join("nested.js"),
        fs::read(temp.path().join("library/nested.js")).unwrap(),
    )
    .unwrap();
    let saved = edit(
        &store,
        "detach-script",
        json!([{"op":"detach_instance","target":"use","client_ref":"detached"}]),
    );
    let id = saved["bindings"]["detached"].as_str().unwrap();
    edit(
        &store,
        "instance-overrides",
        json!([
            {"op":"add_import","alias":"nested","uri":"library/base.lib.pen"},
            {"op":"update","target":id,"set":{"reusable":true}},
            {"op":"insert","node":{"type":"ref","ref":id,"y":40,"inputs":{"selected":"badge","label":"Library"}}},
            {"op":"insert","node":{"type":"ref","ref":id,"x":70,"y":40,"scriptUri":"library/draw.js"}}
        ]),
    );
    let (project, snapshot, _) = snapshot(&store);
    let rendered = render::png(
        &project,
        &snapshot,
        &RenderOptions {
            region: Some(Rect {
                x: 0.,
                y: 40.,
                width: 130.,
                height: 36.,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let image = Image::from_encoded(Data::new_copy(&rendered.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixels = image.peek_pixels().unwrap();
    for (x, color) in [
        (5, Color::BLUE),
        (15, Color::RED),
        (35, Color::GREEN),
        (45, Color::YELLOW),
        (75, Color::GREEN),
        (85, Color::GREEN),
        (105, Color::RED),
        (115, Color::MAGENTA),
    ] {
        assert_eq!(pixels.get_color((x, 5)), color, "x={x}");
    }
}
