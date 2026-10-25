use super::*;

fn files() -> Arc<LocalFiles> {
    Arc::new(
        LocalFiles::new([(
            "file:///original/site/index.html".into(),
            "text/html; charset=utf-8".into(),
            b"captured".to_vec(),
        )])
        .unwrap(),
    )
}

fn state() -> Interception {
    let mut state = Interception::default();
    state.sessions.insert(
        "root".into(),
        Session {
            owner: "root".into(),
            parent: None,
            files: files(),
            stats: LocalFileStats::default(),
        },
    );
    state
}

fn request(session: &str, method: &str, url: &str) -> Value {
    json!({"sessionId":session,"method":"Fetch.requestPaused",
        "params":{"requestId":"request","request":{"url":url,"method":method}}})
}

#[test]
fn captured_bytes_handle_queries_head_and_binary_without_reading_original_paths() {
    let files = files();
    for (method, expected) in [("GET", b"captured".as_slice()), ("HEAD", &[][..])] {
        let event = request(
            "root",
            method,
            "file:///original/site/index.html?theme=dark#view",
        );
        let (command, params, bytes) = files.response(&event["params"]).unwrap();
        assert_eq!(command, "Fetch.fulfillRequest");
        assert_eq!(params["responseCode"], 200);
        assert_eq!(params["responseHeaders"][1]["value"], "8");
        assert_eq!(
            STANDARD.decode(params["body"].as_str().unwrap()).unwrap(),
            expected
        );
        assert_eq!(bytes, expected.len());
    }
    let bytes = vec![0, 255, 13, 10];
    let files =
        LocalFiles::new([("file:///binary".into(), "image/png".into(), bytes.clone())]).unwrap();
    let (_, response, _) = files
        .response(&request("root", "GET", "file:///binary")["params"])
        .unwrap();
    assert_eq!(
        STANDARD.decode(response["body"].as_str().unwrap()).unwrap(),
        bytes
    );
}

#[test]
fn unknown_files_methods_remote_hosts_and_credentialed_urls_never_fall_back() {
    for (method, url) in [
        ("GET", "file:///original/site/missing"),
        ("GET", "file:///original/outside"),
        ("GET", "file://remote/original/site/index.html"),
        ("GET", "https://example.com/original/site/index.html"),
        ("POST", "file:///original/site/index.html"),
    ] {
        let (command, params, bytes) = files()
            .response(&request("root", method, url)["params"])
            .unwrap();
        assert_eq!(command, "Fetch.failRequest");
        assert_eq!(params["errorReason"], "BlockedByClient");
        assert_eq!(bytes, 0);
    }
    for url in [
        "relative/file",
        "data:text/html,hi",
        "file://server/share/index.html",
        "file://user@host/a",
    ] {
        assert!(LocalFiles::new([(url.into(), "text/html".into(), vec![])]).is_err());
    }
}

#[test]
fn inventories_reject_alias_collisions_header_injection_and_oversized_members() {
    for second in [
        "file:///original/site/./index.html",
        "file:///original/site/index.html?q=1",
    ] {
        assert!(
            LocalFiles::new([
                (
                    "file:///original/site/index.html".into(),
                    "text/html".into(),
                    vec![]
                ),
                (second.into(), "text/html".into(), vec![]),
            ])
            .is_err()
        );
    }
    for mime in ["", "text/html\r\nLocation: file:///other", "text/日本語"] {
        assert!(LocalFiles::new([("file:///a".into(), mime.into(), vec![])]).is_err());
    }
    assert!(LocalFiles::new(std::iter::empty()).is_err());
    assert!(
        LocalFiles::new([(
            "file:///a".into(),
            "text/html".into(),
            vec![0; MAX_FILE_BYTES + 1]
        )])
        .is_err()
    );
    assert!(
        LocalFiles::new((0..=MAX_FILES).map(|i| (
            format!("file:///{i}"),
            "text/plain".into(),
            vec![]
        )))
        .is_err()
    );
}

fn attach(state: &mut Interception, parent: &str, child: &str) -> Dispatch {
    state
        .observe(
            &json!({"sessionId":parent,"method":"Target.attachedToTarget",
        "params":{"sessionId":child,"waitingForDebugger":true}}),
        )
        .unwrap()
        .unwrap()
}

#[test]
fn child_startup_waits_for_interception_then_recursive_auto_attach_before_resume() {
    let mut state = state();
    let enabled = attach(&mut state, "root", "worker");
    assert_eq!(enabled.method, "Fetch.enable");
    state.pending.insert(100, (enabled.session, enabled.next));
    // A response belonging to the original command must remain available to its caller.
    assert!(
        state
            .observe(&json!({"id":99,"result":{"outer":true}}))
            .unwrap()
            .is_none()
    );
    assert_eq!(state.pending.len(), 1);
    let attached = state
        .observe(&json!({"id":100,"result":{}}))
        .unwrap()
        .unwrap();
    assert_eq!(attached.method, "Target.setAutoAttach");
    state.pending.insert(101, (attached.session, attached.next));
    let resumed = state
        .observe(&json!({"id":101,"result":{}}))
        .unwrap()
        .unwrap();
    assert_eq!(resumed.method, "Runtime.runIfWaitingForDebugger");
    state.pending.insert(102, (resumed.session, resumed.next));
    assert!(
        state
            .observe(&json!({"id":102,"result":{}}))
            .unwrap()
            .is_none()
    );
    let response = state
        .observe(&request(
            "worker",
            "GET",
            "file:///original/site/index.html",
        ))
        .unwrap()
        .unwrap();
    assert_eq!(response.method, "Fetch.fulfillRequest");
    assert_eq!(response.session, "worker");
    assert_eq!(state.sessions["root"].stats.fulfilled, 1);
    assert_eq!(state.sessions["root"].stats.response_bytes, 8);
    attach(&mut state, "worker", "grandchild");
    assert_eq!(state.sessions["root"].stats.attached_targets, 2);
    assert!(Arc::ptr_eq(
        &state.sessions["root"].files,
        &state.sessions["grandchild"].files
    ));
    state
        .observe(&request("grandchild", "GET", "file:///missing"))
        .unwrap();
    assert_eq!(state.sessions["root"].stats.denied, 1);
}

#[test]
fn child_detach_and_context_disposal_remove_descendants_and_pending_work_only_for_the_owner() {
    let mut state = state();
    attach(&mut state, "root", "frame");
    let child = attach(&mut state, "frame", "worker");
    state.pending.insert(1, (child.session, child.next));
    attach(&mut state, "root", "sibling");
    // An unrelated session cannot detach another context's descendants.
    state.observe(&json!({"sessionId":"sibling","method":"Target.detachedFromTarget","params":{"sessionId":"frame"}})).unwrap();
    assert_eq!(state.sessions.len(), 4);
    state.observe(&json!({"sessionId":"root","method":"Target.detachedFromTarget","params":{"sessionId":"frame"}})).unwrap();
    assert_eq!(state.sessions.len(), 2);
    assert!(state.pending.is_empty());
    state.remove("root");
    assert!(state.sessions.is_empty());
    assert!(
        state
            .observe(&json!({"id":1,"error":{"message":"old detached work"}}))
            .unwrap()
            .is_none()
    );
}

#[test]
fn interception_failures_are_bounded_and_do_not_echo_page_data() {
    let mut state = state();
    let child = attach(&mut state, "root", "child");
    state.pending.insert(1, (child.session, child.next));
    let error = state
        .observe(&json!({"id":1,"error":{"message":"file:///private?secret=123"}}))
        .err()
        .unwrap();
    assert!(!error.message.contains("private"));
    assert!(!error.message.contains("123"));
    assert!(
        state
            .observe(&request(
                "unrelated",
                "GET",
                "file:///original/site/index.html"
            ))
            .unwrap()
            .is_none()
    );
    for i in state.sessions.len()..MAX_SESSIONS {
        attach(&mut state, "root", &format!("child-{i}"));
    }
    assert!(state.observe(&json!({"sessionId":"root","method":"Target.attachedToTarget","params":{"sessionId":"overflow"}})).is_err());
}
