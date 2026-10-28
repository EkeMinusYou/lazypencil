use lazypencil::attachments::prepare;
#[cfg(any(unix, feature = "rendering"))]
use lazypencil::{
    attachments::{AttachmentAccess, AttachmentGrant},
    store::Store,
};
use std::path::Path;
use tempfile::TempDir;

#[cfg(feature = "rendering")]
#[path = "support/fixture.rs"]
mod fixture_support;

#[test]
fn send_image_bounds_are_explicit_positive_and_bounded() {
    use lazypencil::image_input::ImageBounds;
    assert_eq!(
        "1024x768".parse::<ImageBounds>().unwrap(),
        ImageBounds::new(1024, 768).unwrap()
    );
    for invalid in [
        "",
        "1024",
        "0x768",
        "1024x0",
        "-1x2",
        "1x2x3",
        "5000x5000",
        "4294967296x1",
    ] {
        assert!(invalid.parse::<ImageBounds>().is_err(), "{invalid}");
    }
}

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
    fn configured_source_send_and_pixel_limits_require_an_explicit_downscale() {
        use lazypencil::{
            attachments::{AttachmentOptions, prepare_with_options},
            config::attachments::AttachmentSettings,
            image_input::ImageBounds,
        };
        let temp = TempDir::new().unwrap();
        let source = jpeg();
        let large =
            Attachment::from_bytes(&temp.path().join("baseline"), &source, "source.jpg").unwrap();
        let small = large
            .resized(
                &temp.path().join("baseline"),
                ImageBounds::new(1, 1).unwrap(),
            )
            .unwrap();
        assert!(small.bytes < large.bytes);
        let limits = AttachmentSettings {
            max_send_bytes: small.bytes,
            ..Default::default()
        };
        let directory = temp.path().join("limited");
        let options = AttachmentOptions {
            bounds: None,
            limits,
        };
        assert_eq!(
            Attachment::from_bytes_with_options(&directory, &source, "reference.jpg", options)
                .unwrap_err()
                .code,
            "ATTACHMENT_INVALID"
        );
        assert!(
            !directory.exists(),
            "No blobs should be written on a budget failure"
        );
        let image = Attachment::from_bytes_with_options(
            &directory,
            &source,
            "reference.jpg",
            AttachmentOptions {
                bounds: Some(ImageBounds::new(1, 1).unwrap()),
                ..options
            },
        )
        .unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.read_original(&directory).unwrap(), source);
        for limits in [
            AttachmentSettings {
                max_source_bytes: source.len() - 1,
                ..limits
            },
            AttachmentSettings {
                max_source_pixels: 799,
                ..limits
            },
        ] {
            assert!(
                Attachment::from_bytes_with_options(
                    &temp.path().join("rejected"),
                    &source,
                    "source.jpg",
                    AttachmentOptions {
                        limits,
                        bounds: Some(ImageBounds::new(1, 1).unwrap())
                    }
                )
                .is_err()
            );
            assert!(!temp.path().join("rejected").exists());
        }
        let path = temp.path().join("tiny.png");
        std::fs::write(&path, image.read(&directory).unwrap()).unwrap();
        let paths = vec![path; 9];
        assert!(prepare(temp.path(), &paths).is_err());
        let nine = prepare_with_options(
            temp.path(),
            &paths,
            AttachmentOptions {
                limits: AttachmentSettings {
                    max_images: 9,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(nine.len(), 9);
        let total_source = AttachmentSettings {
            max_images: 9,
            max_total_source_bytes: nine[0].source_size() * 9 - 1,
            ..Default::default()
        };
        let total_send = AttachmentSettings {
            max_images: 9,
            max_total_send_bytes: nine[0].bytes * 9 - 1,
            ..Default::default()
        };
        assert_eq!(
            total_source
                .check_request(&nine)
                .unwrap_err()
                .details
                .unwrap()["limit"],
            "max_total_source_bytes"
        );
        assert_eq!(
            total_send
                .check_request(&nine)
                .unwrap_err()
                .details
                .unwrap()["limit"],
            "max_total_send_bytes"
        );
    }

    #[cfg(feature = "tui")]
    #[test]
    fn image_worker_receives_selected_budgets_and_preserves_the_encoded_source() {
        use lazypencil::{
            attachments::AttachmentOptions,
            clipboard::{self, InputContent},
            config::attachments::AttachmentSettings,
            image_input::ImageBounds,
        };
        let temp = TempDir::new().unwrap();
        let source = jpeg();
        let path = temp.path().join("reference.jpg");
        std::fs::write(&path, &source).unwrap();
        let baseline = Attachment::from_file(temp.path(), &path).unwrap();
        let small = baseline
            .resized(temp.path(), ImageBounds::new(1, 1).unwrap())
            .unwrap();
        let executable = Path::new(env!("CARGO_BIN_EXE_lazypencil"));
        let cancel = tokio_util::sync::CancellationToken::new();
        let options = AttachmentOptions {
            bounds: None,
            limits: AttachmentSettings {
                max_send_bytes: small.bytes,
                ..Default::default()
            },
        };
        assert_eq!(
            clipboard::read_image_with_options(executable, &path, options, &cancel)
                .unwrap_err()
                .code,
            "ATTACHMENT_INVALID"
        );
        let options = AttachmentOptions {
            bounds: Some(ImageBounds::new(1, 1).unwrap()),
            ..options
        };
        let InputContent::Images { mut images } =
            clipboard::read_image_with_options(executable, &path, options, &cancel).unwrap()
        else {
            panic!("Expected image");
        };
        let stored = images
            .pop()
            .unwrap()
            .store_with_options(temp.path(), options)
            .unwrap();
        assert_eq!(stored.read_original(temp.path()).unwrap(), source);
        assert_eq!((stored.width, stored.height), (1, 1));
        assert!(
            clipboard::read_image_with_options(
                executable,
                &path,
                AttachmentOptions {
                    limits: AttachmentSettings {
                        max_source_bytes: source.len() - 1,
                        ..Default::default()
                    },
                    ..options
                },
                &cancel
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_send_sizes_use_original_orientation_and_never_compound_or_upscale() {
        use lazypencil::image_input::ImageBounds;
        let temp = TempDir::new().unwrap();
        let source = jpeg();
        let original = Attachment::from_bytes(temp.path(), &source, "reference.jpg").unwrap();
        let small = original
            .resized(temp.path(), ImageBounds::new(10, 10).unwrap())
            .unwrap();
        assert_eq!((small.width, small.height), (5, 10));
        assert_eq!(small.original, original.original);
        assert_eq!(small.read_original(temp.path()).unwrap(), source);
        assert_ne!(small.id, original.id);
        let medium = small
            .resized(temp.path(), ImageBounds::new(12, 30).unwrap())
            .unwrap();
        assert_eq!((medium.width, medium.height), (12, 24));
        let restored = small
            .resized(temp.path(), ImageBounds::new(100, 100).unwrap())
            .unwrap();
        assert_eq!((restored.width, restored.height), (20, 40));
        assert_eq!(restored.id, original.id);
        assert_eq!(
            restored.read(temp.path()).unwrap(),
            original.read(temp.path()).unwrap()
        );
        let direct = Attachment::from_bytes_with_bounds(
            temp.path(),
            &source,
            "direct.jpg",
            Some(ImageBounds::new(10, 10).unwrap()),
        )
        .unwrap();
        assert_eq!(direct.id, small.id);
        assert_eq!(
            direct
                .original
                .as_ref()
                .map(|source| (source.width, source.height)),
            Some((20, 40))
        );
    }

    #[test]
    fn resized_png_uses_premultiplied_linear_sampling_without_transparent_color_bleed() {
        use lazypencil::image_input::{self, ImageBounds};
        let rgba = [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 0];
        let original = image_input::from_rgba8(2, 2, &rgba).unwrap();
        let image = image_input::normalize_with_bounds(
            &original.png,
            None,
            Some(ImageBounds::new(1, 1).unwrap()),
        )
        .unwrap();
        assert_eq!(
            (
                image.source_width,
                image.source_height,
                image.width,
                image.height
            ),
            (2, 2, 1, 1)
        );
        let decoded = image::load_from_memory(&image.png).unwrap().to_rgba8();
        let pixel = decoded.get_pixel(0, 0).0;
        assert!(
            pixel[0] >= 254 && pixel[1] <= 1 && pixel[2] <= 1,
            "{pixel:?}"
        );
        assert!(pixel[3].abs_diff(128) <= 1, "{pixel:?}");
    }

    #[cfg(feature = "tui")]
    #[test]
    fn resized_worker_input_and_pending_replacement_preserve_the_selected_source() {
        use lazypencil::{
            clipboard::{self, InputContent},
            image_input::ImageBounds,
        };
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("reference.jpg");
        let bytes = jpeg();
        std::fs::write(&source, &bytes).unwrap();
        let bounds = ImageBounds::new(10, 10).unwrap();
        let InputContent::Images { mut images } = clipboard::read_image_with_bounds(
            Path::new(env!("CARGO_BIN_EXE_lazypencil")),
            &source,
            Some(bounds),
            &tokio_util::sync::CancellationToken::new(),
        )
        .unwrap() else {
            panic!("Expected image");
        };
        let mut conversation = conversation(&temp);
        let original = Attachment::from_file(conversation.directory(), &source).unwrap();
        conversation.add_image(original.clone()).unwrap();
        let image = images
            .pop()
            .unwrap()
            .store_with_bounds(conversation.directory(), Some(bounds))
            .unwrap();
        assert_eq!((image.width, image.height), (5, 10));
        assert_eq!(
            conversation
                .replace_image(0, "stale-image", image.clone())
                .unwrap_err()
                .code,
            "REVISION_CONFLICT"
        );
        assert_eq!(conversation.images()[0].id, original.id);
        conversation
            .replace_image(0, &original.id, image.clone())
            .unwrap();
        assert_eq!(conversation.images()[0].id, image.id);
        assert_eq!(
            image.read_original(conversation.directory()).unwrap(),
            bytes
        );
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(conversation.directory().join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["pending_attachments"][0]["width"], 5);
        assert_eq!(saved["pending_attachments"][0]["original"]["width"], 20);
        conversation.clear_images().unwrap();
        assert!(image.read_original(conversation.directory()).is_err());
    }

    #[test]
    fn parsed_cli_send_bounds_reach_the_actual_acp_image_without_changing_the_original() {
        use clap::Parser;
        use lazypencil::conversation::{Conversation, ConversationEvent};
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("reference.jpg");
        let source = jpeg();
        std::fs::write(&path, &source).unwrap();
        let args = lazypencil::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            temp.path().join("design.pen").into_os_string(),
            "--create".into(),
            "--adapter".into(),
            fixture_support::helper().into_os_string(),
            "--adapter-arg=acp".into(),
            "--adapter-arg=image-inspect".into(),
            "--image-max-size=10x10".into(),
            "--non-interactive".into(),
            "--timeout=5".into(),
        ])
        .unwrap();
        let lazypencil::cli::Command::Prompt(args) = args.command else {
            unreachable!()
        };
        let mut conversation = Conversation::open_with_executable(
            args,
            Some(temp.path().join("data")),
            env!("CARGO_BIN_EXE_lazypencil").into(),
        )
        .unwrap();
        conversation.attach_file(&path).unwrap();
        let image = conversation.images()[0].clone();
        assert_eq!((image.width, image.height), (5, 10));
        conversation.set_draft("日本語の参考画像".into()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                conversation.send(None).await?;
                loop {
                    if let ConversationEvent::Message(event) = conversation.next_event().await?
                        && matches!(event["type"].as_str(), Some("complete" | "failed"))
                    {
                        return conversation.result().cloned().expect("completed turn");
                    }
                }
            })
            .await;
            conversation.close().await.unwrap();
            result.unwrap().unwrap();
        });
        let received = std::fs::read(temp.path().join("received-image-1-0.png")).unwrap();
        assert_eq!(
            image::load_from_memory(&received)
                .unwrap()
                .to_rgba8()
                .dimensions(),
            (5, 10)
        );
        assert_eq!(received, image.read(conversation.directory()).unwrap());
        assert_eq!(
            image.read_original(conversation.directory()).unwrap(),
            source
        );
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

    #[cfg(feature = "tui")]
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
    fn tightened_settings_block_submission_and_allow_incremental_image_repairs() {
        // A separate harness process owns its environment; parallel tests keep
        // their normal user configuration and never mutate process-global env.
        if std::env::var_os("LAZYPENCIL_TEST_IMAGE_LIMIT_RELOAD").is_none() {
            let temp = TempDir::new().unwrap();
            let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", "originals::tightened_settings_block_submission_and_allow_incremental_image_repairs", "--nocapture"])
                .env("LAZYPENCIL_TEST_IMAGE_LIMIT_RELOAD", "1")
                .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("configuration"))
                .kill_on_drop(true);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let output = runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
                    .await
                    .unwrap()
                    .unwrap()
            });
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let temp = TempDir::new().unwrap();
        let mut conversation = conversation(&temp);
        let image =
            Attachment::from_bytes(conversation.directory(), &jpeg(), "reference.jpg").unwrap();
        conversation
            .add_images(vec![image.clone(), image.clone()])
            .unwrap();
        conversation
            .set_draft("日本語の下書きと元画像を保持".into())
            .unwrap();
        let small = image
            .resized(
                conversation.directory(),
                lazypencil::image_input::ImageBounds::new(1, 1).unwrap(),
            )
            .unwrap();
        assert!(small.bytes < image.bytes);
        let settings = std::path::PathBuf::from(std::env::var_os("LAZYPENCIL_CONFIG_DIR").unwrap());
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("config.toml"),
            format!("[attachments]\nmax_total_send_bytes={}\n", small.bytes * 2),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert_eq!(
                conversation.send(None).await.unwrap_err().code,
                "ATTACHMENT_INVALID"
            );
            assert!(!conversation.active());
            assert_eq!(conversation.images().len(), 2);
            assert_eq!(conversation.draft(), "日本語の下書きと元画像を保持");
            conversation
                .replace_image(0, &image.id, small.clone())
                .unwrap();
            assert_eq!(
                conversation.send(None).await.unwrap_err().code,
                "ATTACHMENT_INVALID"
            );
            conversation
                .replace_image(1, &image.id, small.clone())
                .unwrap();
            conversation
                .attachment_limits()
                .unwrap()
                .check_request(conversation.images())
                .unwrap();
            conversation.close().await.unwrap();
        });
        assert_eq!(
            small.read_original(conversation.directory()).unwrap(),
            jpeg()
        );
        let saved: serde_json::Value = serde_json::from_slice(
            &std::fs::read(conversation.directory().join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["pending_attachments"][0]["width"], 1);
        assert_eq!(saved["pending_attachments"][1]["width"], 1);
        assert_eq!(saved["draft"], "日本語の下書きと元画像を保持");
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
