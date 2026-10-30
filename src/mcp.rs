use crate::{Error, Result, operations::EditRequest, project::Project, service, store::Store};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{borrow::Cow, path::PathBuf, time::Duration};
#[cfg(feature = "rendering")]
use std::{collections::VecDeque, time::Instant};
use tokio::sync::{mpsc, oneshot};

const MAX_TOOL_BYTES: usize = 4 * 1024 * 1024;
#[cfg(feature = "rendering")]
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);
const INSTRUCTIONS: &str = "This server edits one bound .pen document. Start with get_document and get_guidance, inspect existing nodes, components and variables, then apply small atomic batches using current document and dependency revisions. Choose a suitable available style for a new design; preserve existing conventions for edits. Read configured project_guides after the user has explicitly granted local trust. Agents must not grant project trust themselves. Follow the user request, trusted project rules, selected style, then standard guidance. Use get_layout and render to inspect the result and correct layout or visual problems. Render returns real PNG image content. Do not claim a visual check unless you received and inspected the image. Preserve the user's language. English guides do not require English design content. Use only advertised tools and supported capabilities.";

#[derive(Clone)]
struct Server {
    sender: mpsc::Sender<Job>,
}
struct Job {
    name: String,
    arguments: Value,
    reply: oneshot::Sender<CallToolResult>,
    cancel: tokio_util::sync::CancellationToken,
}

pub fn serve(store: Store, fonts: Vec<PathBuf>) -> Result<()> {
    serve_with_attachments(store, fonts, None)
}

pub fn serve_with_attachments(
    store: Store,
    fonts: Vec<PathBuf>,
    attachments: Option<crate::attachments::AttachmentAccess>,
) -> Result<()> {
    serve_with_context(store, fonts, attachments, None)
}

pub fn serve_with_context(
    store: Store,
    fonts: Vec<PathBuf>,
    attachments: Option<crate::attachments::AttachmentAccess>,
    usage: Option<crate::guidance::usage::UsageAccess>,
) -> Result<()> {
    let (sender, mut receiver) = mpsc::channel::<Job>(16);
    let worker = std::thread::Builder::new()
        .name("lazypencil-document".into())
        .spawn(move || -> Result<()> {
            #[cfg(feature = "rendering")]
            let wait_runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(|error| {
                    Error::new("IO_ERROR", format!("Cannot start document timer: {error}"))
                })?;
            let mut document = DocumentWorker {
                store,
                fonts,
                attachments,
                usage,
                #[cfg(feature = "rendering")]
                snapshots: VecDeque::new(),
                #[cfg(feature = "rendering")]
                failed_code: VecDeque::new(),
                #[cfg(feature = "rendering")]
                scripts: None,
                #[cfg(feature = "rendering")]
                dynamic: None,
            };
            loop {
                #[cfg(feature = "rendering")]
                let received = {
                    document.expire_idle(Instant::now());
                    let deadline = document.maintenance_deadline();
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
                if job.reply.is_closed() {
                    continue;
                }
                let called = if job.cancel.is_cancelled() {
                    Err(Error::new(
                        "CANCELLED",
                        "The request was cancelled before execution",
                    ))
                } else {
                    document.call(&job.name, job.arguments, &job.cancel)
                };
                let result = match called {
                    Ok(result) => result,
                    Err(error) => CallToolResult::structured_error(json!({"error":error})),
                };
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
        let running=Server { sender }.serve(rmcp::transport::stdio_pipes()).await
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
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lazypencil", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_instructions(INSTRUCTIONS)
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![ProtocolVersion::V_2025_11_25])
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
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
        Ok(ListToolsResult::with_all_items(tools()))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params("Unknown tool", None));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        if arguments.to_string().len() > MAX_TOOL_BYTES {
            return Ok(CallToolResult::structured_error(
                json!({"error":Error::new("INVALID_ARGUMENT","Tool input exceeds 4 MiB")}),
            )
            .into());
        }
        if context.ct.is_cancelled() {
            return Ok(CallToolResult::structured_error(json!({"error":Error::new("CANCELLED","The request was cancelled before execution")})).into());
        }
        let (reply, receiver) = oneshot::channel();
        if let Err(error) = self.sender.try_send(Job {
            name: request.name.into_owned(),
            arguments,
            reply,
            cancel: context.ct,
        }) {
            return match error {
                mpsc::error::TrySendError::Full(_)=>Ok(CallToolResult::structured_error(json!({"error":Error::new("BUSY","The document request queue is full; retry later")})).into()),
                mpsc::error::TrySendError::Closed(_)=>Err(ErrorData::internal_error("Document worker stopped",None)),
            };
        }
        let result = receiver
            .await
            .map_err(|_| ErrorData::internal_error("Document worker stopped", None))?;
        Ok(result.into())
    }
}

fn tool<T: JsonSchema>(name: &'static str, description: &'static str, read_only: bool) -> Tool {
    let schema = schemars::schema_for!(T)
        .to_value()
        .as_object()
        .expect("object schema")
        .clone();
    let mut tool = Tool::new(name, description, schema);
    tool.annotations = Some(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(!read_only)
            .idempotent(true)
            .open_world(false),
    );
    tool
}

fn tools() -> Vec<Tool> {
    let tools = vec![
        tool::<GuideArgs>(
            "get_guidance",
            "Read an English built-in guide or a trusted project guide. Omit id for the workflow, automatic project_guides and index. Project instructions require an explicit local user grant; never grant trust yourself. Preserve the user's response and design language.",
            true,
        ),
        tool::<StylesArgs>(
            "list_styles",
            "List built-in and trusted project design styles by use case. Project IDs use project:ID. Existing document conventions take priority unless the user requests a visual change.",
            true,
        ),
        tool::<StyleArgs>(
            "get_style",
            "Read style tokens, guidance references, font requirements and a read-only editable .pen example. Import or create elements through normal editing operations; never reuse example IDs as generated IDs.",
            true,
        ),
        tool::<Empty>(
            "get_document",
            "Read the bound document, current revisions, roots, dependency hashes and diagnostics before editing.",
            true,
        ),
        tool::<service::ReadNodesRequest>(
            "read_nodes",
            "Read stored nodes by ID, name or type with bounded depth and pagination. instance_path selects an inherited descendant; expand_instances preserves variable references. library_alias selects declared definitions read-only. calculated:true explicitly executes scripts and returns resolved properties, generated children and matching geometry. Pass snapshot_id from get_layout/render to read that exact calculation without rerunning scripts. Generated IDs are read-only: edit their owning script's inputs or source. Omit calculated and snapshot_id to inspect or repair stored code without executing it.",
            true,
        ),
        tool::<Page>(
            "list_components",
            "List reusable definitions in the bound document and its declared libraries. Library definitions are read-only.",
            true,
        ),
        tool::<service::VariablesRequest>(
            "get_variables",
            "Read paginated variable definitions and resolved values with source documents and per-variable diagnostics. name selects an exact name or declared-library-qualified reference; library_alias selects a declared library. node_id and instance_path select inherited theme context; property selects its origin namespace. theme overrides are hypothetical and do not edit the document. Preserve raw definitions and alias references when editing.",
            true,
        ),
        tool::<crate::icons::ListRequest>(
            "list_icons",
            "Search pinned vector icons by pack, name or tags. Lucide is bundled; Feather requires explicit CLI installation. Results include aliases, pack versions, content hashes and licenses. Use type:'icon', library:'lucide', icon:an_advertised_name, explicit width/height and a fill. Do not substitute missing icons or synthesize weights.",
            true,
        ),
        tool::<EditRequest>(
            "apply_operations",
            "Apply an all-or-nothing edit batch. Read revisions first; supply expected_dependency_revision when external dependencies exist. For edits based on a calculation, also supply its runtime_dependency_revision as expected_runtime_dependency_revision. Generated IDs can be referenced using client_ref and result_of. Reuse request_id only to retry the same request.",
            false,
        ),
        tool::<service::ValidateRequest>(
            "validate_document",
            "Validate structure and references, or use scope:'layout' for measured text, geometry and overflow diagnostics from the same calculation as get_layout/render. Layout scope executes scripts and can reuse snapshot_id. Optional expected revisions reject stale input; diagnostic pages bind to the calculation. Warnings do not invalidate a document; errors do. This does not perform a rendered visual check or validate every graphics/browser/shader capability.",
            true,
        ),
        tool::<History>(
            "get_history",
            "List committed edits and checkpoint IDs, newest first.",
            true,
        ),
        tool::<Restore>(
            "restore_checkpoint",
            "Restore the document content of a checkpoint as a new atomic edit. External files are not restored; their current revisions are checked.",
            false,
        ),
        tool::<crate::assets::ReadAssetRequest>(
            "read_asset",
            "Read captured script or shader text by dependency hash, including nested script code retained after a failed calculation on this connection. Failed captures are not layout snapshots. This tool cannot read an arbitrary filesystem path.",
            true,
        ),
        {
            let mut definition = tool::<crate::assets::PutAssetRequest>(
                "put_asset",
                "Create an immutable managed image, JS, or GLSL asset and return its relative URI and hash. Images use MIME-typed Base64 data or a registered attachment_id and are normalized to PNG. Code uses UTF-8 text. This does not edit the document; link the URI in a subsequent atomic batch. For an image, set fill to the complete object {type:'image',url:returned_uri,mode:'contain'}. Modes are cover, contain, stretch. Updating fill replaces the entire object; keep type and url.",
                false,
            );
            definition.annotations = Some(
                ToolAnnotations::new()
                    .read_only(false)
                    .destructive(false)
                    .idempotent(true)
                    .open_world(false),
            );
            definition
        },
    ];
    #[cfg(feature = "rendering")]
    let tools = {
        let mut tools = tools;
        tools.extend([
        tool::<LayoutArgs>("get_layout","Compute actual positions, transforms, shaped text metrics and overflow warnings. Inspect all pages as needed. Pass snapshot_id to reuse the exact retained calculation.",true),
        tool::<RenderArgs>("render","Return a PNG image content block for the whole design or a document-space crop. node_id selects its visual bounds while preserving document compositing context. Inspect the image, then correct problems using apply_operations. Layout and render share the same snapshot. Supply expected revisions.",true),
        tool::<Empty>("list_fonts","List each face in bundled and explicitly configured fonts, including collection index, source/face hashes, weight, slant, PostScript name, variation axes and the default fallback chain. Layout text metrics report the actual shaped faces. Unavailable requested families fail instead of silent substitution.",true),
        ]);
        tools
    };
    tools
}

pub(crate) fn tool_names() -> Vec<String> {
    tools()
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}
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
        let usage = if matches!(name, "get_guidance" | "get_style" | "list_styles") {
            self.usage
                .as_ref()
                .map(|usage| usage.request(cancel))
                .transpose()?
        } else {
            None
        };
        let value = match name {
            "list_icons" => crate::icons::list(self.store.data_dir(), &parse(arguments)?)?,
            "get_guidance" => {
                let args: GuideArgs = parse(arguments)?;
                service::project_guidance_with_cancellation(
                    &self.store,
                    args.id.as_deref(),
                    Some(cancel),
                )?
            }
            "list_styles" => {
                let args: StylesArgs = parse(arguments)?;
                service::project_styles_with_cancellation(
                    &self.store,
                    args.use_case.as_deref(),
                    args.limit,
                    args.cursor.as_deref(),
                    Some(cancel),
                )?
            }
            "get_style" => {
                let args: StyleArgs = parse(arguments)?;
                service::project_style_with_cancellation(&self.store, &args.id, Some(cancel))?
            }
            "get_document" => {
                let _: Empty = parse(arguments)?;
                let mut value = service::document_metadata_with_cancellation(
                    &self.store,
                    &self.fonts,
                    Some(cancel),
                )?;
                value["guidance"] = json!({"entry_tool":"get_guidance","entry_id":"start","language":"en","styles_tool":"list_styles"});
                value["guidance"]["project_design"] = value["configuration"]["design"].clone();
                if let Some(attachments) = &self.attachments {
                    value["attachments"] = attachments.metadata(&self.store)?;
                }
                value
            }
            "read_nodes" => {
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
            "list_components" => {
                let args: Page = parse(arguments)?;
                let project = self.project(Some(cancel))?;
                let components = project.components();
                let identity = format!(
                    "{}:{}",
                    project.root_document().revision(),
                    project.dependency_revision()
                );
                let mut result = paginate(
                    components,
                    &identity,
                    args.limit,
                    args.cursor.as_deref(),
                    "components",
                )?;
                result["revision"] = json!(project.root_document().revision());
                result["dependency_revision"] = json!(project.dependency_revision());
                result
            }
            "get_variables" => {
                service::get_variables(&self.store, &self.fonts, &parse(arguments)?, Some(cancel))?
            }
            "apply_operations" => {
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
            "validate_document" => {
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
            "get_history" => {
                let args: History = parse(arguments)?;
                if !(1..=1000).contains(&args.limit) {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "History limit must be 1..1000",
                    ));
                }
                let entries = self.store.history_with_cancellation(
                    args.limit,
                    args.before_sequence,
                    Some(cancel),
                )?;
                let next = if entries.len() == args.limit {
                    entries.last().map(|e| e.sequence)
                } else {
                    None
                };
                json!({"revision":self.store.read()?.revision(),"entries":entries,"next_before_sequence":next})
            }
            "restore_checkpoint" => {
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
            "read_asset" => {
                let args: crate::assets::ReadAssetRequest = parse(arguments)?;
                self.read_asset(&args, cancel)?
            }
            "put_asset" => {
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
            "get_layout" => return self.get_layout(parse(arguments)?, cancel),
            #[cfg(feature = "rendering")]
            "render" => return self.render(parse(arguments)?, cancel),
            #[cfg(feature = "rendering")]
            "list_fonts" => {
                let _: Empty = parse(arguments)?;
                let project = self.project(Some(cancel))?;
                let mut fonts = project.load_fonts()?;
                let inventory = fonts.inventory()?;
                project.verify_unchanged()?;
                json!({"revision":project.root_document().revision(),"dependency_revision":project.dependency_revision(),"fonts":inventory,"fallback_families":fonts.fallback_families(),"default_family":fonts.default_family(),"locale":fonts.locale(),"configuration":project.configuration()})
            }
            _ => return Err(Error::new("INVALID_ARGUMENT", "Unknown tool")),
        };
        if !matches!(name, "apply_operations" | "restore_checkpoint")
            && let Some(revision) = value.get("revision").and_then(Value::as_str)
        {
            crate::store::require_revision(&self.store.read()?, revision)?;
        }
        let result = structured(value)?;
        if let Some(usage) = usage {
            usage.record(
                name,
                result
                    .structured_content
                    .as_ref()
                    .expect("structured design resource"),
                cancel,
            )?;
        }
        Ok(result)
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
                        "This calculation snapshot is no longer retained",
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
            .map(|record| serde_json::to_value(record).expect("serializable layout record"))
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
            records,
            &identity,
            args.limit,
            args.cursor.as_deref(),
            "nodes",
        )?;
        let mut result = snapshot.summary();
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
        let mut metadata = cached.layout.summary();
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
    if value.to_string().len() > MAX_TOOL_BYTES {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Tool result exceeds 4 MiB; reduce depth or page size",
        ));
    }
    Ok(CallToolResult::structured(value))
}
fn paginate(
    values: Vec<Value>,
    identity: &str,
    limit: usize,
    cursor: Option<&str>,
    key: &str,
) -> Result<Value> {
    if !(1..=500).contains(&limit) {
        return Err(Error::new("INVALID_ARGUMENT", "Page limit must be 1..500"));
    }
    let offset = if let Some(cursor) = cursor {
        let (key, offset) = cursor
            .rsplit_once(':')
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Malformed page cursor"))?;
        if key != identity {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "This cursor belongs to a different revision or query",
            ));
        }
        offset
            .parse::<usize>()
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Malformed cursor offset"))?
    } else {
        0
    };
    if offset > values.len() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Cursor offset is outside this result",
        ));
    }
    let total = values.len();
    let page = values
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let next = (offset + page.len() < total).then(|| format!("{identity}:{}", offset + page.len()));
    let mut result = json!({"total_matches":total,"next_cursor":next});
    result[key] = json!(page);
    Ok(result)
}

#[cfg(all(test, feature = "rendering"))]
mod tests {
    use super::*;

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
