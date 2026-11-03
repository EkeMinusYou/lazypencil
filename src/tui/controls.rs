//! Apply bounded preview camera operations without editing the document or selection.
use super::{App, Focus, Worker, graphics, preview};
use crate::{
    Error, Result,
    geometry::Rect,
    preview_control::{ControlRequest, Incoming, PreviewTarget, SetPreviewRequest},
};
use serde_json::{Value, json};

pub(super) struct Pending {
    incoming: Incoming,
    request: SetPreviewRequest,
    generation: u64,
    fitted: Option<Value>,
}
impl App {
    pub(super) fn accept_controlled_preview(
        &mut self,
        output: preview::Output,
        worker: &Worker,
        picker: &graphics::Backend,
    ) -> bool {
        let current = output.generation == self.generation;
        let accepted = self.accept_preview(output);
        if accepted && let Err(error) = self.refresh_selection_presentation(picker, worker) {
            self.set_preview_error_status(&error);
        }
        // Both the normal event and the camera deadline can consume an exact
        // frame. Acknowledgements must follow either path exactly once.
        if current {
            self.preview_control_frame(worker, picker);
        }
        accepted
    }
    pub(super) fn receive_preview_control(
        &mut self,
        incoming: Incoming,
        worker: &Worker,
        picker: &graphics::Backend,
    ) {
        if incoming.document != self.conversation.store().path()
            || incoming.conversation != self.conversation.id()
        {
            let _ = incoming.reply.send(Err(Error::new(
                "PREVIEW_UNAVAILABLE",
                "The preview belongs to another conversation or document",
            )));
            return;
        }
        if matches!(incoming.request, ControlRequest::GetState) {
            let state = self.preview_state(picker);
            let _ = incoming.reply.send(Ok(state));
            return;
        }
        if !self.conversation.active() {
            let _ = incoming.reply.send(Err(Error::new(
                "CANCELLED",
                "The requesting conversation is no longer active",
            )));
            return;
        }
        self.queue_preview(incoming, worker);
    }
    pub(super) fn preview_state(&self, picker: &graphics::Backend) -> Value {
        let pixels = (self.cells.width > 0 && self.cells.height > 0)
            .then(|| preview::pixel_size(picker, self.cells).ok())
            .flatten();
        let rendered = self.preview.as_ref().filter(|preview| !preview.attachment);
        let displayed = rendered.map(|preview| json!({
            "view": view_state(preview.view, pixels), "revision": preview.summary["revision"],
            "dependency_revision": preview.summary["dependency_revision"], "preview_snapshot_id": preview.summary["snapshot_id"],
            "document_bounds": preview.summary["bounds"],
        }));
        json!({
            "document": self.conversation.store().path(), "conversation": self.conversation.id(),
            "selected_nodes": self.selection.ids,
            "selected_node": self.selected_node().map(|node| json!({"node_id": node.id, "name": node.name, "bounds": node.bounds})),
            "preview": { "requested_view": view_state(self.view, pixels), "displayed": displayed,
                "viewport_pixels": pixels, "viewport_cells": [self.cells.width, self.cells.height],
                "showing_attachment": self.preview.as_ref().is_some_and(|preview| preview.attachment),
                "stale": self.preview_stale,
                "updating": self.preview_error.is_none() && (self.preview_generation != Some(self.generation) || self.pending_preview.is_some()),
                "error": self.preview_error.as_ref().map(|error| json!({"code":error.code,"message":error.message})), "playing": self.playback.active(),
            }
        })
    }
    fn queue_preview(&mut self, incoming: Incoming, worker: &Worker) {
        let ControlRequest::Set { ref view } = incoming.request else {
            return;
        };
        if let Err(error) = view.validate() {
            let _ = incoming.reply.send(Err(error));
            return;
        }
        if self.pending_preview.is_some()
            || self.has_modal()
            || self.focus == Focus::Attachments
            || self.cells.width == 0
            || self.cells.height == 0
        {
            let _ = incoming.reply.send(Err(Error::new(
                "BUSY",
                "The preview is unavailable while another view or dialog is active",
            )));
            return;
        }
        if incoming.reply.is_closed() {
            return;
        }
        let request = view.clone();
        self.playback.stop();
        // Refresh before resolving IDs so newly edited/created nodes are usable
        // without waiting for the ordinary dependency poll.
        self.request(worker, false, true);
        self.pending_preview = Some(Pending {
            incoming,
            request,
            generation: self.generation,
            fitted: None,
        });
    }
    pub(super) fn expire_preview_control(&mut self) {
        let Some(pending) = &self.pending_preview else {
            return;
        };
        let error = if pending.incoming.reply.is_closed() {
            Some(Error::new("CANCELLED", "The preview request was cancelled"))
        } else if std::time::Instant::now() >= pending.incoming.deadline {
            Some(Error::new(
                "PREVIEW_UNAVAILABLE",
                "Preview rendering timed out",
            ))
        } else if pending.generation != self.generation {
            Some(Error::new(
                "CANCELLED",
                "A newer view superseded the preview request",
            ))
        } else if pending.incoming.conversation != self.conversation.id() {
            Some(Error::new("CANCELLED", "The conversation changed"))
        } else if self.has_modal() || self.focus == Focus::Attachments {
            Some(Error::new(
                "BUSY",
                "The active view changed while updating the preview",
            ))
        } else {
            None
        };
        if let Some(error) = error {
            let _ = self
                .pending_preview
                .take()
                .unwrap()
                .incoming
                .reply
                .send(Err(error));
        }
    }
    pub(super) fn preview_control_frame(&mut self, worker: &Worker, picker: &graphics::Backend) {
        self.expire_preview_control();
        let Some(mut pending) = self.pending_preview.take() else {
            return;
        };
        let result = self.target_identity(&pending.request);
        match result {
            Err(error) => {
                let _ = pending.incoming.reply.send(Err(error));
            }
            Ok((bounds, identity)) => {
                if let Some(fitted) = &pending.fitted {
                    if fitted != &identity {
                        let _ = pending.incoming.reply.send(Err(Error::new(
                            "REVISION_CONFLICT",
                            "The target changed while rendering; inspect it and retry",
                        )));
                    } else {
                        let mut value = identity;
                        value["status"] = json!("updated");
                        value["center"] = json!(self.view.center);
                        value["zoom"] = json!(self.view.zoom);
                        value["view"] =
                            view_state(self.view, preview::pixel_size(picker, self.cells).ok());
                        let _ = pending.incoming.reply.send(Ok(value));
                    }
                    return;
                }
                let view = preview::pixel_size(picker, self.cells)
                    .and_then(|pixels| adjusted_view(self.view, &pending.request, bounds, pixels));
                match view {
                    Err(error) => {
                        let _ = pending.incoming.reply.send(Err(error));
                    }
                    Ok(view) => {
                        self.view = view;
                        self.request(worker, false, false);
                        pending.generation = self.generation;
                        pending.fitted = Some(identity);
                        self.pending_preview = Some(pending);
                        self.dirty = true;
                    }
                }
            }
        }
    }
    fn target_identity(&self, request: &SetPreviewRequest) -> Result<(Option<Rect>, Value)> {
        if let Some(error) = &self.preview_error {
            return Err(error.clone());
        }
        if self.preview_stale {
            return Err(Error::new(
                "PREVIEW_UNAVAILABLE",
                "The latest preview is unavailable; the previous preview is still displayed",
            ));
        }
        let preview = self
            .preview
            .as_ref()
            .filter(|preview| !preview.attachment)
            .ok_or_else(|| {
                Error::new(
                    "PREVIEW_UNAVAILABLE",
                    "No complete document preview is available",
                )
            })?;
        for (expected, key) in [
            (&request.expected_revision, "revision"),
            (&request.expected_dependency_revision, "dependency_revision"),
        ] {
            if let Some(expected) = expected
                && preview.summary[key].as_str() != Some(expected)
            {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "The preview target revision changed; inspect it and retry",
                ));
            }
        }
        let bounds = match &request.target {
            Some(PreviewTarget::Document) => Some(
                serde_json::from_value(preview.summary["bounds"].clone()).map_err(|_| {
                    Error::new("PREVIEW_UNAVAILABLE", "Document bounds are unavailable")
                })?,
            ),
            Some(PreviewTarget::Region { region }) => Some(*region),
            Some(PreviewTarget::Nodes { node_ids }) => {
                let mut bounds: Option<Rect> = None;
                for id in node_ids {
                    let node = preview
                        .nodes
                        .iter()
                        .find(|node| &node.id == id)
                        .ok_or_else(|| {
                            Error::new(
                                "INVALID_ARGUMENT",
                                "A requested node does not exist in the current preview",
                            )
                        })?;
                    require_bounds(node.bounds, false)?;
                    bounds = Some(bounds.map_or(node.bounds, |bounds| bounds.union(node.bounds)));
                }
                bounds
            }
            _ => None,
        };
        if let Some(bounds) = bounds {
            require_bounds(
                bounds,
                matches!(request.target, Some(PreviewTarget::Document)),
            )?;
        }
        let node_ids = match &request.target {
            Some(PreviewTarget::Nodes { node_ids }) => node_ids.clone(),
            _ => vec![],
        };
        Ok((
            bounds,
            json!({"bounds": bounds, "node_ids": node_ids,
            "revision": preview.summary["revision"], "dependency_revision": preview.summary["dependency_revision"]}),
        ))
    }
}
fn require_bounds(bounds: Rect, allow_empty: bool) -> Result<()> {
    if ![
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        bounds.right(),
        bounds.bottom(),
    ]
    .into_iter()
    .all(f32::is_finite)
        || bounds.width < 0.0
        || bounds.height < 0.0
        || (!allow_empty && (bounds.width == 0.0 || bounds.height == 0.0))
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "The target has no finite visible bounds",
        ));
    }
    Ok(())
}
fn adjusted_view(
    mut view: preview::View,
    request: &SetPreviewRequest,
    bounds: Option<Rect>,
    pixels: [u32; 2],
) -> Result<preview::View> {
    if let Some(bounds) = bounds {
        if request.fit && request.zoom.is_none() {
            view.fit(bounds, pixels);
        } else {
            view.center = [
                f64::from(bounds.x) + f64::from(bounds.width) / 2.0,
                f64::from(bounds.y) + f64::from(bounds.height) / 2.0,
            ];
        }
    }
    if let Some(PreviewTarget::Point { center }) = &request.target {
        view.center = *center;
    }
    if let Some(zoom) = request.zoom {
        view.zoom = Some(zoom);
    }
    if let Some(factor) = request.zoom_factor {
        view.zoom =
            Some((view.zoom.unwrap_or(1.0) as f64 * f64::from(factor)).clamp(0.01, 16.0) as f32);
    }
    if let Some(pan) = request.pan {
        view.center[0] += pan[0];
        view.center[1] += pan[1];
    }
    let region = view.region(pixels);
    if !view
        .center
        .into_iter()
        .all(|value| (value as f32).is_finite())
        || ![region.x, region.y, region.right(), region.bottom()]
            .into_iter()
            .all(f32::is_finite)
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "The requested view exceeds the finite coordinate range",
        ));
    }
    Ok(view)
}
fn view_state(view: preview::View, pixels: Option<[u32; 2]>) -> Value {
    json!({"center": view.center, "zoom": view.zoom, "time": view.time,
        "region": pixels.filter(|_| view.zoom.is_some()).map(|pixels| view.region(pixels))})
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use ratatui::layout::Size;
    use tempfile::TempDir;
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn app(temp: &TempDir) -> App {
        let arguments = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "tui".into(),
            temp.path().join("design.pen").into_os_string(),
            "--create".into(),
            "--adapter=offline-preview-test".into(),
        ])
        .unwrap();
        let crate::cli::Command::Tui(args) = arguments.command else {
            unreachable!()
        };
        App::new(
            crate::conversation::Conversation::open(args.prompt, Some(temp.path().join("data")))
                .unwrap(),
        )
    }
    fn incoming(app: &App, id: &str) -> (Incoming, tokio::sync::oneshot::Receiver<Result<Value>>) {
        let (reply, result) = tokio::sync::oneshot::channel();
        (
            Incoming {
                document: app.conversation.store().path().into(),
                conversation: app.conversation.id().into(),
                request: ControlRequest::Set {
                    view: serde_json::from_value(
                        json!({"target":{"kind":"nodes","node_ids":[id]}}),
                    )
                    .unwrap(),
                },
                reply,
                deadline: std::time::Instant::now() + crate::preview_control::TIMEOUT,
            },
            result,
        )
    }
    async fn frame(app: &mut App, worker: &Worker, picker: &graphics::Backend) {
        let output = tokio::time::timeout(std::time::Duration::from_secs(15), worker.next())
            .await
            .unwrap();
        assert_eq!(output.generation, app.generation);
        app.accept_controlled_preview(output, worker, picker);
    }
    #[test]
    fn preview_renders_new_targets_and_preserves_input_selection_and_document() {
        runtime().block_on(async {
            let temp = TempDir::new().unwrap();
            let mut app = app(&temp);
            let store = app.conversation.store().clone();
            let picker = graphics::Backend::halfblocks();
            let worker = Worker::start(store.clone(), vec![], picker.clone(), 110).unwrap();
            app.cells = Size::new(50, 20);
            app.insert_text("入力中の日本語");

            // This node was created after the previously displayed layout.
            let document = json!({"version":"2.21","children":[
                {"type":"rectangle","id":"first","x":0,"y":0,"width":100,"height":100,"fill":"#f00"},
                {"type":"rectangle","id":"button","x":1200,"y":-400,"width":160,"height":80,"fill":"#0f0"}
            ]}).to_string();
            let mut initial: Value = serde_json::from_str(&document).unwrap();
            initial["children"].as_array_mut().unwrap().pop();
            std::fs::write(store.path(), initial.to_string()).unwrap();
            app.request(&worker, false, false);
            frame(&mut app, &worker, &picker).await;
            assert!(app.preview.as_ref().unwrap().nodes.iter().all(|node| node.id != "button"));
            app.filter = "first".into();
            app.refresh_nodes(Some("first"));
            assert_eq!(app.selected_node().unwrap().id, "first");
            std::fs::write(store.path(), &document).unwrap();
            let (request, mut result) = incoming(&app, "button");
            app.queue_preview(request, &worker);
            frame(&mut app, &worker, &picker).await;
            assert!(matches!(result.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
            assert_eq!(app.view.center, [1280.0, -360.0]);
            frame(&mut app, &worker, &picker).await;
            let focused = result.await.unwrap().unwrap();
            assert_eq!(focused["status"], "updated");
            assert_eq!(focused["node_ids"], json!(["button"]));
            assert_eq!(app.focus, Focus::Prompt);
            assert_eq!(app.editor.lines().join("\n"), "入力中の日本語");
            assert_eq!(app.selected_node().unwrap().id, "first");
            assert_eq!(app.filter, "first");
            assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
            assert!(app.pending_preview.is_none());
            let mut edited: Value = serde_json::from_str(&document).unwrap();
            edited["children"][1]["x"] = json!(2000);
            edited["children"][1]["width"] = json!(80);
            let edited = edited.to_string();
            std::fs::write(store.path(), &edited).unwrap();
            let (request, result) = incoming(&app, "button");
            app.queue_preview(request, &worker);
            frame(&mut app, &worker, &picker).await;
            frame(&mut app, &worker, &picker).await;
            assert_eq!(result.await.unwrap().unwrap()["center"], json!([2040.0, -360.0]));
            assert_eq!(app.focus, Focus::Prompt);
            assert_eq!(std::fs::read_to_string(store.path()).unwrap(), edited);
        });
    }
    #[test]
    fn previous_preview_is_reported_as_stale_without_acknowledging_a_failed_update() {
        runtime().block_on(async {
            let temp = TempDir::new().unwrap();
            let mut app = app(&temp);
            let store = app.conversation.store().clone();
            let document = json!({"version":"2.21","children":[{"type":"rectangle","id":"button","width":100,"height":80,"fill":"#0f0"}]}).to_string();
            std::fs::write(store.path(), &document).unwrap();
            let picker = graphics::Backend::halfblocks();
            let worker = Worker::start(store.clone(), vec![], picker.clone(), 112).unwrap();
            app.cells = Size::new(50, 20);
            app.request(&worker, false, false);
            frame(&mut app, &worker, &picker).await;
            let revision = app.preview_state(&picker)["preview"]["displayed"]["revision"].clone();
            std::fs::write(store.path(), document.replace("2.21", "2.8")).unwrap();
            let (request, result) = incoming(&app, "button");
            app.queue_preview(request, &worker);
            frame(&mut app, &worker, &picker).await;
            assert_eq!(result.await.unwrap().unwrap_err().code, "UNSUPPORTED_VERSION");
            let state = app.preview_state(&picker);
            assert_eq!(state["preview"]["stale"], true);
            assert_eq!(state["preview"]["updating"], false);
            assert_eq!(state["preview"]["displayed"]["revision"], revision);
            assert!(state["preview"]["displayed"].is_object());

            app.request(&worker, false, true);
            assert!(app.preview_error.is_none());
            assert_eq!(app.target_identity(&serde_json::from_value(json!({})).unwrap()).unwrap_err().code, "PREVIEW_UNAVAILABLE");
            frame(&mut app, &worker, &picker).await;
            std::fs::write(store.path(), &document).unwrap();
            let updated = set(&mut app, &worker, &picker, json!({"target":{"kind":"nodes","node_ids":["button"]}})).await;
            assert_eq!(updated["status"], "updated");
            assert_eq!(app.preview_state(&picker)["preview"]["stale"], false);
        });
    }

    #[test]
    fn preview_rejects_missing_stale_and_superseded_targets() {
        runtime().block_on(async {
            let temp = TempDir::new().unwrap();
            let mut app = app(&temp);
            let store = app.conversation.store().clone();
            std::fs::write(store.path(), json!({"version":"2.21","children":[{"type":"rectangle","id":"button","width":100,"height":80,"fill":"#0f0"}]}).to_string()).unwrap();
            let picker = graphics::Backend::halfblocks();
            let worker = Worker::start(store.clone(), vec![], picker.clone(), 111).unwrap();
            app.cells = Size::new(50, 20);
            for (id, stale, expected) in [("missing", false, "INVALID_ARGUMENT"), ("button", true, "REVISION_CONFLICT")] {
                let (mut request, result) = incoming(&app, id);
                if stale { let ControlRequest::Set { view } = &mut request.request else { unreachable!() }; view.expected_revision = Some("stale".into()); }
                app.queue_preview(request, &worker);
                frame(&mut app, &worker, &picker).await;
                assert_eq!(result.await.unwrap().unwrap_err().code, expected);
            }
            let (request, result) = incoming(&app, "button");
            app.queue_preview(request, &worker);
            app.request(&worker, false, false);
            app.expire_preview_control();
            assert_eq!(result.await.unwrap().unwrap_err().code, "CANCELLED");
            let (request, result) = incoming(&app, "button");
            app.focus = Focus::Attachments;
            app.queue_preview(request, &worker);
            assert_eq!(result.await.unwrap().unwrap_err().code, "BUSY");
            let (mut request, result) = incoming(&app, "button");
            request.conversation = "other-session".into();
            app.receive_preview_control(request, &worker, &picker);
            assert_eq!(result.await.unwrap().unwrap_err().code, "PREVIEW_UNAVAILABLE");
        });
    }

    fn set_request(
        app: &App,
        value: Value,
    ) -> (Incoming, tokio::sync::oneshot::Receiver<Result<Value>>) {
        let (mut incoming, result) = incoming(app, "unused");
        incoming.request = ControlRequest::Set {
            view: serde_json::from_value(value).unwrap(),
        };
        (incoming, result)
    }
    async fn set(
        app: &mut App,
        worker: &Worker,
        picker: &graphics::Backend,
        value: Value,
    ) -> Value {
        let (incoming, result) = set_request(app, value);
        app.queue_preview(incoming, worker);
        assert_eq!(app.preview_state(picker)["preview"]["updating"], true);
        frame(app, worker, picker).await;
        frame(app, worker, picker).await;
        let result = result.await.unwrap().unwrap();
        assert_eq!(app.preview_state(picker)["preview"]["updating"], false);
        result
    }
    #[test]
    fn generic_preview_combines_targets_pan_zoom_and_state_without_editing() {
        runtime().block_on(async {
            let temp = TempDir::new().unwrap();
            let mut app = app(&temp);
            let store = app.conversation.store().clone();
            let document = json!({"version":"2.21","children":[
                {"type":"rectangle","id":"left","name":"Left button","x":-200,"y":100,"width":100,"height":80,"fill":"#f00"},
                {"type":"rectangle","id":"right","name":"Right button","x":800,"y":300,"width":200,"height":100,"fill":"#0f0"},
                {"type":"rectangle","id":"far","x":3000,"y":1000,"width":50,"height":50,"fill":"#00f"}
            ]}).to_string();
            std::fs::write(store.path(), &document).unwrap();
            let picker = graphics::Backend::halfblocks();
            let worker = Worker::start(store.clone(), vec![], picker.clone(), 112).unwrap();
            app.cells = Size::new(50, 20);
            app.insert_text("この倍率を保って");

            let result = set(&mut app, &worker, &picker, json!({"target":{"kind":"nodes","node_ids":["left","right"]}})).await;
            app.nodes.select(Some(0));
            assert_eq!(app.selected_node().unwrap().id, "left");
            assert_eq!(result["center"], json!([400.0, 250.0]));
            assert_eq!(result["bounds"], json!({"x":-200.0,"y":100.0,"width":1200.0,"height":300.0}));
            let original_zoom = app.view.zoom.unwrap();
            set(&mut app, &worker, &picker, json!({"target":{"kind":"nodes","node_ids":["right"]},"fit":false})).await;
            assert_eq!(app.view.center, [900.0, 350.0]);
            assert_eq!(app.view.zoom.unwrap(), original_zoom);
            set(&mut app, &worker, &picker, json!({"pan":[120.0,-40.0],"zoom_factor":0.5})).await;
            assert_eq!(app.view.center, [1020.0, 310.0]);
            assert_eq!(app.view.zoom.unwrap(), original_zoom * 0.5);
            set(&mut app, &worker, &picker, json!({"target":{"kind":"point","center":[-80.0,-50.0]},"zoom":2.0})).await;
            assert_eq!(app.view.center, [-80.0, -50.0]);
            assert_eq!(app.view.zoom, Some(2.0));
            set(&mut app, &worker, &picker, json!({"target":{"kind":"region","region":{"x":10.0,"y":20.0,"width":80.0,"height":40.0}}})).await;
            assert_eq!(app.view.center, [50.0, 40.0]);
            set(&mut app, &worker, &picker, json!({"target":{"kind":"document"}})).await;
            assert_eq!(app.view.center, [1425.0, 575.0]);

            let state = app.preview_state(&picker);
            assert!(!state.to_string().contains("この倍率を保って"));
            assert!(state.get("focused_panel").is_none());
            assert!(state.get("dialog_open").is_none());
            assert!(state.get("agent_active").is_none());
            assert_eq!(state["selected_node"]["node_id"], "left");
            assert_eq!(state["preview"]["requested_view"], state["preview"]["displayed"]["view"]);
            assert_eq!(state["preview"]["displayed"]["revision"], store.read().unwrap().revision());
            assert_eq!(state["preview"]["displayed"]["preview_snapshot_id"], app.preview.as_ref().unwrap().summary["snapshot_id"]);
            assert!(state["preview"]["displayed"].get("snapshot_id").is_none());
            assert_eq!(state["preview"]["viewport_cells"], json!([50,20]));
            assert_eq!(app.editor.lines().join("\n"), "この倍率を保って");
            assert_eq!(std::fs::read_to_string(store.path()).unwrap(), document);
        });
    }
    #[test]
    fn preview_state_is_available_before_rendering_and_does_not_move_the_camera() {
        runtime().block_on(async {
            let temp = TempDir::new().unwrap();
            let mut app = app(&temp);
            let picker = graphics::Backend::halfblocks();
            let worker = Worker::start(
                app.conversation.store().clone(),
                vec![],
                picker.clone(),
                113,
            )
            .unwrap();
            let (mut incoming, result) = incoming(&app, "unused");
            incoming.request = ControlRequest::GetState;
            app.receive_preview_control(incoming, &worker, &picker);
            let state = result.await.unwrap().unwrap();
            assert!(state["preview"]["displayed"].is_null());
            assert!(state["preview"]["viewport_pixels"].is_null());
            assert!(state["selected_node"].is_null());
            assert!(state.get("agent_active").is_none());
            assert_eq!(app.generation, 0);
            assert_eq!(app.view.center, [0.0, 0.0]);
            assert!(app.view.zoom.is_none());
        });
    }
}
