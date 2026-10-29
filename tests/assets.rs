use lazypencil::{
    assets::{self, AssetKind, AssetSource, PutAssetRequest},
    store::Store,
};
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    sync::{Arc, Barrier},
};
use tempfile::TempDir;
#[path = "support/lock_wait.rs"]
mod lock_wait;

fn setup() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let store = Store::init(
        temp.path().join("scene.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    (temp, store)
}

#[cfg(unix)]
#[test]
fn cli_asset_output_keeps_maintenance_excluded_until_the_response_is_written() {
    use fs2::FileExt;
    use std::{
        io::Read,
        os::{
            fd::{AsRawFd, OwnedFd},
            unix::net::UnixStream,
        },
        process::{Child, Stdio},
        time::{Duration, Instant},
    };
    struct Owner(Child);
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (temp, store) = setup();
    let source = format!("/*{}*/ return [];", "x".repeat(96 * 1024));
    fs::write(temp.path().join("generator.js"), &source).unwrap();
    fs::write(store.path(), json!({"version":"2.21","children":[{"id":"script","type":"script","scriptUri":"generator.js","width":1,"height":1}]}).to_string()).unwrap();
    let locks = temp.path().join("isolated-locks");
    let (mut output, writer) = UnixStream::pair().unwrap();
    let bytes: libc::c_int = 4096;
    // Bound the real output buffer, so receiving its first byte proves main
    // is publishing a result while most of that result still cannot be sent.
    assert_eq!(
        unsafe {
            libc::setsockopt(
                writer.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&bytes as *const libc::c_int).cast(),
                std::mem::size_of_val(&bytes) as libc::socklen_t,
            )
        },
        0
    );
    let mut capacity: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&capacity) as libc::socklen_t;
    assert_eq!(
        unsafe {
            libc::getsockopt(
                writer.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&mut capacity as *mut libc::c_int).cast(),
                &mut length,
            )
        },
        0
    );
    assert!(capacity > 0 && capacity < 65536);
    let writer: OwnedFd = writer.into();
    let mut child = Owner(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .env("LAZYPENCIL_LOCK_DIR", &locks)
            .arg("--json")
            .arg("--data-dir")
            .arg(store.data_dir())
            .args(["assets", "read"])
            .arg(store.path())
            .arg("--hash")
            .arg(lazypencil::document::hash(source.as_bytes()))
            .args(["--limit", "1048576"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(writer))
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    output
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut first = [0u8; 1];
    output.read_exact(&mut first).unwrap();
    assert_eq!(first, *b"{");
    let activity = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(locks.join(format!(
            "{}.lock",
            lazypencil::document::hash(b"lazypencil-managed-assets-activity-v1")
        )))
        .unwrap();
    let error = FileExt::try_lock_exclusive(&activity).unwrap_err();
    assert_eq!(
        error.raw_os_error(),
        fs2::lock_contended_error().raw_os_error()
    );
    let mut result = first.to_vec();
    let mut buffer = [0u8; 8192];
    loop {
        output
            .set_read_timeout(Some(
                deadline
                    .checked_duration_since(Instant::now())
                    .expect("CLI output deadline"),
            ))
            .unwrap();
        let read = output.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        result.extend_from_slice(&buffer[..read]);
        assert!(result.len() < 128 * 1024);
    }
    let result: Value = serde_json::from_slice(&result).unwrap();
    assert!(result.get("error").is_none(), "{result}");
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "CLI exit deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
    FileExt::try_lock_exclusive(&activity).unwrap();
}

#[test]
fn code_assets_are_content_addressed_and_never_change_the_document() {
    let (temp, store) = setup();
    let before = store.read().unwrap().text().to_string();
    let code = "// 日本語\nreturn [];";
    let request = PutAssetRequest {
        kind: AssetKind::Script,
        source: AssetSource::Text { text: code.into() },
    };
    let first = assets::put_asset(&store, &request).unwrap();
    assert_eq!(first.hash, lazypencil::document::hash(code.as_bytes()));
    assert!(!first.reused);
    assert!(first.uri.ends_with(".js"));
    assert_eq!(
        fs::read_to_string(temp.path().join(&first.uri)).unwrap(),
        code
    );
    let second = assets::put_asset(&store, &request).unwrap();
    assert!(second.reused);
    assert_eq!(second.uri, first.uri);
    assert_eq!(store.read().unwrap().text(), before);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn a_redirected_document_directory_cannot_receive_new_managed_assets() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let store = Store::init(project.join("main.pen"), Some(temp.path().join("data"))).unwrap();
    let outside = TempDir::new().unwrap();
    fs::copy(store.path(), outside.path().join("main.pen")).unwrap();
    fs::rename(&project, temp.path().join("original-project")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &project).unwrap();
    let error = assets::put_bytes(&store, AssetKind::Shader, b"void main(){}", None).unwrap_err();
    assert_eq!(error.code, "ASSET_CONFLICT");
    assert!(!outside.path().join("assets").exists());
    assert!(!temp.path().join("original-project/assets").exists());
}

#[test]
fn a_modified_content_addressed_file_is_reported_and_not_overwritten() {
    let (temp, store) = setup();
    let asset = assets::put_bytes(&store, AssetKind::Shader, b"void main(){}", None).unwrap();
    let path = temp.path().join(&asset.uri);
    fs::write(&path, b"modified").unwrap();
    assert_eq!(
        assets::put_bytes(&store, AssetKind::Shader, b"void main(){}", None)
            .unwrap_err()
            .code,
        "ASSET_CONFLICT"
    );
    assert_eq!(fs::read(path).unwrap(), b"modified");
}

#[test]
fn concurrent_identical_assets_converge_to_one_complete_file() {
    let (temp, store) = setup();
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            let store = store.clone();
            std::thread::spawn(move || {
                barrier.wait();
                assets::put_bytes(&store, AssetKind::Script, b"return [];", None).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let assets = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(assets[0].uri, assets[1].uri);
    assert_eq!(assets.iter().filter(|a| a.reused).count(), 1);
    assert_eq!(
        fs::read(temp.path().join(&assets[0].uri)).unwrap(),
        b"return [];"
    );
    assert_eq!(
        fs::read_dir(temp.path().join("assets/lazypencil"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn asset_publication_waits_for_dependency_readers_even_with_other_data_directories() {
    for existing in [false, true] {
        let (temp, store) = setup();
        let code = b"void main(){gl_FragColor=vec4(1.0);}";
        let uri = format!(
            "assets/lazypencil/{}.glsl",
            lazypencil::document::hash(code)
        );
        let path = temp.path().join(&uri);
        if existing {
            assets::put_bytes(&store, AssetKind::Shader, code, None).unwrap();
        }
        let other = Store::init(
            temp.path().join("other.pen"),
            Some(temp.path().join("other-data")),
        )
        .unwrap();
        let locks = other
            .lock_dependencies(std::slice::from_ref(&path))
            .unwrap();
        let source = temp.path().join("source.glsl");
        fs::write(&source, code).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        command
            .arg("--json")
            .arg("--data-dir")
            .arg(temp.path().join("third-data"))
            .args(["assets", "put"])
            .arg(store.path())
            .args(["--kind", "shader", "--input"])
            .arg(source);
        let child = lock_wait::WaitingProcess::spawn(&mut command);
        assert_eq!(
            path.exists(),
            existing,
            "Publication must wait for the dependency's shared lock"
        );
        drop(locks);
        let output = child.wait_with_output();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["uri"], uri);
        assert_eq!(result["reused"], existing);
        assert_eq!(fs::read(path).unwrap(), code);
        assert!(store.history(10, None).unwrap().is_empty());
        assert!(other.history(10, None).unwrap().is_empty());
    }
}

#[test]
fn publication_rechecks_existing_bytes_after_waiting_and_never_overwrites_them() {
    let (temp, store) = setup();
    let code = b"void main(){gl_FragColor=vec4(1.0);}";
    let asset = assets::put_bytes(&store, AssetKind::Shader, code, None).unwrap();
    let path = temp.path().join(&asset.uri);
    let locks = store
        .lock_dependencies(std::slice::from_ref(&path))
        .unwrap();
    let source = temp.path().join("source.glsl");
    fs::write(&source, code).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .arg("--json")
        .args(["assets", "put"])
        .arg(store.path())
        .args(["--kind", "shader", "--input"])
        .arg(source);
    let child = lock_wait::WaitingProcess::spawn(&mut command);
    let mut changed = code.to_vec();
    changed[0] = b'x';
    fs::write(&path, &changed).unwrap();
    drop(locks);
    let output = child.wait_with_output();
    assert_eq!(output.status.code(), Some(4));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "ASSET_CONFLICT");
    assert_eq!(fs::read(&path).unwrap(), changed);
    assert!(store.history(10, None).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn cli_cancels_asset_publication_while_the_dependency_lock_is_still_held() {
    let (temp, store) = setup();
    let code = b"return [];";
    let path = temp.path().join(format!(
        "assets/lazypencil/{}.js",
        lazypencil::document::hash(code)
    ));
    let locks = store
        .lock_dependencies(std::slice::from_ref(&path))
        .unwrap();
    let source = temp.path().join("source.js");
    fs::write(&source, code).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .arg("--json")
        .args(["assets", "put"])
        .arg(store.path())
        .args(["--kind", "script", "--input"])
        .arg(source);
    let child = lock_wait::WaitingProcess::spawn(&mut command);
    let started = std::time::Instant::now();
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let output = child.wait_with_output();
    assert!(started.elapsed() < std::time::Duration::from_millis(1500));
    assert_eq!(output.status.code(), Some(130));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["error"]["code"], "CANCELLED");
    assert!(!temp.path().join("assets").exists());
    assert!(store.history(10, None).unwrap().is_empty());
    drop(locks);
    let saved = assets::put_bytes(&store, AssetKind::Script, code, None).unwrap();
    assert!(!saved.reused);
    assert_eq!(fs::read(path).unwrap(), code);
}

#[test]
fn cancelled_asset_requests_do_not_read_inputs_decode_data_or_create_files() {
    let (temp, store) = setup();
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        assets::put_bytes_with_cancellation(&store, AssetKind::Shader, &[], None, Some(&cancel))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    let request = serde_json::from_value(
        json!({"kind":"script","source":{"mime_type":"text/plain","data":"%%%"}}),
    )
    .unwrap();
    assert_eq!(
        assets::put_asset_with_cancellation(&store, &request, Some(&cancel))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert_eq!(
        assets::put_file_with_cancellation(
            &store,
            AssetKind::Script,
            &temp.path().join("does-not-exist"),
            None,
            Some(&cancel)
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
    assert!(!temp.path().join("assets").exists());
}

#[test]
fn invalid_and_unregistered_inputs_do_not_create_project_assets() {
    let (temp, store) = setup();
    let requests = [
        json!({"kind":"image","source":{"text":"not an image"}}),
        json!({"kind":"script","source":{"data":"%%%","mime_type":"text/javascript"}}),
        json!({"kind":"image","source":{"attachment_id":"not-authorized"}}),
    ];
    for value in requests {
        let request: PutAssetRequest = serde_json::from_value(value).unwrap();
        assert!(assets::put_asset(&store, &request).is_err());
    }
    assert!(assets::put_bytes(&store, AssetKind::Script, &[0xff], None).is_err());
    assert!(
        assets::put_bytes(
            &store,
            AssetKind::Script,
            &vec![b'a'; 1024 * 1024 + 1],
            None
        )
        .is_err()
    );
    assert!(!temp.path().join("assets").exists());
    assert!(
        serde_json::from_value::<PutAssetRequest>(
            json!({"kind":"script","source":{"text":"x","data":"eA==","mime_type":"text/plain"}})
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn managed_asset_directories_never_follow_a_symbolic_link() {
    let (temp, store) = setup();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("assets")).unwrap();
    assert_eq!(
        assets::put_bytes(&store, AssetKind::Script, b"return [];", None)
            .unwrap_err()
            .code,
        "ASSET_CONFLICT"
    );
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}

#[test]
fn real_cli_stores_code_and_reads_it_after_an_atomic_reference_edit() {
    let (temp, store) = setup();
    let source = temp.path().join("draw.js");
    fs::write(&source, "return [];").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["assets", "put"])
        .arg(store.path())
        .args(["--kind", "script", "--input"])
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let asset: Value = serde_json::from_slice(&output.stdout).unwrap();
    let request=serde_json::from_value(json!({"request_id":"script","expected_revision":store.read().unwrap().revision(),
        "operations":[{"op":"insert","node":{"type":"script","scriptUri":asset["uri"],"width":100,"height":100}}]})).unwrap();
    lazypencil::service::apply_request(&store, &request).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .arg("--json")
        .args(["assets", "read"])
        .arg(store.path())
        .arg("--hash")
        .arg(asset["hash"].as_str().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["content"], "return [];");
}

#[cfg(feature = "rendering")]
mod images {
    use super::*;
    use skia_safe::{Color, Data, EncodedOrigin, Image, Paint, Rect};
    fn input(origin: Option<EncodedOrigin>) -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((40, 20)).unwrap();
        surface.canvas().clear(Color::RED);
        let mut paint = Paint::default();
        paint.set_color(Color::BLUE);
        surface
            .canvas()
            .draw_rect(Rect::from_xywh(20.0, 0.0, 20.0, 20.0), &paint);
        if let Some(origin) = origin {
            skia_safe::jpeg_encoder::encode_image(
                None,
                &surface.image_snapshot(),
                &skia_safe::jpeg_encoder::Options {
                    origin: Some(origin),
                    downsample: skia_safe::jpeg_encoder::Downsample::No,
                    ..Default::default()
                },
            )
            .unwrap()
            .as_bytes()
            .into()
        } else {
            skia_safe::png_encoder::encode_image(
                None,
                &surface.image_snapshot(),
                &Default::default(),
            )
            .unwrap()
            .as_bytes()
            .into()
        }
    }
    #[test]
    fn jpeg_orientation_is_applied_and_metadata_is_not_forwarded() {
        let bytes = input(Some(EncodedOrigin::RightTop));
        let image = lazypencil::image_input::normalize(&bytes, Some("image/jpeg")).unwrap();
        assert_eq!((image.width, image.height), (20, 40));
        assert_eq!(image.source_mime, "image/jpeg");
        let normalized = Image::from_encoded(Data::new_copy(&image.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixmap = normalized.peek_pixels().unwrap();
        assert!(pixmap.get_color((10, 5)).r() > 240);
        assert!(pixmap.get_color((10, 35)).b() > 240);
        assert!(!image.png.windows(6).any(|w| w == b"Exif\0\0"));
    }
    #[test]
    fn image_assets_are_normalized_and_can_be_rendered_after_linking() {
        let (temp, store) = setup();
        let bytes = input(None);
        let asset = assets::put_bytes(&store, AssetKind::Image, &bytes, Some("image/png")).unwrap();
        assert_eq!((asset.width, asset.height), (Some(40), Some(20)));
        assert_eq!(asset.mime_type, "image/png");
        let request=serde_json::from_value(json!({"request_id":"image","expected_revision":store.read().unwrap().revision(),"operations":[
            {"op":"insert","node":{"type":"rectangle","width":80,"height":40,"fill":{"type":"image","url":asset.uri}}}
        ]})).unwrap();
        lazypencil::service::apply_request(&store, &request).unwrap();
        let (project, snapshot) =
            lazypencil::service::layout_snapshot(&store, &[], None, None).unwrap();
        let rendered = lazypencil::render::png(&project, &snapshot, &Default::default()).unwrap();
        assert_eq!((rendered.width, rendered.height), (80, 40));
        assert!(temp.path().join(asset.uri).is_file());
    }
    #[test]
    fn false_mime_types_truncation_and_animation_are_rejected() {
        let bytes = input(None);
        assert!(lazypencil::image_input::normalize(&bytes, Some("image/jpeg")).is_err());
        assert!(lazypencil::image_input::normalize(&bytes[..bytes.len() / 2], None).is_err());
        assert!(lazypencil::image_input::normalize(&bytes[..bytes.len() - 12], None).is_err());
        let jpeg = input(Some(EncodedOrigin::TopLeft));
        assert!(lazypencil::image_input::normalize(&jpeg[..jpeg.len() - 2], None).is_err());
        let mut animated = b"\x89PNG\r\n\x1a\n".to_vec();
        animated.extend_from_slice(&0u32.to_be_bytes());
        animated.extend_from_slice(b"acTL");
        animated.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(
            lazypencil::image_input::normalize(&animated, None)
                .unwrap_err()
                .code,
            "ATTACHMENT_INVALID"
        );
        let webp = b"RIFF\x0c\0\0\0WEBPANIM\0\0\0\0";
        assert!(lazypencil::image_input::normalize(webp, None).is_err());
    }
}
