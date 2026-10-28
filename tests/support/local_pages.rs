//! Exercise project capture, ordinary rendering and retained-page invalidation together.

use super::*;
use lazypencil::{geometry::Rect, render::RenderOptions, store::Store};

const APP: &str = r#"
const libraryURL=JSON.stringify(new URL('worker-library.js',location.href).href);
const nestedCode=`importScripts(${libraryURL});postMessage(capturedColor)`;
const workerCode=`importScripts(${libraryURL});const child=new Worker(URL.createObjectURL(new Blob([${JSON.stringify(nestedCode)}],{type:'text/javascript'})));
child.onmessage=e=>{postMessage(capturedColor===e.data?capturedColor:'bad');child.terminate()}`;
const worker=new Worker(URL.createObjectURL(new Blob([workerCode],{type:'text/javascript'})));
worker.onmessage=e=>{
  if(e.data==='bad')return;
  document.documentElement.style.background=e.data;
  const ready=document.createElement('div');ready.id='ready';document.body.append(ready);
  if(window.unsafeHostFile!==true&&location.search==='?theme=$mode'&&location.hash==='#keep')document.documentElement.dataset.safe='yes';
  const initialWidth=innerWidth;
  window.addEventListener('resize',()=>{if(innerWidth!==initialWidth)document.documentElement.style.background='#ffff00'});
  worker.terminate();
};
"#;

struct LocalFixture {
    temp: TempDir,
    store: Store,
    base: String,
}

impl LocalFixture {
    fn new(version: &str) -> Self {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let site = temp.path().join("site with spaces 日本語");
        fs::create_dir(&site).unwrap();
        let base = url::Url::from_directory_path(fs::canonicalize(&site).unwrap())
            .unwrap()
            .to_string();
        fs::write(site.join("index.html"),r#"<!doctype html><html><head><link rel='icon' href='data:,'><link rel='stylesheet' href='style.css'><script src='../unlisted.js'></script><script src='app.js' defer></script></head><body></body></html>"#).unwrap();
        fs::write(site.join("style.css"), "html,body{margin:0}").unwrap();
        fs::write(site.join("app.js"), APP).unwrap();
        fs::write(
            temp.path().join("unlisted.js"),
            "window.unsafeHostFile=true",
        )
        .unwrap();
        fs::write(temp.path().join("lazypencil.toml"),"[runtime.browser]\nwait_until='load'\nready_selector=\"html[data-safe='yes'] #ready\"\nselector_state='attached'\ntimeout_ms=15000\n").unwrap();
        let store = Store::init(
            temp.path().join("scene.pen"),
            Some(temp.path().join("private")),
        )
        .unwrap();
        fs::write(store.path(),json!({"version":version,"children":[{"id":"page","type":"browser","url":format!("{base}index.html?theme=$mode#keep"),"width":32,"height":32}]}).to_string()).unwrap();
        let fixture = Self { temp, store, base };
        fixture.color("#00ff00");
        fixture.roots(true);
        fixture
    }
    fn site(&self) -> std::path::PathBuf {
        self.temp.path().join("site with spaces 日本語")
    }
    fn color(&self, color: &str) {
        fs::write(
            self.site().join("worker-library.js"),
            format!("self.capturedColor='{color}'"),
        )
        .unwrap();
    }
    fn roots(&self, enabled: bool) {
        let roots = if enabled {
            json!([{"url":self.base,"path":"site with spaces 日本語"}])
        } else {
            json!([])
        };
        browser_copy_fixture::apply(
            &self.store,
            &format!("roots-{}", uuid::Uuid::new_v4()),
            json!([{"op":"set_browser_resources","roots":roots}]),
        );
    }
    fn update(&self, id: &str, properties: Value) {
        browser_copy_fixture::apply(
            &self.store,
            id,
            json!([{"op":"update","target":"page","set":properties}]),
        );
    }
    fn runtime(&self) -> DynamicRuntime {
        DynamicRuntime::new(self.store.data_dir())
            .with_supervisor_executable(env!("CARGO_BIN_EXE_lazypencil"))
            .with_executable(std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").unwrap())
    }
    fn capture(
        &self,
        runtime: &mut DynamicRuntime,
        offscreen: bool,
        cancel: Option<CancellationToken>,
    ) -> lazypencil::Result<RenderedImage> {
        self.capture_options(
            runtime,
            &RenderOptions {
                region: offscreen.then_some(Rect::new(0.0, 0.0, 32.0, 32.0)),
                ..Default::default()
            },
            cancel,
        )
    }
    fn capture_options(
        &self,
        runtime: &mut DynamicRuntime,
        options: &RenderOptions,
        cancel: Option<CancellationToken>,
    ) -> lazypencil::Result<RenderedImage> {
        let project = Project::load(&self.store).unwrap();
        let saved = fs::read(self.store.path()).unwrap();
        let snapshot =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        let image = render::png_with_runtime(&project, &snapshot, options, runtime, cancel);
        assert_eq!(fs::read(self.store.path()).unwrap(), saved);
        image
    }
}

#[cfg(unix)]
#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn offline_network_policy_blocks_http_websocket_and_tls_routes_for_pages_and_workers() {
    let server = reference::Server::new();
    let f = LocalFixture::new("2.21");
    let base = format!("http://{}/network", server.address);
    let worker = format!(
        r#"
        const base={base},secure=base.replace('http:','https:');
        const socket=url=>new Promise(r=>{{const s=new WebSocket(url);s.onopen=()=>{{s.close();r('open')}};s.onerror=()=>r('error');setTimeout(()=>r('timeout'),2000)}});
        Promise.all([fetch(base+'/worker').then(r=>r.text()).catch(()=> 'error'),socket(base.replace('http:','ws:')+'/socket?worker'),fetch(secure+'/worker').then(r=>r.text()).catch(()=> 'error'),socket(secure.replace('https:','wss:')+'/socket?worker')]).then(v=>postMessage(v));
    "#,
        base = json!(base)
    );
    let html = format!(
        r#"<!doctype html><html><head><link rel='icon' href='data:,'><style>html{{height:100%;background:red}}body{{margin:0}}</style></head><body><script>
        const base={base},secure=base.replace('http:','https:');
        const socket=url=>new Promise(r=>{{const s=new WebSocket(url);s.onopen=()=>{{s.close();r('open')}};s.onerror=()=>r('error');setTimeout(()=>r('timeout'),2000)}});
        const worker=new Worker(URL.createObjectURL(new Blob([{worker}],{{type:'text/javascript'}})));
        const fromWorker=new Promise(r=>{{worker.onmessage=e=>{{worker.terminate();r(e.data)}};worker.onerror=()=>{{worker.terminate();r('worker-error')}};setTimeout(()=>{{worker.terminate();r('timeout')}},4000)}});
        Promise.all([fetch(base+'/fetch').then(r=>r.text()).catch(()=> 'error'),new Promise(r=>{{const i=new Image();i.onload=()=>r('loaded');i.onerror=()=>r('error');i.src=base+'/image'}}),socket(base.replace('http:','ws:')+'/socket'),new Promise(r=>{{const frame=document.createElement('iframe');frame.style.display='none';frame.onload=()=>r('frame-done');frame.onerror=()=>r('frame-error');frame.src=base+'/frame';document.body.append(frame)}}),fetch(secure+'/fetch').then(r=>r.text()).catch(()=> 'error'),socket(secure.replace('https:','wss:')+'/socket'),fromWorker]).then(results=>{{
          const actual=JSON.stringify(results),online=JSON.stringify(['ok','loaded','open','frame-done','error','error',['ok','open','error','error']]),offline=JSON.stringify(['error','error','error','frame-done','error','error',['error','error','error','error']]);
          document.documentElement.style.background=actual===online||actual===offline?'#00ff00':'#ff0000';
          document.documentElement.dataset.network='yes';
        }});
    </script></body></html>"#,
        base = json!(base),
        worker = json!(worker)
    );
    fs::write(f.site().join("index.html"), html).unwrap();
    let policy = |name: &str| {
        fs::write(f.temp.path().join("lazypencil.toml"),format!("[runtime.browser]\nnetwork_policy='{name}'\nready_selector=\"html[data-network='yes']\"\nselector_state='attached'\ntimeout_ms=15000\n")).unwrap()
    };
    let mut runtime = f.runtime().with_retained_pages(f.store.path());
    let capture = |runtime: &mut DynamicRuntime, stage: &str, refresh: bool| {
        let result = f.capture_options(
            runtime,
            &RenderOptions {
                refresh,
                ..Default::default()
            },
            None,
        );
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            fs::create_dir_all(&directory).unwrap();
            fs::write(std::path::Path::new(&directory).join("codec-network-policy-observation.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"stage":stage,"server_counts":server.network_counts(),"error":result.as_ref().err().map(|error|json!({"code":error.code,"message":error.message}))})).unwrap()).unwrap();
        }
        result.unwrap_or_else(|error| {
            panic!(
                "{stage}: {error:?}; server_counts={:?}",
                server.network_counts()
            )
        })
    };
    let mut rows = Vec::new();
    let mut retain = |stage: &str, image: &RenderedImage| {
        let raster = Image::from_encoded(Data::new_copy(&image.png))
            .unwrap()
            .make_raster_image(None, None)
            .unwrap();
        let pixels = raster.peek_pixels().unwrap();
        for y in 0..image.height as i32 {
            for x in 0..image.width as i32 {
                assert_eq!(pixels.get_color((x, y)), Color::GREEN, "{stage}: {x},{y}");
            }
        }
        let name = format!("codec-network-policy-{}x{}.png", image.width, image.height);
        if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
            let path = std::path::Path::new(&directory).join(&name);
            if path.exists() {
                assert_eq!(fs::read(path).unwrap(), image.png);
            } else {
                fs::create_dir_all(&directory).unwrap();
                fs::write(path, &image.png).unwrap();
            }
        }
        rows.push(json!({"stage":stage,"png":name,"png_hash":lazypencil::document::hash(&image.png),"width":image.width,"height":image.height,"server_counts":server.network_counts(),"acquisitions":image.dynamic_artifacts.iter().map(|row|json!({"network_policy":row["acquisition"]["network_policy"],"limits_policy":row["acquisition"]["limits"]["browser"]["network_policy"],"page_state":row["acquisition"]["page_state"],"acquisition_id":row["acquisition"]["acquisition_id"]})).collect::<Vec<_>>()}));
    };
    policy("online");
    let online = capture(&mut runtime, "online", false);
    let first_counts = server.network_counts();
    assert_eq!((first_counts.0, first_counts.1), (4, 2));
    assert!(
        first_counts.2 >= 4,
        "HTTPS/WSS positive routing controls did not reach the TLS sink: {first_counts:?}"
    );
    assert_eq!(acquisition(&online)["network_policy"], "online");
    retain("online", &online);
    let captured = Project::load(&f.store).unwrap();
    policy("offline");
    assert_eq!(
        captured.verify_unchanged().unwrap_err().code,
        "REVISION_CONFLICT"
    );
    let offline = capture(&mut runtime, "offline", false);
    assert_eq!(server.network_counts(), first_counts);
    assert_eq!(acquisition(&offline)["network_policy"], "offline");
    assert_eq!(
        acquisition(&offline)["limits"]["browser"]["network_policy"],
        "offline"
    );
    assert_ne!(
        acquisition(&offline)["page_state"]["id"],
        acquisition(&online)["page_state"]["id"]
    );
    retain("offline", &offline);
    let cached = capture(&mut runtime, "offline-cached", false);
    assert_eq!(cached.dynamic_artifacts, offline.dynamic_artifacts);
    assert_eq!(server.network_counts(), first_counts);
    let refreshed = capture(&mut runtime, "offline-refresh", true);
    assert_eq!(
        acquisition(&refreshed)["page_state"]["id"],
        acquisition(&offline)["page_state"]["id"]
    );
    assert_eq!(acquisition(&refreshed)["page_state"]["reused"], true);
    assert_ne!(
        acquisition(&refreshed)["acquisition_id"],
        acquisition(&offline)["acquisition_id"]
    );
    assert_eq!(server.network_counts(), first_counts);
    retain("offline-refresh", &refreshed);
    let mut document: Value =
        serde_json::from_str(&fs::read_to_string(f.store.path()).unwrap()).unwrap();
    let page_url = document["children"][0]["url"].clone();
    document["children"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"other","type":"browser","url":page_url,"x":32,"width":32,"height":32}));
    fs::write(f.store.path(), document.to_string()).unwrap();
    let two = capture(&mut runtime, "offline-second-context", false);
    assert_eq!(two.dynamic_artifacts.len(), 2);
    assert_ne!(
        two.dynamic_artifacts[0]["acquisition"]["page_state"]["id"],
        two.dynamic_artifacts[1]["acquisition"]["page_state"]["id"]
    );
    assert!(
        two.dynamic_artifacts
            .iter()
            .all(|row| row["acquisition"]["network_policy"] == "offline")
    );
    assert_eq!(server.network_counts(), first_counts);
    retain("offline-second-context", &two);
    policy("online");
    let resumed = capture(&mut runtime, "online-two-contexts", false);
    let resumed_counts = server.network_counts();
    assert_eq!(
        (resumed_counts.0, resumed_counts.1),
        (first_counts.0 + 8, first_counts.1 + 4)
    );
    assert!(resumed_counts.2 >= first_counts.2 + 8);
    retain("online-two-contexts", &resumed);
    document["children"].as_array_mut().unwrap().truncate(1);
    document["children"][0]["url"] = format!("http://{}/network/green", server.address).into();
    fs::write(f.store.path(), document.to_string()).unwrap();
    policy("offline");
    assert_eq!(
        f.capture(&mut runtime, false, None).err().unwrap().code,
        "PAGE_LOAD_FAILED"
    );
    assert!(runtime.process_ids().is_empty());
    assert_eq!(server.network_counts(), resumed_counts);
    document["children"][0]["url"] = format!("{}index.html?theme=$mode#keep", f.base).into();
    fs::write(f.store.path(), document.to_string()).unwrap();
    fs::write(
        f.site().join("index.html"),
        format!(
            "<script>location.replace({})</script>",
            json!(format!("http://{}/network/green", server.address))
        ),
    )
    .unwrap();
    let redirected = f.capture(&mut runtime, false, None).err().unwrap();
    assert_eq!(
        redirected.code, "PAGE_LOAD_FAILED",
        "Unexpected redirect failure: {redirected:?}"
    );
    assert!(runtime.process_ids().is_empty());
    assert_eq!(server.network_counts(), resumed_counts);
    document["children"][0]["url"] = format!("http://{}/network/green", server.address).into();
    fs::write(f.store.path(), document.to_string()).unwrap();
    policy("online");
    let recovered = capture(&mut runtime, "http-navigation-recovered", false);
    assert_eq!(server.network_counts().0, resumed_counts.0 + 1);
    retain("http-navigation-recovered", &recovered);
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(std::path::Path::new(&directory).join("codec-network-policy.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"cases":rows,"cache_reused":true,"offline_navigation_rejected":true,"offline_redirect_rejected":true,"runtime_reaped_and_recovered":true,"TLS_control":"Intentional failing TLS sink; positive counts prove HTTPS/WSS transport routing, not successful TLS content or certificate validation"})).unwrap()).unwrap();
    }
}

fn record(version: &str, stage: &str, image: &RenderedImage, rgba: [u8; 4], rows: &mut Vec<Value>) {
    let raster = Image::from_encoded(Data::new_copy(&image.png))
        .unwrap()
        .make_raster_image(None, None)
        .unwrap();
    let pixels = raster.peek_pixels().unwrap();
    let expected = Color::from_argb(rgba[3], rgba[0], rgba[1], rgba[2]);
    for y in 0..image.height as i32 {
        for x in 0..image.width as i32 {
            assert_eq!(
                pixels.get_color((x, y)),
                expected,
                "{version} {stage} {x},{y}"
            );
        }
    }
    let acquisition = acquisition(image);
    assert_eq!(acquisition["local_files"]["fulfilled"], 5);
    assert_eq!(acquisition["local_files"]["denied"], 1);
    assert!(
        acquisition["local_files"]["attached_targets"]
            .as_u64()
            .unwrap()
            >= 2
    );
    let name = format!("codec-local-page-{version}-{stage}.png");
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::create_dir_all(&directory).unwrap();
        fs::write(std::path::Path::new(&directory).join(&name), &image.png).unwrap();
    }
    rows.push(json!({"document_version":version,"stage":stage,"name":name,"width":image.width,"height":image.height,"rgba":rgba,"png_hash":lazypencil::document::hash(&image.png),"acquisition":acquisition}));
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn project_local_pages_update_resources_retire_offscreen_state_and_recover() {
    let mut rows = Vec::new();
    let mut cases = Vec::new();
    for version in ["2.19", "2.21"] {
        let fixture = LocalFixture::new(version);
        let mut runtime = fixture.runtime();
        let project = Project::load(&fixture.store).unwrap();
        assert!(
            runtime
                .page(&project.resolve().unwrap().roots[0], 32.0, 32.0, 1.0)
                .is_err()
        );
        assert!(runtime.process_ids().is_empty());
        let first = fixture.capture(&mut runtime, false, None).unwrap();
        record(version, "first", &first, [0, 255, 0, 255], &mut rows);
        let cached = fixture.capture(&mut runtime, false, None).unwrap();
        assert_eq!(
            acquisition(&cached)["acquisition_id"],
            acquisition(&first)["acquisition_id"]
        );
        assert_eq!(cached.png, first.png);
        fixture.color("#0000ff");
        let changed = fixture.capture(&mut runtime, false, None).unwrap();
        record(version, "changed", &changed, [0, 0, 255, 255], &mut rows);
        assert_ne!(
            acquisition(&changed)["resources"]["revision"],
            acquisition(&first)["resources"]["revision"]
        );
        assert_ne!(
            acquisition(&changed)["acquisition_id"],
            acquisition(&first)["acquisition_id"]
        );
        runtime = runtime.with_retained_pages(fixture.store.path());
        let retained = fixture.capture(&mut runtime, false, None).unwrap();
        record(version, "retained", &retained, [0, 0, 255, 255], &mut rows);
        let state = acquisition(&retained)["page_state"]["id"].clone();
        assert!(state.is_string());
        assert_eq!(runtime.retained_page_count(), 1);
        let processes = runtime.process_ids();
        fixture.update("resize", json!({"width":40}));
        let resized = fixture.capture(&mut runtime, false, None).unwrap();
        record(version, "resized", &resized, [255, 255, 0, 255], &mut rows);
        assert_eq!(acquisition(&resized)["page_state"]["id"], state);
        assert_eq!(acquisition(&resized)["page_state"]["reused"], true);
        fixture.color("#ff0000");
        fixture.update("offscreen", json!({"x":200}));
        let offscreen = fixture.capture(&mut runtime, true, None).unwrap();
        assert!(offscreen.dynamic_artifacts.is_empty());
        assert_eq!(runtime.retained_page_count(), 0);
        assert_eq!(runtime.process_ids(), processes);
        fixture.update("visible", json!({"x":0}));
        let returned = fixture.capture(&mut runtime, false, None).unwrap();
        record(version, "returned", &returned, [255, 0, 0, 255], &mut rows);
        assert_ne!(acquisition(&returned)["page_state"]["id"], state);
        fixture.update("offscreen-denied", json!({"x":200}));
        fixture.roots(false);
        assert!(
            fixture
                .capture(&mut runtime, true, None)
                .unwrap()
                .dynamic_artifacts
                .is_empty()
        );
        assert_eq!(runtime.retained_page_count(), 0);
        fixture.update("visible-denied", json!({"x":0}));
        assert_eq!(
            fixture
                .capture(&mut runtime, false, None)
                .err()
                .unwrap()
                .code,
            "ASSET_NOT_FOUND"
        );
        assert!(runtime.process_ids().is_empty());
        fixture.roots(true);
        let restored = fixture.capture(&mut runtime, false, None).unwrap();
        record(version, "restored", &restored, [255, 0, 0, 255], &mut rows);
        fs::write(fixture.site().join("app.js"), "window.waitingForever=true").unwrap();
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let cancellation = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            trigger.cancel()
        });
        let result = fixture.capture(&mut runtime, false, Some(cancel));
        cancellation.join().unwrap();
        assert_eq!(result.err().unwrap().code, "CANCELLED");
        assert!(runtime.process_ids().is_empty());
        fs::write(fixture.site().join("app.js"), APP).unwrap();
        let recovered = fixture.capture(&mut runtime, false, None).unwrap();
        record(
            version,
            "recovered",
            &recovered,
            [255, 0, 0, 255],
            &mut rows,
        );
        assert_ne!(
            acquisition(&recovered)["page_state"]["id"],
            acquisition(&restored)["page_state"]["id"]
        );
        cases.push(json!({"document_version":version,"raster_cache_reused":true,"offscreen_resource_change_retired":true,"offscreen_denied_retired":true,"cancelled_process_reaped":true,"same_browser_after_offscreen_retirement":true}));
    }
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_CODEC_DIR") {
        fs::write(std::path::Path::new(&directory).join("codec-local-pages.json"),serde_json::to_vec_pretty(&json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"sandbox":"enabled","cases":cases,"images":rows})).unwrap()).unwrap();
    }
}
