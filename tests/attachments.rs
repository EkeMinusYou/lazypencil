use lazypencil::attachments::prepare;
#[cfg(feature = "rendering")]
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
