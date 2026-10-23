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
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

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
        DynamicRuntime::new(self.0.path()).with_executable(
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
    f.policy("#never", "visible", 5000);
    let mut runtime = f.runtime();
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
    let result = f.capture(&mut runtime, Some(cancel));
    assert!(
        canceller.join().unwrap(),
        "The page did not observe a selector poll"
    );
    assert_eq!(result.err().unwrap().code, "CANCELLED");
    assert!(runtime.process_ids().is_empty());
    f.policy("", "visible", 5000);
    assert_green(&f.capture(&mut runtime, None).unwrap());
}

#[test]
#[ignore = "Requires sandboxed headless Chromium"]
fn selector_visibility_uses_the_requested_view_without_a_css_smooth_scroll() {
    let url = page(
        "<style>html{background:#f00;scroll-behavior:smooth}body{margin:0}section{position:absolute;top:5000px;left:0;width:32px;height:100px;content-visibility:auto;contain-intrinsic-size:32px 100px}#ready{width:32px;height:100px;background:#0f0}</style><section><div id='ready'></div></section>",
    );
    let f = Fixture::new(&url);
    f.url_and_scroll(&url, 5000);
    f.policy("#ready", "visible", 5000);
    let image = f.capture(&mut f.runtime(), None).unwrap();
    assert_green(&image);
    assert_eq!(acquisition(&image)["scroll_y"], 5000);
}
