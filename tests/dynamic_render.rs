#![cfg(feature = "rendering")]
use base64::Engine;
use lazypencil::{
    document::Document,
    dynamic::DynamicRuntime,
    project::Project,
    render::{self, RenderOptions},
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use tempfile::TempDir;

#[cfg(unix)]
#[path = "support/fixture.rs"]
mod fixture_support;
#[path = "support/page_wait_reference.rs"]
mod page_wait_reference;
#[cfg(unix)]
#[path = "support/terminal/mod.rs"]
mod terminal;

fn executable() -> PathBuf {
    std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
        .map(PathBuf::from)
        .expect("Set LAZYPENCIL_TEST_CHROMIUM to a verified headless Chromium executable")
}
fn scene(children: Value) -> (TempDir, Project, lazypencil::layout::LayoutSnapshot) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("scene.pen");
    let source = json!({"version":"2.21","children":children}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    (temp, project, layout)
}
fn raster(bytes: &[u8]) -> Image {
    Image::from_encoded(Data::new_copy(bytes))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap()
}
fn page(html: &str) -> String {
    format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(html)
    )
}

fn retain_page_image(name: &str, image: &render::RenderedImage) {
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_RETAINED_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(format!("chromium-retained-{name}.png")),
            &image.png,
        )
        .unwrap();
        let report = json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"width":image.width,"height":image.height,"png_hash":lazypencil::document::hash(&image.png),"snapshot_id":image.snapshot_id,"dynamic_artifacts":image.dynamic_artifacts});
        fs::write(
            directory.join(format!("chromium-retained-{name}.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "Requires sandboxed Chromium and software WebGL"]
fn retained_page_frame_capture_preserves_layout_dom_and_static_shader_acquisitions() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("blue.glsl"),
        "precision highp float; void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    let html = "<style>html{background:#f00}</style><script>let n=0;const raf=window.requestAnimationFrame.bind(window);window.requestAnimationFrame=cb=>raf(t=>{document.documentElement.style.background=++n<=2?'#f00':'#0f0';cb(t)});</script>";
    let source = json!({"version":"2.21","children":[
        {"id":"page","type":"browser","width":32,"height":32,"url":page(html)},
        {"id":"shader","type":"rectangle","x":32,"width":32,"height":32,"fill":{"type":"shader","url":"blue.glsl"}}
    ]}).to_string();
    let file = temp.path().join("scene.pen");
    fs::write(&file, &source).unwrap();
    let project =
        Project::from_document(&file, Document::parse(source.clone()).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable())
        .with_retained_pages(&file);
    let first =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let pids = runtime.process_ids();
    let lookup = |image: &render::RenderedImage, kind: &str| -> Value {
        image
            .dynamic_artifacts
            .iter()
            .find(|entry| entry["acquisition"]["kind"] == kind)
            .unwrap()["acquisition"]
            .clone()
    };
    let page_first = lookup(&first, "browser");
    let shader_first = lookup(&first, "shader");
    let frame = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh_pages: true,
            time: 1.0,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    let page_frame = lookup(&frame, "browser");
    assert_ne!(page_frame["acquisition_id"], page_first["acquisition_id"]);
    assert_eq!(
        page_frame["page_state"]["id"],
        page_first["page_state"]["id"]
    );
    assert_eq!(page_frame["page_state"]["reused"], true);
    assert_eq!(
        lookup(&frame, "shader"),
        shader_first,
        "A page frame must preserve a time-independent shader raster"
    );
    assert_eq!(first.snapshot_id, frame.snapshot_id);
    assert_eq!(runtime.process_ids(), pids);
    assert_eq!(
        raster(&first.png)
            .peek_pixels()
            .unwrap()
            .get_color((16, 16)),
        Color::RED
    );
    let bitmap = raster(&frame.png);
    assert_eq!(
        bitmap.peek_pixels().unwrap().get_color((16, 16)),
        Color::GREEN
    );
    assert_eq!(
        bitmap.peek_pixels().unwrap().get_color((48, 16)),
        Color::BLUE
    );
    let paused = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            time: 1.0,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(
        paused.dynamic_artifacts, frame.dynamic_artifacts,
        "Frame refresh must not leak into the next render"
    );
    assert_eq!(paused.png, frame.png);
    assert_eq!(fs::read_to_string(file).unwrap(), source);
    retain_page_image("periodic-first", &first);
    retain_page_image("periodic-frame", &frame);
    #[cfg(unix)]
    {
        // Keep the real frontend reproduction inside this early retained-page
        // gate, without repeating the unrelated 60-second idle PTY test.
        let terminal = TempDir::new().unwrap();
        let result = terminal::run(
            std::path::Path::new(env!("CARGO_BIN_EXE_lazypencil")),
            terminal.path(),
            "page-playback",
            &[("LAZYPENCIL_CHROMIUM".into(), executable().into_os_string())],
        );
        assert!(
            result.is_ok(),
            "{result:?}\n{}\n{}",
            fs::read_to_string(terminal.path().join("terminal-errors.log")).unwrap_or_default(),
            fs::read_to_string(terminal.path().join("terminal-screen.txt")).unwrap_or_default()
        );
        let proof = fs::read(terminal.path().join("page-playback.json")).unwrap();
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_RETAINED_DIR") {
            fs::write(
                PathBuf::from(directory).join("chromium-page-playback.json"),
                proof,
            )
            .unwrap();
        }
    }
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn retained_page_contexts_are_isolated_bounded_and_evicted_in_recency_order() {
    use lazypencil::{
        cdp::{Browser, Budget, MAX_RETAINED_PAGES, Purpose},
        runtime::InstalledBrowser,
    };
    let installed = InstalledBrowser {
        executable: executable(),
        version: "external-unverified".into(),
        platform: lazypencil::runtime::platform().unwrap().into(),
    };
    let mut browser = Browser::launch_with_supervisor(
        &installed,
        Purpose::Page,
        &Budget::new(Duration::from_secs(30), None),
        std::path::Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    let visit = |browser: &mut Browser, key: &str, signature: &str| {
        let budget = Budget::new(Duration::from_secs(30), None);
        browser
            .with_page(
                Some((key, signature)),
                &budget,
                |browser, session, state| {
                    let count = browser.evaluate(
                        session,
                        "(window.lpVisits=(window.lpVisits||0)+1)",
                        &budget,
                    )?;
                    Ok((
                        state.id.clone().unwrap(),
                        count.as_u64().unwrap(),
                        state.reused,
                    ))
                },
            )
            .unwrap()
    };
    let first = visit(&mut browser, "node-0", "url-A");
    assert_eq!((first.1, first.2), (1, false));
    for index in 1..MAX_RETAINED_PAGES {
        assert_eq!(visit(&mut browser, &format!("node-{index}"), "url-A").1, 1);
    }
    let repeated = visit(&mut browser, "node-0", "url-A");
    assert_eq!(repeated, (first.0.clone(), 2, true));
    visit(&mut browser, "overflow", "url-A");
    let recreated = visit(&mut browser, "node-1", "url-A");
    assert_eq!((recreated.1, recreated.2), (1, false));
    assert_eq!(
        visit(&mut browser, "node-0", "url-A"),
        (first.0.clone(), 3, true)
    );
    let replaced = visit(&mut browser, "node-0", "url-B");
    assert_ne!(replaced.0, first.0);
    assert_eq!((replaced.1, replaced.2), (1, false));
    let budget = Budget::new(Duration::from_secs(30), None);
    let contexts = browser
        .command(None, "Target.getBrowserContexts", json!({}), &budget)
        .unwrap();
    assert_eq!(
        contexts["browserContextIds"].as_array().unwrap().len(),
        MAX_RETAINED_PAGES
    );
    let fresh = browser
        .with_tab(&budget, |browser, session| {
            browser.evaluate(session, "window.lpVisits||0", &budget)
        })
        .unwrap();
    assert_eq!(fresh, 0);
    let contexts = browser
        .command(None, "Target.getBrowserContexts", json!({}), &budget)
        .unwrap();
    assert_eq!(
        contexts["browserContextIds"].as_array().unwrap().len(),
        MAX_RETAINED_PAGES
    );
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn retained_page_refresh_preserves_dom_isolates_nodes_and_resets_changed_or_removed_state() {
    let html = "<style>html{background:#f00}</style><script>let n=0;const raf=window.requestAnimationFrame.bind(window);window.requestAnimationFrame=cb=>raf(t=>{document.documentElement.style.background=++n<=2?'#f00':'#0f0';cb(t)});</script>";
    let url = page(html);
    let (temp, _, _) = scene(json!([
        {"id":"a","type":"browser","url":url,"width":32,"height":32},
        {"id":"b","type":"browser","url":url,"x":32,"width":32,"height":32}
    ]));
    fs::create_dir(temp.path().join(".git")).unwrap();
    let file = temp.path().join("scene.pen");
    let saved = fs::read_to_string(&file).unwrap();
    let config = temp.path().join("lazypencil.toml");
    fs::write(&config, "[runtime.browser]\nwait_until='network_idle'\n").unwrap();
    let load = || {
        let project = Project::from_document(
            &file,
            Document::parse(fs::read_to_string(&file).unwrap()).unwrap(),
            &[],
        )
        .unwrap();
        let snapshot =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        (project, snapshot)
    };
    let (project, snapshot) = load();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable())
        .with_retained_pages(&file);
    let acquire = |runtime: &mut DynamicRuntime,
                   project: &Project,
                   snapshot: &lazypencil::layout::LayoutSnapshot,
                   refresh| {
        render::png_with_runtime(
            project,
            snapshot,
            &RenderOptions {
                refresh,
                ..Default::default()
            },
            runtime,
            None,
        )
        .unwrap()
    };
    let first = acquire(&mut runtime, &project, &snapshot, false);
    retain_page_image("first", &first);
    let ids: Vec<_> = first
        .dynamic_artifacts
        .iter()
        .map(|entry| entry["acquisition"]["page_state"]["id"].clone())
        .collect();
    assert_ne!(
        ids[0], ids[1],
        "Identical URLs must not share a node's DOM or cached acquisition"
    );
    for entry in &first.dynamic_artifacts {
        assert_eq!(entry["acquisition"]["page_state"]["retained"], true);
        assert_eq!(entry["acquisition"]["page_state"]["reused"], false);
    }
    let pixels = raster(&first.png);
    assert_eq!(
        pixels.peek_pixels().unwrap().get_color((16, 16)),
        Color::RED
    );
    assert_eq!(
        pixels.peek_pixels().unwrap().get_color((48, 16)),
        Color::RED
    );
    let cached = acquire(&mut runtime, &project, &snapshot, false);
    assert_eq!(cached.dynamic_artifacts, first.dynamic_artifacts);
    let refreshed = acquire(&mut runtime, &project, &snapshot, true);
    retain_page_image("refreshed", &refreshed);
    for (index, entry) in refreshed.dynamic_artifacts.iter().enumerate() {
        assert_eq!(entry["acquisition"]["page_state"]["id"], ids[index]);
        assert_eq!(entry["acquisition"]["page_state"]["reused"], true);
    }
    let pixels = raster(&refreshed.png);
    assert_eq!(
        pixels.peek_pixels().unwrap().get_color((16, 16)),
        Color::GREEN
    );
    assert_eq!(
        pixels.peek_pixels().unwrap().get_color((48, 16)),
        Color::GREEN
    );
    let pids = runtime.process_ids();
    let mut changed: Value = serde_json::from_str(&saved).unwrap();
    changed["children"][0]["width"] = json!(40);
    fs::write(&file, changed.to_string()).unwrap();
    let (project, snapshot) = load();
    let resized = acquire(&mut runtime, &project, &snapshot, false);
    assert_eq!(
        resized.dynamic_artifacts[0]["acquisition"]["page_state"]["id"],
        ids[0]
    );
    assert_eq!(
        resized.dynamic_artifacts[0]["acquisition"]["viewport_width"].as_f64(),
        Some(40.0)
    );
    assert_eq!(runtime.process_ids(), pids);
    changed["children"][0]["url"] = json!(page(&(html.to_owned() + " ")));
    fs::write(&file, changed.to_string()).unwrap();
    let (project, snapshot) = load();
    let navigated = acquire(&mut runtime, &project, &snapshot, false);
    assert_ne!(
        navigated.dynamic_artifacts[0]["acquisition"]["page_state"]["id"],
        ids[0]
    );
    assert_eq!(
        navigated.dynamic_artifacts[0]["acquisition"]["page_state"]["reused"],
        false
    );
    assert_eq!(
        navigated.dynamic_artifacts[1]["acquisition"]["page_state"]["id"],
        ids[1]
    );
    // Restoring the old URL cannot resurrect its earlier green cached acquisition.
    fs::write(&file, &saved).unwrap();
    let (project, snapshot) = load();
    let restored = acquire(&mut runtime, &project, &snapshot, false);
    let restored_a = restored.dynamic_artifacts[0]["acquisition"]["page_state"]["id"].clone();
    assert_ne!(restored_a, ids[0]);
    assert_eq!(
        raster(&restored.png)
            .peek_pixels()
            .unwrap()
            .get_color((16, 16)),
        Color::RED
    );
    changed = serde_json::from_str(&saved).unwrap();
    changed["children"].as_array_mut().unwrap().remove(0);
    fs::write(&file, changed.to_string()).unwrap();
    let (project, snapshot) = load();
    acquire(&mut runtime, &project, &snapshot, false);
    fs::write(&file, &saved).unwrap();
    let (project, snapshot) = load();
    let reinserted = acquire(&mut runtime, &project, &snapshot, false);
    retain_page_image("reinserted", &reinserted);
    assert_ne!(
        reinserted.dynamic_artifacts[0]["acquisition"]["page_state"]["id"],
        restored_a
    );
    assert_eq!(
        reinserted.dynamic_artifacts[1]["acquisition"]["page_state"]["id"],
        ids[1]
    );
    fs::write(
        &config,
        "[runtime.browser]\nwait_until='network_idle'\ntimeout_ms=31000\n",
    )
    .unwrap();
    let (project, snapshot) = load();
    let policy = acquire(&mut runtime, &project, &snapshot, false);
    assert_ne!(
        policy.dynamic_artifacts[1]["acquisition"]["page_state"]["id"],
        ids[1]
    );
    assert!(
        policy
            .dynamic_artifacts
            .iter()
            .all(|entry| entry["acquisition"]["page_state"]["reused"] == false)
    );
    runtime.expire_idle(Instant::now() + Duration::from_secs(61));
    assert!(runtime.process_ids().is_empty());
    let cached = acquire(&mut runtime, &project, &snapshot, false);
    assert_eq!(cached.dynamic_artifacts, policy.dynamic_artifacts);
    assert!(runtime.process_ids().is_empty());
    let restarted = acquire(&mut runtime, &project, &snapshot, true);
    retain_page_image("restarted", &restarted);
    assert_ne!(
        restarted.dynamic_artifacts[0]["acquisition"]["page_state"]["id"],
        policy.dynamic_artifacts[0]["acquisition"]["page_state"]["id"]
    );
    let mut fresh = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let first_fresh = acquire(&mut fresh, &project, &snapshot, false);
    let second_fresh = acquire(&mut fresh, &project, &snapshot, true);
    assert_eq!(first_fresh.png, second_fresh.png);
    assert!(
        second_fresh.dynamic_artifacts.iter().all(
            |entry| entry["acquisition"]["page_state"]["retained"] == false
                && entry["acquisition"]["page_state"]["id"].is_null()
        )
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), saved);
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn cancelling_a_retained_capture_discards_the_context_and_recovers_with_new_state() {
    let html = "<style>html{background:#f00}</style><script>let n=0;const raf=window.requestAnimationFrame.bind(window);window.requestAnimationFrame=cb=>raf(t=>{if(++n>2)for(;;){}cb(t)});</script>";
    let (temp, project, snapshot) =
        scene(json!([{"id":"a","type":"browser","url":page(html),"width":32,"height":32}]));
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable())
        .with_retained_pages(&temp.path().join("scene.pen"));
    let first =
        render::png_with_runtime(&project, &snapshot, &Default::default(), &mut runtime, None)
            .unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let token = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        token.cancel();
    });
    let failed = render::png_with_runtime(
        &project,
        &snapshot,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        Some(cancel),
    )
    .err()
    .expect("Retained DOM becomes unresponsive");
    thread.join().unwrap();
    assert_eq!(failed.code, "CANCELLED");
    assert!(runtime.process_ids().is_empty());
    let recovered = render::png_with_runtime(
        &project,
        &snapshot,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(recovered.png, first.png);
    assert_ne!(
        recovered.dynamic_artifacts[0]["acquisition"]["page_state"]["id"],
        first.dynamic_artifacts[0]["acquisition"]["page_state"]["id"]
    );
    assert_eq!(
        recovered.dynamic_artifacts[0]["acquisition"]["page_state"]["reused"],
        false
    );
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn page_network_idle_waits_for_post_load_fetches_and_recovers_after_timeout_and_cancel() {
    let server = page_wait_reference::Server::new();
    let (temp, _, _) = scene(json!([{"id":"page","type":"browser",
        "url":format!("http://{}/delayed", server.address),"width":32,"height":32}]));
    fs::create_dir(temp.path().join(".git")).unwrap();
    let file = temp.path().join("scene.pen");
    let saved = fs::read_to_string(&file).unwrap();
    let config = temp.path().join("lazypencil.toml");
    let load = || {
        let project = Project::from_document(
            &file,
            Document::parse(fs::read_to_string(&file).unwrap()).unwrap(),
            &[],
        )
        .unwrap();
        let layout =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        (project, layout)
    };
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let (first_project, layout) = load();
    let first = render::png_with_runtime(
        &first_project,
        &layout,
        &Default::default(),
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(
        first.dynamic_artifacts[0]["acquisition"]["wait_until"],
        "load"
    );
    fs::write(
        &config,
        "[runtime.browser]\nwait_until='network_idle'\ntimeout_ms=5000\n",
    )
    .unwrap();
    assert_eq!(
        first_project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let (project, layout) = load();
    let acquired =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(
        raster(&acquired.png)
            .peek_pixels()
            .unwrap()
            .get_color((16, 16)),
        Color::GREEN
    );
    let metadata = &acquired.dynamic_artifacts[0]["acquisition"];
    assert_eq!(metadata["wait_until"], "network_idle");
    assert_eq!(metadata["limits"]["browser"]["wait_until"], "network_idle");
    assert_ne!(
        metadata["acquisition_id"],
        first.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
    let reused =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(reused.dynamic_artifacts, acquired.dynamic_artifacts);
    assert_eq!(fs::read_to_string(&file).unwrap(), saved);

    let mut busy: Value = serde_json::from_str(&saved).unwrap();
    busy["children"][0]["url"] = json!(format!("http://{}/busy", server.address));
    fs::write(&file, busy.to_string()).unwrap();
    fs::write(
        &config,
        "[runtime.browser]\nwait_until='network_idle'\ntimeout_ms=1500\n",
    )
    .unwrap();
    let (project, layout) = load();
    let started = Instant::now();
    let error =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .err()
            .unwrap();
    assert_eq!(error.code, "RENDER_TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(runtime.process_ids().is_empty());

    // Recover through a new isolated acquisition, then cancel another busy page.
    fs::write(&file, &saved).unwrap();
    fs::write(
        &config,
        "[runtime.browser]\nwait_until='network_idle'\ntimeout_ms=5000\n",
    )
    .unwrap();
    let (project, layout) = load();
    let recovered =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(
        raster(&recovered.png)
            .peek_pixels()
            .unwrap()
            .get_color((16, 16)),
        Color::GREEN
    );
    fs::write(&file, busy.to_string()).unwrap();
    let (project, layout) = load();
    let cancel = tokio_util::sync::CancellationToken::new();
    let signal = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        signal.cancel();
    });
    let started = Instant::now();
    let error = render::png_with_runtime(
        &project,
        &layout,
        &Default::default(),
        &mut runtime,
        Some(cancel),
    )
    .err()
    .unwrap();
    canceller.join().unwrap();
    assert_eq!(error.code, "CANCELLED");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(runtime.process_ids().is_empty());
    assert_eq!(fs::read_to_string(&file).unwrap(), busy.to_string());
}

#[test]
fn uniform_annotations_bind_to_the_correct_declaration() {
    let parsed=lazypencil::shader::uniforms("/** @resolution */ uniform highp vec2 size;\n/** @time */ uniform float clock;\nuniform vec4 color; /* @color */\n").unwrap();
    assert_eq!(parsed.len(), 3);
    assert!(parsed[0].resolution);
    assert!(!parsed[0].time);
    assert!(parsed[1].time);
    assert!(!parsed[2].time);
    assert!(lazypencil::shader::uniforms("/** @resolution */ uniform float broken;").is_err());
    let arrays = lazypencil::shader::uniforms("uniform vec2 array[4];").unwrap();
    assert!(arrays[0].array);
    assert_eq!(arrays[0].length, Some(4));
}

#[cfg(unix)]
#[test]
fn captured_page_shader_and_render_deadlines_cap_launch_and_reap_the_child() {
    use lazypencil::config::runtime::RuntimeSettings;
    let (temp, project, _) =
        scene(json!([{"id":"page","type":"browser","url":"about:blank","width":8,"height":8}]));
    let executable = temp.path().join("waiting-browser");
    let pid_file = temp.path().join("pid");
    fixture_support::process(&executable, json!({"pid_file":pid_file,"sleep_ms":60000}));
    let resolved = project.resolve().unwrap();
    for shader in [false, true] {
        for limiting_scope in ["launch", "operation", "render"] {
            let mut settings = RuntimeSettings::default();
            settings.browser.timeout_ms = 5000;
            settings.shader.timeout_ms = 5000;
            settings.browser.launch_timeout_ms = 5000;
            settings.shader.launch_timeout_ms = 5000;
            match limiting_scope {
                "launch" => {
                    settings.browser.launch_timeout_ms = 300;
                    settings.shader.launch_timeout_ms = 300;
                }
                "operation" => {
                    settings.browser.timeout_ms = 300;
                    settings.shader.timeout_ms = 300;
                }
                "render" => settings.dynamic.render_timeout_ms = 300,
                _ => unreachable!(),
            }
            let _ = fs::remove_file(&pid_file);
            let mut runtime = DynamicRuntime::new(temp.path())
                .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
                .with_executable(&executable);
            runtime.configure(settings).unwrap();
            runtime.begin_render(None, false);
            let start = Instant::now();
            let result = if shader {
                runtime.shader(json!({
                    "source":"precision mediump float; void main(){gl_FragColor=vec4(1.0);}",
                    "width":8,"height":8
                }))
            } else {
                runtime.page(&resolved.roots[0], 8., 8., 1.)
            };
            let error = result.err().unwrap();
            assert_eq!(error.code, "RENDER_TIMEOUT", "{shader}/{limiting_scope}");
            assert!(start.elapsed() < Duration::from_secs(3));
            let pid: i32 = fs::read_to_string(&pid_file).unwrap().parse().unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
            assert!(runtime.process_ids().is_empty());
        }
    }
}

#[test]
fn invalid_shader_sources_fail_before_browser_launch() {
    let root = TempDir::new().unwrap();
    let mut runtime =
        DynamicRuntime::new(root.path()).with_executable(root.path().join("must-not-be-launched"));
    for source in [Value::Null, json!(42), json!("x".repeat(1024 * 1024 + 1))] {
        let error = runtime
            .shader(json!({"source":source,"width":8,"height":8}))
            .err()
            .unwrap();
        assert_eq!(error.code, "SHADER_COMPILE_FAILED");
        assert!(error.message.contains("Shader source"));
        assert!(runtime.process_ids().is_empty());
    }
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn browser_pixels_are_captured_composited_and_reused_until_refresh() {
    let url = page(
        "<style>html,body{margin:0;background:#00f}div{width:20px;height:100vh;background:#f00}</style><div></div>",
    );
    let (temp, project, layout) = scene(
        json!([{"id":"page","type":"browser","url":url,"width":80,"height":40,"cornerRadius":4}]),
    );
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let a = render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
        .unwrap();
    let image = raster(&a.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((10, 20)), Color::RED);
    assert_eq!(pixels.get_color((50, 20)), Color::BLUE);
    assert!(pixels.get_color((0, 0)).a() < 255);
    let b = render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
        .unwrap();
    assert_eq!(
        a.dynamic_artifacts[0]["acquisition"]["acquisition_id"],
        b.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
    let c = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_ne!(
        a.dynamic_artifacts[0]["acquisition"]["acquisition_id"],
        c.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
}

#[test]
#[ignore = "Requires headless Chromium, software WebGL, and permission to launch its sandbox"]
fn captured_dynamic_policy_reloads_cache_and_expires_purposes_independently() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let file = temp.path().join("scene.pen");
    let config = temp.path().join("lazypencil.toml");
    fs::write(
        temp.path().join("blue.glsl"),
        "precision highp float; void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    let saved = json!({"version":"2.21","children":[
        {"id":"page","type":"browser","width":16,"height":16,"url":page("<style>html{background:#f00}</style>")},
        {"id":"shader","type":"rectangle","x":16,"width":16,"height":16,"fill":{"type":"shader","url":"blue.glsl"}}
    ]}).to_string();
    fs::write(&file, &saved).unwrap();
    let write_policy = |cache_mib, shader_idle| {
        fs::write(
            &config,
            format!("[runtime.browser]\ntimeout_ms=30000\nlaunch_timeout_ms=15000\nidle_seconds=0\n[runtime.shader]\ntimeout_ms=15000\nlaunch_timeout_ms=15000\nidle_seconds={shader_idle}\n[runtime.dynamic]\nrender_timeout_ms=45000\ncache_mib={cache_mib}\ncache_entries=32\n"),
        )
        .unwrap();
    };
    let load = || {
        let project =
            Project::from_document(&file, Document::parse(saved.clone()).unwrap(), &[]).unwrap();
        let layout =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        (project, layout)
    };
    write_policy(64, 300);
    let (project, layout) = load();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let first =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(runtime.process_ids().len(), 2);
    let first_pids = runtime.process_ids();
    runtime.expire_idle(Instant::now());
    assert_eq!(runtime.process_ids(), vec![first_pids[1]]);
    let cached =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(cached.png, first.png);
    assert_eq!(cached.dynamic_artifacts, first.dynamic_artifacts);
    assert_eq!(runtime.process_ids(), vec![first_pids[1]]);
    assert_eq!(
        first.dynamic_artifacts[0]["acquisition"]["limits"]["browser"]["idle_seconds"],
        0
    );
    assert_eq!(
        first.dynamic_artifacts[1]["acquisition"]["limits"]["shader"]["idle_seconds"],
        300
    );

    write_policy(0, 0);
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let (project, layout) = load();
    let changed =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(changed.png, first.png);
    assert_ne!(
        changed.dynamic_artifacts[0]["acquisition"]["acquisition_id"],
        first.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
    assert_eq!(
        changed.dynamic_artifacts[1]["acquisition"]["limits"]["shader"]["idle_seconds"],
        0
    );
    for artifact in &changed.dynamic_artifacts {
        assert_eq!(artifact["acquisition"]["limits"]["dynamic"]["cache_mib"], 0);
    }
    runtime.expire_idle(Instant::now());
    assert!(runtime.process_ids().is_empty());
    let uncached =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(uncached.png, first.png);
    assert_ne!(
        uncached.dynamic_artifacts[0]["acquisition"]["acquisition_id"],
        changed.dynamic_artifacts[0]["acquisition"]["acquisition_id"]
    );
    runtime.expire_idle(Instant::now());
    assert!(runtime.process_ids().is_empty());
    let pixels = raster(&uncached.png);
    assert_eq!(pixels.peek_pixels().unwrap().get_color((8, 8)), Color::RED);
    assert_eq!(
        pixels.peek_pixels().unwrap().get_color((24, 8)),
        Color::BLUE
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), saved);
}

#[test]
#[ignore = "Requires headless Chromium, software WebGL, and permission to launch its sandbox"]
fn page_and_shader_processes_expire_independently_preserve_rasters_and_restart() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("blue.glsl"),
        "precision highp float; void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    let source = json!({"version":"2.21","children":[
        {"id":"page","type":"browser","width":16,"height":16,"url":page("<style>html{background:#f00}</style>")},
        {"id":"shader","type":"rectangle","x":16,"width":16,"height":16,"fill":{"type":"shader","url":"blue.glsl"}}
    ]}).to_string();
    fs::write(temp.path().join("scene.pen"), &source).unwrap();
    let project = Project::from_document(
        &temp.path().join("scene.pen"),
        Document::parse(source).unwrap(),
        &[],
    )
    .unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    assert_eq!(runtime.idle_deadline(), None);
    let first =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let pids = runtime.process_ids();
    assert_eq!(pids.len(), 2);
    let deadline = runtime.idle_deadline().unwrap();
    runtime.expire_idle(deadline - Duration::from_nanos(1));
    assert_eq!(runtime.process_ids(), pids);
    let repeated = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(runtime.process_ids(), pids);
    assert_eq!(first.png, repeated.png);
    let first_deadline = runtime.idle_deadline().unwrap();
    runtime.expire_idle(first_deadline);
    assert_eq!(runtime.process_ids(), vec![pids[1]]);
    let last_deadline = runtime.idle_deadline().unwrap();
    assert!(last_deadline > first_deadline);
    runtime.expire_idle(last_deadline);
    assert!(runtime.process_ids().is_empty());
    assert_eq!(runtime.idle_deadline(), None);
    #[cfg(unix)]
    for pid in &pids {
        assert_eq!(
            unsafe { libc::waitpid(*pid as i32, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
    let cached =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert!(runtime.process_ids().is_empty());
    assert_eq!(cached.png, repeated.png);
    assert_eq!(cached.dynamic_artifacts, repeated.dynamic_artifacts);
    let refreshed = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            refresh: true,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_eq!(runtime.process_ids().len(), 2);
    assert!(runtime.process_ids().iter().all(|pid| !pids.contains(pid)));
    assert_eq!(refreshed.png, first.png);
    assert_ne!(refreshed.dynamic_artifacts, cached.dynamic_artifacts);
    let image = raster(&refreshed.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((8, 8)), Color::RED);
    assert_eq!(pixels.get_color((24, 8)), Color::BLUE);
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn software_webgl_pixels_follow_resolution_time_and_color_uniforms() {
    let temp = TempDir::new().unwrap();
    let shader = temp.path().join("fill.glsl");
    fs::write(&shader,"precision highp float;\n/** @resolution */ uniform vec2 size;\n/** @time */ uniform float clock;\nuniform vec4 tint;\nvoid main(){vec2 p=gl_FragCoord.xy/size;gl_FragColor=vec4(p.x,clock,tint.b,tint.a);}").unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","children":[{"id":"rect","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":"fill.glsl","uniforms":{"tint":"#0000FFFF"}}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            time: 0.5,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    let image = raster(&output.png);
    let color = image.peek_pixels().unwrap().get_color((20, 10));
    assert!(color.r() > 120 && color.r() < 140);
    assert!((color.g() as i32 - 128).abs() < 2);
    assert_eq!(color.b(), 255);
    assert!(
        output.dynamic_artifacts[0]["acquisition"]["renderer"]
            .as_str()
            .unwrap()
            .contains("SwiftShader")
    );
    let changed = render::png_with_runtime(
        &project,
        &layout,
        &RenderOptions {
            time: 0.0,
            ..Default::default()
        },
        &mut runtime,
        None,
    )
    .unwrap();
    assert_ne!(changed.png, output.png);
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn an_infinite_page_script_can_be_cancelled_and_the_runtime_recovers() {
    let (temp, project, layout) = scene(
        json!([{"id":"page","type":"browser","url":page("<script>for(;;){}</script>"),"width":40,"height":20}]),
    );
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let cancel = tokio_util::sync::CancellationToken::new();
    let token = cancel.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        token.cancel();
    });
    let start = Instant::now();
    let error = render::png_with_runtime(
        &project,
        &layout,
        &Default::default(),
        &mut runtime,
        Some(cancel),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "CANCELLED");
    assert!(start.elapsed() < Duration::from_secs(3));
    thread.join().unwrap();
    let (_temp, project, layout) = scene(
        json!([{"id":"page","type":"browser","url":page("<style>body{background:#00f}</style>"),"width":40,"height":20}]),
    );
    render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None).unwrap();
}

fn effect_scene(
    effect: Value,
    children: Value,
) -> (TempDir, Project, lazypencil::layout::LayoutSnapshot) {
    let temp = TempDir::new().unwrap();
    let sample = "precision highp float;\n/** @resolution */ uniform vec2 size;\nuniform sampler2D src;\nvoid main(){gl_FragColor=texture2D(src,gl_FragCoord.xy/size).bgra;}";
    fs::write(temp.path().join("swap.glsl"), sample).unwrap();
    fs::write(
        temp.path().join("sample.glsl"),
        sample.replace(".bgra", ".rgba"),
    )
    .unwrap();
    let mut children = children;
    children[1]["effect"] = effect;
    let path = temp.path().join("scene.pen");
    let source = json!({"version":"2.21","children":children}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    (temp, project, layout)
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn shader_sampler_arrays_accept_object_and_background_in_effects() {
    let children = json!([
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#00FF00"},
        {"id":"front","type":"rectangle","x":20,"y":10,"width":40,"height":20,"fill":"#FF0000"}
    ]);
    let effect = json!({"type":"shader","url":"combine.glsl","uniforms":{"images":["pencil:object","pencil:background"]}});
    let (temp, _, _) = effect_scene(
        json!({"type":"shader","url":"sample.glsl","uniforms":{"src":"pencil:object"}}),
        children,
    );
    fs::write(temp.path().join("combine.glsl"),"precision highp float;\n/** @resolution */ uniform vec2 size;\nuniform sampler2D images[2];void main(){vec2 uv=gl_FragCoord.xy/size;gl_FragColor=mix(texture2D(images[0],uv),texture2D(images[1],uv),0.5);}").unwrap();
    let path = temp.path().join("scene.pen");
    let mut source: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    source["children"][1]["effect"] = effect.clone();
    fs::write(&path, source.to_string()).unwrap();
    let load = || {
        let project = Project::from_document(
            &path,
            Document::parse(fs::read_to_string(&path).unwrap()).unwrap(),
            &[],
        )
        .unwrap();
        let layout =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        (project, layout)
    };
    let (project, layout) = load();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    let mixed = pixels.get_color((30, 20));
    assert!(
        (mixed.r() as i32 - 128).abs() <= 1 && (mixed.g() as i32 - 128).abs() <= 1,
        "{mixed:?}"
    );
    assert_eq!((mixed.b(), mixed.a()), (0, 255));
    assert_eq!(pixels.get_color((5, 5)), Color::GREEN);
    source["children"][1]
        .as_object_mut()
        .unwrap()
        .remove("effect");
    source["children"][1]["fill"] = effect;
    fs::write(&path, source.to_string()).unwrap();
    let (project, layout) = load();
    let error =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .err()
            .unwrap();
    assert_eq!(error.code, "SHADER_COMPILE_FAILED");
    assert!(
        error.message.contains("pencil:object is unavailable"),
        "{error:?}"
    );
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn shader_effects_receive_only_the_object_and_run_in_declared_order() {
    let children = json!([
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#00FF00"},
        {"id":"front","type":"rectangle","x":20,"y":10,"width":40,"height":20,"fill":"#FF0000"}
    ]);
    let effect = json!({"type":"shader","url":"swap.glsl","uniforms":{"src":"pencil:object"}});
    let (temp, project, layout) = effect_scene(effect.clone(), children.clone());
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((30, 20)), Color::BLUE);
    assert_eq!(pixels.get_color((5, 5)), Color::GREEN);
    let (temp, project, layout) = effect_scene(json!([effect.clone(), effect]), children);
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!(
        raster(&output.png)
            .peek_pixels()
            .unwrap()
            .get_color((30, 20)),
        Color::RED
    );
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn background_sampling_includes_the_scene_and_parent_layers() {
    let children = json!([
        {"id":"back","type":"rectangle","width":80,"height":40,"fill":"#0000FF"},
        {"id":"front","type":"frame","x":20,"y":0,"width":40,"height":40,"fill":"#FF0000","opacity":0.5,"children":[
            {"id":"child","type":"rectangle","width":40,"height":40,"fill":{"type":"shader","url":"sample.glsl","uniforms":{"src":"pencil:background"}}}
        ]}
    ]);
    let (temp, project, layout) = effect_scene(json!([]), children);
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    let color = pixels.get_color((30, 20));
    assert!((color.r() as i32 - 128).abs() < 2, "{color:?}");
    assert!((color.b() as i32 - 127).abs() < 2, "{color:?}");
    assert_eq!(color.g(), 0);
    assert_eq!(pixels.get_color((5, 5)), Color::BLUE);
}

#[test]
#[ignore = "Requires headless Chromium and permission to launch its sandbox"]
fn browser_zoom_scroll_and_export_scale_change_the_capture() {
    let url = page(
        "<style>html,body{margin:0;width:200px;height:300px;background:#00f}div{width:20px;height:40px;background:#f00}</style><div></div>",
    );
    for (zoom, scroll, scale, point, expected) in [
        (2.0, 0.0, 1.0, (30, 20), Color::RED),
        (1.0, 60.0, 1.0, (10, 20), Color::BLUE),
        (1.0, 0.0, 2.0, (30, 20), Color::RED),
    ] {
        let (temp, project, layout) = scene(
            json!([{"id":"page","type":"browser","url":url,"width":80,"height":40,"zoom":zoom,"scrollY":scroll}]),
        );
        let mut runtime = DynamicRuntime::new(temp.path())
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(executable());
        let output = render::png_with_runtime(
            &project,
            &layout,
            &RenderOptions {
                scale,
                ..Default::default()
            },
            &mut runtime,
            None,
        )
        .unwrap();
        assert_eq!(
            raster(&output.png).peek_pixels().unwrap().get_color(point),
            expected,
            "zoom={zoom} scroll={scroll} scale={scale}"
        );
    }
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn sampler_images_keep_their_orientation_and_fill_opacity() {
    let temp = TempDir::new().unwrap();
    let mut surface = skia_safe::surfaces::raster_n32_premul((8, 4)).unwrap();
    surface.canvas().clear(Color::RED);
    let mut paint = skia_safe::Paint::default();
    paint.set_color(Color::BLUE);
    surface
        .canvas()
        .draw_rect(skia_safe::Rect::from_xywh(0.0, 2.0, 8.0, 2.0), &paint);
    fs::write(
        temp.path().join("texture.png"),
        skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    fs::write(temp.path().join("sample.glsl"),"precision highp float;\n/** @resolution */ uniform vec2 size;\nuniform sampler2D image;\nvoid main(){gl_FragColor=texture2D(image,gl_FragCoord.xy/size);}").unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","children":[{"id":"rect","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":"sample.glsl","opacity":0.5,"uniforms":{"image":"texture.png"}}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    let top = pixels.get_color((20, 3));
    let bottom = pixels.get_color((20, 17));
    assert_eq!(top.r(), 255);
    assert_eq!(bottom.b(), 255);
    assert!((top.a() as i32 - 128).abs() < 2);
    assert!((bottom.a() as i32 - 128).abs() < 2);
}

#[test]
fn uniform_names_that_match_other_literal_properties_still_resolve_variables() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("shade.glsl"),
        "precision highp float;uniform float name;void main(){gl_FragColor=vec4(name,0.,0.,1.);}",
    )
    .unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","variables":{"level":{"type":"number","value":0.75}},"children":[{"id":"rect","type":"rectangle","width":20,"height":20,"fill":{"type":"shader","url":"shade.glsl","uniforms":{"name":"$level"}}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let scene = project.resolve().unwrap();
    let fill = &scene.roots[0].properties["fill"];
    let request = lazypencil::shader::request(
        &project,
        &scene.roots[0],
        fill,
        "fill",
        20,
        20,
        0.0,
        None,
        None,
    )
    .unwrap();
    assert_eq!(request["uniforms"]["name"], 0.75);
}

#[test]
#[ignore = "Requires LAZYPENCIL_TEST_CHROMIUM and permission to launch Chromium"]
fn real_cli_uses_the_runtime_and_interrupts_without_writing_an_output() {
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;
    let (temp, project, _layout) = scene(
        json!([{"id":"page","type":"browser","url":page("<style>body{margin:0;background:#00f}</style>"),"width":40,"height":20}]),
    );
    let file = temp.path().join("scene.pen");
    let output_path = temp.path().join("image.png");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .env("LAZYPENCIL_CHROMIUM", executable())
        .args(["--json", "render"])
        .arg(&file)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        raster(&fs::read(&output_path).unwrap())
            .peek_pixels()
            .unwrap()
            .get_color((10, 10)),
        Color::BLUE
    );
    assert!(project.verify_unchanged().is_ok());
    #[cfg(unix)]
    {
        fs::remove_file(&output_path).unwrap();
        let source = fs::read_to_string(&file).unwrap();
        let mut document: Value = serde_json::from_str(&source).unwrap();
        document["children"][0]["url"] = json!(page("<script>for(;;){}</script>"));
        fs::write(&file, document.to_string()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .env("LAZYPENCIL_CHROMIUM", executable())
            .args(["--json", "render"])
            .arg(&file)
            .arg("--output")
            .arg(&output_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(130),
            "stdout: {}; stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output_path.exists());
    }
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn derivatives_and_uv_varyings_link_with_fragment_precision() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("uv.glsl"), "#extension GL_OES_standard_derivatives : enable\nprecision mediump float;varying vec2 v_uv;void main(){gl_FragColor=vec4(v_uv.x,v_uv.y,step(0.01,fwidth(v_uv.x)),1.);}").unwrap();
    let path = temp.path().join("scene.pen");
    let source = json!({"version":"2.21","children":[{"id":"uv","type":"rectangle","width":40,"height":20,"fill":{"type":"shader","url":"uv.glsl"}}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    let image = raster(&output.png);
    let color = image.peek_pixels().unwrap().get_color((20, 10));
    assert!((color.r() as i32 - 131).abs() < 3, "{color:?}");
    assert!((color.g() as i32 - 121).abs() < 3, "{color:?}");
    assert_eq!(color.b(), 255);
    assert!(
        output.dynamic_artifacts[0]["acquisition"]["extensions"]
            .as_array()
            .unwrap()
            .contains(&json!("OES_standard_derivatives"))
    );
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn shader_strokes_use_the_library_origin_and_paint_the_outer_border() {
    let temp = TempDir::new().unwrap();
    let library = temp.path().join("lib");
    fs::create_dir(&library).unwrap();
    fs::write(
        library.join("stroke.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(0.,0.,1.,1.);}",
    )
    .unwrap();
    fs::write(
        temp.path().join("stroke.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(0.,1.,0.,1.);}",
    )
    .unwrap();
    fs::write(library.join("ui.pen"),json!({"version":"2.21","children":[{"id":"card","type":"rectangle","reusable":true,"width":40,"height":20,"fill":"#FF0000","stroke":{"type":"shader","url":"stroke.glsl"},"strokeWidth":4,"strokeAlignment":"outer"}]}).to_string()).unwrap();
    let path = temp.path().join("scene.pen");
    let source=json!({"version":"2.21","imports":{"ui":"lib/ui.pen"},"children":[{"id":"card","type":"ref","ref":"ui:card"}]}).to_string();
    fs::write(&path, &source).unwrap();
    let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
    let layout =
        lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
            .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    let output =
        render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
            .unwrap();
    assert_eq!((output.width, output.height), (48, 28));
    let image = raster(&output.png);
    let pixels = image.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((1, 14)), Color::BLUE);
    assert_eq!(pixels.get_color((24, 14)), Color::RED);
    assert_eq!(output.dynamic_artifacts[0]["node_id"], "card");
}

#[test]
#[ignore = "Requires software WebGL Chromium and permission to launch its sandbox"]
fn background_and_shader_effect_order_preserves_replacement_coverage() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("quarter.glsl"),
        "precision highp float;void main(){gl_FragColor=vec4(1.,0.,0.,0.25);}",
    )
    .unwrap();
    let mut runtime = DynamicRuntime::new(temp.path())
        .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
        .with_executable(executable());
    for background_first in [true, false] {
        let mut effects = vec![
            json!({"type":"background_blur","radius":8}),
            json!({"type":"shader","url":"quarter.glsl"}),
        ];
        if !background_first {
            effects.reverse();
        }
        let path = temp.path().join("scene.pen");
        let source = json!({"version":"2.21","children":[
            {"id":"back","type":"rectangle","width":60,"height":40,"fill":"#0000FF80"},
            {"id":"glass","type":"rectangle","x":10,"y":10,"width":40,"height":20,"effect":effects}
        ]})
        .to_string();
        fs::write(&path, &source).unwrap();
        let project = Project::from_document(&path, Document::parse(source).unwrap(), &[]).unwrap();
        let layout =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        let output =
            render::png_with_runtime(&project, &layout, &Default::default(), &mut runtime, None)
                .unwrap();
        let image = raster(&output.png);
        let color = image.peek_pixels().unwrap().get_color((30, 20));
        if background_first {
            assert!((color.a() as i32 - 64).abs() <= 1, "{color:?}");
            assert_eq!(color.r(), 255);
            assert_eq!(color.b(), 0);
        } else {
            assert!((color.a() as i32 - 160).abs() <= 2, "{color:?}");
            assert!((color.r() as i32 - 102).abs() <= 3, "{color:?}");
            assert!((color.b() as i32 - 153).abs() <= 3, "{color:?}");
        }
    }
}
