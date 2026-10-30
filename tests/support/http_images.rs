use super::*;
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[path = "fixture.rs"]
mod fixture_support;
#[path = "page_wait_reference.rs"]
mod reference;
pub(super) use reference::Server;

pub(super) fn cli(store: &Store, args: &[&str]) -> Value {
    let value = cli_result(store, args);
    assert!(value.get("error").is_none(), "{value}");
    value
}

pub(super) fn cli_result(store: &Store, args: &[&str]) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .arg("--json")
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(args);
    let output = fixture_support::output(&mut command, Duration::from_secs(35));
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(
        output.status.success(),
        value.get("error").is_none(),
        "{value}"
    );
    value
}

pub(super) fn apply(store: &Store, operations: Value, dry: bool) -> Value {
    let request = tempfile::NamedTempFile::new().unwrap();
    let metadata = cli(store, &["inspect", store.path().to_str().unwrap()]);
    fs::write(request.path(), operations.to_string()).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut arguments = vec![
        "apply",
        store.path().to_str().unwrap(),
        "--operations",
        request.path().to_str().unwrap(),
        "--expected-revision",
        metadata["revision"].as_str().unwrap(),
        "--expected-dependency-revision",
        metadata["dependency_revision"].as_str().unwrap(),
        "--request-id",
        &id,
    ];
    if dry {
        arguments.push("--dry-run");
    }
    cli(store, &arguments)
}

fn load(store: &Store) -> lazypencil::Result<Project> {
    Project::load_with_remote_worker(store, Path::new(env!("CARGO_BIN_EXE_lazypencil")), None)
}

fn unavailable(store: &Store) -> lazypencil::Error {
    match load(store).and_then(|project| project.resolve()) {
        Ok(_) => panic!("Accepted an unavailable HTTP image"),
        Err(error) => error,
    }
}

pub(super) fn fixture(version: &str) -> (TempDir, Store, reference::Server, [String; 2]) {
    let (temp, store) = array_fixture();
    let server = reference::Server::new();
    let urls = ["redirect.png", "blue.png"].map(|name| {
        format!(
            "http://{}/network/http-image/{name}?original=value#keep",
            server.address
        )
    });
    let source = temp.path().join("library/data/images.lib.pen");
    let mut data: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    data["variables"]["leaf"]["value"] =
        json!([{"value":urls[0]},{"theme":{"mode":"dark"},"value":urls[1]}]);
    write(&source, data);
    let library = temp.path().join("library/ui.lib.pen");
    let mut data: Value = serde_json::from_slice(&fs::read(&library).unwrap()).unwrap();
    // The raw URL label is much wider than a local filename. Clip it to the
    // tile so shader-effect bounds retain the intended two sampler halves.
    data["children"][0]["clip"] = json!(true);
    write(&library, data);
    for path in [
        source,
        temp.path().join("library/ui.lib.pen"),
        store.path().into(),
    ] {
        let mut data: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        data["version"] = json!(version);
        write(&path, data);
    }
    (temp, store, server, urls)
}

#[test]
fn http_sampler_copy_preserves_both_versions_themes_text_after_network_and_source_removal() {
    for version in ["2.19", "2.21"] {
        let (temp, store, server, urls) = fixture(version);
        let original = fs::read(store.path()).unwrap();
        let captured = load(&store).unwrap();
        let node = captured.resolve().unwrap().roots.remove(0);
        let key = captured
            .resolve_image(node.property_source("fill"), &urls[0])
            .unwrap();
        let expected = captured.asset(&key).unwrap().revision.clone();
        assert!(
            captured
                .dependencies()
                .iter()
                .any(|dependency| dependency.kind == "remote_image" && dependency.path == key)
        );
        assert!(
            captured.resolve_file(store.path(), &urls[0]).is_err(),
            "HTTP images must not extend code/import roots"
        );
        let before_requests = server.network_counts().0;
        captured.verify_unchanged().unwrap();
        assert_eq!(
            server.network_counts().0,
            before_requests,
            "Verifying a captured image must not refetch its source"
        );
        apply(&store, copy_operations(), true);
        assert_eq!(fs::read(store.path()).unwrap(), original);
        assert!(!temp.path().join("assets").exists());
        assert!(store.history(10, None).unwrap().is_empty());
        let result = apply(&store, copy_operations(), false);
        let target = result["generated_ids"]["0"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let component = result["generated_ids"]["0"]["component_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(server.network_counts().0 > before_requests);
        drop(server);
        captured.verify_unchanged().unwrap();
        assert_eq!(captured.asset(&key).unwrap().revision, expected);
        fs::remove_dir_all(temp.path().join("library")).unwrap();
        let moved = TempDir::new().unwrap();
        fs::create_dir(moved.path().join(".git")).unwrap();
        fs::rename(temp.path().join("assets"), moved.path().join("assets")).unwrap();
        fs::rename(store.path(), moved.path().join("main.pen")).unwrap();
        let moved = Store::open(
            moved.path().join("main.pen"),
            Some(moved.path().join("data")),
        )
        .unwrap();
        fs::write(
            moved.path().parent().unwrap().join("lazypencil.toml"),
            "[runtime.browser]\nnetwork_policy='offline'\n",
        )
        .unwrap();
        for (index, mode) in ["light", "dark"].into_iter().enumerate() {
            edit(
                &moved,
                &format!("http-{mode}"),
                json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
            );
            let project = Project::load(&moved).unwrap();
            let node = project
                .resolve()
                .unwrap()
                .roots
                .into_iter()
                .find(|node| node.id() == target)
                .unwrap();
            assert_eq!(node.children[0].properties["content"], urls[index]);
            let image = Path::new(
                node.properties["fill"]["uniforms"]["image"][0]
                    .as_str()
                    .unwrap(),
            );
            assert!(image.starts_with(moved.path().parent().unwrap().join("assets/lazypencil")));
            if index == 0 {
                assert_eq!(project.asset(image).unwrap().revision, expected);
            }
            assert!(
                project
                    .dependencies()
                    .iter()
                    .all(|dependency| dependency.kind != "remote_image")
            );
        }
        let copied = edit(
            &moved,
            "http-recopy",
            json!([
                {"op":"import_component","source":{"node_id":component},"mode":"copy","options":{"variable_prefix":"again_"}},
                {"op":"delete","target":target}
            ]),
        );
        let target = copied["generated_ids"]["0"]["id"].as_str().unwrap();
        for (index, mode) in ["light", "dark"].into_iter().enumerate() {
            edit(
                &moved,
                &format!("http-recopied-{mode}"),
                json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
            );
            let project = Project::load(&moved).unwrap();
            let scene = project.resolve().unwrap();
            let node = scene.roots.iter().find(|node| node.id() == target).unwrap();
            assert_eq!(node.children[0].properties["content"], urls[index]);
            let path = Path::new(
                node.properties["fill"]["uniforms"]["image"][0]
                    .as_str()
                    .unwrap(),
            );
            assert!(path.starts_with(moved.path().parent().unwrap().join("assets/lazypencil")));
            assert!(!project.asset(path).unwrap().bytes.is_empty());
        }
        let future = reference::Server::new();
        fs::remove_file(moved.path().parent().unwrap().join("lazypencil.toml")).unwrap();
        let uri = format!(
            "http://{}/network/http-image/yellow.png?future",
            future.address
        );
        apply(
            &moved,
            json!([{"op":"set_variable","name":"again_copied_leaf","definition":{"type":"string","value":uri}}]),
            false,
        );
        let project = load(&moved).unwrap();
        let scene = project.resolve().unwrap();
        let node = scene.roots.iter().find(|node| node.id() == target).unwrap();
        assert_eq!(node.children[0].properties["content"], uri);
        assert!(
            project
                .dependencies()
                .iter()
                .any(|dependency| dependency.kind == "remote_image")
        );
    }
}

#[test]
fn http_images_reject_invalid_redirects_responses_credentials_and_offline_without_leaking_urls() {
    let server = reference::Server::new();
    for route in [
        "file.png",
        "credentials.png",
        "loop.png",
        "oversized.png",
        "truncated.png",
        "invalid.png",
    ] {
        let (temp, store) = fixture_image(format!(
            "http://{}/network/http-image/{route}?private-query-token",
            server.address
        ));
        let error = unavailable(&store);
        let message = serde_json::to_string(&error).unwrap();
        for secret in [
            "private-query-token",
            "private-response-body",
            "fixture-password",
        ] {
            assert!(!message.contains(secret), "{message}");
        }
        assert!(!temp.path().join("assets").exists());
    }
    let before = server.network_counts();
    let (temp, store) = fixture_image(format!(
        "http://{}/network/http-image/yellow.png",
        server.address
    ));
    fs::write(
        temp.path().join("lazypencil.toml"),
        "[runtime.browser]\nnetwork_policy='offline'\n",
    )
    .unwrap();
    assert!(!load(&store).unwrap().assets_complete());
    assert_eq!(server.network_counts(), before);
    for uri in [
        format!(
            "http://user:private-query-token@{}/network/http-image/yellow.png",
            server.address
        ),
        "http:relative".into(),
    ] {
        let (_temp, store) = fixture_image(uri);
        assert!(load(&store).is_err());
    }
    assert_eq!(server.network_counts(), before);
}

fn fixture_image(uri: String) -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("image.pen");
    write(
        &file,
        json!({"version":"2.21","children":[{"id":"image","type":"rectangle","width":16,"height":16,"fill":{"type":"image","url":uri}}]}),
    );
    let store = Store::open(file, Some(temp.path().join("data"))).unwrap();
    (temp, store)
}

#[test]
fn http_image_cancellation_and_deadlines_stop_an_actual_blocked_response() {
    for cancellation in [true, false] {
        let observed = Arc::new(AtomicBool::new(false));
        let server = reference::Server::with_selector_probe(observed.clone());
        let (temp, store) = fixture_image(format!(
            "http://{}/network/http-image/blocked.png",
            server.address
        ));
        fs::write(
            temp.path().join("lazypencil.toml"),
            format!(
                "[runtime.browser]\ntimeout_ms={}\n",
                if cancellation { 30000 } else { 1000 }
            ),
        )
        .unwrap();
        let token = CancellationToken::new();
        let started = Instant::now();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !observed.load(Ordering::SeqCst) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(
                    observed.load(Ordering::SeqCst),
                    "The actual image response must be blocked first"
                );
                if cancellation {
                    token.cancel();
                }
            });
            Project::load_with_remote_worker(
                &store,
                Path::new(env!("CARGO_BIN_EXE_lazypencil")),
                Some(&token),
            )
        });
        let error = match result.and_then(|project| project.resolve()) {
            Ok(_) => panic!("Accepted a blocked response"),
            Err(error) => error,
        };
        assert_eq!(
            error.code,
            if cancellation {
                "CANCELLED"
            } else {
                "ASSET_NOT_FOUND"
            }
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        drop(server);
    }
}

#[cfg(unix)]
#[test]
fn http_image_cli_interrupt_reaps_the_worker_during_an_asset_read() {
    struct Owner(std::process::Child);
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let observed = Arc::new(AtomicBool::new(false));
    let server = reference::Server::with_selector_probe(observed.clone());
    let (_temp, store) = fixture_image(format!(
        "http://{}/network/http-image/blocked.png",
        server.address
    ));
    let before = fs::read(store.path()).unwrap();
    let output = tempfile::NamedTempFile::new().unwrap();
    let mut child = Owner(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .arg("--json")
            .arg("--data-dir")
            .arg(store.data_dir())
            .args(["assets", "read"])
            .arg(store.path())
            .arg("--hash")
            .arg("0".repeat(64))
            .stdin(std::process::Stdio::null())
            .stdout(output.reopen().unwrap())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !observed.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "CLI image request never reached the owned server"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let workers = fixture_support::child_pids(child.0.id());
    assert_eq!(workers.len(), 1);
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGINT) }, 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "Interrupted HTTP image acquisition did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let result: Value = serde_json::from_slice(&fs::read(output.path()).unwrap()).unwrap();
    assert_eq!(result["error"]["code"], "CANCELLED", "{result}");
    assert_eq!(
        unsafe { libc::kill(workers[0] as i32, 0) },
        -1,
        "HTTP worker was not reaped"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert_eq!(fs::read(store.path()).unwrap(), before);
}

#[test]
fn http_image_retention_review_never_starts_a_network_acquisition() {
    let server = reference::Server::new();
    let (temp, store) = fixture_image(format!(
        "http://{}/network/http-image/yellow.png",
        server.address
    ));
    let bytes = b"orphan shader";
    let managed = temp.path().join(format!(
        "assets/lazypencil/{}.glsl",
        lazypencil::document::hash(bytes)
    ));
    fs::create_dir_all(managed.parent().unwrap()).unwrap();
    fs::write(&managed, bytes).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .env("LAZYPENCIL_LOCK_DIR", temp.path().join("isolated-locks"))
        .arg("--json")
        .arg("--data-dir")
        .arg(store.data_dir())
        .args(["assets", "prune"])
        .arg(store.path())
        .arg("--project-root")
        .arg(temp.path());
    let output = fixture_support::output(&mut command, Duration::from_secs(35));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{value}");
    assert_eq!(
        value["candidates"][0]["path"],
        json!(fs::canonicalize(&managed).unwrap()),
        "{value}"
    );
    assert_eq!(server.network_counts(), (0, 0, 0));
}

#[test]
fn http_image_failures_remain_inspectable_and_structurally_repairable() {
    let server = reference::Server::new();
    let (_temp, store) = fixture_image(format!(
        "http://{}/network/http-image/invalid.png?private-query-token",
        server.address
    ));
    let metadata = cli(&store, &["inspect", store.path().to_str().unwrap()]);
    assert_eq!(metadata["assets_complete"], false);
    assert!(
        metadata["asset_diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["kind"] == "remote_image")
    );
    let result = apply(
        &store,
        json!([{"op":"update","target":"image","set":{"fill":"#00ff00"}}]),
        false,
    );
    assert_eq!(result["assets_complete"], true, "{result}");
    assert!(load(&store).unwrap().assets_complete());
}

#[cfg(feature = "rendering")]
pub(super) fn real_webgl_case(evidence: &mut Vec<Value>) -> Value {
    use lazypencil::{dynamic::DynamicRuntime, render};
    use skia_safe::{Color, Data, Image};
    let (temp, store, server, _urls) = fixture("2.21");
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap());
    let mut originals = Vec::new();
    for mode in ["light", "dark"] {
        let mut document = store.read().unwrap().value().clone();
        document["children"][0]["theme"] = json!({"mode":mode});
        write(store.path(), document);
        let project = load(&store).unwrap();
        let snapshot = lazypencil::layout::compute(
            project.resolve().unwrap(),
            &mut project.load_fonts().unwrap(),
        )
        .unwrap();
        let rendered =
            render::png_with_runtime(&project, &snapshot, &Default::default(), &mut runtime, None)
                .unwrap();
        let image = Image::from_encoded(Data::new_copy(&rendered.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        assert_eq!(
            image.peek_pixels().unwrap().get_color((10, 10)),
            if mode == "light" {
                Color::YELLOW
            } else {
                Color::BLUE
            }
        );
        assert_eq!(
            image.peek_pixels().unwrap().get_color((75, 10)),
            Color::BLUE
        );
        originals.push(rendered.png);
    }
    let result = apply(&store, copy_operations(), false);
    let target = result["generated_ids"]["0"]["id"].as_str().unwrap();
    drop(server);
    fs::remove_dir_all(temp.path().join("library")).unwrap();
    for (index, mode) in ["light", "dark"].into_iter().enumerate() {
        edit(
            &store,
            &format!("http-pixels-{mode}"),
            json!([{"op":"set_theme","target":target,"set":{"mode":mode}}]),
        );
        let (project, snapshot) = service::layout_snapshot(&store, &[], None, None).unwrap();
        let rendered =
            render::png_with_runtime(&project, &snapshot, &Default::default(), &mut runtime, None)
                .unwrap();
        assert_eq!(
            originals[index], rendered.png,
            "HTTP copied sampler pixels differ for {mode}"
        );
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            let directory = Path::new(&directory);
            let before = format!("codec-sampler-copy-http-{mode}-before.png");
            let after = format!("codec-sampler-copy-http-{mode}-after.png");
            fs::write(directory.join(&before), &originals[index]).unwrap();
            fs::write(directory.join(&after), &rendered.png).unwrap();
            evidence.push(json!({"source_kind":"http","absolute":false,"theme":mode,"before":before,"after":after,"width":rendered.width,"height":rendered.height,"png_hash":lazypencil::document::hash(&rendered.png)}));
        }
    }
    // This repository-owned image exercises the real public root store/TLS
    // path once in the hosted browser gate, without weakening TLS for fixtures.
    let url = "https://raw.githubusercontent.com/EkeMinusYou/lazypencil/094a4c284e66c4ce8a96ffd4d027db614f3ea659/examples/assets/crop-source.png";
    let (_temp, store) = fixture_image(url.into());
    let project = load(&store).unwrap();
    let path = project.resolve_image(store.path(), url).unwrap();
    let asset = project.asset(&path).unwrap();
    let expected =
        lazypencil::document::hash(include_bytes!("../../examples/assets/crop-source.png"));
    assert_eq!(asset.revision, expected);
    json!({"source_commit":"094a4c284e66c4ce8a96ffd4d027db614f3ea659","sha256":expected,"bytes":asset.bytes.len()})
}
