//! Coalesced previews; Skia and dynamic runtimes stay on their owning thread.

use super::graphics::{Backend, Encoded};
use crate::{
    Error, Result, attachments::Attachment, geometry::Rect, project::Project, store::Store,
};
use ratatui::layout::Size;
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

mod cache;
pub(super) mod labels;
mod preparation;
mod revision;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
    fn fit_canvas(&mut self, bounds: Rect, pixels: [u32; 2], top_padding: u32) {
        // Reserve display space above the artwork for root names.
        self.fit(bounds, pixels);
        if (pixels[1] as f32 - bounds.height * self.zoom.unwrap()) / 2.0 >= top_padding as f32 {
            return;
        }
        let padding = top_padding.min(pixels[1].saturating_sub(1));
        self.fit(bounds, [pixels[0], pixels[1] - padding]);
        self.center[1] -= f64::from(padding) / 2.0 / f64::from(self.zoom.unwrap());
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
#[derive(Clone)]
pub struct Preview {
    pub protocol: Encoded,
    pub(crate) image: PreviewImage,
    pub view: View,
    pub nodes: Arc<[Node]>,
    pub summary: Value,
    pub attachment: bool,
    pub time_dependent: bool,
    pub live_pages: bool,
    pub frame_time: std::time::Duration,
}
#[derive(Clone)]
pub(crate) enum PreviewImage {
    Png(Arc<[u8]>),
    Pixels(crate::render::PreviewPixels),
}
impl PreviewImage {
    pub(super) fn identity(&self) -> usize {
        match self {
            Self::Png(png) => png.as_ptr() as usize,
            Self::Pixels(pixels) => pixels.identity(),
        }
    }
    pub(super) fn bitmap(&self) -> Result<image::RgbaImage> {
        match self {
            Self::Png(png) => image::load_from_memory(png)
                .map(|image| image.into_rgba8())
                .map_err(|error| Error::new("RENDER_ERROR", error.to_string())),
            Self::Pixels(pixels) => Ok(pixels.bitmap()),
        }
    }
    pub fn png(&self) -> Result<std::borrow::Cow<'_, [u8]>> {
        match self {
            Self::Png(png) => Ok(std::borrow::Cow::Borrowed(png)),
            Self::Pixels(pixels) => pixels.png().map(std::borrow::Cow::Owned),
        }
    }
}
pub struct Node {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub bounds: Rect,
    pub(super) hit: super::selection::HitGeometry,
    pub target: Value,
    pub kind: String,
    pub text: String,
    pub diagnostic_count: usize,
    pub has_error_diagnostic: bool,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}
pub struct Output {
    pub generation: u64,
    pub result: Result<Preview>,
}
struct Shared {
    canvas_background: Mutex<image::Rgba<u8>>,
    request: Mutex<(Option<Request>, bool)>,
    wake: Condvar,
    active: Mutex<Option<CancellationToken>>,
    output: Mutex<Option<Output>>,
    notify: Notify,
    generation: AtomicU64,
    presentation_request: Mutex<Option<super::selection::render::Request>>,
    presentation_output: Mutex<Option<super::selection::render::Finished>>,
    presentation_clock: Arc<AtomicU64>,
    presentation_notify: Notify,
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
            canvas_background: Mutex::new(picker.canvas_background()),
            request: Mutex::new((None, false)),
            wake: Condvar::new(),
            active: Mutex::new(None),
            output: Mutex::new(None),
            notify: Notify::new(),
            generation: AtomicU64::new(0),
            presentation_request: Mutex::new(None),
            presentation_output: Mutex::new(None),
            presentation_clock: Arc::default(),
            presentation_notify: Notify::new(),
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
    pub(super) fn set_canvas_background(&self, background: image::Rgba<u8>) {
        *self.shared.canvas_background.lock().unwrap() = background;
    }
    pub fn request(&self, request: Request) {
        if !request.check_only {
            self.shared
                .presentation_clock
                .fetch_add(1, Ordering::Relaxed);
        }
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
        self.shared
            .presentation_clock
            .fetch_add(1, Ordering::Relaxed);
        pending.0 = Some(request);
        self.shared.wake.notify_one();
        true
    }
    pub(super) fn present(&self, mut request: super::selection::render::Request) -> Arc<AtomicU64> {
        // Use the same queue lock as scene updates so the wake cannot be lost.
        let _state = self.shared.request.lock().unwrap();
        // Drag updates share an epoch: finish the active frame while coalescing
        // the next one, rather than starving the marquee until pointer release.
        request.serial = self.shared.presentation_clock.load(Ordering::Relaxed);
        request.clock = self.shared.presentation_clock.clone();
        *self.shared.presentation_request.lock().unwrap() = Some(request);
        self.shared.wake.notify_one();
        self.shared.presentation_clock.clone()
    }
    pub(super) async fn next_presentation(&self) -> super::selection::render::Finished {
        loop {
            let notified = self.shared.presentation_notify.notified();
            if let Some(output) = self.shared.presentation_output.lock().unwrap().take() {
                return output;
            }
            notified.await;
        }
    }
    pub async fn next(&self) -> Output {
        loop {
            self.shared.notify.notified().await;
            if let Some(output) = self.shared.output.lock().unwrap().take() {
                return output;
            }
        }
    }
    pub fn try_next(&self) -> Option<Output> {
        self.shared.output.lock().unwrap().take()
    }
    pub fn cancel(&self) {
        self.shared
            .presentation_clock
            .fetch_add(1, Ordering::Relaxed);
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
    let _thread_pool = crate::native_memory::AutoreleasePool::new();
    let mut cached: Option<(Project, crate::layout::LayoutSnapshot)> = None;
    let mut document_preview_time = None;
    let mut document_preview_background = None;
    let mut renderer = crate::render::PreviewRenderer::default();
    let mut prepared_camera = None;
    let mut first_camera_frame = true;
    let mut frames = cache::Frames::new(picker.font_size());
    let mut previous_view = None;
    let mut predicted_view = None;
    let mut speculation: Option<(View, crate::render::PreviewRenderer)> = None;
    let mut labels = labels::Labels::new(picker.font_size());
    let mut presentation_cache = None;
    let mut navigation: Option<Arc<[Node]>> = None;
    let mut revision = revision::RevisionGuard::default();
    // Owned measurements retain stderr separately from terminal output.
    // Never log document contents, file paths, or settings; cap completed reports.
    let profile_mode = std::env::var("LAZYPENCIL_PROFILE_PREVIEW");
    let profile = profile_mode.as_deref() == Ok("owned-local-measurement")
        || (std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && profile_mode.as_deref() == Ok("owned-gha-measurement"));
    let mut profile_reports = 0;
    let mut script =
        crate::script::ScriptRuntime::new(std::env::current_exe().expect("current executable"));
    let mut dynamic =
        crate::dynamic::DynamicRuntime::new(store.data_dir()).with_retained_pages(store.path());
    loop {
        let _pool = crate::native_memory::AutoreleasePool::new();
        let now = Instant::now();
        script.expire_idle(now);
        dynamic.expire_idle(now);
        let mut state = shared.request.lock().unwrap();
        let mut release_working_at = Some(now + std::time::Duration::from_millis(500));
        while state.0.is_none() && shared.presentation_request.lock().unwrap().is_none() && !state.1
        {
            if release_working_at.is_some_and(|deadline| Instant::now() >= deadline) {
                renderer.release_working_buffers();
                if let Some((_, predicted)) = &speculation {
                    predicted.release_working_buffers();
                }
                release_working_at = None;
            }
            let deadline = [
                script.idle_deadline(),
                dynamic.idle_deadline(),
                release_working_at,
            ]
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
            let presentation = shared.presentation_request.lock().unwrap().take();
            drop(state);
            if let Some(request) = presentation {
                let output =
                    super::selection::render::run(request, &mut presentation_cache, &mut labels);
                *shared.presentation_output.lock().unwrap() = Some(output);
                shared.presentation_notify.notify_one();
            }
            continue;
        };
        let cancel = CancellationToken::new();
        *shared.active.lock().unwrap() = Some(cancel.clone());
        drop(state);
        let canvas_background = *shared.canvas_background.lock().unwrap();
        let picker = &picker.clone().with_canvas_background(canvas_background);
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
                let protocol = encode(picker, image, request.cells, image_id)?;
                return Ok(Some(Preview {
                    protocol,
                    image: PreviewImage::Png(png.into()),
                    view: request.view,
                    nodes: Arc::default(),
                    summary: json!({"attachment":attachment}),
                    attachment: true,
                    time_dependent: false,
                    live_pages: false,
                    frame_time: std::time::Duration::ZERO,
                }));
            }
            let unchanged = !request.refresh
                && cached.as_ref().is_some_and(|(project, snapshot)| {
                    revision.verify(store, &snapshot.revision).is_ok()
                        && project.verify_unchanged().is_ok()
                });
            if request.check_only
                && !request.refresh_pages
                && unchanged
                && document_preview_time == Some(request.view.time)
                && document_preview_background == Some(canvas_background)
            {
                return Ok(None);
            }
            // Capturing a layout does not mean an image has been published. A
            // failed or cancelled render must be retried by dependency polls,
            // even when that captured layout is still current.
            document_preview_time = None;
            if !unchanged {
                cached = Some(crate::service::preview_layout_snapshot(
                    store,
                    fonts,
                    &mut script,
                )?);
                renderer.clear();
                frames.clear();
                previous_view = None;
                predicted_view = None;
                speculation = None;
                prepared_camera = None;
                navigation = None;
            }
            let (project, layout) = cached.as_ref().unwrap();
            let viewport = super::viewport::Viewport::new(picker.font_size(), request.cells)?;
            let pixels = viewport.display;
            let mut view = request.view;
            if view.zoom.is_none() {
                view.fit_canvas(layout.bounds, pixels, labels.top_padding());
            }
            if let Some((predicted, prepared)) = speculation.take()
                && cache::same_pixels(predicted, view, pixels)
            {
                renderer = prepared;
            }
            renderer.set_viewport(viewport.raster);
            if !request.refresh_pages
                && let Some(mut preview) = frames.get(request.cells, canvas_background, view)
            {
                revision.verify(store, &layout.revision)?;
                project.verify_unchanged()?;
                prepared_camera = Some((request.cells, canvas_background));
                preview.frame_time = std::time::Duration::ZERO;
                timing.mark("frame_cache_ms");
                return Ok(Some(preview));
            }
            let image::Rgba([r, g, b, a]) = picker.canvas_background();
            let options = crate::render::RenderOptions {
                region: Some(view.region(pixels)),
                scale: view.zoom.unwrap() * viewport.scale,
                background: skia_safe::Color::from_argb(a, r, g, b),
                refresh: request.refresh,
                refresh_pages: request.refresh_pages,
                time: view.time,
                ..Default::default()
            };
            timing.mark("snapshot_ms");
            if profile && profile_reports < 128 {
                let (pictures, bytes, scale_recorded, nearest_scale_ulps) =
                    renderer.retained_commands(options.scale);
                eprintln!(
                    "LAZYPENCIL_PREVIEW_CACHE {}",
                    json!({"generation":request.generation,"pictures":pictures,"bytes":bytes,"scale_recorded":scale_recorded,"nearest_scale_ulps":nearest_scale_ulps})
                );
            }
            let frame_started = Instant::now();
            let image = renderer.render(
                project,
                layout,
                &options,
                &mut dynamic,
                Some(cancel.clone()),
                Some(&mut |phase| timing.mark(phase)),
            )?;
            revision.verify(store, &layout.revision)?;
            project.verify_unchanged()?;
            if cancel.is_cancelled() {
                return Ok(None);
            }
            timing.mark("verification_ms");
            let camera_key = (request.cells, canvas_background);
            let static_camera = image.retain_raw_pixels
                && picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty
                && !picker.tmux_detected();
            let prepare_camera = static_camera && first_camera_frame;
            // Native retained scenes keep direct pixels for low-latency camera
            // updates. Other scenes retain a lossless PNG instead of a full
            // framebuffer, and release its raw storage before terminal encoding.
            let retained_image = if image.retain_raw_pixels {
                PreviewImage::Pixels(image.pixels)
            } else {
                let png = image.pixels.png()?;
                drop(image.pixels);
                PreviewImage::Png(png.into())
            };
            let nodes = navigation.get_or_insert_with(|| layout.items.iter().map(|item| Node {
                id: item.record.id.clone(), name: item.node.properties.get("name").and_then(Value::as_str).unwrap_or(&item.record.id).into(), display_name: super::nodes::node_label(item.node.properties.get("name").and_then(Value::as_str).unwrap_or(&item.record.id)), bounds: item.record.visual_bounds,
                hit: super::selection::HitGeometry {
                    enabled: item.record.enabled && item.node.properties.get("opacity").and_then(Value::as_f64).unwrap_or(1.0) > 0.0,
                    rect: Rect::new(0.0, 0.0, item.record.rect.width, item.record.rect.height),
                    transform: item.record.transform,
                    clip: item.node.properties.get("clip") == Some(&Value::Bool(true)),
                },
                target: json!({"id":item.record.id,"address":item.record.address,"source_id":item.record.source_id,"source_path":item.record.source_path,"generated_by":item.record.generated_by}),
                kind: item.node.kind().into(),
                text: item.node.properties.get("content").and_then(Value::as_str).unwrap_or("").into(),
                diagnostic_count: item.record.diagnostics.len(),
                has_error_diagnostic: item.record.diagnostics.iter().any(|diagnostic| diagnostic.severity == "error"),
                parent: item.parent, children: item.children.clone(),
            }).collect()).clone();
            let mut bitmap = image.bitmap.into_rgba8();
            labels.paint(&mut bitmap, &nodes, view, pixels, &[])?;
            let protocol = encode(
                picker,
                image::DynamicImage::ImageRgba8(bitmap),
                request.cells,
                image_id,
            )?;
            timing.mark("terminal_encode_ms");
            let time_dependent = image.dynamic_artifacts.iter().any(|artifact| {
                artifact["acquisition"]["kind"] == "shader"
                    && artifact["acquisition"]["time_dependent"] == true
            });
            let live_pages = image
                .dynamic_artifacts
                .iter()
                .any(|artifact| artifact["acquisition"]["kind"] == "browser");
            let mut summary = layout.summary();
            summary["provisional"] = json!(
                layout
                    .text_blocks
                    .values()
                    .any(|block| block.preview_warning.is_some())
            );
            summary["dynamic_artifacts"] = Value::Array(image.dynamic_artifacts);
            timing.mark("metadata_ms");
            if prepare_camera {
                // Prepare exact camera frames before publishing the initial view.
                // Bound startup work to the six immediate camera operations.
                // More distant views are prepared only for repeated gestures.
                let preparation = preparation::Frame {
                    project,
                    layout,
                    picker,
                    cells: request.cells,
                    pixels,
                    raster_scale: viewport.scale,
                    raster: viewport.raster,
                    image_id,
                    nodes: nodes.clone(),
                    summary: summary.clone(),
                    cancel: cancel.clone(),
                    interrupted: None,
                };
                let mut preparation_renderer = renderer.fork();
                for neighbor in initial_camera_views(view) {
                    if neighbor == view {
                        continue;
                    }
                    // A first pan in any direction starts from the actual
                    // initial raster, rather than another prepared pan's origin.
                    let mut pan_renderer = (neighbor.zoom == view.zoom).then(|| renderer.fork());
                    let neighbor_renderer =
                        pan_renderer.as_mut().unwrap_or(&mut preparation_renderer);
                    if let Ok(preview) =
                        preparation.render(neighbor_renderer, &mut dynamic, &mut labels, neighbor)
                    {
                        frames.insert(request.cells, canvas_background, &preview);
                    } else if cancel.is_cancelled() {
                        return Ok(None);
                    }
                    // A failed optional view must not hide the valid first frame.
                }
                renderer.retain_prepared_commands(&preparation_renderer);
                revision.verify(store, &layout.revision)?;
                project.verify_unchanged()?;
                if profile && profile_reports < 128 {
                    let (frames, bytes) = frames.retained();
                    eprintln!(
                        "LAZYPENCIL_PREVIEW_PREPARED {}",
                        json!({"frames":frames,"bytes":bytes})
                    );
                }
                timing.mark("camera_prepare_ms");
            }
            if static_camera {
                prepared_camera = Some(camera_key);
            }
            Ok(Some(Preview {
                protocol,
                image: retained_image,
                view,
                nodes,
                summary,
                attachment: false,
                time_dependent,
                live_pages,
                frame_time: frame_started.elapsed(),
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
                    first_camera_frame = false;
                    document_preview_time = Some(preview.view.time);
                    document_preview_background = Some(canvas_background);
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
        let retain = result
            .as_ref()
            .ok()
            .filter(|preview| {
                !preview.attachment
                    && !preview.time_dependent
                    && !preview.live_pages
                    && preview.protocol.native_bytes().is_some()
            })
            .cloned();
        *shared.output.lock().unwrap() = Some(Output {
            generation: request.generation,
            result,
        });
        shared.notify.notify_one();
        // Recent frames reuse owned pixels without another compression pass.
        if let Some(preview) = retain {
            frames.insert_recent(request.cells, canvas_background, &preview);
            if prepared_camera != Some((request.cells, canvas_background)) {
                continue;
            }
            let Ok(viewport) = super::viewport::Viewport::new(picker.font_size(), request.cells)
            else {
                continue;
            };
            let Some((project, layout)) = cached.as_ref() else {
                continue;
            };
            let interrupted = || preparation_pending(shared);
            let preparation = preparation::Frame {
                project,
                layout,
                picker,
                cells: request.cells,
                pixels: viewport.display,
                raster_scale: viewport.scale,
                raster: viewport.raster,
                image_id,
                nodes: preview.nodes.clone(),
                summary: preview.summary.clone(),
                cancel: cancel.clone(),
                interrupted: Some(&interrupted),
            };
            if preview.frame_time.is_zero()
                && previous_view.is_some_and(|previous: View| {
                    previous.zoom == preview.view.zoom
                        && !cache::same_pixels(previous, preview.view, viewport.display)
                })
            {
                // Keep pan rasters in actual display order. Prepared zooms need
                // no second rasterization: they already carry the exact pixels
                // and retained commands for their own native scale.
                let _ = preparation.hydrate(&mut renderer, &mut dynamic, preview.view);
            }
            let repeated = predicted_view.is_some_and(|predicted| {
                cache::same_pixels(predicted, preview.view, viewport.display)
            });
            let next = previous_view.and_then(|previous| {
                preparation::predict(previous, preview.view, viewport.display)
            });
            previous_view = Some(preview.view);
            predicted_view = next;
            if profile && profile_reports < 128 {
                eprintln!(
                    "LAZYPENCIL_PREVIEW_PREDICTION {}",
                    json!({"generation":request.generation,"repeated":repeated,"next":next.is_some(),"queued":preparation_pending(shared)})
                );
            }
            let Some(next) = next.filter(|_| repeated) else {
                continue;
            };
            if frames.get(request.cells, canvas_background, next).is_some()
                || preparation_pending(shared)
            {
                continue;
            }
            // A different request preempts speculation. An exact request for the
            // predicted pixels can consume the completed frame without redrawing it.
            let interrupted = || {
                prediction_superseded(
                    shared,
                    request.cells,
                    canvas_background,
                    next,
                    viewport.display,
                )
            };
            let prediction = preparation::Frame {
                interrupted: Some(&interrupted),
                ..preparation
            };
            let mut speculative_renderer = renderer.fork();
            let prediction_started = Instant::now();
            let prepared =
                prediction.render(&mut speculative_renderer, &mut dynamic, &mut labels, next);
            if profile && profile_reports < 128 {
                eprintln!(
                    "LAZYPENCIL_PREVIEW_PREDICTED {}",
                    json!({"generation":request.generation,"ready":prepared.is_ok(),"queued":preparation_pending(shared),"cancelled":cancel.is_cancelled(),"elapsed_ms":prediction_started.elapsed().as_secs_f64()*1000.0})
                );
            }
            if let Ok(preview) = prepared
                && !interrupted()
            {
                frames.insert_recent(request.cells, canvas_background, &preview);
            }
            speculation = Some((next, speculative_renderer));
        }
    }
}

fn preparation_pending(shared: &Shared) -> bool {
    shared.request.lock().unwrap().0.is_some()
        || shared.presentation_request.lock().unwrap().is_some()
}

fn prediction_superseded(
    shared: &Shared,
    cells: Size,
    background: image::Rgba<u8>,
    predicted: View,
    pixels: [u32; 2],
) -> bool {
    let pending = shared.request.lock().unwrap();
    let superseded = pending.1
        || pending.0.as_ref().is_some_and(|request| {
            request.cells != cells
                || request.refresh
                || request.refresh_pages
                || request.attachment.is_some()
                || !cache::same_pixels(request.view, predicted, pixels)
        });
    drop(pending);
    superseded
        || *shared.canvas_background.lock().unwrap() != background
        || shared.presentation_request.lock().unwrap().is_some()
}

fn initial_camera_views(view: View) -> [View; 6] {
    let mut neighbors = [view; 6];
    for (neighbor, delta) in
        neighbors
            .iter_mut()
            .zip([[64.0, 0.0], [-64.0, 0.0], [0.0, 64.0], [0.0, -64.0]])
    {
        neighbor.pan(delta);
    }
    let zoom = view.zoom.unwrap_or(1.0);
    neighbors[4].zoom = Some((zoom * 1.25).min(16.0));
    neighbors[5].zoom = Some((zoom / 1.25).max(0.01));
    neighbors
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
            json!({"format_version":3,"generation":generation,"phases_ms":self.phases,"worker_total_ms":started.elapsed().as_secs_f64()*1000.0})
        })
    }
}

pub fn pixel_size(picker: &Backend, cells: Size) -> Result<[u32; 2]> {
    super::viewport::Viewport::new(picker.font_size(), cells).map(|viewport| viewport.display)
}
fn encode(picker: &Backend, image: image::DynamicImage, size: Size, id: u32) -> Result<Encoded> {
    picker.encode_preview(image, size, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn native_worker_keeps_exact_viewport_sizes_for_prepared_frames_and_resize_revisits() {
        let directory = TempDir::new().unwrap();
        let document = directory.path().join("design.pen");
        std::fs::write(
            &document,
            json!({"version":"2.21","children":[{
                "type":"rectangle","id":"shape","width":7562,"height":17560,"fill":"#2563EB"
            }]})
            .to_string(),
        )
        .unwrap();
        let store = Store::open(&document, Some(directory.path().join("data"))).unwrap();
        let picker = Backend::native_for_test();
        let worker = Worker::start(store, vec![], picker.clone(), 175).unwrap();
        let view = View {
            center: [3781.0, 8780.0],
            zoom: Some(0.052460138),
            time: 0.0,
        };
        let mut zoomed = view;
        zoomed.zoom = Some(view.zoom.unwrap() * 1.25);
        let mut panned = view;
        panned.pan([64.0, 0.0]);
        let mut hidpi = view;
        hidpi.zoom = Some(view.zoom.unwrap() * 2.0);
        let mut hidpi_zoomed = hidpi;
        hidpi_zoomed.zoom = Some(hidpi.zoom.unwrap() * 1.01);
        let normal = Size::new(100, 49);
        let large = Size::new(200, 98);
        let requests = [
            (normal, view, false),
            (normal, zoomed, true),
            (normal, panned, true),
            (large, hidpi, false),
            (large, hidpi_zoomed, false),
            (normal, zoomed, true),
        ];
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                for (index, (cells, view, cached)) in requests.into_iter().enumerate() {
                    let generation = index as u64 + 1;
                    worker.request(Request {
                        generation,
                        cells,
                        view,
                        refresh: false,
                        refresh_pages: false,
                        check_only: false,
                        attachment: None,
                    });
                    let output = loop {
                        let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                            .await
                            .unwrap();
                        if output.generation == generation {
                            break output;
                        }
                    };
                    let preview = output.result.unwrap();
                    let viewport =
                        super::super::viewport::Viewport::new(picker.font_size(), cells).unwrap();
                    let bitmap = preview.image.bitmap().unwrap();
                    assert_eq!([bitmap.width(), bitmap.height()], viewport.raster);
                    assert_eq!(preview.protocol.size(), cells);
                    assert_eq!(preview.view, view);
                    if cached {
                        assert!(
                            preview.frame_time.is_zero(),
                            "Exercise prepared and revisited source frames: request {index}, cells {cells:?}"
                        );
                    }
                }
            });
        worker.cancel();
    }

    #[test]
    fn initial_camera_preparation_is_bounded_to_the_first_six_operations() {
        for zoom in [0.01, 0.06482758, 1.0, 16.0] {
            let initial = View {
                center: [1234.25, -3456.75],
                zoom: Some(zoom),
                time: 2.0,
            };
            let neighbors = initial_camera_views(initial);
            assert_eq!(
                neighbors.len(),
                6,
                "Startup must not prepare the zoom ladder"
            );
            for (neighbor, delta) in
                neighbors[..4]
                    .iter()
                    .zip([[64.0, 0.0], [-64.0, 0.0], [0.0, 64.0], [0.0, -64.0]])
            {
                let mut expected = initial;
                expected.pan(delta);
                assert_eq!(*neighbor, expected);
            }
            assert_eq!(neighbors[4].zoom, Some((zoom * 1.25).min(16.0)));
            assert_eq!(neighbors[5].zoom, Some((zoom / 1.25).max(0.01)));
            assert!(neighbors[4..].iter().all(|neighbor| {
                neighbor.center == initial.center && neighbor.time == initial.time
            }));
        }
    }

    #[test]
    fn revisited_native_frames_reuse_pixels_but_invalidate_changed_documents_backgrounds_and_sizes()
    {
        let directory = TempDir::new().unwrap();
        let store = Store::init(
            directory.path().join("design.pen"),
            Some(directory.path().join("data")),
        )
        .unwrap();
        let document = json!({"version":"2.21","children":[{"id":"shape","type":"rectangle","width":120,"height":80,"fill":"#FF0000"}]}).to_string();
        std::fs::write(store.path(), &document).unwrap();
        let worker = Worker::start(store.clone(), vec![], Backend::native_for_test(), 198).unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut request = Request {
                    generation: 1,
                    cells: Size::new(40, 16),
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
                let fitted = initial.view;
                request.view.pan([64.0, 0.0]);
                request.generation += 1;
                worker.request(request.clone());
                let moved = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                assert_ne!(moved.image.png().unwrap(), initial.image.png().unwrap());
                assert!(
                    matches!(&moved.image, PreviewImage::Png(_)),
                    "Initial preparation retains distant sources losslessly compressed"
                );
                request.view = fitted;
                request.view.center[0] = f64::from_bits(fitted.center[0].to_bits() + 1);
                request.generation += 1;
                worker.request(request.clone());
                let returned = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                assert_eq!(returned.image.png().unwrap(), initial.image.png().unwrap());
                assert!(
                    matches!(&returned.image, PreviewImage::Pixels(_)),
                    "Recent frames avoid another source compression pass"
                );
                assert_eq!(returned.frame_time, Duration::ZERO);
                assert_eq!(
                    returned.view, request.view,
                    "Reuse must preserve precise requested camera coordinates"
                );
                worker.set_canvas_background(image::Rgba([20, 30, 40, 255]));
                request.generation += 1;
                worker.request(request.clone());
                let dark = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                assert_ne!(dark.image.png().unwrap(), initial.image.png().unwrap());
                std::fs::write(store.path(), document.replace("#FF0000", "#00FF00")).unwrap();
                request.generation += 1;
                worker.request(request.clone());
                let edited = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                assert_ne!(edited.image.png().unwrap(), dark.image.png().unwrap());
                request.generation += 1;
                request.cells.width += 1;
                worker.request(request.clone());
                let resized = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                assert_eq!(resized.protocol.size(), Size::new(41, 16));
                let mut recent = cache::Frames::new(Backend::native_for_test().font_size());
                let cells = Size::new(40, 16);
                let background = Backend::native_for_test().canvas_background();
                let dark_background = image::Rgba([20, 30, 40, 255]);
                recent.insert_recent(cells, background, &initial);
                recent.insert_recent(cells, dark_background, &dark);
                recent.insert_recent(request.cells, dark_background, &resized);
                assert_eq!(recent.retained().0, 2);
                assert!(recent.get(cells, background, initial.view).is_none());
                assert_eq!(
                    recent
                        .get(cells, dark_background, dark.view)
                        .unwrap()
                        .image
                        .png()
                        .unwrap(),
                    dark.image.png().unwrap(),
                    "Evicting old raw sources must preserve recent exact pixels"
                );
                request.view = resized.view;
                for _ in 0..2 {
                    request.generation += 1;
                    request.view.pan([64.0, 0.0]);
                    worker.request(request.clone());
                    let actual = tokio::time::timeout(Duration::from_secs(15), worker.next())
                        .await
                        .unwrap()
                        .result
                        .unwrap();
                    assert_eq!(actual.view, request.view);
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(200), worker.next())
                        .await
                        .is_err(),
                    "Speculative frames must never become visible outputs"
                );
            });
    }

    #[test]
    fn root_labels_decorate_initial_and_selected_previews_without_changing_export_pixels() {
        use ratatui::{buffer::Buffer, widgets::Widget};
        let directory = TempDir::new().unwrap();
        let store = Store::init(
            directory.path().join("design.pen"),
            Some(directory.path().join("data")),
        )
        .unwrap();
        let document = json!({"version":"2.21","children":[{
            "id":"page","name":"旧案 / Page 1","type":"frame","layout":"none",
            "x":-120,"width":160,"height":80,"fill":"#FFFFFF"
        }]})
        .to_string();
        std::fs::write(store.path(), &document).unwrap();
        let picker = Backend::halfblocks();
        let cells = Size::new(40, 16);
        let worker = Worker::start(store.clone(), vec![], picker.clone(), 198).unwrap();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let view = View {
                    zoom: Some(1.0),
                    ..Default::default()
                };
                worker.request(Request {
                    generation: 1,
                    cells,
                    view,
                    refresh: false,
                    refresh_pages: false,
                    check_only: false,
                    attachment: None,
                });
                let preview = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                let png = preview.image.png().unwrap().into_owned();
                let bitmap = image::load_from_memory(&png).unwrap().into_rgba8();
                assert!(
                    bitmap
                        .enumerate_pixels()
                        .filter(|(_, y, _)| *y < 160)
                        .all(|(_, _, p)| p.0 == [248, 250, 252, 255])
                );
                let raw_protocol =
                    encode(&picker, image::DynamicImage::ImageRgba8(bitmap), cells, 198).unwrap();
                let area = ratatui::layout::Rect::new(0, 0, cells.width, cells.height);
                let render = |protocol: &Encoded| {
                    let Encoded::Protocol(protocol) = protocol else {
                        panic!("Expected halfblocks");
                    };
                    let mut buffer = Buffer::empty(area);
                    super::super::graphics::Image(protocol).render(area, &mut buffer);
                    buffer
                };
                let plain = render(&raw_protocol);
                let initial = render(&preview.protocol);
                assert_ne!(initial, plain, "Initial terminal frame includes root names");
                worker.present(super::super::selection::render::Request {
                    source: preview.image.clone(),
                    nodes: preview.nodes.clone(),
                    selected: vec!["page".into()],
                    before: view,
                    view,
                    size: cells,
                    viewport: pixel_size(&picker, cells).unwrap(),
                    outlines: vec![],
                    marquee: None,
                    picker: picker.clone(),
                    image_id: 198,
                    serial: 0,
                    clock: Arc::default(),
                });
                let selected =
                    tokio::time::timeout(Duration::from_secs(15), worker.next_presentation())
                        .await
                        .unwrap()
                        .result
                        .unwrap()
                        .unwrap();
                assert_ne!(render(&selected), initial, "Selected root names turn blue");
                assert_eq!(preview.image.png().unwrap().as_ref(), png);
            });
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
    }

    #[test]
    fn oversized_previews_keep_the_camera_and_allow_minimum_zoom() {
        let directory = TempDir::new().unwrap();
        let store = Store::init(
            directory.path().join("design.pen"),
            Some(directory.path().join("data")),
        )
        .unwrap();
        let document = json!({"version":"2.21","children":[{"type":"rectangle","id":"red","x":-100,"y":-100,"width":200,"height":200,"fill":"#FF0000"}]}).to_string();
        std::fs::write(store.path(), &document).unwrap();
        let picker = Backend::halfblocks();
        let worker = Worker::start(store.clone(), vec![], picker.clone(), 199).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let cells = Size::new(900, 4);
            let pixels = pixel_size(&picker, cells).unwrap();
            assert_eq!(pixels, [9000, 80]);
            for (generation, zoom) in [(1, Some(1.0)), (2, Some(0.01)), (3, None)] {
                let view = View {
                    center: [0.0, 0.0],
                    zoom,
                    time: 0.0,
                };
                worker.request(Request {
                    generation,
                    cells,
                    view,
                    refresh: false,
                    refresh_pages: false,
                    check_only: false,
                    attachment: None,
                });
                let output = tokio::time::timeout(Duration::from_secs(15), worker.next())
                    .await
                    .unwrap();
                let preview = output.result.unwrap();
                assert_eq!(preview.protocol.size(), cells);
                let mut expected = view;
                if zoom.is_none() {
                    expected.fit_canvas(
                        Rect::new(-100.0, -100.0, 200.0, 200.0),
                        pixels,
                        labels::Labels::new(picker.font_size()).top_padding(),
                    );
                }
                assert_eq!(preview.view, expected);
                assert_eq!(preview.view.region(pixels), expected.region(pixels));
                let png = preview.image.png().unwrap();
                let image = image::load_from_memory(png.as_ref()).unwrap();
                assert!(image.width() <= super::super::viewport::MAX_SIDE);
                assert!(
                    u64::from(image.width()) * u64::from(image.height())
                        <= super::super::viewport::MAX_PIXELS
                );
                assert_eq!(preview.nodes[0].id, "red");
            }
        });
        assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
    }

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
            "render_pixel_copy_ms",
            "metadata_ms",
        ] {
            enabled.mark(phase);
        }
        let report = enabled.report(7).unwrap();
        assert_eq!(report["generation"], 7);
        assert_eq!(report["format_version"], 3);
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
    fn predictions_accept_exact_queued_pixels_but_yield_to_different_work() {
        let background = Backend::halfblocks().canvas_background();
        let shared = Shared {
            canvas_background: Mutex::new(background),
            request: Mutex::new((None, false)),
            wake: Condvar::new(),
            active: Mutex::new(None),
            output: Mutex::new(None),
            notify: Notify::new(),
            generation: AtomicU64::new(1),
            presentation_request: Mutex::new(None),
            presentation_output: Mutex::new(None),
            presentation_clock: Arc::default(),
            presentation_notify: Notify::new(),
        };
        let request = Request {
            generation: 2,
            cells: Size::new(20, 8),
            view: View {
                zoom: Some(1.0),
                ..View::default()
            },
            refresh: false,
            refresh_pages: false,
            check_only: false,
            attachment: None,
        };
        let superseded =
            || prediction_superseded(&shared, request.cells, background, request.view, [200, 160]);
        assert!(!superseded());
        *shared.request.lock().unwrap() = (Some(request.clone()), false);
        assert!(!superseded());
        for change in 0..5 {
            let mut other = request.clone();
            match change {
                0 => other.cells.width += 1,
                1 => other.view.pan([64.0, 0.0]),
                2 => other.view.time += 1.0,
                3 => other.refresh = true,
                _ => other.refresh_pages = true,
            }
            shared.request.lock().unwrap().0 = Some(other);
            assert!(superseded(), "Changed work {change} must take priority");
        }
        shared.request.lock().unwrap().0 = Some(request.clone());
        *shared.canvas_background.lock().unwrap() = image::Rgba([1, 2, 3, 255]);
        assert!(superseded());
        *shared.canvas_background.lock().unwrap() = background;
        assert!(!superseded());
        shared.request.lock().unwrap().1 = true;
        assert!(superseded());
    }

    #[test]
    fn automatic_frames_do_not_cancel_work_overwrite_pending_output_or_queue_a_backlog() {
        let shared = Arc::new(Shared {
            canvas_background: Mutex::new(Backend::halfblocks().canvas_background()),
            request: Mutex::new((None, false)),
            wake: Condvar::new(),
            active: Mutex::new(None),
            output: Mutex::new(None),
            notify: Notify::new(),
            generation: AtomicU64::new(7),
            presentation_request: Mutex::new(None),
            presentation_output: Mutex::new(None),
            presentation_clock: Arc::default(),
            presentation_notify: Notify::new(),
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
            assert_eq!(changed.image.png().unwrap(), initial.image.png().unwrap());
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
    fn a_periodic_preview_rejects_changed_font_locks_and_recovers_without_losing_its_view() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        let path = temp.path().join("design.pen");
        std::fs::write(
            temp.path().join("lazypencil.toml"),
            "font_lock='fonts.lock.json'\n",
        )
        .unwrap();
        let saved = json!({"version":"2.21","children":[{
            "id":"label","type":"text","content":"固定フォント","fontFamily":"Noto Sans JP","fontSize":18,"width":180,"height":30
        }]}).to_string();
        std::fs::write(&path, &saved).unwrap();
        let store = Store::open(&path, Some(temp.path().join("data"))).unwrap();
        crate::font_lock::create(&store, &[], None, None).unwrap();
        let lock_path = temp.path().join("fonts.lock.json");
        let baseline = std::fs::read(&lock_path).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let worker = Worker::start(store, vec![], Backend::halfblocks(), 134).unwrap();
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
            let mut changed: Value = serde_json::from_slice(&baseline).unwrap();
            changed["engine"] = json!("changed-font-engine");
            std::fs::write(&lock_path, serde_json::to_vec(&changed).unwrap()).unwrap();
            worker.request(request.clone());
            let rejected = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap();
            assert_eq!(rejected.result.err().unwrap().code, "FONT_LOCK_MISMATCH");
            std::fs::write(&lock_path, &baseline).unwrap();
            worker.request(request.clone());
            let restored = tokio::time::timeout(Duration::from_secs(15), worker.next())
                .await
                .unwrap()
                .result
                .unwrap();
            assert_eq!(restored.image.png().unwrap(), initial.image.png().unwrap());
            assert_eq!(restored.view.center, initial.view.center);
            assert_eq!(restored.view.zoom, initial.view.zoom);
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
            assert_eq!(initial.image.png().unwrap(), changed.image.png().unwrap());
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
                initial.image.png().unwrap(),
                changed.image.png().unwrap(),
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
            let bitmap = image::load_from_memory(&first.image.png().unwrap())
                .unwrap()
                .to_rgba8();
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
            let bitmap = image::load_from_memory(&changed.image.png().unwrap())
                .unwrap()
                .to_rgba8();
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
                    let bitmap =
                        image::load_from_memory(&output.result.unwrap().image.png().unwrap())
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
