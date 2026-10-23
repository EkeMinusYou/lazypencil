use lazypencil::{
    document::Document,
    operations::{self, EditRequest, Operation},
    project::{NodeAddress, Project},
    service,
    store::Store,
};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

fn batch(ops: Value) -> Vec<Operation> {
    serde_json::from_value(ops).unwrap()
}
fn fixture() -> Document {
    Document::parse(r##"{
      "version":"2.21",
      "themes":{"mode":["light","dark"],"density":["normal","compact"]},
      "variables":{"color":{"type":"color","value":[{"value":"#ffffff"},{"theme":{"mode":"dark"},"value":"#111111"}]}},
      "children":[
        {"id":"button","type":"frame","reusable":true,"fill":"$color","children":[
          {"id":"wrapper","type":"frame","children":[{"id":"label","type":"text","content":"Continue"}]}
        ]},
        {"id":"card","type":"frame","reusable":true,"children":[
          {"id":"action","type":"ref","ref":"button","descendants":{"label":{"content":"Inner"}}},
          {"id":"body","type":"frame","slot":["button"],"children":[]}
        ]},
        {"id":"first","type":"ref","ref":"card"},
        {"id":"second","type":"ref","ref":"card"}
      ]
    }"##.into()).unwrap()
}

fn resolve(document: &Document, id: &str, path: &[&str]) -> Value {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("scene.pen");
    fs::write(&file, document.text()).unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    let project = Project::load(&store).unwrap();
    let node = project
        .expanded_node(&NodeAddress {
            node_id: id.into(),
            instance_path: path.iter().map(|s| s.to_string()).collect(),
        })
        .unwrap();
    json!({"properties":node.properties,"children":node.children,"id":node.id()})
}

#[test]
fn nested_updates_only_write_the_selected_instances_override() {
    let before = fixture();
    let (after,_)=operations::apply(&before,&batch(json!([
        {"op":"update","target":{"node_id":"first","instance_path":["action","label"]},"set":{"content":"Save","fontSize":14}}
    ]))).unwrap();
    assert_eq!(after.node("label").unwrap()["content"], "Continue");
    assert_eq!(
        after.node("action").unwrap()["descendants"]["label"]["content"],
        "Inner"
    );
    assert_eq!(
        resolve(&after, "first", &["action", "label"])["properties"]["content"],
        "Save"
    );
    assert_eq!(
        resolve(&after, "second", &["action", "label"])["properties"]["content"],
        "Inner"
    );
    assert!(after.node("first").unwrap().get("content").is_none());
}

#[test]
fn resetting_properties_restores_inheritance_and_stale_paths_can_be_removed() {
    let (edited,_)=operations::apply(&fixture(),&batch(json!([
        {"op":"set_override","target":{"node_id":"first","instance_path":["action","label"]},"set":{"content":"Save","fontSize":14}},
        {"op":"reset_override","target":{"node_id":"first","instance_path":["action","label"]},"properties":["content"]}
    ]))).unwrap();
    assert_eq!(
        resolve(&edited, "first", &["action", "label"])["properties"]["content"],
        "Inner"
    );
    assert_eq!(
        resolve(&edited, "first", &["action", "label"])["properties"]["fontSize"],
        14
    );
    let (reset, _) = operations::apply(
        &edited,
        &batch(json!([
            {"op":"reset_override","target":{"node_id":"first","instance_path":["action","label"]}}
        ])),
    )
    .unwrap();
    assert!(
        reset.node("first").unwrap()["descendants"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    let mut stale = fixture();
    stale
        .set_property(
            "/children/2",
            "descendants",
            &json!({"gone":{"content":"stale"}}),
        )
        .unwrap();
    assert!(operations::apply(&stale, &[]).is_err());
    operations::apply(
        &stale,
        &batch(
            json!([{"op":"reset_override","target":{"node_id":"first","instance_path":["gone"]}}]),
        ),
    )
    .unwrap();
}

#[test]
fn replacements_allocate_owned_ids_and_can_be_updated_later_in_the_same_batch() {
    let (after,result)=operations::apply(&fixture(),&batch(json!([
        {"op":"replace_override","target":{"node_id":"first","instance_path":["action","label"]},"client_ref":"replacement",
         "node":{"type":"rectangle","width":20,"height":20,"fill":"#2563EB"}},
        {"op":"update","target":{"result_of":"replacement"},"set":{"width":24}}
    ]))).unwrap();
    let id = result["bindings"]["replacement"].as_str().unwrap();
    assert_eq!(after.node(id).unwrap()["width"], 24);
    assert_eq!(
        resolve(&after, "first", &["action", "label"])["properties"]["type"],
        "rectangle"
    );
    assert_eq!(
        resolve(&after, "second", &["action", "label"])["properties"]["type"],
        "text"
    );
    let (reset,_)=operations::apply(&after,&batch(json!([{"op":"reset_override","target":{"node_id":"first","instance_path":["action","label"]}}]))).unwrap();
    assert!(reset.node(id).is_err());
    assert_eq!(
        resolve(&reset, "first", &["action", "label"])["properties"]["content"],
        "Inner"
    );
}

#[test]
fn slot_children_replace_the_array_and_expose_generated_ids_for_following_operations() {
    let (after,result)=operations::apply(&fixture(),&batch(json!([
        {"op":"set_slot_children","target":{"node_id":"first","instance_path":["body"]},"client_refs":["custom"],
         "children":[{"id":"template","type":"text","content":"Original"}]},
        {"op":"update","target":{"result_of":"custom"},"set":{"content":"Changed"}}
    ]))).unwrap();
    let id = result["bindings"]["custom"].as_str().unwrap();
    assert_ne!(id, "template");
    assert_eq!(after.node(id).unwrap()["content"], "Changed");
    assert_eq!(
        resolve(&after, "first", &["body"])["children"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        resolve(&after, "second", &["body"])["children"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (empty,_)=operations::apply(&after,&batch(json!([{"op":"set_slot_children","target":{"node_id":"first","instance_path":["body"]},"children":[]}]))).unwrap();
    assert!(empty.node(id).is_err());
    assert!(
        resolve(&empty, "first", &["body"])["children"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn theme_changes_merge_and_unset_axes_without_overriding_other_instances() {
    let (after,_)=operations::apply(&fixture(),&batch(json!([
        {"op":"set_theme","target":{"node_id":"first","instance_path":["action"]},"set":{"mode":"dark","density":"compact"}},
        {"op":"set_theme","target":{"node_id":"first","instance_path":["action"]},"unset":["density"]}
    ]))).unwrap();
    assert_eq!(
        after.node("first").unwrap()["descendants"]["action"]["theme"],
        json!({"mode":"dark"})
    );
    assert!(after.node("second").unwrap().get("theme").is_none());
    assert!(after.node("action").unwrap().get("theme").is_none());
}

#[test]
fn invalid_structural_targets_and_duplicate_owned_ids_are_rejected() {
    let before = fixture();
    for operation in [
        json!({"op":"delete","target":{"node_id":"first","instance_path":["action"]}}),
        json!({"op":"move","target":{"node_id":"first","instance_path":["action"]}}),
        json!({"op":"update","target":{"node_id":"first","instance_path":["action"]},"set":{"ref":"button"}}),
        json!({"op":"set_slot_children","target":{"node_id":"first","instance_path":["action","label"]},"children":[]}),
        json!({"op":"set_override","target":{"node_id":"first","instance_path":["missing"]},"set":{"content":"No"}}),
    ] {
        assert!(operations::apply(&before, &batch(json!([operation]))).is_err());
    }
    let mut duplicate = before.value().clone();
    duplicate["children"][2]["descendants"] =
        json!({"body":{"children":[{"id":"label","type":"text"}]}});
    assert!(Document::parse(duplicate.to_string()).is_err());
    assert_eq!(before.text(), fixture().text());
}

#[test]
fn imported_components_can_be_overridden_without_writing_the_library() {
    let temp = TempDir::new().unwrap();
    let library = temp.path().join("ui.lib.pen");
    fs::write(&library, fixture().text()).unwrap();
    let library_before = fs::read(&library).unwrap();
    let main = temp.path().join("main.pen");
    fs::write(&main,json!({"version":"2.21","imports":{"ui":"ui.lib.pen"},"children":[{"id":"card","type":"ref","ref":"ui:card"}]}).to_string()).unwrap();
    let store = Store::open(&main, Some(temp.path().join("data"))).unwrap();
    let revision = store.read().unwrap().revision();
    let dependencies = Project::load(&store).unwrap().dependency_revision();
    let request:EditRequest=serde_json::from_value(json!({"request_id":"override","expected_revision":revision,"expected_dependency_revision":dependencies,
        "operations":[{"op":"update","target":{"node_id":"card","instance_path":["action","label"]},"set":{"content":"外部部品"}}]})).unwrap();
    service::apply_request(&store, &request).unwrap();
    assert_eq!(fs::read(library).unwrap(), library_before);
    assert_eq!(
        store.read().unwrap().node("card").unwrap()["descendants"]["action/label"]["content"],
        "外部部品"
    );
}

#[test]
fn copying_an_instance_remaps_its_owned_slot_content_and_internal_references() {
    let source = r#"{"version":"2.21","children":[
      {"id":"base","type":"frame","reusable":true,"children":[{"id":"slot","type":"frame","slot":[]}]},
      {"id":"instance","type":"ref","ref":"base","descendants":{"slot":{"children":[
        // keep this template comment
        {"id":"part","type":"rectangle","width":1.000e2,"height":10,"reusable":true},
        {"id":"use","type":"ref","ref":"part"}
      ]}}}
    ]}"#;
    let before = Document::parse(source.into()).unwrap();
    let (after, result) = operations::apply(
        &before,
        &batch(json!([{"op":"copy","target":"instance","client_ref":"copy"}])),
    )
    .unwrap();
    let copy = result["bindings"]["copy"].as_str().unwrap();
    let children = after.node(copy).unwrap()["descendants"]["slot"]["children"]
        .as_array()
        .unwrap();
    assert_ne!(children[0]["id"], "part");
    assert_eq!(children[1]["ref"], children[0]["id"]);
    assert_eq!(
        after
            .text()
            .matches("// keep this template comment")
            .count(),
        2
    );
    assert_eq!(after.text().matches("1.000e2").count(), 2);
}

#[test]
fn detaching_one_level_preserves_nested_refs_variables_and_other_instances() {
    let (before, _) = operations::apply(&fixture(), &batch(json!([
        {"op":"set_override","target":{"node_id":"first","instance_path":["action","label"]},"set":{"content":"Save"}},
        {"op":"set_override","target":"first","set":{"x":80,"theme":{"mode":"dark"}}}
    ]))).unwrap();
    let base = before.node("card").unwrap().clone();
    let (after, result) = operations::apply(
        &before,
        &batch(json!([
            {"op":"detach_instance","target":"first","client_ref":"detached"},
            {"op":"update","target":{"result_of":"detached"},"set":{"name":"Detached card"}}
        ])),
    )
    .unwrap();
    let first = after.node("first").unwrap();
    assert_eq!(first["type"], "frame");
    assert_eq!(first["x"], 80);
    assert_eq!(first["theme"]["mode"], "dark");
    assert!(first.get("ref").is_none());
    assert!(first.get("reusable").is_none());
    assert!(first.get("descendants").is_none());
    let action = &first["children"][0];
    assert_eq!(action["type"], "ref");
    assert_eq!(action["ref"], "button");
    assert_ne!(action["id"], "action");
    assert_eq!(action["descendants"]["label"]["content"], "Save");
    assert_eq!(after.node("button").unwrap()["fill"], "$color");
    assert_eq!(after.node("card").unwrap(), &base);
    assert_eq!(after.node("second").unwrap()["type"], "ref");
    assert_eq!(result["bindings"]["detached"], "first");
    let (changed, _) = operations::apply(
        &after,
        &batch(json!([
            {"op":"update","target":"label","set":{"fontSize":18}},
            {"op":"set_variable","name":"color","definition":{"type":"color","value":"#222222"}}
        ])),
    )
    .unwrap();
    let id = action["id"].as_str().unwrap();
    assert_eq!(
        resolve(&changed, id, &["label"])["properties"]["fontSize"],
        18
    );
    assert_eq!(
        resolve(&changed, id, &["label"])["properties"]["content"],
        "Save"
    );
}

#[test]
fn detach_remaps_owned_slot_definitions_and_keeps_internal_links() {
    let (with_slot,_) = operations::apply(&fixture(),&batch(json!([
        {"op":"set_slot_children","target":{"node_id":"first","instance_path":["body"]},"children":[
            {"id":"custom","type":"rectangle","reusable":true,"width":20,"fill":"$color"},
            {"id":"use","type":"ref","ref":"custom"}
        ]}
    ]))).unwrap();
    let old = with_slot.node("first").unwrap()["descendants"]["body"]["children"][0]["id"]
        .as_str()
        .unwrap();
    let (after, _) = operations::apply(
        &with_slot,
        &batch(json!([{"op":"detach_instance","target":"first"}])),
    )
    .unwrap();
    let children = &after.node("first").unwrap()["children"][1]["children"];
    assert_ne!(children[0]["id"], old);
    assert_eq!(children[1]["ref"], children[0]["id"]);
    assert_eq!(children[0]["fill"], "$color");
    assert!(after.node(old).is_err());
}

#[test]
fn detach_keeps_replacement_paths_valid_and_ref_definition_chains_materialize_once() {
    let before=Document::parse(r#"{"version":"2.21","children":[
        {"id":"button","type":"frame","reusable":true,"children":[{"id":"label","type":"text","content":"Base"}]},
        {"id":"base","type":"frame","reusable":true,"children":[{"id":"action","type":"ref","ref":"button"}]},
        {"id":"variant","type":"ref","ref":"base","reusable":true,"descendants":{"action":{"id":"replacement","type":"ref","ref":"button"}}},
        {"id":"instance","type":"ref","ref":"variant","descendants":{"action/label":{"content":"Outer"}}}
    ]}"#.into()).unwrap();
    let (after, _) = operations::apply(
        &before,
        &batch(json!([{"op":"detach_instance","target":"instance"}])),
    )
    .unwrap();
    let child = &after.node("instance").unwrap()["children"][0];
    assert_eq!(child["type"], "ref");
    assert_eq!(child["ref"], "button");
    assert_eq!(child["descendants"]["label"]["content"], "Outer");
    assert_eq!(
        resolve(&after, child["id"].as_str().unwrap(), &["label"])["properties"]["content"],
        "Outer"
    );
}

#[test]
fn detaching_an_inherited_instance_folds_outer_overrides_and_keeps_siblings_linked() {
    let (before, _) = operations::apply(&fixture(), &batch(json!([
        {"op":"set_override","target":{"node_id":"first","instance_path":["action","label"]},"set":{"content":"Save","fontSize":18}},
        {"op":"set_override","target":{"node_id":"first","instance_path":["body"]},"set":{"name":"Keep this sibling"}}
    ]))).unwrap();
    let original = before.node("card").unwrap().clone();
    let (after, result) = operations::apply(&before, &batch(json!([
        {"op":"detach_instance","target":{"node_id":"first","instance_path":["action"]},"client_ref":"button"},
        {"op":"update","target":{"result_of":"button"},"set":{"name":"Detached action"}}
    ]))).unwrap();
    let first = after.node("first").unwrap();
    assert_eq!(first["type"], "ref");
    assert_eq!(after.node("card").unwrap(), &original);
    let detached = &first["descendants"]["action"];
    assert_eq!(detached["type"], "frame");
    assert_eq!(detached["name"], "Detached action");
    assert_eq!(detached["fill"], "$color");
    assert!(detached.get("ref").is_none());
    assert!(detached.get("reusable").is_none());
    assert!(first["descendants"].get("action/label").is_none());
    assert_eq!(first["descendants"]["body"]["name"], "Keep this sibling");
    let label = &detached["children"][0]["children"][0];
    assert_ne!(label["id"], "label");
    assert_eq!(label["content"], "Save");
    assert_eq!(label["fontSize"], 18);
    assert_eq!(result["bindings"]["button"], detached["id"]);
    assert_eq!(
        result["generated_ids"]["0"]["address"]["instance_path"],
        json!(["action"])
    );
    assert_eq!(
        resolve(&after, "second", &["action", "label"])["properties"]["content"],
        "Inner"
    );
    assert_eq!(
        resolve(&after, "first", &["action"])["properties"]["name"],
        "Detached action"
    );
    let (changed, _) = operations::apply(
        &after,
        &batch(json!([
            {"op":"update","target":"label","set":{"content":"Global change","fontSize":24}}
        ])),
    )
    .unwrap();
    assert_eq!(
        changed.node(label["id"].as_str().unwrap()).unwrap()["content"],
        "Save"
    );
    assert_eq!(
        resolve(&changed, "second", &["action", "label"])["properties"]["fontSize"],
        24
    );
}

#[test]
fn detach_traverses_nested_instances_replacements_and_preserves_nested_links() {
    let before = Document::parse(json!({"version":"2.21","children":[
        {"id":"icon","type":"rectangle","reusable":true,"width":8,"height":8,"fill":"#ff0000"},
        {"id":"button","type":"frame","reusable":true,"children":[{"id":"glyph","type":"ref","ref":"icon"}]},
        {"id":"panel","type":"frame","reusable":true,"children":[{"id":"action","type":"ref","ref":"button"}]},
        {"id":"page","type":"frame","reusable":true,"children":[{"id":"panel-use","type":"ref","ref":"panel"}]},
        {"id":"variant","type":"ref","ref":"page","reusable":true,"descendants":{
            "panel-use":{"type":"ref","id":"replacement","ref":"panel"}}},
        {"id":"use","type":"ref","ref":"variant","descendants":{
            "panel-use/action":{"name":"Customized button"},
            "panel-use/action/glyph":{"width":12}}}
    ]}).to_string()).unwrap();
    let (after, result) = operations::apply(&before, &batch(json!([
        {"op":"detach_instance","target":{"node_id":"use","instance_path":["panel-use","action"]},"client_ref":"detached"}
    ]))).unwrap();
    let id = result["bindings"]["detached"].as_str().unwrap();
    let detached = after.node(id).unwrap();
    assert_eq!(detached["type"], "frame");
    assert_eq!(detached["name"], "Customized button");
    assert_eq!(detached["children"][0]["type"], "ref");
    assert_eq!(detached["children"][0]["ref"], "icon");
    assert_eq!(detached["children"][0]["width"], 12);
    assert!(
        after.node("use").unwrap()["descendants"]
            .get("panel-use/action/glyph")
            .is_none()
    );
    assert_eq!(
        resolve(&after, "use", &["panel-use", "action"])["properties"]["name"],
        "Customized button"
    );
    assert_eq!(
        resolve(&after, detached["children"][0]["id"].as_str().unwrap(), &[])["properties"]["width"],
        12
    );
    let error = operations::apply(&after, &batch(json!([
        {"op":"detach_instance","target":{"node_id":"use","instance_path":["panel-use","action"]}}
    ]))).unwrap_err();
    assert_eq!(error.code, "INVALID_OPERATION");
}

fn store_fixture() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("scene.pen");
    fs::write(&file, fixture().text()).unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn read(store: &Store, query: Value) -> lazypencil::Result<Value> {
    service::read_store_nodes(store, &[], &serde_json::from_value(query).unwrap())
}

#[test]
fn inherited_reads_include_addresses_origins_and_bounded_children() {
    let (_temp, store) = store_fixture();
    let value = read(
        &store,
        json!({"node_id":"first","instance_path":["action","label"],"depth":0}),
    )
    .unwrap();
    assert_eq!(value["view"], "expanded");
    let node = &value["nodes"][0];
    assert_eq!(
        node["address"],
        json!({"node_id":"first","instance_path":["action","label"]})
    );
    assert_eq!(node["properties"]["content"], "Inner");
    assert_eq!(node["source_id"], "label");
    assert_eq!(node["property_sources"]["content"], json!(store.path()));
    let root = read(
        &store,
        json!({"node_id":"first","expand_instances":true,"depth":0}),
    )
    .unwrap();
    assert_eq!(root["nodes"][0]["children_count"], 2);
    assert!(root["nodes"][0].get("children").is_none());
    assert_eq!(root["nodes"][0]["properties"]["type"], "frame");
    assert!(read(&store, json!({"instance_path":["action"]})).is_err());
    assert!(
        read(
            &store,
            json!({"node_id":"first","instance_path":["missing"]})
        )
        .is_err()
    );
}

#[test]
fn raw_node_catalog_includes_owned_overrides_and_depth_bounds_them() {
    let (with_nodes,result)=operations::apply(&fixture(),&batch(json!([
        {"op":"replace_override","target":{"node_id":"first","instance_path":["action","label"]},"node":{"type":"rectangle","width":20},"client_ref":"replacement"},
        {"op":"set_slot_children","target":{"node_id":"first","instance_path":["body"]},"children":[{"type":"text","content":"Slot"}],"client_refs":["child"]}
    ]))).unwrap();
    let found = service::read_nodes(&with_nodes, None, None, None, 0, 100, None).unwrap();
    let ids = found["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].clone())
        .collect::<Vec<_>>();
    assert!(ids.contains(&result["bindings"]["replacement"]));
    assert!(ids.contains(&result["bindings"]["child"]));
    let shallow = service::read_nodes(&with_nodes, Some("first"), None, None, 0, 1, None).unwrap();
    assert_eq!(shallow["nodes"][0]["descendant_overrides_count"], 2);
    assert!(shallow["nodes"][0].get("descendants").is_none());
    let bounded = service::read_nodes(&with_nodes, Some("first"), None, None, 1, 1, None).unwrap();
    assert_eq!(
        bounded["nodes"][0]["descendants"]["body"]["children_count"],
        1
    );
    assert!(
        bounded["nodes"][0]["descendants"]["body"]
            .get("children")
            .is_none()
    );
}

#[test]
fn library_queries_are_read_only_and_pagination_rejects_changed_dependencies() {
    let (temp, store) = store_fixture();
    let library = temp.path().join("ui.lib.pen");
    fs::write(&library, fixture().text()).unwrap();
    let mut main = store.read().unwrap();
    main.set_property("", "imports", &json!({"ui":"ui.lib.pen"}))
        .unwrap();
    fs::write(store.path(), main.text()).unwrap();
    let library_before = fs::read(&library).unwrap();
    let page = read(&store, json!({"library_alias":"ui","limit":1,"depth":0})).unwrap();
    assert_eq!(page["nodes"][0]["id"], "button");
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = read(
        &store,
        json!({"library_alias":"ui","limit":2,"depth":0,"cursor":cursor}),
    )
    .unwrap();
    assert_eq!(next["nodes"].as_array().unwrap().len(), 2);
    let child = read(
        &store,
        json!({"library_alias":"ui","node_id":"first","instance_path":["action","label"]}),
    )
    .unwrap();
    assert_eq!(
        child["nodes"][0]["source_path"],
        json!(lazypencil::paths::canonical(&library).unwrap())
    );
    assert!(read(&store, json!({"library_alias":"undeclared"})).is_err());
    assert_eq!(fs::read(&library).unwrap(), library_before);
    fs::write(&library, fixture().text().replace("Continue", "Changed")).unwrap();
    assert_eq!(
        read(
            &store,
            json!({"library_alias":"ui","limit":1,"depth":0,"cursor":cursor})
        )
        .unwrap_err()
        .code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn missing_assets_do_not_prevent_reading_stored_nodes_for_repairs() {
    let (_temp, store) = store_fixture();
    let mut document = store.read().unwrap();
    document
        .set_property(
            "/children/0",
            "fill",
            &json!({"type":"image","url":"missing.png"}),
        )
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let value = read(&store, json!({"node_id":"button"})).unwrap();
    assert_eq!(value["nodes"][0]["id"], "button");
    assert!(value.get("dependency_error").is_none());
    assert_eq!(value["assets_complete"], false);
    assert_eq!(value["asset_diagnostics"][0]["code"], "ASSET_NOT_FOUND");
    assert!(value["dependency_revision"].is_string());
}

#[test]
fn import_edits_are_guarded_and_reject_dangling_references_atomically() {
    let (temp, store) = store_fixture();
    let library = temp.path().join("ui.lib.pen");
    fs::write(&library, fixture().text()).unwrap();
    let before = store.read().unwrap();
    let request: EditRequest = serde_json::from_value(
        json!({"request_id":"add-import","expected_revision":before.revision(),
        "operations":[{"op":"add_import","alias":"ui","uri":"ui.lib.pen"}]}),
    )
    .unwrap();
    let result = service::apply_request(&store, &request).unwrap();
    let revision = result["revision"].as_str().unwrap();
    let dependency = result["dependency_revision"].as_str().unwrap();
    let inserted:EditRequest=serde_json::from_value(json!({"request_id":"link","expected_revision":revision,"expected_dependency_revision":dependency,
        "operations":[{"op":"insert","node":{"type":"ref","ref":"ui:card"},"client_ref":"instance"}]})).unwrap();
    let result = service::apply_request(&store, &inserted).unwrap();
    let original = fs::read(store.path()).unwrap();
    let removal:EditRequest=serde_json::from_value(json!({"request_id":"dangling","expected_revision":result["revision"],"expected_dependency_revision":result["dependency_revision"],
        "operations":[{"op":"remove_import","alias":"ui"}]})).unwrap();
    assert!(service::apply_request(&store, &removal).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), original);
    let id = result["bindings"]["instance"].as_str().unwrap();
    let removal:EditRequest=serde_json::from_value(json!({"request_id":"remove","expected_revision":result["revision"],"expected_dependency_revision":result["dependency_revision"],
        "operations":[{"op":"delete","target":id},{"op":"remove_import","alias":"ui"}]})).unwrap();
    service::apply_request(&store, &removal).unwrap();
    assert!(
        store.read().unwrap().value()["imports"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read_to_string(&library).unwrap(), fixture().text());
}

#[test]
fn transitive_library_definitions_expose_a_usable_api_alias() {
    let (temp, store) = store_fixture();
    fs::write(temp.path().join("leaf.pen"), fixture().text()).unwrap();
    fs::write(
        temp.path().join("ui.pen"),
        r#"{"version":"2.21","imports":{"leaf":"leaf.pen"},"children":[]}"#,
    )
    .unwrap();
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"ui":"ui.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let project = Project::load(&store).unwrap();
    let components = project.components();
    let component = components
        .iter()
        .find(|c| c["node_id"] == "button" && c["local"] == false)
        .unwrap();
    assert_eq!(component["library_alias"], "ui:leaf");
    let value = read(
        &store,
        json!({"library_alias":component["library_alias"],"node_id":"button","depth":0}),
    )
    .unwrap();
    assert_eq!(value["nodes"][0]["id"], "button");
}

#[test]
fn real_cli_reads_inherited_addresses_and_declared_library_nodes() {
    let (temp, store) = store_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "inspect"])
        .arg(store.path())
        .args([
            "--node",
            "first",
            "--instance-path",
            "action/label",
            "--depth",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["nodes"][0]["properties"]["content"], "Inner");
    fs::write(temp.path().join("ui.pen"), fixture().text()).unwrap();
    let mut document = store.read().unwrap();
    document
        .set_property("", "imports", &json!({"ui":"ui.pen"}))
        .unwrap();
    fs::write(store.path(), document.text()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "inspect"])
        .arg(store.path())
        .args(["--library-alias", "ui", "--node", "button", "--depth", "0"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["nodes"][0]["id"], "button");
}
