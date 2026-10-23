#![cfg(feature = "rendering")]

use lazypencil::{
    operations::EditRequest,
    service::{self, ReadNodesRequest},
    store::Store,
};
use serde_json::{Value, json};
use std::{fs, process::Command};
use tempfile::TempDir;

fn request(value: Value) -> ReadNodesRequest {
    serde_json::from_value(value).unwrap()
}

fn fixture(code: &str) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("design.pen");
    fs::write(temp.path().join("generate.js"), code).unwrap();
    fs::write(&file,json!({"version":"2.21","variables":{"accent":{"type":"color","value":"#FF0000"}},"children":[
        {"id":"script","type":"script","scriptUri":"generate.js","width":160,"height":80}
    ]}).to_string()).unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

fn snapshot(
    store: &Store,
) -> (
    lazypencil::project::Project,
    lazypencil::layout::LayoutSnapshot,
) {
    service::layout_snapshot_with_runtime(
        store,
        &[],
        None,
        None,
        &mut lazypencil::script::ScriptRuntime::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap()
}

const CODE: &str = "/** @schema 2.21 */ return [{type:'frame',name:'Generated group',layout:'none',width:160,height:80,children:[{type:'text',name:'Generated label',content:'生成された文字',fontSize:12,x:8,y:5,width:140,textGrowth:'fixed-width',fill:'$accent'},{type:'rectangle',name:'Generated tile',x:8,y:35,width:20,height:20,fill:'$accent'}]}];";

#[test]
fn calculated_reads_return_generated_properties_provenance_and_matching_geometry() {
    let (_temp, store) = fixture(CODE);
    let before = fs::read(store.path()).unwrap();
    let stored =
        service::read_store_nodes(&store, &[], &request(json!({"node_id":"script"}))).unwrap();
    assert_eq!(stored["view"], "stored");
    assert!(stored["nodes"][0].get("children").is_none());
    let (project, snapshot) = snapshot(&store);
    let result = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true,"node_id":"script","depth":3})),
        None,
    )
    .unwrap();
    assert_eq!(result["snapshot_id"], snapshot.id);
    assert_eq!(result["view"], "calculated");
    assert_eq!(result["variables_resolved"], true);
    let label = &result["nodes"][0]["children"][0]["children"][0];
    assert_eq!(label["properties"]["content"], "生成された文字");
    assert_eq!(label["properties"]["fill"], "#FF0000");
    assert_eq!(label["generated_by"]["node_id"], "script");
    assert_eq!(label["layout"]["rect"]["x"].as_f64(), Some(8.0));
    assert_eq!(label["layout"]["rect"]["y"].as_f64(), Some(5.0));
    let record = snapshot
        .items
        .iter()
        .find(|item| item.record.id == label["id"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        label["layout"],
        serde_json::to_value(&record.record).unwrap()
    );
    let exact = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"snapshot_id":snapshot.id,"node_id":label["id"],"depth":0})),
        None,
    )
    .unwrap();
    assert_eq!(exact["nodes"][0]["properties"], label["properties"]);
    assert_eq!(exact["nodes"][0]["children_count"], 0);
    let rendered = lazypencil::render::png(&project, &snapshot, &Default::default()).unwrap();
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(&rendered.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        image.peek_pixels().unwrap().get_color((10, 40)),
        skia_safe::Color::from_rgb(255, 0, 0)
    );
    let attempt:EditRequest=serde_json::from_value(json!({"request_id":"generated-is-read-only","expected_revision":snapshot.revision,"operations":[{"op":"update","target":label["id"],"set":{"fill":"#00FF00"}}]})).unwrap();
    assert!(service::apply_request(&store, &attempt).is_err());
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[test]
fn calculated_pagination_binds_the_snapshot_and_semantic_query_not_page_size_or_entry_flag() {
    let (_temp, store) = fixture(CODE);
    let (project, snapshot) = snapshot(&store);
    let first = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true,"depth":0,"limit":1})),
        None,
    )
    .unwrap();
    assert_eq!(first["total_matches"], 4);
    let next = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(
            json!({"snapshot_id":snapshot.id,"depth":0,"limit":3,"cursor":first["next_cursor"]}),
        ),
        None,
    )
    .unwrap();
    assert_eq!(next["nodes"].as_array().unwrap().len(), 3);
    assert!(next["next_cursor"].is_null());
    let filtered = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true,"name":"GENERATED LABEL","type":"text","depth":0})),
        None,
    )
    .unwrap();
    assert_eq!(filtered["total_matches"], 1);
    assert_eq!(
        filtered["nodes"][0]["properties"]["content"],
        "生成された文字"
    );
    let changed = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true,"depth":1,"cursor":first["next_cursor"]})),
        None,
    )
    .unwrap_err();
    assert_eq!(changed.code, "REVISION_CONFLICT");
    let unknown = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"snapshot_id":"unknown"})),
        None,
    )
    .unwrap_err();
    assert_eq!(unknown.code, "REVISION_CONFLICT");
}

#[test]
fn runtime_asset_changes_invalidate_calculated_reads() {
    let (temp, store) = fixture(
        "/** @schema 2.21 */ return [{type:'rectangle',width:20,height:20,fill:{type:'image',url:'runtime.png',mode:'contain'}}];",
    );
    let path = temp.path().join("runtime.png");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png"),
        &path,
    )
    .unwrap();
    let (project, snapshot) = snapshot(&store);
    assert!(!snapshot.runtime_dependencies.is_empty());
    service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true})),
        None,
    )
    .unwrap();
    fs::write(&path, b"changed after calculation").unwrap();
    let error = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true})),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "REVISION_CONFLICT");
}

#[test]
fn inherited_scripts_keep_library_themes_addresses_and_nested_generation_origins() {
    let temp = TempDir::new().unwrap();
    let library = temp.path().join("library");
    fs::create_dir(&library).unwrap();
    fs::write(library.join("outer.js"), "/** @schema 2.21 */ return [{type:'script',name:'Nested generator',scriptUri:'inner.js',width:40,height:20}];").unwrap();
    fs::write(library.join("inner.js"), "/** @schema 2.21 */ return [{type:'rectangle',name:'Library tile',width:40,height:20,fill:'$accent'}];").unwrap();
    let definition = library.join("ui.pen");
    fs::write(&definition, json!({"version":"2.21","themes":{"mode":["light","dark"]},
        "variables":{"accent":{"type":"color","value":[{"value":"#FFFFFF"},{"theme":{"mode":"dark"},"value":"#0000FF"}]}},
        "children":[{"id":"card","type":"frame","reusable":true,"width":40,"height":20,
            "children":[{"id":"chart","type":"script","scriptUri":"outer.js","width":40,"height":20}]}]}).to_string()).unwrap();
    let file = temp.path().join("design.pen");
    fs::write(&file,json!({"version":"2.21","imports":{"ui":"library/ui.pen"},"themes":{"mode":["light","dark"]},
        "variables":{"accent":{"type":"color","value":"#FF0000"}},
        "children":[{"id":"use","type":"ref","ref":"ui:card","theme":{"mode":"dark"}}]}).to_string()).unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    let definition = fs::canonicalize(definition).unwrap();
    let (project, snapshot) = snapshot(&store);
    let query =
        request(json!({"calculated":true,"node_id":"use","instance_path":["chart"],"depth":2}));
    let result = service::read_calculated_nodes(&project, &snapshot, &query, None).unwrap();
    let owner = &result["nodes"][0];
    assert_eq!(
        owner["address"],
        json!({"node_id":"use","instance_path":["chart"]})
    );
    assert_eq!(owner["source_id"], "chart");
    assert_eq!(owner["source_path"], definition.to_str().unwrap());
    let nested = &owner["children"][0];
    let tile = &nested["children"][0];
    assert_eq!(nested["generated_by"], owner["address"]);
    assert_eq!(tile["generated_by"], nested["address"]);
    assert_eq!(tile["properties"]["fill"], "#0000FF");
    assert_eq!(tile["theme"]["mode"], "dark");
    assert_eq!(
        tile["property_sources"]["fill"],
        definition.to_str().unwrap()
    );
    let selected = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"snapshot_id":snapshot.id,"node_id":tile["id"],"depth":0})),
        None,
    )
    .unwrap();
    assert_eq!(selected["nodes"][0]["properties"], tile["properties"]);
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        service::read_calculated_nodes(&project, &snapshot, &query, Some(&cancel))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    service::read_calculated_nodes(&project, &snapshot, &query, None).unwrap();
    fs::write(
        &definition,
        fs::read_to_string(&definition)
            .unwrap()
            .replace("#0000FF", "#00FF00"),
    )
    .unwrap();
    assert_eq!(
        service::read_calculated_nodes(&project, &snapshot, &query, None)
            .unwrap_err()
            .code,
        "REVISION_CONFLICT"
    );
}

#[test]
fn invalid_calculated_queries_and_cancellation_do_not_open_missing_code() {
    let (temp, store) = fixture(CODE);
    fs::remove_file(temp.path().join("generate.js")).unwrap();
    for value in [
        json!({"calculated":true,"depth":17}),
        json!({"calculated":true,"library_alias":"unread"}),
        json!({"calculated":true,"instance_path":["child"]}),
    ] {
        let error = service::read_store_nodes(&store, &[], &request(value)).unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENT");
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let error = service::read_store_nodes_with_cancellation(
        &store,
        &[],
        &request(json!({"calculated":true})),
        Some(&cancel),
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    let unavailable = service::read_store_nodes(
        &store,
        &[],
        &request(json!({"snapshot_id":"another-process"})),
    )
    .unwrap_err();
    assert_eq!(unavailable.code, "SNAPSHOT_UNAVAILABLE");
}

#[test]
fn large_overlapping_subtrees_hit_the_response_budget_and_a_smaller_read_recovers() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("large.pen");
    fs::write(&file,json!({"version":"2.21","children":[{"id":"root","type":"frame","width":1,"height":1,"children":[{"id":"leaf","type":"rectangle","width":1,"height":1,"name":"x".repeat(2*1024*1024)}]}]}).to_string()).unwrap();
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    let (project, snapshot) = snapshot(&store);
    let error = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true,"depth":2})),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_ARGUMENT");
    assert!(error.message.contains("4 MiB"));
    let reduced = service::read_calculated_nodes(
        &project,
        &snapshot,
        &request(json!({"calculated":true,"node_id":"root","depth":0})),
        None,
    )
    .unwrap();
    assert_eq!(reduced["nodes"][0]["children_count"], 1);
}

#[test]
fn actual_cli_calculated_inspection_executes_scripts_and_keeps_stored_reads_separate() {
    let (temp, store) = fixture(CODE);
    let original = fs::read(store.path()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(temp.path().join("cli-data"))
        .args(["inspect"])
        .arg(store.path())
        .args(["--calculated", "--node", "script", "--depth", "3"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["view"], "calculated");
    assert_eq!(
        result["nodes"][0]["children"][0]["children"][0]["properties"]["content"],
        "生成された文字"
    );
    assert_eq!(fs::read(store.path()).unwrap(), original);
    fs::write(
        temp.path().join("generate.js"),
        "/** @schema 2.21 */ while(true){}",
    )
    .unwrap();
    let stored =
        service::read_store_nodes(&store, &[], &request(json!({"node_id":"script"}))).unwrap();
    assert_eq!(stored["view"], "stored");
    assert!(stored["nodes"][0].get("children").is_none());
}
