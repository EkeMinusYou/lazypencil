use lazypencil::attachments::prepare;
#[cfg(any(unix, feature = "rendering"))]
use lazypencil::{
    attachments::{AttachmentAccess, AttachmentGrant},
    store::Store,
};
use std::path::Path;
use tempfile::TempDir;

#[test]
fn legacy_attachment_records_preserve_png_access_without_inventing_an_original() {
    use lazypencil::{attachments::Attachment, document::hash};
    let temp = TempDir::new().unwrap();
    let png = include_bytes!("../examples/assets/crop-source.png");
    let digest = hash(png);
    std::fs::create_dir(temp.path().join("attachments")).unwrap();
    std::fs::write(
        temp.path()
            .join("attachments")
            .join(format!("{digest}.png")),
        png,
    )
    .unwrap();
    let attachment: Attachment = serde_json::from_value(serde_json::json!({
        "id":format!("img_{digest}"), "hash":digest, "display_name":"legacy.png",
        "width":320,"height":160,"bytes":png.len(),"source_bytes":png.len()
    }))
    .unwrap();
    assert!(attachment.original.is_none());
    assert_eq!(attachment.read(temp.path()).unwrap(), png);
    assert_eq!(
        attachment.read_original(temp.path()).unwrap_err().code,
        "ATTACHMENT_ORIGINAL_UNAVAILABLE"
    );
    assert!(
        serde_json::to_value(attachment)
            .unwrap()
            .get("original")
            .is_none()
    );
}

#[cfg(feature = "rendering")]
mod originals {
    use super::*;
    use base64::Engine;
    use lazypencil::{attachments::Attachment, document::hash};

    fn jpeg() -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((40, 20)).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        skia_safe::jpeg_encoder::encode_image(
            None,
            &surface.image_snapshot(),
            &skia_safe::jpeg_encoder::Options {
                origin: Some(skia_safe::EncodedOrigin::RightTop),
                ..Default::default()
            },
        )
        .unwrap()
        .as_bytes()
        .to_vec()
    }

    #[test]
    fn exact_original_survives_source_edits_and_metadata_roundtrip_but_is_never_sent() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("reference.jpg");
        let bytes = jpeg();
        std::fs::write(&source, &bytes).unwrap();
        let directory = temp.path().join("conversation");
        let image = Attachment::from_file(&directory, &source).unwrap();
        let original = image.original.as_ref().unwrap();
        assert_eq!(original.mime_type, "image/jpeg");
        assert_eq!(original.hash, hash(&bytes));
        assert_eq!(original.bytes, bytes.len());
        assert_eq!((original.width, original.height), (20, 40));
        assert_eq!((image.width, image.height), (20, 40));
        std::fs::write(&source, b"replacement").unwrap();
        assert_eq!(image.read_original(&directory).unwrap(), bytes);
        let restored: Attachment =
            serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
        assert_eq!(restored.id, image.id);
        assert_eq!(restored.read_original(&directory).unwrap(), bytes);
        let normalized = restored.read(&directory).unwrap();
        assert!(normalized.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!normalized.windows(6).any(|window| window == b"Exif\0\0"));
        let content = serde_json::to_value(restored.content(&directory).unwrap()).unwrap();
        assert_eq!(content["mimeType"], "image/png");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(content["data"].as_str().unwrap())
                .unwrap(),
            normalized
        );
        let captured = directory
            .join("attachments")
            .join(format!("{}.jpg", original.hash));
        std::fs::write(&captured, b"changed").unwrap();
        assert_eq!(
            restored.read_original(&directory).unwrap_err().code,
            "ATTACHMENT_INVALID"
        );
        assert_eq!(restored.read(&directory).unwrap(), normalized);
        assert!(Attachment::from_bytes(&directory, &bytes, "retry.jpg").is_err());
        assert_eq!(std::fs::read(&captured).unwrap(), b"changed");
    }

    #[test]
    fn file_worker_preserves_jpeg_encoding_and_rejects_inconsistent_source_counts() {
        use lazypencil::clipboard::{self, InputContent};
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("reference.jpg");
        let bytes = jpeg();
        std::fs::write(&source, &bytes).unwrap();
        let InputContent::Images { mut images } = clipboard::read_image(
            Path::new(env!("CARGO_BIN_EXE_lazypencil")),
            &source,
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap() else {
            panic!("Expected worker images");
        };
        assert_eq!(images.len(), 1);
        let image = images.pop().unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&image.data)
                .unwrap(),
            bytes
        );
        let directory = temp.path().join("conversation");
        let stored = image.store(&directory).unwrap();
        assert_eq!(stored.read_original(&directory).unwrap(), bytes);
        assert_eq!((stored.width, stored.height), (20, 40));
        let mismatch = clipboard::InputImage {
            name: "mismatch.jpg".into(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            source_bytes: bytes.len() - 1,
        };
        assert!(mismatch.store(&directory).is_err());
    }

    fn conversation(temp: &TempDir) -> lazypencil::conversation::Conversation {
        use clap::Parser;
        let args = lazypencil::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            temp.path().join("design.pen").into_os_string(),
            "--create".into(),
            "--adapter".into(),
            env!("CARGO_BIN_EXE_lazypencil").into(),
            "--non-interactive".into(),
        ])
        .unwrap();
        let lazypencil::cli::Command::Prompt(args) = args.command else {
            unreachable!()
        };
        lazypencil::conversation::Conversation::open_with_executable(
            args,
            Some(temp.path().join("data")),
            env!("CARGO_BIN_EXE_lazypencil").into(),
        )
        .unwrap()
    }

    #[test]
    fn shared_png_blobs_and_distinct_originals_are_reclaimed_only_after_the_last_reference() {
        let temp = TempDir::new().unwrap();
        let mut conversation = conversation(&temp);
        let original = lazypencil::image_input::normalize(
            include_bytes!("../examples/assets/crop-source.png"),
            None,
        )
        .unwrap()
        .png;
        let first =
            Attachment::from_bytes(conversation.directory(), &original, "first.png").unwrap();
        assert_eq!(first.hash, first.original.as_ref().unwrap().hash);
        assert_eq!(
            std::fs::read_dir(conversation.directory().join("attachments"))
                .unwrap()
                .count(),
            1
        );
        let mut second_encoding = original.clone();
        second_encoding.extend_from_slice(b"original trailing input");
        let second =
            Attachment::from_bytes(conversation.directory(), &second_encoding, "second.png")
                .unwrap();
        assert_eq!(first.hash, second.hash);
        assert_ne!(first.id, second.id);
        assert_eq!(
            second.read_original(conversation.directory()).unwrap(),
            second_encoding
        );
        conversation
            .add_images(vec![first.clone(), first.clone(), second.clone()])
            .unwrap();
        conversation.remove_image(0).unwrap();
        first.read_original(conversation.directory()).unwrap();
        conversation.remove_image(0).unwrap();
        // The second original still needs the same normalized PNG.
        first.read_original(conversation.directory()).unwrap();
        second.read_original(conversation.directory()).unwrap();
        conversation.remove_image(0).unwrap();
        assert!(first.read(conversation.directory()).is_err());
        assert!(second.read_original(conversation.directory()).is_err());
        assert_eq!(
            std::fs::read_dir(conversation.directory().join("attachments"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn stored_original_paths_reject_invalid_hashes_formats_and_dimensions() {
        let temp = TempDir::new().unwrap();
        let image = Attachment::from_bytes(temp.path(), &jpeg(), "reference.jpg").unwrap();
        let original = serde_json::to_value(&image).unwrap();
        for (field, value) in [
            ("hash", serde_json::json!("../../outside")),
            ("mime_type", serde_json::json!("../../outside")),
            ("width", serde_json::json!(0)),
            ("bytes", serde_json::json!(0)),
        ] {
            let mut record = original.clone();
            record["original"][field] = value;
            let changed: Attachment = serde_json::from_value(record).unwrap();
            assert_eq!(
                changed.read_original(temp.path()).unwrap_err().code,
                "ATTACHMENT_INVALID"
            );
        }
    }

    #[test]
    fn a_full_retained_inventory_includes_original_metadata_with_a_bounded_grant() {
        let temp = TempDir::new().unwrap();
        let store = Store::init(
            temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        let directory = temp.path().join("conversation");
        let image = Attachment::from_bytes(&directory, &jpeg(), &"x".repeat(1024)).unwrap();
        let images = vec![image.clone(); 256];
        let grant = AttachmentGrant::create(&store, &directory, "full-inventory", &images).unwrap();
        let bytes = std::fs::read(grant.path()).unwrap().len();
        assert!(bytes > 256 * 1024 && bytes < 512 * 1024);
        let access = AttachmentAccess::open(&store, grant.path()).unwrap();
        let metadata = access.metadata(&store).unwrap();
        assert_eq!(metadata["images"].as_array().unwrap().len(), 256);
        assert_eq!(metadata["images"][0]["original"]["mime_type"], "image/jpeg");
        assert_eq!(
            access.read(&store, &image.id).unwrap(),
            image.read(&directory).unwrap()
        );
    }
}

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
