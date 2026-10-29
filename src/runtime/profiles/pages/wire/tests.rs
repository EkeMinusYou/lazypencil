use super::*;

fn page() -> Value {
    serde_json::json!({"op":"page","timeout_ms":5000,"files":[],"page":{
        "url":"data:text/html,hello","width":32,"height":32,"scale":1,
        "zoom":1,"scroll_x":0,"scroll_y":0,"device":null,
        "settings":{},"dynamic":{},"retained":null,"resource_identity":null
    }})
}

#[test]
fn profile_identity_and_deadlines_cannot_be_overridden_by_request_headers() {
    let valid: Request = serde_json::from_value(page()).unwrap();
    assert_eq!(valid.validate().unwrap(), 0);
    for (pointer, value) in [
        (
            "/page/settings/profile",
            serde_json::json!("another-profile"),
        ),
        ("/timeout_ms", serde_json::json!(0)),
        ("/timeout_ms", serde_json::json!(300001)),
        ("/page/settings/timeout_ms", serde_json::json!(4999)),
        ("/page/dynamic/render_timeout_ms", serde_json::json!(4999)),
        (
            "/page/retained",
            serde_json::json!(["not-a-key", "not-a-hash"]),
        ),
        (
            "/page/resource_identity",
            serde_json::json!({"revision":"missing-files"}),
        ),
    ] {
        let mut invalid = page();
        // Optional settings start as empty maps; insert these test properties.
        if let Some((parent, key)) = pointer.rsplit_once('/') {
            invalid
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.into(), value);
        }
        let request: Request = serde_json::from_value(invalid).unwrap();
        assert!(request.validate().is_err(), "Accepted {pointer}");
    }
    let mut private = page();
    private["page"]["settings"]["profile"] = serde_json::json!("private-profile");
    let request: Request = serde_json::from_value(private).unwrap();
    assert!(
        !serde_json::to_string(&request)
            .unwrap()
            .contains("private-profile")
    );
}

#[test]
fn resource_limits_apply_before_body_allocation_and_to_the_aggregate() {
    let mut value = page();
    value["page"]["resource_identity"] = serde_json::json!({"revision":"fixture"});
    let header =
        |bytes| serde_json::json!({"url":"file:///fixture/index.html","mime":null,"bytes":bytes});
    value["files"] = serde_json::json!([header(MAX_PNG)]);
    assert_eq!(
        serde_json::from_value::<Request>(value.clone())
            .unwrap()
            .validate()
            .unwrap(),
        MAX_PNG
    );
    for files in [
        vec![header(MAX_PNG + 1)],
        vec![header(MAX_PNG); 5],
        vec![header(0); 4097],
    ] {
        value["files"] = serde_json::json!(files);
        assert!(
            serde_json::from_value::<Request>(value.clone())
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let excessive = Request::Retain {
        timeout_ms: 5000,
        signatures: (0..9)
            .map(|i| (format!("{i:064x}"), "a".repeat(64)))
            .collect(),
    };
    assert!(excessive.validate().is_err());
}

#[test]
fn oversize_frames_are_rejected_without_waiting_for_their_body() {
    for size in [0, MAX_HEADER as u64 + 1, u64::MAX] {
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        sender.write_all(&size.to_be_bytes()).unwrap();
        let budget = Budget::new(Duration::from_secs(5), None);
        let mut stream = LocalStream::new(receiver, &budget);
        let error = read_json::<Hello>(&mut stream, None).err().unwrap();
        assert_eq!(error.details.unwrap()["stage"], "profile_protocol_limit");
        budget.check().unwrap();
    }
}

#[test]
fn cancellation_interrupts_an_incomplete_local_frame_and_a_new_budget_can_resume() {
    use tokio_util::sync::CancellationToken;
    let (mut sender, receiver) = UnixStream::pair().unwrap();
    let token = CancellationToken::new();
    let signal = token.clone();
    let (resuming, resumed) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let budget = Budget::new(Duration::from_secs(30), Some(&token));
        let mut stream = LocalStream::new(receiver, &budget);
        assert!(read_json::<Hello>(&mut stream, None).is_err());
        assert_eq!(budget.check().unwrap_err().code, "CANCELLED");
        resuming.send(()).unwrap();
        // This transport has no poisoned global timeout after a cancellation.
        stream.set_budget(&Budget::new(Duration::from_secs(5), None));
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'x']);
    });
    sender.write_all(&100u64.to_be_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    signal.cancel();
    resumed.recv_timeout(Duration::from_secs(5)).unwrap();
    sender.write_all(b"x").unwrap();
    reader.join().unwrap();
}
