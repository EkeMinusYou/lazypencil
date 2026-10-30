use super::*;

const PNG: &[u8] = include_bytes!("../../examples/assets/crop-source.png");

#[test]
fn inline_images_decode_base64_and_percent_bytes_without_changing_url_identity() {
    let encoded = base64::engine::general_purpose::STANDARD.encode(PNG);
    let raw = format!("data:image/png;base64,{encoded}");
    let escaped = encoded
        .replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D");
    let percent = PNG
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect::<String>();
    for uri in [
        raw.clone(),
        format!("DATA:IMAGE/PNG;name=fixture%20image;BASE64,{escaped}#fragment"),
        format!("data:image/png,{percent}"),
    ] {
        validate(&uri).unwrap();
        assert_eq!(decode(&uri, None).unwrap().as_ref(), PNG);
        let key = identity(&uri);
        assert!(is_key(&key));
        assert!(!key.to_str().unwrap().contains("fixture"));
    }
    assert_ne!(identity(&raw), identity(&format!("{raw}#fragment")));
    assert_eq!(percent_bytes("+%2B?%23", 5, false, None).unwrap(), b"++?#");
}

#[test]
fn inline_images_reject_non_images_invalid_encodings_mime_mismatches_and_oversized_inputs() {
    let encoded = base64::engine::general_purpose::STANDARD.encode(PNG);
    for uri in [
        format!("data:image/jpeg;base64,{encoded}"),
        "data:text/html;base64,PHNjcmlwdD4=".into(),
        "data:;base64,aGVsbG8=".into(),
        "data:image/png;base64,not-base64".into(),
        "data:image/png,%".into(),
        "data:image/png,%0".into(),
        "data:image/png,%GG".into(),
        "data:image/png,private-payload".into(),
        "data:image/png;private-payload,anything".into(),
        format!(
            "data:image/png;name={},anything",
            "x".repeat(MAX_HEADER_BYTES)
        ),
    ] {
        let error = decode(&uri, None).unwrap_err();
        assert_eq!(error.code, "ASSET_NOT_FOUND");
        assert!(!error.message.contains("private-payload"));
        assert!(!error.message.contains(&encoded));
    }
    assert!(percent_bytes("abcd", 3, false, None).is_err());
    assert_eq!(percent_bytes("a%62c", 3, false, None).unwrap(), b"abc");
    let body = "A".repeat(MAX_SOURCE_BYTES.div_ceil(3) * 4 + 4);
    assert!(decode(&format!("data:image/png;base64,{body}"), None).is_err());
}

#[test]
fn inline_image_decoding_obeys_cancellation_without_starting_a_worker() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        decode("data:image/png,anything", Some(&cancel))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
}
