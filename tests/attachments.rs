use lazypencil::attachments::prepare;
#[cfg(any(unix, feature = "rendering"))]
use lazypencil::{
    attachments::{AttachmentAccess, AttachmentGrant},
    store::Store,
};
use std::path::Path;
use tempfile::TempDir;

#[test]
fn image_count_limit_is_checked_before_reading_files() {
    let temp = TempDir::new().unwrap();
    let files = vec![temp.path().join("missing.png"); 9];
    let error = prepare(temp.path(), &files).unwrap_err();
    assert_eq!(error.code, "ATTACHMENT_INVALID");
    assert!(error.message.contains("eight"));
}

#[cfg(unix)]
#[test]
fn attachment_metadata_requires_its_original_living_connection_even_without_rendering() {
    let temp = TempDir::new().unwrap();
    let store = Store::init(
        temp.path().join("design.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let directory = temp.path().join("conversation");
    std::fs::create_dir(&directory).unwrap();
    let a = AttachmentGrant::create(&store, &directory, "conversation-a", &[]).unwrap();
    let b = AttachmentGrant::create(&store, &directory, "conversation-b", &[]).unwrap();
    let access = AttachmentAccess::open(&store, a.path()).unwrap();
    assert_eq!(
        access.metadata(&store).unwrap()["conversation"],
        "conversation-a"
    );
    let original = std::fs::read(a.path()).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let other: serde_json::Value =
        serde_json::from_slice(&std::fs::read(b.path()).unwrap()).unwrap();
    manifest["owner"] = other["owner"].clone();
    std::fs::write(a.path(), manifest.to_string()).unwrap();
    assert!(access.metadata(&store).is_err());
    assert!(AttachmentAccess::open(&store, a.path()).is_err());
    manifest.as_object_mut().unwrap().remove("owner");
    std::fs::write(a.path(), manifest.to_string()).unwrap();
    assert!(AttachmentAccess::open(&store, a.path()).is_err());
    std::fs::write(a.path(), original).unwrap();
    assert!(access.metadata(&store).is_ok());
    let owner = a.path().with_extension("owner");
    drop(a);
    assert!(!owner.exists());
    assert!(access.metadata(&store).is_err());
    assert!(AttachmentAccess::open(&store, b.path()).is_ok());
}

#[cfg(feature = "rendering")]
#[test]
fn attachment_grants_are_bound_revoked_and_checked_for_changes() {
    let temp = TempDir::new().unwrap();
    let store = Store::init(
        temp.path().join("design.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let other = Store::init(
        temp.path().join("other.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let directory = temp.path().join("conversation");
    let images = prepare(
        &directory,
        &[Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png")],
    )
    .unwrap();
    let grant = AttachmentGrant::create(&store, &directory, "test-conversation", &images).unwrap();
    let access = AttachmentAccess::open(&store, grant.path()).unwrap();
    assert!(AttachmentAccess::open(&other, grant.path()).is_err());
    assert!(access.read(&store, "../../outside").is_err());
    let bytes = access.read(&store, &images[0].id).unwrap();
    assert_eq!(lazypencil::document::hash(&bytes), images[0].hash);
    assert_eq!(access.metadata(&store).unwrap()["images"][0]["width"], 320);
    let path = directory
        .join("attachments")
        .join(format!("{}.png", images[0].hash));
    std::fs::write(&path, b"changed").unwrap();
    assert!(
        access
            .read(&store, &images[0].id)
            .unwrap_err()
            .message
            .contains("changed")
    );
    std::fs::write(&path, bytes).unwrap();
    drop(grant);
    assert!(access.read(&store, &images[0].id).is_err());
}

#[cfg(not(feature = "rendering"))]
#[test]
fn image_normalization_requires_the_rendering_build() {
    let temp = TempDir::new().unwrap();
    let error = prepare(
        temp.path(),
        &[Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/assets/crop-source.png")],
    )
    .unwrap_err();
    assert_eq!(error.code, "UNSUPPORTED_FEATURE");
}
