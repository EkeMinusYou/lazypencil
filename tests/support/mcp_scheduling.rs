//! Exercise independent native owners through the real stdio transport.
use super::*;
use std::collections::HashSet;

fn wait_for(client: &mut Client, wanted: u64, abandoned: &HashSet<u64>) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let message = client
            .messages
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("Scheduled request did not finish");
        if message["id"] == wanted {
            return message["result"].clone();
        }
        if let Some(id) = message["id"].as_u64() {
            assert!(abandoned.contains(&id), "Unexpected response: {message}");
        }
    }
}
fn finish(client: &mut Client, abandoned: &HashSet<u64>) {
    client.input.take();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = client.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Connection shutdown did not reap document workers"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for message in client.messages.try_iter() {
        assert!(
            message["id"]
                .as_u64()
                .is_none_or(|id| abandoned.contains(&id)),
            "Unexpected response: {message}"
        );
    }
}
fn wait_for_http(server: &http_image_reference::Server) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.network_counts().0 == 0 {
        assert!(
            Instant::now() < deadline,
            "Acquisition never reached HTTP fixture"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn pending_inputs_are_bounded_without_blocking_other_documents_or_close() {
    for (requests, payload_bytes, scope) in [
        (32, 0, "document_requests"),
        (17, 1024 * 1024, "document_input_bytes"),
    ] {
        let temp = TempDir::new().unwrap();
        let server = http_image_reference::Server::new();
        let a = temp.path().join("a.pen");
        fs::write(&a, json!({"version":"2.22","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":format!("http://{}/network/http-image/blocked.png",server.address)}}]}).to_string()).unwrap();
        let data = temp.path().join("data");
        let b = Store::init(temp.path().join("b.pen"), Some(data.clone())).unwrap();
        let mut client = Client::start(&a, &data);
        let opened = client.data("open_document", json!({"path":b.path()}));
        let blocked = client.start_request(
            "tools/call",
            json!({"name":"get_document","arguments":{"acquire_images":true}}),
        );
        wait_for_http(&server);
        let mut abandoned = HashSet::from([blocked]);
        for _ in 0..requests {
            abandoned.insert(client.start_request(
            "tools/call",
            json!({"name":"search_nodes","arguments":{"name":"x".repeat(payload_bytes),"limit":1}}),
        ));
        }
        let rejected = client
            .messages
            .recv_timeout(Duration::from_secs(5))
            .expect("Excess inputs should fail before the blocked request completes");
        assert!(
            abandoned.contains(&rejected["id"].as_u64().unwrap()),
            "{rejected}"
        );
        assert_eq!(
            rejected["result"]["structuredContent"]["error"]["code"], "RESOURCE_LIMIT",
            "{rejected}"
        );
        assert_eq!(
            rejected["result"]["structuredContent"]["error"]["details"]["scope"],
            scope
        );
        let other = client.start_request(
            "tools/call",
            json!({"name":"get_document","arguments":{"document_id":opened["document_id"]}}),
        );
        assert_ne!(wait_for(&mut client, other, &abandoned)["isError"], true);
        let close = client.start_request(
            "tools/call",
            json!({"name":"close_document","arguments":{}}),
        );
        assert_eq!(
            wait_for(&mut client, close, &abandoned)["structuredContent"]["closed"],
            true
        );
        let reopened = client.start_request(
            "tools/call",
            json!({"name":"open_document","arguments":{"path":a}}),
        );
        let id =
            wait_for(&mut client, reopened, &abandoned)["structuredContent"]["document_id"].clone();
        let metadata = client.start_request(
            "tools/call",
            json!({"name":"get_document","arguments":{"document_id":id}}),
        );
        assert_ne!(wait_for(&mut client, metadata, &abandoned)["isError"], true);
        finish(&mut client, &abandoned);
    }
}

#[test]
fn blocked_and_saturated_document_does_not_block_other_documents_or_close() {
    let temp = TempDir::new().unwrap();
    let server = http_image_reference::Server::new();
    let a = temp.path().join("a.pen");
    fs::write(&a, json!({"version":"2.22","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":format!("http://{}/network/http-image/blocked.png",server.address)}}]}).to_string()).unwrap();
    let b = temp.path().join("b.pen");
    let data = temp.path().join("data");
    Store::init(&b, Some(data.clone())).unwrap();
    let mut client = Client::start(&a, &data);
    let old = client.document_id.clone().unwrap();
    let blocked = client.start_request(
        "tools/call",
        json!({"name":"get_document","arguments":{"acquire_images":true}}),
    );
    wait_for_http(&server);
    let workers = fixture_support::child_pids(client.child.id());
    assert_eq!(workers.len(), 1);
    let opened = client.data("open_document", json!({"path":b}));
    let mut abandoned = HashSet::from([blocked]);
    // Saturate execution admission without overflowing the separate pending-input pool.
    for _ in 0..28 {
        abandoned.insert(
            client.start_request("tools/call", json!({"name":"get_history","arguments":{}})),
        );
    }
    abandoned.insert(client.start_request("tools/call", json!({"name":"apply_operations","arguments":{"request_id":"queued-edit","expected_revision":lazypencil::document::hash(fs::read(&a).unwrap().as_slice()),"operations":[{"op":"update","target":"image","set":{"width":99}}]}})));
    client.response_timeout = Duration::from_secs(3);
    let metadata = client.data("get_document", json!({"document_id":opened["document_id"]}));
    client.data("get_history", json!({"document_id":opened["document_id"]}));
    #[cfg(feature = "rendering")]
    client.data(
        "get_layout",
        json!({"document_id":opened["document_id"],"expected_revision":metadata["revision"]}),
    );
    #[cfg(not(feature = "rendering"))]
    let _ = metadata;
    let close = client.start_request(
        "tools/call",
        json!({"name":"close_document","arguments":{}}),
    );
    let closed = wait_for(&mut client, close, &abandoned);
    assert_eq!(closed["structuredContent"]["closed"], true, "{closed}");
    assert_eq!(
        unsafe { libc::kill(workers[0] as i32, 0) },
        -1,
        "Close acknowledged before its HTTP worker was reaped"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    let reopen = client.start_request(
        "tools/call",
        json!({"name":"open_document","arguments":{"path":a}}),
    );
    let reopened = wait_for(&mut client, reopen, &abandoned)["structuredContent"].clone();
    assert_ne!(reopened["document_id"], old);
    let stale = client.start_request(
        "tools/call",
        json!({"name":"get_document","arguments":{"document_id":old}}),
    );
    assert_eq!(
        wait_for(&mut client, stale, &abandoned)["structuredContent"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    let store = Store::open(&a, Some(data)).unwrap();
    assert!(
        store.history(100, None).unwrap().is_empty(),
        "Closing a document allowed a queued edit to execute"
    );
    finish(&mut client, &abandoned);
}

#[test]
fn blocked_image_copy_holds_no_save_lock_and_cancelled_preparation_publishes_nothing() {
    let temp = TempDir::new().unwrap();
    let server = http_image_reference::Server::new();
    let data = temp.path().join("data");
    let store = Store::init(temp.path().join("main.pen"), Some(data.clone())).unwrap();
    fs::write(temp.path().join("library.pen"), json!({"version":"2.22","children":[{"id":"image","reusable":true,"type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":format!("http://{}/network/http-image/blocked.png",server.address)}}]}).to_string()).unwrap();
    let mut client = Client::start(store.path(), &data);
    let before = client.data("get_document", json!({}));
    let blocked = client.start_request("tools/call", json!({"name":"apply_operations","arguments":{"request_id":"blocked-copy","expected_revision":before["revision"],"expected_dependency_revision":before["dependency_revision"],"operations":[{"op":"import_component","mode":"copy","source":{"uri":"library.pen","node_id":"image","import_alias":"library"}}]}}));
    wait_for_http(&server);
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.lock_file_path())
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).expect("Network preparation held the save lock");
    fs2::FileExt::unlock(&lock).unwrap();
    let independent: lazypencil::operations::EditRequest = serde_json::from_value(json!({"request_id":"independent-edit","expected_revision":before["revision"],"operations":[{"op":"insert","node":{"name":"Kept","type":"rectangle","width":20,"height":20}}]})).unwrap();
    lazypencil::service::apply_request(&store, &independent).unwrap();
    let close = client.start_request(
        "tools/call",
        json!({"name":"close_document","arguments":{}}),
    );
    let abandoned = HashSet::from([blocked]);
    assert_eq!(
        wait_for(&mut client, close, &abandoned)["structuredContent"]["closed"],
        true
    );
    assert_eq!(store.history(100, None).unwrap().len(), 1);
    assert!(
        store.read().unwrap().value()["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["name"] == "Kept")
    );
    assert!(!temp.path().join("assets").exists());
    finish(&mut client, &abandoned);
}

#[cfg(feature = "rendering")]
#[test]
fn runtime_owner_limit_is_connection_wide_and_close_releases_capacity_and_snapshots() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    fs::write(
        temp.path().join("draw.js"),
        "/** @schema 2.22 */ return [];",
    )
    .unwrap();
    let paths: Vec<_> = (0..5).map(|index| {
        let path = temp.path().join(format!("{index}.pen"));
        fs::write(&path, json!({"version":"2.22","children":[{"id":"script","type":"script","scriptUri":"draw.js","width":20,"height":20}]}).to_string()).unwrap();
        path
    }).collect();
    let mut client = Client::start(&paths[0], &data);
    let mut opened = Vec::new();
    let mut snapshots = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let mut doc = client.data("open_document", json!({"path":path}));
        let metadata = client.data("get_document", json!({"document_id":doc["document_id"]}));
        doc["dependency_revision"] = metadata["dependency_revision"].clone();
        let result = client.call(
            "get_layout",
            json!({"document_id":doc["document_id"],"expected_revision":doc["revision"],"expected_dependency_revision":doc["dependency_revision"]}),
        );
        if index < 4 {
            assert_ne!(result["isError"], true, "{result}");
            snapshots.push(result["structuredContent"]["snapshot_id"].clone());
        } else {
            assert_eq!(
                result["structuredContent"]["error"]["code"], "RESOURCE_LIMIT",
                "{result}"
            );
        }
        opened.push(doc);
    }
    assert_eq!(fixture_support::child_pids(client.child.id()).len(), 4);
    client.data(
        "close_document",
        json!({"document_id":opened[0]["document_id"]}),
    );
    let fifth = &opened[4];
    client.data(
        "get_layout",
        json!({"document_id":fifth["document_id"],"expected_revision":fifth["revision"],"expected_dependency_revision":fifth["dependency_revision"]}),
    );
    assert_eq!(fixture_support::child_pids(client.child.id()).len(), 4);
    let reopened = client.data("open_document", json!({"path":paths[0]}));
    let missing = client.call("get_layout", json!({"document_id":reopened["document_id"],"expected_revision":reopened["revision"],"snapshot_id":snapshots[0]}));
    assert_eq!(
        missing["structuredContent"]["error"]["code"],
        "SNAPSHOT_UNAVAILABLE"
    );
    for doc in &opened[1..] {
        client.data("close_document", json!({"document_id":doc["document_id"]}));
    }
    assert!(fixture_support::child_pids(client.child.id()).is_empty());
    client.finish();
}

#[test]
fn eof_cancels_running_and_waiting_document_owners_and_reaps_http_workers() {
    let temp = TempDir::new().unwrap();
    let server = http_image_reference::Server::new();
    let data = temp.path().join("data");
    let paths: Vec<_> = (0..3).map(|index| {
        let path = temp.path().join(format!("{index}.pen"));
        fs::write(&path, json!({"version":"2.22","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":format!("http://{}/network/http-image/blocked.png",server.address)}}]}).to_string()).unwrap();
        path
    }).collect();
    let mut client = Client::start(&paths[0], &data);
    let mut abandoned = HashSet::new();
    for path in &paths {
        let opened = client.data("open_document", json!({"path":path}));
        abandoned.insert(client.start_request("tools/call", json!({"name":"get_document","arguments":{"document_id":opened["document_id"],"acquire_images":true}})));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.network_counts().0 < 2 {
        assert!(
            Instant::now() < deadline,
            "Independent acquisitions did not start"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let workers = fixture_support::child_pids(client.child.id());
    assert_eq!(
        workers.len(),
        2,
        "Execution limit allowed another HTTP worker"
    );
    finish(&mut client, &abandoned);
    for pid in workers {
        assert_eq!(
            unsafe { libc::kill(pid as i32, 0) },
            -1,
            "EOF retained an HTTP worker"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
