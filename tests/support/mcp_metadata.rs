//! Large metadata stays usable through the advertised stdio contracts.
use super::*;

#[test]
fn large_document_overviews_page_complete_entries_and_scope_their_cursors() {
    let (temp, file, data) = fixture();
    let mut document = json!({"version":"2.22","children":(0..10000).map(|index| json!({
        "id":format!("root{index}"),"type":"rectangle","name":"n".repeat(450),
        "width":1,"height":1,"fontSzie":12
    })).collect::<Vec<_>>()});
    let source = document.to_string();
    assert!(source.len() > 4 * 1024 * 1024);
    fs::write(&file, &source).unwrap();
    // Parsing and refreshing this oversized fixture can exceed the ordinary
    // response deadline when other consumers share the CI runner.
    let mut client = Client::start_with_response_timeout(&file, &data, Duration::from_secs(60));
    let overview = client.data("get_document", json!({}));
    assert!(overview.to_string().len() < 256 * 1024);
    assert_eq!(overview["node_count"], 10000);
    for collection in ["roots", "diagnostics"] {
        assert_eq!(overview["collections"][collection]["total"], 10000);
        assert_eq!(overview["collections"][collection]["returned"], 100);
        assert_eq!(overview["collections"][collection]["truncated"], true);
    }
    let first = client.data("get_document", json!({"collection":"roots","limit":500}));
    let cursor = first["next_cursor"].as_str().unwrap();
    assert!(cursor.starts_with(&format!("{}:", client.document_id.as_ref().unwrap())));
    let second = client.data(
        "get_document",
        json!({"collection":"roots","limit":500,"cursor":cursor}),
    );
    for (page, offset) in [(&first, 0), (&second, 500)] {
        assert_eq!(page["collection"], "roots");
        assert_eq!(page["total_matches"], 10000);
        assert_eq!(page["collections"]["roots"]["offset"], offset);
        assert_eq!(page["roots"].as_array().unwrap().len(), 500);
        assert_eq!(page["roots"][0]["id"], format!("root{offset}"));
        assert_eq!(page["roots"][0]["name"], "n".repeat(450));
        for key in [
            "revision",
            "dependency_revision",
            "runtime_dependency_revision",
        ] {
            assert_eq!(page[key], overview[key]);
        }
    }
    let diagnostics = client.data(
        "get_document",
        json!({"collection":"diagnostics","limit":1}),
    );
    assert_eq!(diagnostics["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(diagnostics["diagnostics"][0]["code"], "UNKNOWN_PROPERTY");
    let nodes = client.data("read_nodes", json!({"node_id":"root500","depth":0}));
    assert_eq!(nodes["nodes"][0]["id"], "root500");
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
    let other = Store::init(temp.path().join("other.pen"), Some(data)).unwrap();
    let opened = client.data("open_document", json!({"path":other.path()}));
    let crossed = client.call(
        "get_document",
        json!({"document_id":opened["document_id"],"collection":"roots","cursor":cursor}),
    );
    assert_eq!(
        crossed["structuredContent"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    let wrong_collection = client.call(
        "get_document",
        json!({"collection":"diagnostics","cursor":cursor}),
    );
    assert_eq!(
        wrong_collection["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    document["children"][0]["name"] = json!("changed");
    fs::write(&file, document.to_string()).unwrap();
    let stale = client.call(
        "get_document",
        json!({"collection":"roots","cursor":cursor}),
    );
    assert_eq!(
        stale["structuredContent"]["error"]["code"],
        "REVISION_CONFLICT"
    );
    client.finish();
}
