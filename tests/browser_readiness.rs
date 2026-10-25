#![cfg(feature = "rendering")]

use base64::Engine;
use lazypencil::{
    document::Document,
    dynamic::DynamicRuntime,
    project::Project,
    render::{self, RenderedImage},
    text::Fonts,
};
use serde_json::{Value, json};
use skia_safe::{Color, Data, Image};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "support/browser_copy_fixture.rs"]
mod browser_copy_fixture;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn copied_browser_namespaces_keep_pixels_across_removal_recopy_themes_and_detach() {
    use lazypencil::{
        cdp::{Browser, Budget, Purpose},
        runtime::InstalledBrowser,
    };
    let installed = InstalledBrowser {
        executable: std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap().into(),
        version: "test-managed-runtime".into(),
        platform: lazypencil::runtime::platform().unwrap().into(),
    };
    let startup = Budget::new(Duration::from_secs(30), None);
    let mut browser = Browser::launch_with_supervisor(
        &installed,
        Purpose::Page,
        &startup,
        std::path::Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    let directory = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR").map(std::path::PathBuf::from);
    if let Some(directory) = &directory {
        fs::create_dir_all(directory).unwrap();
    }
    let mut cases = Vec::new();
    for version in ["2.19", "2.21"] {
        for mode in browser_copy_fixture::MODES {
            let (_temp, frames) = browser_copy_fixture::study(version, mode, false);
            let mut originals = std::collections::BTreeMap::<&str, Vec<u8>>::new();
            let mut stages = std::collections::BTreeMap::<&str, Vec<Value>>::new();
            for frame in frames {
                let budget = Budget::new(Duration::from_secs(15), None);
                let png=browser.with_tab(&budget, |browser,session| {
                    browser.command(Some(session),"Page.enable",json!({}),&budget)?;
                    browser.intercept_local_files(session,frame.files.clone(),&budget)?;
                    browser.command(Some(session),"Emulation.setDeviceMetricsOverride",json!({"width":32,"height":16,"deviceScaleFactor":1,"mobile":false}),&budget)?;
                    let navigation=browser.command(Some(session),"Page.navigate",json!({"url":frame.url}),&budget)?;
                    assert!(navigation.get("errorText").is_none(),"{navigation}");
                    let expression=format!("location.href==={}&&document.readyState==='complete'",serde_json::to_string(&frame.url).unwrap());
                    while browser.evaluate(session,&expression,&budget)? != true {
                        std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
                    }
                    browser.evaluate(session,"(async()=>{await document.fonts.ready;await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r(true))));return true})()",&budget)?;
                    let screenshot=browser.command(Some(session),"Page.captureScreenshot",json!({"format":"png","captureBeyondViewport":false}),&budget)?;
                    let stats=browser.local_file_stats(session).unwrap();
                    assert_eq!(stats.fulfilled,2);assert_eq!(stats.denied,0);
                    Ok(base64::engine::general_purpose::STANDARD.decode(screenshot["data"].as_str().unwrap()).unwrap())
                }).unwrap();
                let image = Image::from_encoded(Data::new_copy(&png))
                    .unwrap()
                    .make_raster_image(None, None)
                    .unwrap();
                assert_eq!((image.width(), image.height()), (32, 16));
                let pixels = image.peek_pixels().unwrap();
                let expected =
                    Color::from_argb(frame.rgba[3], frame.rgba[0], frame.rgba[1], frame.rgba[2]);
                for y in 0..16 {
                    for x in 0..32 {
                        assert_eq!(
                            pixels.get_color((x, y)),
                            expected,
                            "{version} {mode} {} {} {x},{y}",
                            frame.stage,
                            frame.theme
                        );
                    }
                }
                if frame.stage == "before" {
                    originals.insert(frame.theme, png.clone());
                } else {
                    assert_eq!(
                        png, originals[frame.theme],
                        "{version} {mode} {} {}",
                        frame.stage, frame.theme
                    );
                }
                let name = format!(
                    "codec-browser-copy-{version}-{mode}-{}-{}.png",
                    frame.theme, frame.stage
                );
                if let Some(directory) = &directory {
                    fs::write(directory.join(&name), &png).unwrap();
                }
                stages.entry(frame.theme).or_default().push(json!({"stage":frame.stage,"name":name,"png_hash":lazypencil::document::hash(&png),"resource_revision":frame.revision}));
            }
            for (theme, stages) in stages {
                cases.push(json!({"document_version":version,"mode":mode,"theme":theme,"width":32,"height":16,
                "rgba":browser_copy_fixture::color(mode,theme),"stages":stages,"sources_removed":true}));
            }
        }
    }
    if let Some(directory) = directory {
        fs::write(directory.join("codec-browser-copy.json"),serde_json::to_vec_pretty(&json!({
        "source_commit":std::env::var("GITHUB_SHA").ok(),"browser":browser.version,"sandbox":"enabled","cases":cases
    })).unwrap()).unwrap();
    }
}

#[path = "support/page_wait_reference.rs"]
mod reference;

struct Fixture(TempDir);
impl Fixture {
    fn new(url: &str) -> Self {
        let f = Self(TempDir::new().unwrap());
        fs::create_dir(f.0.path().join(".git")).unwrap();
        f.url(url);
        f.policy("", "visible", 5000);
        f
    }
    fn url(&self, url: &str) {
        self.url_and_scroll(url, 0);
    }
    fn url_and_scroll(&self, url: &str, scroll: u64) {
        fs::write(
            self.0.path().join("scene.pen"),
            json!({"version":"2.21","children":[
                {"id":"page","type":"browser","url":url,"width":32,"height":32,"scrollY":scroll}
            ]})
            .to_string(),
        )
        .unwrap();
    }
    fn policy(&self, selector: &str, state: &str, timeout: u64) {
        // JSON escaping for these strings is valid TOML basic-string syntax.
        let selector = json!(selector);
        let state = json!(state);
        fs::write(self.0.path().join("lazypencil.toml"), format!("[runtime.browser]\nwait_until='load'\nready_selector={selector}\nselector_state={state}\ntimeout_ms={timeout}\n")).unwrap();
    }
    fn project(&self) -> Project {
        let file = self.0.path().join("scene.pen");
        Project::from_document(
            &file,
            Document::parse(fs::read_to_string(&file).unwrap()).unwrap(),
            &[],
        )
        .unwrap()
    }
    fn runtime(&self) -> DynamicRuntime {
        DynamicRuntime::new(self.0.path())
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(
                std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
            )
    }
    fn capture(
        &self,
        runtime: &mut DynamicRuntime,
        cancel: Option<CancellationToken>,
    ) -> lazypencil::Result<RenderedImage> {
        let project = self.project();
        let file = self.0.path().join("scene.pen");
        let saved = fs::read(&file).unwrap();
        let layout =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        let result =
            render::png_with_runtime(&project, &layout, &Default::default(), runtime, cancel);
        assert_eq!(fs::read(&file).unwrap(), saved);
        result
    }
}

fn page(html: &str) -> String {
    format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(html)
    )
}
fn acquisition(image: &RenderedImage) -> &Value {
    &image.dynamic_artifacts[0]["acquisition"]
}
fn assert_green(image: &RenderedImage) {
    let raster = Image::from_encoded(Data::new_copy(&image.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!(
        raster.peek_pixels().unwrap().get_color((16, 16)),
        Color::GREEN
    );
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn captured_local_files_render_after_source_removal_and_deny_unlisted_host_files() {
    use lazypencil::{
        cdp::{Browser, Budget, LocalFiles, Purpose},
        runtime::InstalledBrowser,
        service,
        store::Store,
    };
    let original = TempDir::new().unwrap();
    let site = original.path().join("site with spaces 日本語");
    fs::create_dir(&site).unwrap();
    let sources = [
        (
            "index.html",
            r#"<!doctype html><html><head>
            <link rel="icon" href="data:,"><link rel="stylesheet" href="style.css?v=1"><script src="app.js"></script>
            <script src="../unlisted.js"></script></head><body>
            <img src="blue.svg?color=blue" width="8" height="8">
            <iframe src="frame.html#child" width="8" height="8"></iframe></body></html>"#,
        ),
        (
            "style.css",
            "@import url('theme.css?theme=green');body{margin:0}img{position:absolute;left:0;top:0}iframe{position:absolute;left:8px;top:0;border:0}",
        ),
        ("theme.css", "html{background:#00ff00}"),
        (
            "app.js",
            r#"window.addEventListener('message',e=>{if(e.data==='frame-ready')window.frameReady=true});
            const libraryURL=JSON.stringify(new URL('worker-library.js',location.href).href);
            const nestedCode=`importScripts(${libraryURL});postMessage(capturedWorkerValue)`;
            const workerCode=`importScripts(${libraryURL});const child=new Worker(URL.createObjectURL(new Blob([${JSON.stringify(nestedCode)}],{type:'text/javascript'})));
                child.onerror=e=>postMessage('worker-failure:'+e.message);
                child.onmessage=e=>{postMessage(capturedWorkerValue+':'+e.data);child.terminate()}`;
            const worker=new Worker(URL.createObjectURL(new Blob([workerCode],{type:'text/javascript'})));
            worker.onerror=e=>window.workerFailure=e.message;
            worker.onmessage=e=>{if(e.data==='worker-captured:worker-captured')window.workerReady=true;
                else window.workerFailure=String(e.data);worker.terminate()};
            window.addEventListener('load',async()=>{await document.images[0].decode();window.localProbe={
            background:getComputedStyle(document.documentElement).backgroundColor,
            location:location.href,unsafe:window.unsafeHostFile===true,image:document.images[0].naturalWidth}});"#,
        ),
        (
            "worker-library.js",
            "self.capturedWorkerValue='worker-captured'",
        ),
        (
            "blue.svg",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><path fill="blue" d="M0 0h8v8H0z"/></svg>"#,
        ),
        (
            "frame.html",
            "<!doctype html><style>html{background:#ff0000}</style><script>parent.postMessage('frame-ready','*')</script>",
        ),
        (
            "extensionless",
            "<!doctype html><script>window.nativeNoExtRan=true</script><p>original local contents</p>",
        ),
    ];
    for (name, contents) in sources {
        let path = site.join(name);
        fs::write(&path, contents).unwrap();
    }
    let captured_bytes: usize = sources
        .iter()
        .filter(|(name, _)| *name != "extensionless")
        .map(|(_, contents)| contents.len())
        .sum();
    let mut url = url::Url::from_file_path(site.join("index.html")).unwrap();
    url.set_query(Some("theme=green"));
    url.set_fragment(Some("original-location"));
    let file = original.path().join("scene.pen");
    let document = json!({"version":"2.21","metadata":{"lazypencil":{"browser_resources":{
        "version":1,"roots":[{"url":url::Url::from_directory_path(&site).unwrap().as_str(),"path":site.file_name().unwrap().to_str().unwrap()}]
    }}},"children":[]}).to_string();
    fs::write(&file, &document).unwrap();
    let project = Project::from_document(&file, Document::parse(document).unwrap(), &[]).unwrap();
    let captured = project
        .browser_page_resources(&fs::canonicalize(&file).unwrap(), url.as_str())
        .unwrap();
    let resource_revision = captured.revision;
    let data = TempDir::new().unwrap();
    let store = Store::open(&file, Some(data.path().to_path_buf())).unwrap();
    let metadata = service::document_metadata(&store).unwrap();
    let pack = serde_json::from_value(json!({"request_id":"pack-browser-probe",
        "expected_revision":metadata["revision"],"expected_dependency_revision":metadata["dependency_revision"],
        "operations":[{"op":"pack_browser_resources"}]})).unwrap();
    service::apply_request(&store, &pack).unwrap();

    let installed = InstalledBrowser {
        executable: std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap().into(),
        version: "test-managed-runtime".into(),
        platform: lazypencil::runtime::platform().unwrap().into(),
    };
    let startup = Budget::new(Duration::from_secs(30), None);
    let mut browser = Browser::launch_with_supervisor(
        &installed,
        Purpose::Page,
        &startup,
        std::path::Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap();
    let plain_url = url::Url::from_file_path(site.join("extensionless")).unwrap();
    let plain_url_json = serde_json::to_string(plain_url.as_str()).unwrap();
    let plain = |browser: &mut Browser, captured: Option<Arc<LocalFiles>>| {
        let budget = Budget::new(Duration::from_secs(15), None);
        browser.with_tab(&budget, |browser, session| {
            browser.command(Some(session), "Page.enable", json!({}), &budget)?;
            if let Some(files) = captured { browser.intercept_local_files(session, files, &budget)?; }
            browser.command(Some(session), "Emulation.setDeviceMetricsOverride", json!({"width":64,"height":64,"deviceScaleFactor":1,"mobile":false}), &budget)?;
            let navigation = browser.command(Some(session), "Page.navigate", json!({"url":plain_url.as_str()}), &budget)?;
            assert!(navigation.get("errorText").is_none(), "{navigation}");
            assert_ne!(navigation["isDownload"], true, "{navigation}");
            let expression = format!("(()=>{{if(location.href!=={plain_url_json}||document.readyState!=='complete')return null;return {{content_type:document.contentType,text:document.body.textContent,executed:window.nativeNoExtRan===true}}}})()");
            let probe = loop {
                let value = browser.evaluate(session, &expression, &budget)?;
                if !value.is_null() { break value; }
                std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
            };
            browser.evaluate(session, "(async()=>{await document.fonts.ready;await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r(true))));return true})()", &budget)?;
            let screenshot = browser.command(Some(session), "Page.captureScreenshot", json!({"format":"png","captureBeyondViewport":false}), &budget)?;
            let png = base64::engine::general_purpose::STANDARD.decode(screenshot["data"].as_str().unwrap()).unwrap();
            Ok((png, probe))
        }).unwrap()
    };
    let (native_plain, native_probe) = plain(&mut browser, None);
    assert_eq!(native_probe["content_type"], "text/plain");
    assert_eq!(native_probe["executed"], false);
    fs::remove_dir_all(&site).unwrap();
    assert!(!site.exists());
    // Reload from published managed files after removing the source, rather
    // than relying only on the first in-memory capture.
    let packed = Project::load(&store).unwrap();
    let reloaded = packed
        .browser_page_resources(store.path(), url.as_str())
        .unwrap();
    assert_eq!(reloaded.revision, resource_revision);
    packed.verify_unchanged().unwrap();
    let files = reloaded.files;
    assert_eq!(
        project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    // This live file would execute if an unknown request reached the host.
    fs::write(
        original.path().join("unlisted.js"),
        "window.unsafeHostFile=true",
    )
    .unwrap();
    let (captured_plain, captured_probe) = plain(&mut browser, Some(files.clone()));
    assert_eq!(native_probe, captured_probe);
    assert_eq!(native_plain, captured_plain);
    let budget = Budget::new(Duration::from_secs(15), None);
    let (png, stats, probe) = browser
        .with_tab(&budget, |browser, session| {
            browser.command(Some(session), "Page.enable", json!({}), &budget)?;
            browser.intercept_local_files(session, files, &budget)?;
            browser.command(
                Some(session),
                "Emulation.setDeviceMetricsOverride",
                json!({"width":32,"height":32,"deviceScaleFactor":1,"mobile":false}),
                &budget,
            )?;
            let navigation = browser.command(
                Some(session),
                "Page.navigate",
                json!({"url":url.as_str()}),
                &budget,
            )?;
            assert!(navigation.get("errorText").is_none(), "{navigation}");
            let probe = loop {
                let value = browser.evaluate(
                    session,
                    "window.workerFailure ? {worker_failure:window.workerFailure} : window.frameReady && window.workerReady && window.localProbe || null",
                    &budget,
                )?;
                if !value.is_null() {
                    break value;
                }
                std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
            };
            assert_eq!(probe["background"], "rgb(0, 255, 0)", "{probe}");
            assert_eq!(probe["image"], 8);
            assert_eq!(probe["unsafe"], false);
            assert_eq!(probe["location"], url.as_str());
            browser.evaluate(
                session,
                "new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>r(true))))",
                &budget,
            )?;
            let screenshot = browser.command(
                Some(session),
                "Page.captureScreenshot",
                json!({"format":"png","captureBeyondViewport":false}),
                &budget,
            )?;
            let png = base64::engine::general_purpose::STANDARD
                .decode(screenshot["data"].as_str().unwrap())
                .unwrap();
            let stats = browser.local_file_stats(session).unwrap();
            assert_eq!(stats.fulfilled, 8);
            assert_eq!(stats.denied, 1);
            assert_eq!(stats.response_bytes, (captured_bytes + sources.iter().find(|(name, _)| *name == "worker-library.js").unwrap().1.len()) as u64);
            assert!(stats.attached_targets >= 2);
            Ok((png, stats, probe))
        })
        .unwrap();
    let raster = Image::from_encoded(Data::new_copy(&png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    assert_eq!((raster.width(), raster.height()), (32, 32));
    let pixels = raster.peek_pixels().unwrap();
    for y in 0..32 {
        for x in 0..32 {
            let expected = if y < 8 && x < 8 {
                Color::BLUE
            } else if y < 8 && x < 16 {
                Color::RED
            } else {
                Color::GREEN
            };
            assert_eq!(
                pixels.get_color((x, y)),
                expected,
                "local resource pixel {x},{y}"
            );
        }
    }
    assert!(!site.exists());
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("codec-browser-local-files.png"), &png).unwrap();
        fs::write(
            directory.join("codec-browser-local-files-native-plain.png"),
            &native_plain,
        )
        .unwrap();
        fs::write(
            directory.join("codec-browser-local-files-captured-plain.png"),
            &captured_plain,
        )
        .unwrap();
        fs::write(directory.join("codec-browser-local-files.json"), serde_json::to_vec_pretty(&json!({
            "source_commit":std::env::var("GITHUB_SHA").ok(),"width":32,"height":32,
            "png_hash":lazypencil::document::hash(&png),"stats":stats,"source_removed":true,
            "location_preserved":probe["location"]==url.as_str(),"unsafe_host_file":probe["unsafe"],
            "sandbox":"enabled","browser":browser.version,"project_resource_revision":resource_revision,
            "managed_bundle_reloaded_after_source_removal":true,
            "extensionless":{"content_type":native_probe["content_type"],"executed":native_probe["executed"],
                "text_sha256":lazypencil::document::hash(native_probe["text"].as_str().unwrap().as_bytes()),
                "png_hash":lazypencil::document::hash(&native_plain),"equivalent":true}
        })).unwrap()).unwrap();
    }
}

#[test]
fn reference_server_keeps_speculative_connections_until_headers_and_stops_when_idle() {
    let server = reference::Server::new();
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    stream
        .write_all(b"GET /polled HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    let _idle = TcpStream::connect(server.address).unwrap();
    let started = Instant::now();
    drop(server);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn visible_selector_waits_for_post_load_content_and_keeps_selector_text_as_data() {
    let server = reference::Server::new();
    let f = Fixture::new(&format!("http://{}/selector", server.address));
    let mut runtime = f.runtime();
    let first_project = f.project();
    let first = f.capture(&mut runtime, None).unwrap();
    f.policy("#ready", "visible", 5000);
    assert_eq!(
        first_project.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let image = f.capture(&mut runtime, None).unwrap();
    assert_green(&image);
    let data = acquisition(&image);
    assert_eq!(data["ready_selector"], "#ready");
    assert_eq!(data["selector_state"], "visible");
    assert_eq!(data["limits"]["browser"]["ready_selector"], "#ready");
    assert_ne!(
        data["acquisition_id"],
        acquisition(&first)["acquisition_id"]
    );
    let reused = f.capture(&mut runtime, None).unwrap();
    assert_eq!(reused.dynamic_artifacts, image.dynamic_artifacts);

    let marker = "done');document.documentElement.style.background='red';//";
    f.url(&page(&format!(
        "<style>html{{background:#0f0}}</style><div data-ready=\"{marker}\" style='width:1px;height:1px'></div>"
    )));
    let selector = format!("[data-ready=\"{marker}\"]");
    f.policy(&selector, "visible", 5000);
    let quoted = f.capture(&mut runtime, None).unwrap();
    assert_green(&quoted);
    assert_eq!(acquisition(&quoted)["ready_selector"], selector);
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn selector_attachment_visibility_missing_targets_and_syntax_have_distinct_results() {
    let f = Fixture::new(&page(
        "<style>html{background:#0f0}</style><div id='ready' style='visibility:hidden'>Ready</div>",
    ));
    let mut runtime = f.runtime();
    f.policy("#ready", "attached", 5000);
    let attached = f.capture(&mut runtime, None).unwrap();
    assert_green(&attached);
    assert_eq!(acquisition(&attached)["selector_state"], "attached");
    f.policy("#ready", "visible", 1500);
    let started = Instant::now();
    assert_eq!(
        f.capture(&mut runtime, None).err().unwrap().code,
        "RENDER_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(runtime.process_ids().is_empty());

    f.policy("[", "visible", 5000);
    let invalid = f.capture(&mut runtime, None).err().unwrap();
    assert_eq!(invalid.code, "PAGE_LOAD_FAILED");
    assert_eq!(invalid.message, "Invalid page readiness selector");
    assert!(runtime.process_ids().is_empty());

    for html in [
        "<div id='ready' style='width:0;height:0'></div>",
        "<div style='opacity:0'><div id='ready'>Ready</div></div>",
        "<div style='content-visibility:hidden'><div id='ready'>Ready</div></div>",
        "<div id='another'>Missing</div>",
    ] {
        f.url(&page(html));
        // Warm the process with the selector explicitly cleared, then change
        // only the policy so a cached acquisition cannot hide a missing target.
        f.policy("", "visible", 5000);
        f.capture(&mut runtime, None).unwrap();
        f.policy("#ready", "visible", 1500);
        assert_eq!(
            f.capture(&mut runtime, None).err().unwrap().code,
            "RENDER_TIMEOUT",
            "{html}"
        );
        assert!(runtime.process_ids().is_empty());
    }
    f.url(&page("<style>html{background:#0f0}</style>"));
    f.policy("", "visible", 5000);
    let recovered = f.capture(&mut runtime, None).unwrap();
    assert_green(&recovered);
    assert_eq!(acquisition(&recovered)["ready_selector"], "");
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn cancelling_an_observed_selector_poll_reaps_the_process_and_recovers() {
    let probe = Arc::new(AtomicBool::new(false));
    let server = reference::Server::with_selector_probe(probe.clone());
    let f = Fixture::new(&format!("http://{}/selector-cancel", server.address));
    let mut runtime = f.runtime();
    // This case cancels selector polling. Complete cold startup before starting
    // its observation deadline; launch cancellation has separate coverage.
    let startup_timeout = lazypencil::config::runtime::BrowserSettings::default().timeout_ms;
    f.policy("", "visible", startup_timeout);
    let startup = Instant::now();
    let initial = f.capture(&mut runtime, None).unwrap_or_else(|error| {
        panic!(
            "Cold browser startup failed after {:?} with the default {startup_timeout}ms acquisition limit: {error:?}",
            startup.elapsed()
        )
    });
    assert_green(&initial);
    assert!(!probe.load(Ordering::SeqCst));
    assert!(!runtime.process_ids().is_empty());
    f.policy("#never", "visible", 5000);
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    let canceller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !probe.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let observed = probe.load(Ordering::SeqCst);
        signal.cancel();
        observed
    });
    let started = Instant::now();
    let result = f.capture(&mut runtime, Some(cancel));
    assert!(
        canceller.join().unwrap(),
        "The page did not observe the target selector poll; elapsed: {:?}, error: {:?}",
        started.elapsed(),
        result.as_ref().err()
    );
    assert_eq!(result.err().unwrap().code, "CANCELLED");
    assert!(runtime.process_ids().is_empty());
    // Cancellation reaps the process, so recovery also includes a cold launch.
    f.policy("", "visible", startup_timeout);
    assert_green(&f.capture(&mut runtime, None).unwrap());
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn selector_visibility_uses_the_requested_view_without_a_css_smooth_scroll() {
    let url = page(
        "<style>html{background:#f00;scroll-behavior:smooth}body{margin:0}section{position:absolute;top:5000px;left:0;width:32px;height:100px;content-visibility:auto;contain-intrinsic-size:32px 100px}#ready{width:32px;height:100px;background:#0f0}</style><section><div id='ready'></div></section><script>const scrollX=-9,scrollY=-7;document.documentElement.dataset.shadowed='yes';</script>",
    );
    let f = Fixture::new(&url);
    f.url_and_scroll(&url, 5000);
    f.policy("html[data-shadowed=yes] #ready", "visible", 5000);
    let image = f.capture(&mut f.runtime(), None).unwrap();
    assert_green(&image);
    assert_eq!(acquisition(&image)["scroll_y"], 5000);
    assert_eq!(acquisition(&image)["scroll_x"], 0);
}
