use super::*;

#[test]
fn image_urls_require_explicit_http_authorities_and_never_embed_credentials_in_keys() {
    for uri in [
        "http:relative",
        "file:///private.png",
        "data:image/png;base64,eA==",
        "ftp://host/image",
        "http://user:secret@host/image",
        "http://@host/image",
        "http://host/path\nheader",
        "http://host\\image",
    ] {
        assert!(validate(uri).is_err(), "Accepted {uri:?}");
    }
    for uri in [
        "http://127.0.0.1:1234/image.png",
        "https://example.com/画像.png?private=token#keep",
        "HTTPS://example.com/image",
    ] {
        validate(uri).unwrap();
        let path = key(uri).unwrap();
        assert!(is_key(&path));
        assert!(!path.to_string_lossy().contains("private"));
        assert!(!path.to_string_lossy().contains("example.com"));
    }
    assert_ne!(
        key("https://example.com/image?a").unwrap(),
        key("https://example.com/image?b").unwrap()
    );
    assert!(validate(&format!("https://example.com/{}", "x".repeat(4096))).is_err());
}
