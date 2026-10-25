use lazypencil::{
    operations::EditRequest,
    project::{NodeAddress, Project, ResolvedNode},
    service,
    store::Store,
};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::TempDir;

pub const MODES: &[&str] = &[
    "inputs", "code", "metadata", "combined", "literal", "nullable",
];
pub const BASE: &str = r#"/**
 * @schema 2.21
 * @input selected: ref = "badge"
 * @input amount: number = 1
 */
return [{type:'ref',ref:pencil.input.selected},{type:'rectangle',x:12,width:10,height:10,fill:'$tone'},
 {type:'rectangle',x:24,width:pencil.input.amount*4,height:8,fill:'#ffffff'}];"#;
pub const UI: &str = r#"/**
 * @schema 2.21
 * @input selected: ref = "badge"
 * @input amount: number = 2
 */
return [{type:'ref',ref:pencil.input.selected},{type:'rectangle',x:12,width:10,height:10,fill:'$tone'},
 {type:'rectangle',x:24,width:pencil.input.amount*4,height:8,fill:'#ffffff'}];"#;

pub fn write(path: &Path, value: Value) {
    fs::write(path, value.to_string()).unwrap();
}

pub fn fixture(version: &str, mode: &str) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    fs::create_dir(temp.path().join("library")).unwrap();
    fs::write(temp.path().join("library/base.js"), BASE).unwrap();
    fs::write(temp.path().join("library/ui.js"), UI).unwrap();
    write(
        &temp.path().join("library/base.pen"),
        json!({"version":version,"variables":{
        "selected":{"type":"string","value":"badge"},"amount":{"type":"number","value":1},"tone":{"type":"color","value":"#ff0000"}},"children":[
        {"id":"badge","name":"Base badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff0000"},
        {"id":"generator","name":"Base generator","type":"script","reusable":true,"width":36,"height":12,"scriptUri":"base.js","inputs":{"selected":"$selected","amount":"$amount","stale":"$missing"},"metadata":{"type":"test","keep":"base"}},
        {"id":"card","name":"Base card","type":"frame","reusable":true,"layout":"none","width":36,"height":12,"children":[{"id":"draw","type":"ref","ref":"generator"}]}]}),
    );
    let mut changes = json!({});
    if ["inputs", "combined"].contains(&mode) {
        changes["inputs"] = json!({"selected":"$selected","amount":"$amount","stale":"$missing"});
    }
    if mode == "literal" {
        changes["inputs"] = json!({"selected":"badge","amount":2});
    }
    if mode == "nullable" {
        changes["inputs"] = json!({"selected":null,"amount":2});
    }
    if ["code", "combined", "nullable"].contains(&mode) {
        changes["scriptUri"] = json!("ui.js");
    }
    if ["metadata", "combined"].contains(&mode) {
        changes["metadata"] = json!({"type":"test","keep":"override","unknown":[1,2,3]});
    }
    write(
        &temp.path().join("library/ui.pen"),
        json!({"version":version,"imports":{"base":"base.pen"},"variables":{
        "selected":{"type":"string","value":"badge"},"amount":{"type":"number","value":3},"tone":{"type":"color","value":"#00ff00"}},"children":[
        {"id":"badge","name":"UI badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#00ff00"},
        {"id":"panel","name":"UI panel","type":"frame","reusable":true,"layout":"none","width":36,"height":12,"children":[
            {"id":"inner","type":"ref","ref":"base:card","descendants":{"draw":changes}},
            {"id":"backup","type":"script","enabled":false,"width":36,"height":12,"scriptUri":"ui.js"}]}]}),
    );
    write(
        &temp.path().join("main.pen"),
        json!({"version":version,"imports":{"ui":"library/ui.pen"},"variables":{"tone":{"type":"color","value":"#0000ff"}},"children":[
        {"id":"consumer-definitions","type":"frame","enabled":false,"children":[
        {"id":"badge","name":"Consumer badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#0000ff"}]},
        {"id":"use","type":"ref","ref":"ui:panel"}]}),
    );
    let store = Store::open(temp.path().join("main.pen"), Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

pub fn request(store: &Store, id: &str, operations: Value, dry_run: bool) -> EditRequest {
    let meta = service::document_metadata(store).unwrap();
    serde_json::from_value(json!({"request_id":id,"expected_revision":meta["revision"],"expected_dependency_revision":meta["dependency_revision"],"operations":operations,"dry_run":dry_run})).unwrap()
}
pub fn apply(store: &Store, id: &str, operations: Value) -> Value {
    service::apply_request(store, &request(store, id, operations, false))
        .unwrap_or_else(|error| panic!("request {id}: {error:?}"))
}
pub fn operations() -> Value {
    let mut variables = serde_json::Map::new();
    for (source, prefix) in [("ui", "u_"), ("ui:base", "b_")] {
        for name in ["selected", "amount", "tone"] {
            variables.insert(
                format!("{source}:{name}"),
                json!({"action":"rename","name":format!("{prefix}{name}")}),
            );
        }
    }
    json!([{"op":"import_component","source":{"library_alias":"ui","node_id":"panel"},"mode":"copy","options":{"component_prefix":"Copy ","variables":variables}},
        {"op":"delete","target":"use"},{"op":"remove_import","alias":"ui"}])
}

pub fn stored_override(store: &Store) -> Value {
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
pub fn active(store: &Store, id: &str) -> ResolvedNode {
    fn find(node: ResolvedNode) -> Option<ResolvedNode> {
        if node.kind() == "script" && node.properties.get("enabled") != Some(&json!(false)) {
            return Some(node);
        }
        for child in node.children {
            if let Some(node) = find(child) {
                return Some(node);
            }
        }
        None
    }
    find(
        Project::load(store)
            .unwrap()
            .expanded_node(&NodeAddress {
                node_id: id.into(),
                instance_path: vec![],
            })
            .unwrap(),
    )
    .unwrap()
}
pub fn recopy(store: &Store, component: &str, dir: &Path) -> (Store, String) {
    fs::create_dir_all(dir).unwrap();
    write(
        &dir.join("main.pen"),
        json!({"version":store.read().unwrap().version(),"imports":{"source":store.path()},"variables":{"tone":{"type":"color","value":"#ff00ff"}},"children":[
        {"id":"consumer-definitions","type":"frame","enabled":false,"children":[
        {"id":"badge","name":"Consumer badge","type":"rectangle","reusable":true,"width":10,"height":10,"fill":"#ff00ff"}]}]}),
    );
    let copied = Store::open(dir.join("main.pen"), Some(dir.join("data"))).unwrap();
    let result = apply(
        &copied,
        "recopy",
        json!([{"op":"import_component","source":{"library_alias":"source","node_id":component},"mode":"copy","options":{"component_prefix":"Again ","variable_prefix":"again_"}}, {"op":"remove_import","alias":"source"}]),
    );
    (
        copied,
        result["generated_ids"]["0"]["id"].as_str().unwrap().into(),
    )
}
