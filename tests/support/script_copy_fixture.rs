use lazypencil::{operations::EditRequest, service, store::Store};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::TempDir;

pub(super) const CODE: &str = r##"/**
 * @schema 2.21
 * @input selected: ref = "badge"
 * @input alternate: boolean = false
 * @input shader: boolean = false
 * @input nested: boolean = true
 * @input bad: boolean = false
 */
return [
 {id:"chosen",type:"ref",ref:pencil.input.alternate?"base:future":pencil.input.selected},
 {id:"tone",type:"rectangle",x:20,width:10,height:10,fill:pencil.input.alternate?"$futureTone":"$tone"},
 {id:"image",type:"rectangle",x:40,width:10,height:10,fill:pencil.input.shader?
  {type:"shader",url:"sample.glsl",uniforms:{tex:"$texture"}}:
  {type:"image",url:pencil.input.bad?"consumer.png":(pencil.input.alternate?"future.png":"pixel.png")}},
 {id:"filename",type:"text",y:20,content:"$texture",fontFamily:"Noto Sans JP",fontSize:10,fill:"#000000"},
 ...(pencil.input.nested?[{id:"nested",type:"script",scriptUri:"nested.js",x:60,width:20,height:10}]:[]),
 {id:"themed",type:"frame",x:80,width:10,height:10,layout:"none",theme:{mode:"dark"},children:[{type:"rectangle",width:10,height:10,fill:"$tone"}]}
];"##;

pub(super) fn write(path: &Path, value: Value) {
    fs::write(path, value.to_string()).unwrap();
}

pub(super) fn fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(
        temp.path().join("library/pixel.png"),
        include_bytes!("../../examples/assets/crop-source.png"),
    )
    .unwrap();
    fs::write(
        temp.path().join("library/future.png"),
        include_bytes!("../../examples/assets/crop-source.png"),
    )
    .unwrap();
    fs::write(temp.path().join("library/draw.js"), CODE).unwrap();
    fs::write(temp.path().join("library/nested.js"),"/* @schema 2.21 */ return [{type:'ref',ref:'base:other'},{type:'rectangle',x:10,width:10,height:10,fill:'$tone'}];").unwrap();
    fs::write(temp.path().join("library/sample.glsl"),"precision highp float;uniform sampler2D tex;void main(){gl_FragColor=texture2D(tex,vec2(0.5));}").unwrap();
    write(
        &temp.path().join("library/base.lib.pen"),
        json!({"version":"2.21","children":[
        {"id":"other","name":"Other","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#0000ff"},
        {"id":"future","name":"Future","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ffff"}]}),
    );
    write(
        &temp.path().join("library/ui.lib.pen"),
        json!({"version":"2.21","imports":{"base":"base.lib.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"tone":{"type":"color","value":[{"value":"#ff0000"},{"theme":{"mode":"dark"},"value":"#0000ff"}]},
            "texture":{"type":"string","value":"pixel.png"},"futureTone":{"type":"color","value":"#00ffff"}},
        "children":[{"id":"badge","name":"Badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"},
            {"id":"generator","name":"Generator","type":"script","reusable":true,"width":100,"height":36,"scriptUri":"draw.js","inputs":{"stale":"$missing"},"metadata":{"type":"example","keep":[1,2,3]}},
            {"id":"card","name":"Card","type":"frame","reusable":true,"layout":"none","width":100,"height":36,"children":[{"id":"draw","type":"ref","ref":"generator"}]}]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":"2.21","imports":{"ui":"library/ui.lib.pen"},
        "variables":{"tone":{"type":"color","value":"#00ff00"}},"children":[
            {"id":"definitions","type":"frame","enabled":false,"width":0,"height":0,"children":[
                {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ff00"}]},
            {"id":"before-use","type":"ref","ref":"ui:generator"}]}),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

pub(super) fn operations() -> Value {
    json!([{"op":"import_component","source":{"library_alias":"ui","node_id":"generator"},"mode":"copy",
        "options":{"variable_prefix":"copy.","component_prefix":"Copy ","themes":{"ui:mode":{"action":"rename","name":"copy.mode"}},
            "script_assets":{"ui:generator":[{"kind":"script","uri":"nested.js"},{"kind":"shader","uri":"sample.glsl"},{"kind":"image","uri":"future.png"}]}},"client_ref":"copied"},
        {"op":"delete","target":"before-use"},{"op":"remove_import","alias":"ui"}])
}

pub(super) fn request(store: &Store, id: &str, operations: Value, dry_run: bool) -> EditRequest {
    let metadata = service::document_metadata(store).unwrap();
    serde_json::from_value(json!({"request_id":id,"expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],"operations":operations,"dry_run":dry_run})).unwrap()
}

pub(crate) fn apply(store: &Store, id: &str, operations: Value) -> Value {
    service::apply_request(store, &request(store, id, operations, false)).unwrap()
}

pub(crate) fn recopy(
    source: &Store,
    source_id: &str,
    destination: &Path,
    prefix: &str,
) -> (Store, String) {
    fs::create_dir_all(destination).unwrap();
    write(
        &destination.join("main.pen"),
        json!({"version":source.read().unwrap().version(),
        "imports":{"source":source.path().to_str().unwrap()},"children":[
            {"id":"local-definitions","type":"frame","enabled":false,"children":[
                {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff00ff"}]}]}),
    );
    let target = Store::open(destination.join("main.pen"), Some(destination.join("data"))).unwrap();
    let response = apply(
        &target,
        "recopy-script",
        json!([
        {"op":"import_component","source":{"library_alias":"source","node_id":source_id},"mode":"copy",
            "options":{"variable_prefix":prefix,"component_prefix":prefix},"client_ref":"recopied"},
        {"op":"remove_import","alias":"source"}]),
    );
    let id = response["bindings"]["recopied"]
        .as_str()
        .unwrap()
        .to_owned();
    (target, id)
}

pub(super) fn linked(source: &Store, source_id: &str, destination: &Path) -> Store {
    fs::create_dir_all(destination).unwrap();
    write(
        &destination.join("main.pen"),
        json!({"version":source.read().unwrap().version(),
        "imports":{"source":source.path().to_str().unwrap()},"children":[
            {"id":"local-definitions","type":"frame","enabled":false,"children":[
                {"id":"badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff00ff"}]},
            {"id":"use","type":"ref","ref":format!("source:{source_id}")}]}),
    );
    Store::open(destination.join("main.pen"), Some(destination.join("data"))).unwrap()
}
