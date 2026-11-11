//! Search/detail contracts through the real MCP transport.
use super::*;

fn detail_query(page: &Value, candidate: &Value) -> Value {
    let mut query = candidate["read_target"].clone();
    if let Some(document_id) = page.get("document_id") {
        query["document_id"] = document_id.clone();
    }
    if let Some(snapshot) = page.get("snapshot_id") {
        query["snapshot_id"] = snapshot.clone();
    } else if page["view"] == "expanded" {
        query["expand_instances"] = json!(true);
    }
    if !page["library_alias"].is_null() {
        query["library_alias"] = page["library_alias"].clone();
    }
    query
}

#[test]
fn node_search_is_shallow_scoped_and_reads_exact_targets_without_executing_scripts() {
    let (temp, file, data) = fixture();
    let long_name = format!("{}TAIL", "日本語\"\\".repeat(1000));
    let source = json!({"version":"2.22","children":[
        {"id":"root","type":"frame","name":"GENERAL","width":100,"height":100,"children":[
            {"id":"label","type":"text","name":long_name,"content":"Only in details"},
            {"id":"script","type":"script","name":"General script","scriptUri":"broken.js","width":10,"height":10}]},
        {"id":"outside","type":"frame","name":"general"}
    ]}).to_string();
    fs::write(&file, &source).unwrap();
    fs::write(temp.path().join("broken.js"), "This is not JavaScript").unwrap();
    let mut client = Client::start(&file, &data);
    let page = client.data("search_nodes", json!({"name":"gEnErAl","type":"frame"}));
    assert_eq!(page["total_matches"], 2);
    assert_eq!(page["view"], "stored");
    assert!(page.get("dependencies").is_none());
    for candidate in page["nodes"].as_array().unwrap() {
        assert!(candidate.get("properties").is_none());
        assert!(candidate.get("property_sources").is_none());
        assert!(candidate.get("layout").is_none());
        assert!(candidate.get("children").is_none());
        assert!(candidate["bounds"].is_null());
        let detail = client.data("read_nodes", detail_query(&page, candidate));
        assert_eq!(detail["nodes"][0]["id"], candidate["id"]);
        assert!(detail.get("total_matches").is_none());
        assert!(detail.get("next_cursor").is_none());
        assert!(detail["nodes"][0].get("children").is_none());
    }
    let scoped = client.data("search_nodes", json!({"root":{"node_id":"root"}}));
    assert_eq!(scoped["total_matches"], 3);
    assert_eq!(scoped["nodes"][0]["id"], "root");
    assert_eq!(scoped["nodes"][1]["parent"]["node_id"], "root");
    let named = client.data("search_nodes", json!({"name":"TAIL"}));
    assert_eq!(named["total_matches"], 1);
    assert_eq!(named["nodes"][0]["name_truncated"], true);
    assert_eq!(
        named["nodes"][0]["name"].as_str().unwrap().chars().count(),
        256
    );
    let detail = client.data("read_nodes", detail_query(&named, &named["nodes"][0]));
    assert_eq!(detail["nodes"][0]["name"], long_name);
    assert_eq!(detail["nodes"][0]["content"], "Only in details");
    let missing = client.call("search_nodes", json!({"root":{"node_id":"absent"}}));
    assert_eq!(missing["isError"], true);
    for query in [
        json!({}),
        json!({"name":"General"}),
        json!({"node_id":"root","limit":1}),
        json!({"node_id":"root","cursor":"old"}),
        json!({"node_id":"root","type":"frame"}),
    ] {
        let invalid = client.call("read_nodes", query);
        assert_eq!(
            invalid["structuredContent"]["error"]["code"],
            "INVALID_ARGUMENT"
        );
    }
    let tools = client.request("tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let read = tools.iter().find(|t| t["name"] == "read_nodes").unwrap();
    assert!(
        read["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("node_id"))
    );
    assert!(read["inputSchema"]["properties"].get("name").is_none());
    let search = tools.iter().find(|t| t["name"] == "search_nodes").unwrap();
    assert_eq!(search["annotations"]["readOnlyHint"], true);
    client.finish();
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
}

#[test]
fn node_search_pages_by_bytes_with_exact_totals_and_rejects_nonprogressing_entries() {
    let (_temp, file, data) = fixture();
    let children: Vec<_> = (0..45).map(|i| json!({"id":format!("{i}-{}", "\\\"".repeat(1200)),"type":"rectangle","name":"Candidate"})).collect();
    fs::write(
        &file,
        json!({"version":"2.22","children":children}).to_string(),
    )
    .unwrap();
    let mut client = Client::start(&file, &data);
    let mut query = json!({"limit":100});
    let mut ids = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        let page = client.data("search_nodes", query.clone());
        assert!(serde_json::to_vec(&page).unwrap().len() <= 64 * 1024);
        assert_eq!(page["total_matches"], 45);
        let nodes = page["nodes"].as_array().unwrap();
        assert!(!nodes.is_empty());
        for node in nodes {
            assert!(ids.insert(node["id"].as_str().unwrap().to_owned()));
        }
        pages += 1;
        assert!(pages <= 45);
        if page["next_cursor"].is_null() {
            break;
        }
        let wrong = client.call(
            "search_nodes",
            json!({"name":"different","cursor":page["next_cursor"]}),
        );
        assert_eq!(
            wrong["structuredContent"]["error"]["code"],
            "REVISION_CONFLICT"
        );
        query["cursor"] = page["next_cursor"].clone();
    }
    assert!(pages > 1);
    assert_eq!(ids.len(), 45);
    let page = client.data("search_nodes", json!({"limit":1}));
    let edited=client.data("apply_operations",json!({"request_id":"change","expected_revision":page["revision"],"expected_dependency_revision":page["dependency_revision"],"operations":[{"op":"insert","node":{"type":"rectangle","name":"new"}}]}));
    assert_ne!(edited["revision"], page["revision"]);
    let stale = client.call(
        "search_nodes",
        json!({"limit":2,"cursor":page["next_cursor"]}),
    );
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    client.finish();
    fs::write(
        &file,
        json!({"version":"2.22","children":[{"id":"x".repeat(70000),"type":"rectangle"}]})
            .to_string(),
    )
    .unwrap();
    let mut client = Client::start(&file, &data);
    let failure = client.call("search_nodes", json!({}));
    assert_eq!(
        failure["structuredContent"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    assert!(
        failure["structuredContent"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("64 KiB")
    );
    client.finish();
}

#[test]
fn node_search_expanded_and_library_targets_round_trip_without_guessing_ids() {
    let (temp, file, data) = fixture();
    fs::write(temp.path().join("ui.pen"),json!({"version":"2.22","children":[{"id":"card","type":"frame","reusable":true,"width":100,"height":60,"children":[{"id":"label","type":"text","name":"Library label","content":"Original"}]}]}).to_string()).unwrap();
    fs::write(&file,json!({"version":"2.22","imports":{"ui":"ui.pen"},"children":[{"id":"instance","type":"ref","ref":"ui:card","descendants":{"label":{"content":"Override"}}}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let page = client.data(
        "search_nodes",
        json!({"expand_instances":true,"name":"label"}),
    );
    assert_eq!(page["total_matches"], 1);
    assert_eq!(page["nodes"][0]["origin"], "inherited");
    assert_eq!(
        page["nodes"][0]["read_target"],
        json!({"node_id":"instance","instance_path":["label"]})
    );
    let detail = client.data("read_nodes", detail_query(&page, &page["nodes"][0]));
    assert_eq!(detail["nodes"][0]["properties"]["content"], "Override");
    let root = client.data(
        "search_nodes",
        json!({"root":page["nodes"][0]["read_target"]}),
    );
    assert_eq!(root["view"], "expanded");
    assert_eq!(root["total_matches"], 1);
    let library = client.data("search_nodes", json!({"library_alias":"ui","name":"label"}));
    let detail = client.data("read_nodes", detail_query(&library, &library["nodes"][0]));
    assert_eq!(detail["nodes"][0]["content"], "Original");
    let missing = client.call(
        "search_nodes",
        json!({"expand_instances":true,"root":{"node_id":"missing"}}),
    );
    assert_eq!(
        missing["structuredContent"]["error"]["code"],
        "NODE_NOT_FOUND"
    );
    let mut document: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    document["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"broken","type":"ref","ref":"ui:missing"}));
    fs::write(&file, document.to_string()).unwrap();
    let scoped = client.data(
        "search_nodes",
        json!({"expand_instances":true,"root":{"node_id":"instance"}}),
    );
    assert_eq!(scoped["total_matches"], 2);
    client.finish();
}

#[cfg(feature = "rendering")]
#[test]
fn node_search_calculated_targets_reuse_snapshots_and_preserve_generated_owners() {
    let (temp, file, data) = fixture();
    fs::write(temp.path().join("generate.js"),"/** @schema 2.22 */ return [{type:'rectangle',name:'Generated tile',x:5,y:7,width:12,height:8,fill:'#00FF00'}];").unwrap();
    fs::write(&file,json!({"version":"2.22","children":[{"id":"script","type":"script","scriptUri":"generate.js","x":100,"y":200,"width":40,"height":30},{"id":"hidden","type":"rectangle","enabled":false,"width":10,"height":10}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let page = client.data("search_nodes", json!({"calculated":true,"name":"tile"}));
    assert_eq!(page["total_matches"], 1);
    assert!(page.get("script_executions").is_none());
    assert!(page.get("dependencies").is_none());
    let node = &page["nodes"][0];
    assert_eq!(node["origin"], "generated");
    assert_eq!(node["generated_by"]["node_id"], "script");
    assert_eq!(node["bounds"]["x"], 105.0);
    assert_eq!(node["bounds"]["y"], 207.0);
    let detail = client.data("read_nodes", detail_query(&page, node));
    assert_eq!(detail["snapshot_id"], page["snapshot_id"]);
    assert_eq!(detail["nodes"][0]["properties"]["fill"], "#00FF00");
    assert_eq!(
        detail["nodes"][0]["layout"]["visual_bounds"],
        node["bounds"]
    );
    let scoped = client.data(
        "search_nodes",
        json!({"snapshot_id":page["snapshot_id"],"root":node["read_target"]}),
    );
    assert_eq!(scoped["total_matches"], 1);
    assert_eq!(scoped["nodes"][0]["id"], node["id"]);
    let hidden = client.data(
        "search_nodes",
        json!({"snapshot_id":page["snapshot_id"],"root":{"node_id":"hidden"}}),
    );
    assert_eq!(hidden["nodes"][0]["enabled"], false);
    for args in [
        json!({"calculated":true,"expand_instances":true}),
        json!({"calculated":true,"library_alias":"ui"}),
    ] {
        let invalid = client.call("search_nodes", args);
        assert_eq!(
            invalid["structuredContent"]["error"]["code"],
            "INVALID_ARGUMENT"
        );
    }
    let other = Store::init(temp.path().join("other.pen"), Some(data.clone())).unwrap();
    let opened = client.data("open_document", json!({"path":other.path()}));
    let crossed = client.call(
        "search_nodes",
        json!({"document_id":opened["document_id"],"snapshot_id":page["snapshot_id"]}),
    );
    assert_eq!(
        crossed["structuredContent"]["error"]["code"],
        "SNAPSHOT_UNAVAILABLE"
    );
    fs::write(
        temp.path().join("generate.js"),
        "/** @schema 2.22 */ return []; ",
    )
    .unwrap();
    let stale = client.call("read_nodes", detail_query(&page, node));
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    client.finish();
}

#[test]
fn node_search_stored_repairs_capture_absent_dependencies_and_disclose_capture_failures() {
    let (temp, file, data) = fixture();
    fs::write(&file,json!({"version":"2.22","imports":{"ui":"missing.pen"},"children":[{"id":"one","type":"ref","ref":"ui:card"},{"id":"two","type":"rectangle"}]}).to_string()).unwrap();
    let mut client = Client::start(&file, &data);
    let page = client.data("search_nodes", json!({"limit":1}));
    assert_eq!(page["total_matches"], 2);
    assert!(page["dependency_revision"].is_string());
    assert_ne!(page["status"]["assets_complete"], true);
    assert_eq!(page["status"]["asset_diagnostic_count"], 1);
    assert_eq!(page["status"]["details_tool"], "get_document");
    let detail = client.data("read_nodes", detail_query(&page, &page["nodes"][0]));
    assert_eq!(detail["nodes"][0]["ref"], "ui:card");
    let continuation = client.data(
        "search_nodes",
        json!({"limit":1,"cursor":page["next_cursor"]}),
    );
    assert_eq!(continuation["nodes"][0]["id"], "two");
    fs::write(
        temp.path().join("missing.pen"),
        json!({"version":"2.22","children":[{"id":"card","type":"frame","reusable":true}]})
            .to_string(),
    )
    .unwrap();
    let stale = client.call(
        "search_nodes",
        json!({"limit":1,"cursor":page["next_cursor"]}),
    );
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    fs::write(temp.path().join("lazypencil.toml"), "not valid TOML = [").unwrap();
    let unavailable = client.data("search_nodes", json!({"limit":1}));
    assert!(unavailable["dependency_revision"].is_null());
    assert!(unavailable["status"]["dependency_error"].is_string());
    let continuation = client.call(
        "search_nodes",
        json!({"limit":1,"cursor":unavailable["next_cursor"]}),
    );
    assert_eq!(
        continuation["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    client.finish();
}
