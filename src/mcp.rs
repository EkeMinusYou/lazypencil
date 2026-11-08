use crate::{Error, Result, operations::EditRequest, project::Project, service, store::Store};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
#[cfg(feature = "rendering")]
use std::collections::VecDeque;
use std::{
    borrow::Cow,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

mod catalog;
pub(crate) mod timing;
mod workspace;
use catalog::{ToolKind, tool, tools_for};

const MAX_TOOL_BYTES: usize = 4 * 1024 * 1024;
#[cfg(feature = "rendering")]
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);
const INSTRUCTIONS: &str = "This server edits multiple .pen documents. Start with list_documents, open_document or create_document. Pass the returned document_id explicitly to every document-specific tool, even when only one document is open. IDs belong to this connection; reopen paths after reconnecting. Absolute paths are preferred; relative paths resolve against the server working directory shown below. Catalogue tools can run without a document_id for built-in resources only. Then use get_document and get_guidance, inspect existing nodes, components and variables, then apply small atomic batches using current document and dependency revisions. Choose a suitable available style for a new design; preserve existing conventions for edits. Read configured project_guides after the user has explicitly granted local trust. Agents must not grant project trust themselves. Follow the user request, trusted project rules, selected style, then standard guidance. Use get_layout and render to inspect the result and correct layout or visual problems. Render returns real PNG image content. Do not claim a visual check unless you received and inspected the image. Preserve the user's language. English guides do not require English design content. Use only advertised tools and supported capabilities.";

#[derive(Clone)]
struct Server {
    sender: mpsc::Sender<Job>,
    preview_control: bool,
    usage: Option<crate::guidance::usage::UsageAccess>,
}
struct Job {
    name: String,
    arguments: Value,
    reply: oneshot::Sender<CallToolResult>,
    cancel: tokio_util::sync::CancellationToken,
    timing: timing::Timer,
    usage: Result<Option<crate::guidance::usage::UsageRequest>>,
}

#[derive(Default)]
pub struct Options {
    pub data_dir: Option<PathBuf>,
    pub fonts: Vec<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub attachments_manifest: Option<PathBuf>,
    pub guidance_audit_manifest: Option<PathBuf>,
    pub preview_control: Option<String>,
}

pub fn serve(options: Options) -> Result<()> {
    let prepared = workspace::Workspace::new(options)?;
    let has_preview = prepared.has_preview();
    let server_usage = prepared.usage();
    let options = prepared.into_options();
    let (sender, mut receiver) = mpsc::channel::<Job>(16);
    let worker = std::thread::Builder::new()
        .name("lazypencil-document".into())
        // Bounded scene expansion can reach 128 references and 256 child levels.
        // Reserve enough stack for debug frames before those guards reject input.
        .stack_size(crate::project::SCENE_WORKER_STACK_BYTES)
        .spawn(move || -> Result<()> {
            let mut documents = workspace::Workspace::new(options)?;
            #[cfg(feature = "rendering")]
            let wait_runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(|error| {
                    Error::new("IO_ERROR", format!("Cannot start document timer: {error}"))
                })?;
            loop {
                #[cfg(feature = "rendering")]
                let received = {
                    documents.expire_idle(Instant::now());
                    let deadline = documents.maintenance_deadline();
                    wait_runtime.block_on(async {
                        if let Some(deadline) = deadline {
                            tokio::time::timeout_at(deadline.into(), receiver.recv()).await
                        } else {
                            Ok(receiver.recv().await)
                        }
                    })
                };
                #[cfg(feature = "rendering")]
                let job = match received {
                    Ok(job) => job,
                    Err(_) => continue,
                };
                #[cfg(not(feature = "rendering"))]
                let job = receiver.blocking_recv();
                let Some(job) = job else { break };
                let started = Instant::now();
                let skipped = job.reply.is_closed() || job.cancel.is_cancelled();
                let called = if skipped {
                    Err(Error::new(
                        "CANCELLED",
                        "The request was cancelled before execution",
                    ))
                } else {
                    documents.call(&job.name, job.arguments, &job.cancel)
                };
                let result = match called {
                    Ok(result) => result,
                    Err(error) => CallToolResult::structured_error(json!({"error":error})),
                };
                let result =
                    job.timing
                        .finish(&job.name, (!skipped).then_some(started), result, job.usage);
                let _ = job.reply.send(result);
            }
            Ok(())
        })
        .map_err(|e| Error::new("IO_ERROR", format!("Cannot start document worker: {e}")))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::new("IO_ERROR", format!("Cannot start MCP runtime: {e}")))?;
    let result=runtime.block_on(async move {
        let running=Server { sender, preview_control: has_preview, usage: server_usage }.serve(rmcp::transport::stdio_pipes()).await
            .map_err(|e| Error::new("IO_ERROR",format!("MCP connection failed: {e}")))?;
        let cancel=running.cancellation_token();
        tokio::select! {
            result=running.waiting()=>result.map(|_| ()).map_err(|e| Error::new("IO_ERROR",format!("MCP service failed: {e}"))),
            signal=tokio::signal::ctrl_c()=>{
                cancel.cancel();
                signal.map_err(|e| Error::new("IO_ERROR",format!("Cannot handle interrupt: {e}")))?;
                Err(Error::new("CANCELLED","MCP server interrupted"))
            }
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    worker
        .join()
        .map_err(|_| Error::new("INTERNAL_ERROR", "Document worker terminated unexpectedly"))??;
    result
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let instructions = if self.preview_control {
            format!("{INSTRUCTIONS}\n\n{}", crate::preview_control::INSTRUCTIONS)
        } else {
            INSTRUCTIONS.into()
        };
        let instructions = format!(
            "{instructions}\nServer working directory: {}",
            std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        );
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lazypencil", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_instructions(instructions)
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![ProtocolVersion::V_2025_11_25])
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        catalog::find(name, self.preview_control)
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|r| r.cursor.is_some()) {
            return Err(ErrorData::invalid_params(
                "No tools/list cursor is available",
                None,
            ));
        }
        Ok(ListToolsResult::with_all_items(tools_for(
            self.preview_control,
        )))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let timing = timing::Timer::new();
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params("Unknown tool", None));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        if let Err(error) = crate::query::check_response_size(
            &arguments,
            MAX_TOOL_BYTES,
            None,
            "Tool input exceeds 4 MiB",
        ) {
            return Ok(CallToolResult::structured_error(json!({"error":error})).into());
        }
        if context.ct.is_cancelled() {
            return Ok(CallToolResult::structured_error(json!({"error":Error::new("CANCELLED","The request was cancelled before execution")})).into());
        }
        let (reply, receiver) = oneshot::channel();
        // Capture the active turn before queueing, so cancelled requests cannot
        // append timing data to a later turn on the same connection.
        let usage = if let Some(access) = self.usage.clone() {
            let cancel = context.ct.clone();
            tokio::task::spawn_blocking(move || access.request(&cancel))
                .await
                .map_err(|_| crate::Error::new("INTERNAL_ERROR", "Timing grant worker stopped"))
                .and_then(|request| request.map(Some))
        } else {
            Ok(None)
        };
        let job = Job {
            name: request.name.into_owned(),
            arguments,
            reply,
            cancel: context.ct.clone(),
            timing,
            usage,
        };
        tokio::select! {
            biased;
            _ = context.ct.cancelled() => {
                return Ok(CallToolResult::structured_error(json!({"error":Error::new("CANCELLED","The request was cancelled before execution")})).into());
            }
            result = self.sender.send(job) => {
                result.map_err(|_| ErrorData::internal_error("Document worker stopped", None))?;
            }
        }
        let result = tokio::select! {
            biased;
            _ = context.ct.cancelled() => {
                return Ok(CallToolResult::structured_error(json!({"error":Error::new("CANCELLED","The request was cancelled")})).into());
            }
            result = receiver => result.map_err(|_| ErrorData::internal_error("Document worker stopped", None))?,
        };
        Ok(result.into())
    }
}

pub(crate) fn tool_names() -> Vec<String> {
    tools_for(true)
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocumentArgs {
    /// Explicitly acquire HTTP image bytes; false keeps inspection free of HTTP requests.
    #[serde(default)]
    acquire_images: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GuideArgs {
    #[serde(default)]
    id: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StylesArgs {
    #[serde(default)]
    use_case: Option<String>,
    #[serde(default = "hundred")]
    limit: usize,
    #[serde(default)]
    cursor: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StyleArgs {
    id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default = "hundred")]
    limit: usize,
    #[serde(default)]
    cursor: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct History {
    #[serde(default = "hundred")]
    limit: usize,
    #[serde(default)]
    before_sequence: Option<u64>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Restore {
    request_id: String,
    expected_revision: String,
    #[serde(default)]
    expected_dependency_revision: Option<String>,
    checkpoint_id: String,
}
fn hundred() -> usize {
    100
}

#[cfg(feature = "rendering")]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LayoutArgs {
    expected_revision: String,
    #[serde(default)]
    expected_dependency_revision: Option<String>,
    #[serde(default)]
    snapshot_id: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default = "hundred")]
    limit: usize,
    #[serde(default)]
    cursor: Option<String>,
}
#[cfg(feature = "rendering")]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenderArgs {
    expected_revision: String,
    #[serde(default)]
    expected_dependency_revision: Option<String>,
    #[serde(default)]
    snapshot_id: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    region: Option<crate::geometry::Rect>,
    #[serde(default = "scale_one")]
    scale: f32,
    #[serde(default)]
    time: f64,
    #[serde(default)]
    refresh: bool,
}
#[cfg(feature = "rendering")]
fn scale_one() -> f32 {
    1.0
}

struct DocumentWorker {
    store: Store,
    attachments: Option<crate::attachments::AttachmentAccess>,
    usage: Option<crate::guidance::usage::UsageAccess>,
    preview_control: Option<crate::preview_control::Client>,
    fonts: Vec<PathBuf>,
    #[cfg(feature = "rendering")]
    snapshots: VecDeque<CachedSnapshot>,
    #[cfg(feature = "rendering")]
    failed_code: VecDeque<CachedFailure>,
    #[cfg(feature = "rendering")]
    scripts: Option<crate::script::ScriptRuntime>,
    #[cfg(feature = "rendering")]
    dynamic: Option<crate::dynamic::DynamicRuntime>,
}
#[cfg(feature = "rendering")]
struct CachedSnapshot {
    project: Project,
    layout: crate::layout::LayoutSnapshot,
    created: Instant,
}
#[cfg(feature = "rendering")]
struct CachedFailure {
    source: crate::project::FailedCodeCapture,
    created: Instant,
}

impl DocumentWorker {
    #[cfg(feature = "rendering")]
    fn maintenance_deadline(&self) -> Option<Instant> {
        self.scripts
            .as_ref()
            .and_then(crate::script::ScriptRuntime::idle_deadline)
            .into_iter()
            .chain(
                self.dynamic
                    .as_ref()
                    .and_then(crate::dynamic::DynamicRuntime::idle_deadline),
            )
            .chain(
                self.snapshots
                    .iter()
                    .map(|snapshot| snapshot.created + SNAPSHOT_TTL),
            )
            .chain(
                self.failed_code
                    .iter()
                    .map(|capture| capture.created + SNAPSHOT_TTL),
            )
            .min()
    }

    #[cfg(feature = "rendering")]
    fn expire_idle(&mut self, now: Instant) {
        if let Some(scripts) = &mut self.scripts {
            scripts.expire_idle(now);
        }
        if let Some(dynamic) = &mut self.dynamic {
            dynamic.expire_idle(now);
        }
        self.snapshots
            .retain(|snapshot| snapshot.created + SNAPSHOT_TTL > now);
        self.failed_code
            .retain(|capture| capture.created + SNAPSHOT_TTL > now);
    }

    fn call(
        &mut self,
        name: &str,
        arguments: Value,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult> {
        #[cfg(unix)]
        {
            crate::cancel::check(Some(cancel))?;
            let expired = || {
                Error::new(
                    "AGENT_ERROR",
                    "The owned MCP connection is no longer active; reconnect to issue a new grant",
                )
            };
            if let Some(access) = &self.attachments {
                access
                    .check_connection(&self.store)
                    .map_err(|_| expired())?;
            }
            if let Some(access) = &self.usage {
                access.check_connection().map_err(|_| expired())?;
            }
        }
        let kind =
            ToolKind::parse(name).ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unknown tool"))?;
        let value = match kind {
            ToolKind::GetPreviewState => {
                let _: Empty = parse(arguments)?;
                self.preview_control
                    .as_ref()
                    .ok_or_else(|| {
                        Error::new("PREVIEW_UNAVAILABLE", "No TUI preview is connected")
                    })?
                    .state(cancel)?
            }
            ToolKind::SetPreview => self
                .preview_control
                .as_ref()
                .ok_or_else(|| Error::new("PREVIEW_UNAVAILABLE", "No TUI preview is connected"))?
                .set(parse(arguments)?, cancel)?,
            ToolKind::ListIcons => crate::icons::list(self.store.data_dir(), &parse(arguments)?)?,
            ToolKind::GetGuidance => {
                let args: GuideArgs = parse(arguments)?;
                service::project_guidance_with_cancellation(
                    &self.store,
                    args.id.as_deref(),
                    Some(cancel),
                )?
            }
            ToolKind::ListStyles => {
                let args: StylesArgs = parse(arguments)?;
                service::project_styles_with_cancellation(
                    &self.store,
                    args.use_case.as_deref(),
                    args.limit,
                    args.cursor.as_deref(),
                    Some(cancel),
                )?
            }
            ToolKind::GetStyle => {
                let args: StyleArgs = parse(arguments)?;
                service::project_style_with_cancellation(&self.store, &args.id, Some(cancel))?
            }
            ToolKind::GetDocument => {
                let args: DocumentArgs = parse(arguments)?;
                let mut value = service::document_metadata_with_images(
                    &self.store,
                    &self.fonts,
                    args.acquire_images,
                    Some(cancel),
                )?;
                value["guidance"] = json!({"entry_tool":"get_guidance","entry_id":"start","language":"en","styles_tool":"list_styles"});
                value["guidance"]["project_design"] = value["configuration"]["design"].clone();
                if let Some(attachments) = &self.attachments {
                    value["attachments"] = attachments.metadata(&self.store)?;
                }
                value
            }
            ToolKind::ReadNodes => {
                let args: service::ReadNodesRequest = parse(arguments)?;
                if args.calculated || args.snapshot_id.is_some() {
                    #[cfg(not(feature = "rendering"))]
                    return Err(Error::new(
                        "UNSUPPORTED_FEATURE",
                        "Calculated node reads require the rendering feature",
                    ));
                    #[cfg(feature = "rendering")]
                    {
                        service::validate_calculated_nodes_request(&args)?;
                        let index = self.snapshot_with_preconditions(
                            None,
                            None,
                            args.snapshot_id.as_deref(),
                            false,
                            cancel,
                        )?;
                        let cached = &self.snapshots[index];
                        service::read_calculated_nodes(
                            &cached.project,
                            &cached.layout,
                            &args,
                            Some(cancel),
                        )?
                    }
                } else {
                    service::read_store_nodes_with_cancellation(
                        &self.store,
                        &self.fonts,
                        &args,
                        Some(cancel),
                    )?
                }
            }
            ToolKind::ListComponents => {
                let args: Page = parse(arguments)?;
                let project = self.project(Some(cancel))?;
                let components = project.components();
                let identity = format!(
                    "{}:{}",
                    project.root_document().revision(),
                    project.dependency_revision()
                );
                let mut result = paginate(
                    &components,
                    &identity,
                    args.limit,
                    args.cursor.as_deref(),
                    "components",
                )?;
                result["revision"] = json!(project.root_document().revision());
                result["dependency_revision"] = json!(project.dependency_revision());
                result
            }
            ToolKind::GetVariables => {
                service::get_variables(&self.store, &self.fonts, &parse(arguments)?, Some(cancel))?
            }
            ToolKind::ApplyOperations => {
                let request: EditRequest = parse(arguments)?;
                #[cfg(feature = "rendering")]
                if request.expected_runtime_dependency_revision.is_some() {
                    if self.scripts.is_none() {
                        self.scripts = Some(crate::script::ScriptRuntime::current()?);
                    }
                    let runtime = self.scripts.as_mut().expect("script runtime");
                    runtime.set_cancellation(Some(cancel.clone()));
                    return service::apply_request_with_runtime(
                        &self.store,
                        &request,
                        &self.fonts,
                        runtime,
                    )
                    .and_then(structured);
                }
                service::apply_request_with_cancellation(
                    &self.store,
                    &request,
                    &self.fonts,
                    Some(cancel),
                )?
            }
            ToolKind::ValidateDocument => {
                let args: service::ValidateRequest = parse(arguments)?;
                args.validate()?;
                #[cfg(feature = "rendering")]
                if args.scope == service::ValidationScope::Layout {
                    let index = self.snapshot_with_preconditions(
                        args.expected_revision.as_deref(),
                        args.expected_dependency_revision.as_deref(),
                        args.snapshot_id.as_deref(),
                        false,
                        cancel,
                    )?;
                    let cached = &self.snapshots[index];
                    let value =
                        service::validation_from_snapshot(&cached.project, &cached.layout, &args)?;
                    crate::store::require_revision(&self.store.read()?, &cached.layout.revision)?;
                    crate::cancel::check(Some(cancel))?;
                    return structured(value);
                }
                service::validate_document_with_options(
                    &self.store,
                    &self.fonts,
                    &args,
                    Some(cancel),
                )?
            }
            ToolKind::GetHistory => {
                let args: History = parse(arguments)?;
                if !(1..=1000).contains(&args.limit) {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "History limit must be 1..1000",
                    ));
                }
                let page = self.store.history_page_with_cancellation(
                    args.limit,
                    args.before_sequence,
                    Some(cancel),
                )?;
                json!({"revision":self.store.read()?.revision(),"entries":page.entries,"next_before_sequence":page.next_before_sequence})
            }
            ToolKind::RestoreCheckpoint => {
                let args: Restore = parse(arguments)?;
                service::restore_checkpoint_with_cancellation(
                    &self.store,
                    &service::RestoreRequest {
                        request_id: &args.request_id,
                        expected_revision: &args.expected_revision,
                        expected_dependency_revision: args.expected_dependency_revision.as_deref(),
                        checkpoint: &args.checkpoint_id,
                    },
                    &self.fonts,
                    Some(cancel),
                )?
            }
            ToolKind::ReadAsset => {
                let args: crate::assets::ReadAssetRequest = parse(arguments)?;
                self.read_asset(&args, cancel)?
            }
            ToolKind::PutAsset => {
                let args: crate::assets::PutAssetRequest = parse(arguments)?;
                let asset = if let crate::assets::AssetSource::Attachment { attachment_id } =
                    &args.source
                {
                    if !matches!(args.kind, crate::assets::AssetKind::Image) {
                        return Err(Error::new(
                            "ATTACHMENT_INVALID",
                            "Image attachments cannot be imported as code",
                        ));
                    }
                    let access = self.attachments.as_ref().ok_or_else(|| {
                        Error::new(
                            "ATTACHMENT_INVALID",
                            "No images are registered for this MCP connection",
                        )
                    })?;
                    let bytes = access.read(&self.store, attachment_id)?;
                    crate::assets::put_bytes_with_cancellation(
                        &self.store,
                        args.kind,
                        &bytes,
                        Some("image/png"),
                        Some(cancel),
                    )?
                } else {
                    crate::assets::put_asset_with_cancellation(&self.store, &args, Some(cancel))?
                };
                serde_json::to_value(asset)
                    .map_err(|e| Error::new("INTERNAL_ERROR", e.to_string()))?
            }
            #[cfg(feature = "rendering")]
            ToolKind::GetLayout => return self.get_layout(parse(arguments)?, cancel),
            #[cfg(feature = "rendering")]
            ToolKind::Render => return self.render(parse(arguments)?, cancel),
            #[cfg(feature = "rendering")]
            ToolKind::ListFonts => {
                let _: Empty = parse(arguments)?;
                let project = self.project(Some(cancel))?;
                let mut fonts = project.load_fonts()?;
                let inventory = fonts.inventory()?;
                project.verify_unchanged()?;
                json!({"revision":project.root_document().revision(),"dependency_revision":project.dependency_revision(),"fonts":inventory,"fallback_families":fonts.fallback_families(),"default_family":fonts.default_family(),"locale":fonts.locale(),"configuration":project.configuration()})
            }
            ToolKind::OpenDocument | ToolKind::CreateDocument | ToolKind::ListDocuments => {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Tool requires workspace dispatch",
                ));
            }
        };
        if kind.checks_revision()
            && let Some(revision) = value.get("revision").and_then(Value::as_str)
        {
            crate::store::require_revision(&self.store.read()?, revision)?;
        }
        structured(value)
    }

    fn project(&self, cancel: Option<&tokio_util::sync::CancellationToken>) -> Result<Project> {
        let mut project = Project::load_with_cancellation(&self.store, cancel)?;
        project.add_font_files(&self.fonts)?;
        Ok(project)
    }

    fn read_asset(
        &self,
        args: &crate::assets::ReadAssetRequest,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Value> {
        let project = self.project(Some(cancel))?;
        let result = crate::assets::read_asset(&project, args);
        #[cfg(feature = "rendering")]
        if result
            .as_ref()
            .is_err_and(|error| error.code == "ASSET_NOT_FOUND")
        {
            enum Capture<'a> {
                Snapshot(&'a CachedSnapshot),
                Failure(&'a CachedFailure),
            }
            let mut captures = self
                .snapshots
                .iter()
                .map(|s| (s.created, Capture::Snapshot(s)))
                .chain(
                    self.failed_code
                        .iter()
                        .map(|s| (s.created, Capture::Failure(s))),
                )
                .collect::<Vec<_>>();
            captures.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
            for (created, capture) in captures {
                if created.elapsed() >= SNAPSHOT_TTL {
                    continue;
                }
                match capture {
                    Capture::Snapshot(cached) => {
                        if cached.project.root_document().revision()
                            != project.root_document().revision()
                        {
                            continue;
                        }
                        let result = crate::assets::read_asset(&cached.project, args);
                        if result
                            .as_ref()
                            .is_err_and(|error| error.code == "ASSET_NOT_FOUND")
                        {
                            continue;
                        }
                        cached.project.verify_unchanged()?;
                        let mut value = result?;
                        value["snapshot_id"] = json!(cached.layout.id);
                        return Ok(value);
                    }
                    Capture::Failure(cached) => {
                        if cached.source.hash() == args.hash
                            && cached.source.revision() == project.root_document().revision()
                        {
                            return cached.source.read(&project, args);
                        }
                    }
                }
            }
        }
        result
    }

    #[cfg(feature = "rendering")]
    fn snapshot(
        &mut self,
        revision: &str,
        dependency: Option<&str>,
        id: Option<&str>,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<usize> {
        self.snapshot_with_preconditions(Some(revision), dependency, id, true, cancel)
    }

    #[cfg(feature = "rendering")]
    fn snapshot_with_preconditions(
        &mut self,
        revision: Option<&str>,
        dependency: Option<&str>,
        id: Option<&str>,
        require_dependency: bool,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<usize> {
        crate::cancel::check(Some(cancel))?;
        self.snapshots
            .retain(|s| s.created.elapsed() < SNAPSHOT_TTL);
        if let Some(id) = id {
            let index = self
                .snapshots
                .iter()
                .position(|s| s.layout.id == id)
                .ok_or_else(|| {
                    Error::new(
                        "SNAPSHOT_UNAVAILABLE",
                        "This calculation snapshot is not retained on this MCP connection. Request a fresh calculation without snapshot_id or cursor, then use its returned snapshot_id for subsequent reads",
                    )
                })?;
            let cached = &self.snapshots[index];
            crate::store::require_revision(&self.store.read()?, &cached.layout.revision)?;
            if let Some(revision) = revision {
                crate::store::require_revision(cached.project.root_document(), revision)?;
            }
            if require_dependency || dependency.is_some() {
                cached.project.require_dependencies(dependency)?;
            }
            cached.project.verify_unchanged()?;
            return Ok(index);
        }
        if self.scripts.is_none() {
            self.scripts = Some(crate::script::ScriptRuntime::current()?);
        }
        self.scripts
            .as_mut()
            .expect("script runtime")
            .set_cancellation(Some(cancel.clone()));
        let mut failed_code = None;
        let result = service::layout_snapshot_observing_failure(
            &self.store,
            &self.fonts,
            revision,
            dependency,
            self.scripts.as_mut().expect("script runtime"),
            |project, declared, error| {
                failed_code = project.capture_failed_code(declared, error);
            },
        );
        let (project, layout) = match result {
            Ok(value) => value,
            Err(mut error) => {
                if let Some(source) = failed_code {
                    self.failed_code.retain(|capture| {
                        capture.created.elapsed() < SNAPSHOT_TTL
                            && capture.source.id() != source.id()
                    });
                    if self.failed_code.len() >= 4 {
                        self.failed_code.pop_front();
                    }
                    error.details.get_or_insert_with(|| json!({}))["failure_capture_id"] =
                        json!(source.id());
                    self.failed_code.push_back(CachedFailure {
                        source,
                        created: Instant::now(),
                    });
                }
                return Err(error);
            }
        };
        if require_dependency || dependency.is_some() {
            project.require_dependencies(dependency)?;
        }
        if let Some(index) = self.snapshots.iter().position(|s| s.layout.id == layout.id) {
            return Ok(index);
        }
        if self.snapshots.len() >= 4 {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(CachedSnapshot {
            project,
            layout,
            created: Instant::now(),
        });
        Ok(self.snapshots.len() - 1)
    }

    #[cfg(feature = "rendering")]
    fn get_layout(
        &mut self,
        args: LayoutArgs,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult> {
        let index = self.snapshot(
            &args.expected_revision,
            args.expected_dependency_revision.as_deref(),
            args.snapshot_id.as_deref(),
            cancel,
        )?;
        let snapshot = &self.snapshots[index].layout;
        let records = snapshot
            .records()
            .into_iter()
            .filter(|r| args.node_id.as_ref().is_none_or(|id| &r.id == id))
            .collect::<Vec<_>>();
        if args.node_id.is_some() && records.is_empty() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The requested node does not exist",
            ));
        }
        let identity = crate::document::hash(
            json!({"snapshot":snapshot.id,"node_id":args.node_id})
                .to_string()
                .as_bytes(),
        );
        let page = paginate(
            &records,
            &identity,
            args.limit,
            args.cursor.as_deref(),
            "nodes",
        )?;
        let mut result = snapshot.response_summary();
        result
            .as_object_mut()
            .expect("summary object")
            .extend(page.as_object().expect("page object").clone());
        structured(result)
    }

    #[cfg(feature = "rendering")]
    fn render(
        &mut self,
        args: RenderArgs,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult> {
        use base64::Engine;
        let index = self.snapshot(
            &args.expected_revision,
            args.expected_dependency_revision.as_deref(),
            args.snapshot_id.as_deref(),
            cancel,
        )?;
        let cached = &self.snapshots[index];
        if self.dynamic.is_none() {
            self.dynamic = Some(crate::dynamic::DynamicRuntime::new(self.store.data_dir()));
        }
        let image = crate::render::png_with_runtime(
            &cached.project,
            &cached.layout,
            &crate::render::RenderOptions {
                scale: args.scale,
                time: args.time,
                refresh: args.refresh,
                node: args.node_id,
                region: args.region,
                ..Default::default()
            },
            self.dynamic.as_mut().expect("dynamic runtime"),
            Some(cancel.clone()),
        )?;
        crate::store::require_revision(&self.store.read()?, &cached.layout.revision)?;
        if image.png.len() > 8 * 1024 * 1024 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "PNG exceeds 8 MiB; render a smaller region or reduce scale",
            ));
        }
        let mut metadata = cached.layout.response_summary();
        metadata["width"] = json!(image.width);
        metadata["height"] = json!(image.height);
        metadata["region"] = json!(image.region);
        metadata["png_hash"] = json!(crate::document::hash(&image.png));
        metadata["dynamic_artifacts"] = json!(image.dynamic_artifacts);
        let mut result = structured(metadata)?;
        result.content.push(rmcp::model::ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(image.png),
            "image/png",
        ));
        Ok(result)
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|e| Error::new("INVALID_ARGUMENT", format!("Invalid tool arguments: {e}")))
}
fn structured(value: Value) -> Result<CallToolResult> {
    crate::query::check_response_size(
        &value,
        MAX_TOOL_BYTES,
        None,
        "Tool result exceeds 4 MiB; reduce depth or page size",
    )?;
    Ok(CallToolResult::structured(value))
}
fn paginate<T: serde::Serialize>(
    values: &[T],
    identity: &str,
    limit: usize,
    cursor: Option<&str>,
    key: &str,
) -> Result<Value> {
    let offset = crate::query::page_offset(identity, cursor, limit, values.len())?;
    let total = values.len();
    let page = &values[offset..offset.saturating_add(limit).min(total)];
    let next = (offset + page.len() < total).then(|| format!("{identity}:{}", offset + page.len()));
    let mut result = json!({"total_matches":total,"next_cursor":next});
    result[key] = json!(page);
    Ok(result)
}

#[cfg(all(test, feature = "rendering"))]
mod tests {
    use super::*;

    #[test]
    fn layout_pages_serialize_only_selected_records_and_reject_invalid_cursors_first() {
        struct Row(usize);
        impl serde::Serialize for Row {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                assert_eq!(self.0, 1, "Unselected records must never be serialized");
                serializer.serialize_u64(self.0 as u64)
            }
        }
        let rows = [Row(0), Row(1), Row(2)];
        let page = paginate(&rows, "snapshot", 1, Some("snapshot:1"), "nodes").unwrap();
        assert_eq!(
            page,
            json!({"nodes":[1],"total_matches":3,"next_cursor":"snapshot:2"})
        );
        for (cursor, code) in [
            ("other:0", "REVISION_CONFLICT"),
            ("snapshot:4", "INVALID_ARGUMENT"),
        ] {
            assert_eq!(
                paginate(&rows, "snapshot", 1, Some(cursor), "nodes")
                    .unwrap_err()
                    .code,
                code
            );
        }
        let end = paginate(&rows, "snapshot", 1, Some("snapshot:3"), "nodes").unwrap();
        assert_eq!(end["nodes"], json!([]));
        assert!(end["next_cursor"].is_null());
    }

    #[test]
    fn failed_code_expiry_releases_bytes_and_cancellation_does_not_poison_the_capture() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("design.pen");
        let store = Store::init(&file, Some(temp.path().join("data"))).unwrap();
        std::fs::write(
            temp.path().join("outer.js"),
            "/** @schema 2.21 */ return [];",
        )
        .unwrap();
        let code = "/** @schema 2.21 */ throw new Error('nested');";
        std::fs::write(temp.path().join("nested.js"), code).unwrap();
        std::fs::write(&file, json!({"version":"2.21","children":[{"id":"outer","type":"script","scriptUri":"outer.js","width":20,"height":20}]}).to_string()).unwrap();
        let mut project = Project::load(&store).unwrap();
        let declared = project.dependencies();
        let owner = project.resolve().unwrap().roots.remove(0);
        project
            .generated_children(
                &owner,
                &[json!({"type":"script","scriptUri":"nested.js","width":20,"height":20})],
                &mut Default::default(),
            )
            .unwrap();
        let source_hash = crate::document::hash(code.as_bytes());
        let error = Error::new("SCRIPT_EXECUTION_FAILED", "nested").details(json!({"script_path":crate::paths::canonical(temp.path().join("nested.js")).unwrap(),"source_hash":source_hash}));
        let source = project.capture_failed_code(&declared, &error).unwrap();
        drop(project);
        let created = Instant::now();
        let mut worker = DocumentWorker {
            store,
            attachments: None,
            usage: None,
            preview_control: None,
            fonts: vec![],
            snapshots: VecDeque::new(),
            failed_code: VecDeque::from([CachedFailure { source, created }]),
            scripts: None,
            dynamic: None,
        };
        let args = crate::assets::ReadAssetRequest {
            hash: source_hash,
            offset: 0,
            limit: 1024,
        };
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            worker.read_asset(&args, &cancel).unwrap_err().code,
            "CANCELLED"
        );
        let cancel = tokio_util::sync::CancellationToken::new();
        assert_eq!(worker.read_asset(&args, &cancel).unwrap()["content"], code);
        let expiry = created + SNAPSHOT_TTL;
        assert_eq!(worker.maintenance_deadline(), Some(expiry));
        worker.expire_idle(expiry - Duration::from_nanos(1));
        assert_eq!(worker.failed_code.len(), 1);
        worker.expire_idle(expiry);
        assert!(worker.failed_code.is_empty());
        assert!(worker.maintenance_deadline().is_none());
        assert_eq!(
            worker.read_asset(&args, &cancel).unwrap_err().code,
            "ASSET_NOT_FOUND"
        );
        let large = " ".repeat(1024 * 1024 + 1);
        std::fs::write(temp.path().join("nested.js"), &large).unwrap();
        let mut project = worker.project(None).unwrap();
        let declared = project.dependencies();
        let owner = project.resolve().unwrap().roots.remove(0);
        project
            .generated_children(
                &owner,
                &[json!({"type":"script","scriptUri":"nested.js","width":20,"height":20})],
                &mut Default::default(),
            )
            .unwrap();
        let error = Error::new("SCRIPT_EXECUTION_FAILED", "oversized code").details(json!({"script_path":crate::paths::canonical(temp.path().join("nested.js")).unwrap(),"source_hash":crate::document::hash(large.as_bytes())}));
        assert!(project.capture_failed_code(&declared, &error).is_none());
    }

    #[test]
    fn expired_snapshots_release_their_resources_and_reject_reuse_without_editing() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Store::init(
            temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        let saved = store.read().unwrap();
        let revision = saved.revision();
        let mut worker = DocumentWorker {
            store,
            attachments: None,
            usage: None,
            preview_control: None,
            fonts: vec![],
            snapshots: VecDeque::new(),
            failed_code: VecDeque::new(),
            scripts: None,
            dynamic: None,
        };
        let cancel = tokio_util::sync::CancellationToken::new();
        let index = worker.snapshot(&revision, None, None, &cancel).unwrap();
        let snapshot_id = worker.snapshots[index].layout.id.clone();
        let expiry = worker.maintenance_deadline().unwrap();
        worker.expire_idle(expiry - Duration::from_nanos(1));
        assert_eq!(worker.snapshots.len(), 1);
        worker.expire_idle(expiry);
        assert!(worker.snapshots.is_empty());
        assert!(worker.maintenance_deadline().is_none());
        assert_eq!(
            worker
                .call(
                    "render",
                    json!({"expected_revision":revision,"snapshot_id":snapshot_id}),
                    &cancel
                )
                .unwrap_err()
                .code,
            "SNAPSHOT_UNAVAILABLE"
        );
        assert_eq!(worker.store.read().unwrap().text(), saved.text());
    }
}
