//! Coalesced previews; Skia and dynamic runtimes stay on their owning thread.

use super::graphics::Backend;
use crate::{
    Error, Result, attachments::Attachment, geometry::Rect, project::Project, store::Store,
};
use ratatui::layout::Size;
use ratatui_image::{Resize, protocol::Protocol};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Default)]
pub struct View {
    // Accumulate camera movement in double precision; Skia regions remain f32.
    // At fractional zoom, opposite f32 pan steps can leave a visible ULP drift.
    pub center: [f64; 2],
    pub zoom: Option<f32>,
    pub time: f64,
}
impl View {
    pub fn set_time(&mut self, time: f64) -> Result<()> {
        if !time.is_finite() || time < 0.0 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Shader time must be a finite nonnegative number of seconds",
            ));
        }
        self.time = time;
        Ok(())
    }
    pub fn fit(&mut self, bounds: Rect, pixels: [u32; 2]) {
        self.center = [
            f64::from(bounds.x + bounds.width / 2.0),
            f64::from(bounds.y + bounds.height / 2.0),
        ];
        self.zoom = Some(
            ((pixels[0] as f32 / bounds.width.max(1.0))
                .min(pixels[1] as f32 / bounds.height.max(1.0))
                * 0.94)
                .clamp(0.01, 16.0),
        );
    }
    pub fn pan(&mut self, pixels: [f64; 2]) {
        let scale = f64::from(self.zoom.unwrap_or(1.0));
        self.center[0] += pixels[0] / scale;
        self.center[1] += pixels[1] / scale;
    }
    pub fn region(self, pixels: [u32; 2]) -> Rect {
        let scale = self.zoom.unwrap_or(1.0);
        let width = pixels[0] as f32 / scale;
        let height = pixels[1] as f32 / scale;
        Rect::new(
            self.center[0] as f32 - width / 2.0,
            self.center[1] as f32 - height / 2.0,
            width,
            height,
        )
    }
}

#[derive(Clone)]
pub struct Request {
    pub generation: u64,
    pub cells: Size,
    pub view: View,
    pub refresh: bool,
    pub refresh_pages: bool,
    pub check_only: bool,
    pub attachment: Option<(Attachment, PathBuf)>,
}
pub struct Preview {
    pub protocol: Protocol,
    pub png: Vec<u8>,
    pub view: View,
    pub nodes: Vec<Node>,
    pub summary: Value,
    pub attachment: bool,
    pub time_dependent: bool,
    pub live_pages: bool,
}
pub struct Node {
    pub id: String,
    pub name: String,
    pub bounds: Rect,
    pub target: Value,
}
pub struct Output {
    pub generation: u64,
    pub result: Result<Preview>,
}
struct Shared {
    request: Mutex<(Option<Request>, bool)>,
    wake: Condvar,
    active: Mutex<Option<CancellationToken>>,
    output: Mutex<Option<Output>>,
    notify: Notify,
    generation: AtomicU64,
}
pub struct Worker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        store: Store,
        fonts: Vec<PathBuf>,
        picker: Backend,
        image_id: u32,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            request: Mutex::new((None, false)),
            wake: Condvar::new(),
            active: Mutex::new(None),
            output: Mutex::new(None),
            notify: Notify::new(),
            generation: AtomicU64::new(0),
        });
        let state = shared.clone();
        let thread = std::thread::Builder::new()
            .name("lazypencil-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&state, &store, &fonts, &picker, image_id)
                }));
                if result.is_err() {
                    let generation = state.generation.load(Ordering::Relaxed);
                    *state.output.lock().unwrap() = Some(Output {
                        generation,
                        result: Err(Error::new(
                            "RENDER_ERROR",
                            "Preview worker stopped unexpectedly",
                        )),
                    });
                    state.notify.notify_one();
                }
            })
            .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    pub fn request(&self, request: Request) {
        let mut pending = self.shared.request.lock().unwrap();
        self.shared
            .generation
            .store(request.generation, Ordering::Relaxed);
        if !request.check_only
            && let Some(cancel) = self.shared.active.lock().unwrap().as_ref()
        {
            cancel.cancel();
        }
        // A periodic dependency check must never replace a requested viewport change.
        if !request.check_only || pending.0.is_none() {
            pending.0 = Some(request);
        }
        self.shared.wake.notify_one();
    }
    pub fn request_if_idle(&self, request: Request) -> bool {
        let mut pending = self.shared.request.lock().unwrap();
        if pending.1
            || pending.0.is_some()
            || self.shared.active.lock().unwrap().is_some()
            || self.shared.output.lock().unwrap().is_some()
        {
            return false;
        }
        self.shared
            .generation
            .store(request.generation, Ordering::Relaxed);
        pending.0 = Some(request);
        self.shared.wake.notify_one();
        true
    }
    pub async fn next(&self) -> Output {
        loop {
            self.shared.notify.notified().await;
            if let Some(output) = self.shared.output.lock().unwrap().take() {
                return output;
            }
        }
    }
    pub fn cancel(&self) {
        let mut state = self.shared.request.lock().unwrap();
        state.1 = true;
        if let Some(cancel) = self.shared.active.lock().unwrap().as_ref() {
            cancel.cancel();
        }
        self.shared.wake.notify_one();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn work(shared: &Shared, store: &Store, fonts: &[PathBuf], picker: &Backend, image_id: u32) {
    let mut cached: Option<(Project, crate::layout::LayoutSnapshot)> = None;
    let mut document_preview_time = None;
    // Owned hosted measurements retain stderr separately from terminal output.
    // Never log document contents, file paths, or settings; cap completed reports.
    let profile = std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
        && std::env::var("LAZYPENCIL_PROFILE_PREVIEW").as_deref() == Ok("owned-gha-measurement");
    let mut profile_reports = 0;
    let mut script =
        crate::script::ScriptRuntime::new(std::env::current_exe().expect("current executable"));
    let mut dynamic =
        crate::dynamic::DynamicRuntime::new(store.data_dir()).with_retained_pages(store.path());
    loop {
        let now = Instant::now();
        script.expire_idle(now);
        dynamic.expire_idle(now);
        let mut state = shared.request.lock().unwrap();
        while state.0.is_none() && !state.1 {
            let deadline = [script.idle_deadline(), dynamic.idle_deadline()]
                .into_iter()
                .flatten()
                .min();
            state = if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                shared.wake.wait_timeout(state, remaining).unwrap().0
            } else {
                shared.wake.wait(state).unwrap()
            };
        }
        if state.1 {
            return;
        }
        let Some(request) = state.0.take() else {
            continue;
        };
        let cancel = CancellationToken::new();
        *shared.active.lock().unwrap() = Some(cancel.clone());
        drop(state);
        script.set_cancellation(Some(cancel.clone()));
        let mut timing = PreviewTiming::new(profile && profile_reports < 128);
        let result = (|| -> Result<Option<Preview>> {
            if let Some((attachment, directory)) = &request.attachment {
                if request.check_only {
                    return Ok(None);
                }
                let png = attachment.read(directory)?;
                let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                    .map_err(|error| Error::new("RENDER_ERROR", error.to_string()))?;
                let image = Resize::Fit(None).resize(
                    &image,
                    picker.font_size(),
                    request.cells,
                    Some(image::Rgba([248, 250, 252, 255])),
                );
                let protocol = encode(picker, image, request.cells, image_id)?;
                return Ok(Some(Preview {
                    protocol,
                    png,
                    view: request.view,
                    nodes: vec![],
                    summary: json!({"attachment":attachment}),
                    attachment: true,
                    time_dependent: false,
                    live_pages: false,
                }));
            }
            let unchanged = !request.refresh
                && cached.as_ref().is_some_and(|(project, snapshot)| {
                    store
                        .read()
                        .is_ok_and(|document| document.revision() == snapshot.revision)
                        && project.verify_unchanged().is_ok()
                });
            if request.check_only
                && !request.refresh_pages
                && unchanged
                && document_preview_time == Some(request.view.time)
            {
                return Ok(None);
            }
            // Capturing a layout does not mean an image has been published. A
            // failed or cancelled render must be retried by dependency polls,
            // even when that captured layout is still current.
            document_preview_time = None;
            if !unchanged {
                cached = Some(crate::service::layout_snapshot_with_runtime(
                    store,
                    fonts,
                    None,
                    None,
                    &mut script,
                )?);
            }
            let (project, layout) = cached.as_ref().unwrap();
            let pixels = pixel_size(picker, request.cells)?;
            let mut view = request.view;
            if view.zoom.is_none() {
                view.fit(layout.bounds, pixels);
            }
            let options = crate::render::RenderOptions {
                region: Some(view.region(pixels)),
                scale: view.zoom.unwrap(),
                background: skia_safe::Color::from_rgb(248, 250, 252),
                refresh: request.refresh,
                refresh_pages: request.refresh_pages,
                time: view.time,
                ..Default::default()
            };
            timing.mark("snapshot_ms");
            let image = crate::render::png_with_runtime_observed(
                project,
                layout,
                &options,
                &mut dynamic,
                Some(cancel.clone()),
                Some(&mut |phase| timing.mark(phase)),
            )?;
            crate::store::require_revision(&store.read()?, &layout.revision)?;
            project.verify_unchanged()?;
            if cancel.is_cancelled() {
                return Ok(None);
            }
            timing.mark("verification_ms");
            let bitmap = image::load_from_memory_with_format(&image.png, image::ImageFormat::Png)
                .map_err(|error| Error::new("RENDER_ERROR", error.to_string()))?;
            timing.mark("decode_png_ms");
            let protocol = encode(picker, bitmap, request.cells, image_id)?;
            timing.mark("terminal_encode_ms");
            let nodes = layout.items.iter().map(|item| Node {
                id: item.record.id.clone(), name: item.node.properties.get("name").and_then(Value::as_str).unwrap_or(&item.record.id).into(), bounds: item.record.visual_bounds,
                target: json!({"id":item.record.id,"address":item.record.address,"source_id":item.record.source_id,"source_path":item.record.source_path,"generated_by":item.record.generated_by}),
            }).collect();
            let time_dependent = image.dynamic_artifacts.iter().any(|artifact| {
                artifact["acquisition"]["kind"] == "shader"
                    && artifact["acquisition"]["time_dependent"] == true
            });
            let live_pages = image
                .dynamic_artifacts
                .iter()
                .any(|artifact| artifact["acquisition"]["kind"] == "browser");
            let mut summary = layout.summary();
            summary["dynamic_artifacts"] = Value::Array(image.dynamic_artifacts);
            timing.mark("metadata_ms");
            Ok(Some(Preview {
                protocol,
                png: image.png,
                view,
                nodes,
                summary,
                attachment: false,
                time_dependent,
                live_pages,
            }))
        })();
        let cancelled = cancel.is_cancelled();
        *shared.active.lock().unwrap() = None;
        if cancelled {
            continue;
        }
        let result = match result {
            Ok(Some(preview)) => {
                if !preview.attachment {
                    document_preview_time = Some(preview.view.time);
                    if let Some(report) = timing.report(request.generation) {
                        eprintln!("LAZYPENCIL_PREVIEW_TIMING {report}");
                        profile_reports += 1;
                    }
                }
                Ok(preview)
            }
            Ok(None) => continue,
            Err(error) => Err(error),
        };
        *shared.output.lock().unwrap() = Some(Output {
            generation: request.generation,
            result,
        });
        shared.notify.notify_one();
    }
}

struct PreviewTiming {
    clocks: Option<(Instant, Instant)>,
    phases: serde_json::Map<String, Value>,
}
impl PreviewTiming {
    fn new(enabled: bool) -> Self {
        Self {
            clocks: enabled.then(|| {
                let now = Instant::now();
                (now, now)
            }),
            phases: serde_json::Map::new(),
        }
    }
    fn mark(&mut self, name: &str) {
        if let Some((_, previous)) = &mut self.clocks {
            let now = Instant::now();
            self.phases.insert(
                name.into(),
                json!(now.duration_since(*previous).as_secs_f64() * 1000.0),
            );
            *previous = now;
        }
    }
    fn report(&self, generation: u64) -> Option<Value> {
        self.clocks.map(|(started, _)| {
            json!({"format_version":2,"generation":generation,"phases_ms":self.phases,"worker_total_ms":started.elapsed().as_secs_f64()*1000.0})
        })
    }
}

pub fn pixel_size(picker: &Backend, cells: Size) -> Result<[u32; 2]> {
    let font = picker.font_size();
    let size = [
        u32::from(cells.width) * u32::from(font.width),
        u32::from(cells.height) * u32::from(font.height),
    ];
    if size.contains(&0)
        || size.iter().any(|n| *n > 8192)
        || u64::from(size[0]) * u64::from(size[1]) > 16 * 1024 * 1024
    {
        return Err(Error::new(
            "RENDER_ERROR",
            "Preview size must be within 8192 pixels per side and 16 megapixels; reduce the terminal window size",
        ));
    }
    Ok(size)
}
fn encode(picker: &Backend, image: image::DynamicImage, size: Size, id: u32) -> Result<Protocol> {
    picker.encode(image, size, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn preview_worker_profiling_is_opt_in_and_partitions_one_monotonic_interval() {
        let mut disabled = PreviewTiming::new(false);
        disabled.mark("snapshot_ms");
        assert!(disabled.report(1).is_none());
        assert!(disabled.phases.is_empty());
        let mut enabled = PreviewTiming::new(true);
        for phase in [
            "snapshot_ms",
            "render_raster_ms",
            "render_png_encode_ms",
            "metadata_ms",
        ] {
            enabled.mark(phase);
        }
        let report = enabled.report(7).unwrap();
        assert_eq!(report["generation"], 7);
        assert_eq!(report["format_version"], 2);
        let phases = report["phases_ms"].as_object().unwrap();
        assert_eq!(phases.len(), 4);
        let sum = phases
            .values()
            .map(|value| {
                let value = value.as_f64().unwrap();
                assert!(value.is_finite() && value >= 0.0);
                value
            })
            .sum::<f64>();
        assert!(report["worker_total_ms"].as_f64().unwrap() >= sum);
    }

    #[test]
    fn fractional_fit_pan_round_trips_preserve_the_exact_render_region() {
        for bounds in [
            Rect::new(0.0, 0.0, 1920.0, 1080.0),
            Rect::new(-1920.0, -1080.0, 1920.0, 1080.0),
        ] {
            let pixels = [900, 640];
            let mut view = View::default();
            view.fit(bounds, pixels);
            let original = view.region(pixels);
            for _ in 0..1000 {
                view.pan([64.0, 0.0]);
                view.pan([-64.0, 0.0]);
                view.pan([0.0, 64.0]);
                view.pan([0.0, -64.0]);
                assert_eq!(view.region(pixels), original);
            }
        }
    }

    #[test]
    fn automatic_frames_do_not_cancel_work_overwrite_pending_output_or_queue_a_backlog() {
        let shared = Arc::new(Shared {
            request: Mutex::new((None, false)),
            wake: Condvar::new(),
            active: Mutex::new(None),
            output: Mutex::new(None),
            notify: Notify::new(),
            generation: AtomicU64::new(7),
        });
        let worker = Worker {
            shared: shared.clone(),
            thread: None,
        };
        let request = Request {
            generation: 8,
            cells: Size::new(20, 8),
            view: View::default(),
            refresh: false,
            refresh_pages: false,
            check_only: false,
            attachment: None,
        };
        let cancel = CancellationToken::new();
        *shared.active.lock().unwrap() = Some(cancel.clone());
        assert!(!worker.request_if_idle(request.clone()));
        assert!(!cancel.is_cancelled());
        assert!(shared.request.lock().unwrap().0.is_none());
        assert_eq!(shared.generation.load(Ordering::Relaxed), 7);
        shared.active.lock().unwrap().take();
        *shared.output.lock().unwrap() = Some(Output {
            generation: 7,
            result: Err(Error::new("RENDER_ERROR", "Pending result")),
        });
        assert!(!worker.request_if_idle(request.clone()));
        assert_eq!(
            shared.output.lock().unwrap().as_ref().unwrap().generation,
            7
        );
        shared.output.lock().unwrap().take();
        shared.request.lock().unwrap().1 = true;
        assert!(!worker.request_if_idle(request.clone()));
        shared.request.lock().unwrap().1 = false;
        assert!(worker.request_if_idle(request.clone()));
        assert!(!worker.request_if_idle(request));
        assert_eq!(shared.generation.load(Ordering::Relaxed), 8);
        assert_eq!(
            shared
                .request
                .lock()
                .unwrap()
                .0
                .as_ref()
                .unwrap()
                .generation,
            8
        );
    }

    #[test]
    fn time_changes_republish_without_invalidating_layout_or_repainting_idle_polls() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("design.pen");
        let saved = json!({"version":"2.21","children":[{
            "id":"rectangle","type":"rectangle","width":180,"height":90,"fill":"#00FF00"
        }]})
        .to_string();
        std::fs::write(&path, &saved).unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let worker = Worker::start(store, vec![], Backend::halfblocks(), 133).unwrap();
            let mut request = Request {
                generation: 1,
                cells: Size::new(20, 8),
                view: View::default(),
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            };
            worker.request(request.clone());
            let initial = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(initial.view.time, 0.0);
            request.view = initial.view;
            request.view.set_time(1.25).unwrap();
            request.check_only = true;
            worker.request(request.clone());
            let changed = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(changed.view.time, 1.25);
            assert_eq!(changed.view.center, initial.view.center);
            assert_eq!(changed.view.zoom, initial.view.zoom);
            assert_eq!(
                changed.summary["snapshot_id"],
                initial.summary["snapshot_id"]
            );
            assert_eq!(changed.png, initial.png);
            worker.request(request);
            assert!(
                tokio::time::timeout(Duration::from_millis(200), worker.next())
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read_to_string(path).unwrap(), saved);
        });
    }

    #[test]
    fn a_periodic_policy_reload_updates_identity_even_when_pixels_are_identical() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("design.pen");
        let configuration = temp.path().join("lazypencil.toml");
        let policy = "[runtime.javascript]\nheap_mib=8\nidle_seconds=0\n";
        std::fs::write(&configuration, policy).unwrap();
        let saved = json!({"version":"2.21","children":[{
            "id":"rectangle","type":"rectangle","width":180,"height":90,"fill":"#00FF00"
        }]})
        .to_string();
        std::fs::write(&path, &saved).unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let worker = Worker::start(store, vec![], Backend::halfblocks(), 132).unwrap();
            let mut request = Request {
                generation: 1,
                cells: Size::new(60, 22),
                view: View::default(),
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            };
            worker.request(request.clone());
            let initial = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            request.view = initial.view;
            request.check_only = true;
            std::fs::write(&configuration, policy.replace("heap_mib=8", "heap_mib=12")).unwrap();
            worker.request(request.clone());
            let changed = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(initial.png, changed.png);
            assert_ne!(
                initial.summary["snapshot_id"],
                changed.summary["snapshot_id"]
            );
            assert_ne!(
                initial.summary["dependency_revision"],
                changed.summary["dependency_revision"]
            );
            assert_eq!(initial.view.center, changed.view.center);
            assert_eq!(initial.view.zoom, changed.view.zoom);
            worker.request(request);
            assert!(
                tokio::time::timeout(Duration::from_millis(200), worker.next())
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
        });
    }

    #[test]
    fn a_periodic_preview_reloads_the_default_family_in_a_live_font_worker() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("design.pen");
        let configuration = temp.path().join("lazypencil.toml");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts");
        let settings = format!(
            "[fonts]\nfiles={}\ndefault_family='LP Test Sans'\nlocale='en-US'\n",
            json!([root.join("faces.ttc"), root.join("cff.otf")])
        );
        std::fs::write(&configuration, &settings).unwrap();
        std::fs::write(
            &path,
            json!({"version":"2.21","children":[{
                "id":"label","type":"text","content":"A","fontSize":80,"fill":"#FF0000"
            }]})
            .to_string(),
        )
        .unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let worker = Worker::start(store, vec![], Backend::halfblocks(), 131).unwrap();
            let mut request = Request {
                generation: 1,
                cells: Size::new(60, 22),
                view: View::default(),
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            };
            worker.request(request.clone());
            let initial = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            request.view = initial.view;
            request.check_only = true;
            std::fs::write(
                &configuration,
                settings.replace("LP Test Sans", "LP Test CFF"),
            )
            .unwrap();
            worker.request(request);
            let changed = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_ne!(
                initial.png, changed.png,
                "The new CFF default must replace TrueType pixels"
            );
            assert_eq!(initial.nodes[0].id, changed.nodes[0].id);
            assert_eq!(initial.view.center, changed.view.center);
            assert_eq!(initial.view.zoom, changed.view.zoom);
            assert!(
                changed.summary["diagnostics"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        });
    }

    #[test]
    fn preview_tracks_negative_coordinates_latest_view_and_external_image_changes() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("design.pen");
        let asset = temp.path().join("image.png");
        let write_color = |rgba: [u8; 4]| {
            let image = crate::image_input::from_rgba8(1, 1, &rgba).unwrap();
            crate::store::atomic_replace(&asset, &image.png).unwrap();
        };
        write_color([255, 0, 0, 255]);
        std::fs::write(&path, json!({"version":"2.21","children":[{"id":"negative","name":"画像","type":"rectangle","x":-100,"y":-50,"width":80,"height":40,"fill":{"type":"image","url":"image.png","mode":"stretch"}}]}).to_string()).unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        let original_revision = store.read().unwrap().revision();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let worker = Worker::start(store.clone(), vec![], Backend::halfblocks(), 125).unwrap();
            let mut request = Request {
                generation: 1,
                cells: Size::new(20, 8),
                view: View::default(),
                refresh: false,
                refresh_pages: false,
                check_only: false,
                attachment: None,
            };
            worker.request(request.clone());
            let first = tokio::time::timeout(Duration::from_secs(10), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(first.view.center, [-60.0, -30.0]);
            let bitmap = image::load_from_memory(&first.png).unwrap().to_rgba8();
            assert_eq!(
                bitmap.get_pixel(bitmap.width() / 2, bitmap.height() / 2).0,
                [255, 0, 0, 255]
            );
            assert_eq!(first.nodes[0].target["address"]["node_id"], "negative");
            request.view = first.view;
            // Only the image changes; a root revision watcher would miss this update.
            write_color([0, 0, 255, 255]);
            request.check_only = true;
            worker.request(request.clone());
            let changed = tokio::time::timeout(Duration::from_secs(10), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(store.read().unwrap().revision(), original_revision);
            let bitmap = image::load_from_memory(&changed.png).unwrap().to_rgba8();
            assert_eq!(
                bitmap.get_pixel(bitmap.width() / 2, bitmap.height() / 2).0,
                [0, 0, 255, 255]
            );
            request.check_only = false;
            for generation in 2..=20 {
                request.generation = generation;
                request.view.center[0] = 500.0 + generation as f64;
                worker.request(request.clone());
            }
            loop {
                let output = tokio::time::timeout(Duration::from_secs(10), worker.next())
                    .await
                    .unwrap();
                if output.generation == 20 {
                    let bitmap = image::load_from_memory(&output.result.unwrap().png)
                        .unwrap()
                        .to_rgba8();
                    assert!(
                        !bitmap.pixels().any(|pixel| pixel.0 == [0, 0, 255, 255]),
                        "The final panned view must replace earlier requests"
                    );
                    break;
                }
            }
        });
    }
}
