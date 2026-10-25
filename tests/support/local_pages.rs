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
        let project = Project::load(&self.store).unwrap();
        let saved = fs::read(self.store.path()).unwrap();
        let snapshot =
            lazypencil::layout::compute(project.resolve().unwrap(), &mut Fonts::new(&[]).unwrap())
                .unwrap();
        let image = render::png_with_runtime(
            &project,
            &snapshot,
            &RenderOptions {
                region: offscreen.then_some(Rect::new(0.0, 0.0, 32.0, 32.0)),
                ..Default::default()
            },
            runtime,
            cancel,
        );
        assert_eq!(fs::read(self.store.path()).unwrap(), saved);
        image
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
