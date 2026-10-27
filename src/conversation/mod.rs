//! Frontend-independent, persistent conversations on a live ACP connection.

use crate::{
    Error, Result,
    agent::{AgentEvent, AgentHost, AgentOptions, permissions::DocumentPermissions},
    attachments::{Attachment, AttachmentGrant, MAX_REQUEST_BYTES, MAX_REQUEST_IMAGES},
    prompt::{AuthMode, DocumentTools, PromptArgs},
    store::Store,
};
use agent_client_protocol::schema::v1::{
    ContentBlock, EnvVariable, McpServer, McpServerStdio, PermissionOptionKind,
    RequestPermissionRequest, TextContent,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

mod continuity;
pub mod history;
#[cfg(unix)]
pub(crate) mod lease;
pub mod management;
mod storage;
pub use storage::sanitize_event;
use storage::{Session, Transcript};

pub enum ConversationEvent {
    Message(Value),
    Permission {
        id: String,
        request: Box<RequestPermissionRequest>,
    },
}
struct PendingPermission {
    request: Box<RequestPermissionRequest>,
    reply: oneshot::Sender<Option<String>>,
}
struct ActiveTurn {
    id: String,
    before: String,
    content: Option<Vec<ContentBlock>>,
    cancel: CancellationToken,
    denied: bool,
}

pub struct Conversation {
    store: Store,
    args: PromptArgs,
    command: PathBuf,
    executable: PathBuf,
    session: Session,
    host: Option<AgentHost>,
    grant: Option<AttachmentGrant>,
    usage: Option<crate::guidance::usage::UsageGrant>,
    ready: bool,
    transcript: Option<Transcript>,
    active: Option<ActiveTurn>,
    permissions: DocumentPermissions,
    pending_permissions: HashMap<String, PendingPermission>,
    events: VecDeque<ConversationEvent>,
    result: Option<Result<()>>,
}

impl Conversation {
    /// Opens local metadata without launching an agent or reading the OS clipboard.
    pub fn open(args: PromptArgs, data_dir: Option<PathBuf>) -> Result<Self> {
        let executable =
            std::env::current_exe().map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
        Self::open_with_executable(args, data_dir, executable)
    }

    /// Library hosts supply the real lazypencil binary for bound MCP servers
    /// and the adapter supervisor, rather than their embedding binary.
    pub fn open_with_executable(
        mut args: PromptArgs,
        data_dir: Option<PathBuf>,
        executable: PathBuf,
    ) -> Result<Self> {
        if args.restart_session && args.session.is_none() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Restarting a native session requires a local conversation ID",
            ));
        }
        if !executable.is_absolute() || !executable.is_file() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Conversation helpers require an absolute path to the lazypencil executable",
            ));
        }
        let executable = crate::paths::canonical(&executable)?;
        let store = if args.create {
            Store::init(&args.file, data_dir)?
        } else {
            Store::open(&args.file, data_dir)?
        };
        store.read()?;
        let command = args
            .adapter
            .clone()
            .unwrap_or_else(|| PathBuf::from(args.agent.command()));
        let command = if command.components().count() > 1 && args.project_execution.is_none() {
            crate::paths::canonical(&command)?
        } else {
            command
        };
        for font in &mut args.fonts {
            *font = crate::paths::canonical(&*font)?;
        }
        let session = Session::open(&store, &args, &command)?;
        let mut conversation = Self {
            store,
            args,
            command,
            executable,
            session,
            host: None,
            grant: None,
            usage: None,
            ready: false,
            transcript: None,
            active: None,
            permissions: DocumentPermissions::new(),
            pending_permissions: HashMap::new(),
            events: VecDeque::new(),
            result: None,
        };
        if !conversation.args.prompt.is_empty() {
            conversation.set_draft(conversation.args.prompt.clone())?;
        }
        Ok(conversation)
    }
    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn directory(&self) -> &Path {
        &self.session.directory
    }
    pub fn id(&self) -> &str {
        &self.session.state.id
    }
    pub fn draft(&self) -> &str {
        &self.session.state.draft
    }
    pub fn images(&self) -> &[Attachment] {
        &self.session.state.pending_attachments
    }
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    pub fn result(&self) -> Option<&Result<()>> {
        self.result.as_ref()
    }
    pub fn settings(&self) -> &PromptArgs {
        &self.args
    }
    pub fn set_draft(&mut self, text: String) -> Result<()> {
        self.require_idle()?;
        if text.len() > 1024 * 1024 {
            return Err(Error::new("INVALID_ARGUMENT", "Prompt text exceeds 1 MiB"));
        }
        self.session.state.draft = text;
        self.session.save()
    }
    pub fn add_image(&mut self, image: Attachment) -> Result<()> {
        self.add_images(vec![image])
    }
    pub fn add_images(&mut self, images: Vec<Attachment>) -> Result<()> {
        self.require_idle()?;
        if self.images().len() + images.len() > MAX_REQUEST_IMAGES {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "A request accepts at most eight images",
            ));
        }
        for image in &images {
            image.read(self.directory())?;
        }
        let source = self
            .images()
            .iter()
            .chain(&images)
            .map(|image| image.source_bytes.unwrap_or(image.bytes))
            .sum::<usize>();
        let normalized = self
            .images()
            .iter()
            .chain(&images)
            .map(|image| image.bytes)
            .sum::<usize>();
        if source > MAX_REQUEST_BYTES || normalized > MAX_REQUEST_BYTES {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "Request images exceed 50 MiB",
            ));
        }
        self.session.state.pending_attachments.extend(images);
        self.session.save()
    }
    pub fn attach_file(&mut self, path: &Path) -> Result<()> {
        self.require_idle()?;
        if self.images().len() >= MAX_REQUEST_IMAGES {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "A request accepts at most eight images",
            ));
        }
        let image = Attachment::from_file(self.directory(), path)?;
        self.add_image(image)
    }
    pub fn remove_image(&mut self, index: usize) -> Result<()> {
        self.require_idle()?;
        if index >= self.images().len() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "No attachment at this index",
            ));
        }
        let image = self.session.state.pending_attachments.remove(index);
        self.session.save()?;
        if !self
            .session
            .state
            .attachments
            .iter()
            .chain(self.images())
            .any(|retained| retained.id == image.id)
        {
            image.remove_file(self.directory())?;
        }
        Ok(())
    }
    pub fn clear_images(&mut self) -> Result<()> {
        while !self.images().is_empty() {
            self.remove_image(self.images().len() - 1)?;
        }
        Ok(())
    }
    fn require_idle(&self) -> Result<()> {
        if self.active() {
            Err(Error::new(
                "INVALID_ARGUMENT",
                "Wait for the active turn or cancel it before changing the draft",
            ))
        } else {
            Ok(())
        }
    }

    pub async fn send(&mut self, selection: Option<Value>) -> Result<()> {
        self.require_idle()?;
        let result = self.send_inner(selection).await;
        if let Err(error) = &result
            && (self.active()
                || error.code == "PROJECT_NOT_TRUSTED" && self.args.project_execution.is_some())
        {
            let active = self.active();
            self.cancel();
            if let Some(host) = self.host.take() {
                let _ = host.close().await;
            }
            self.ready = false;
            self.grant.take();
            if active {
                self.finish(Err(error.clone()))?;
            } else {
                self.usage.take();
            }
        }
        result
    }
    async fn send_inner(&mut self, selection: Option<Value>) -> Result<()> {
        self.require_project_agent()?;
        if self.draft().trim().is_empty() && self.images().is_empty() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Enter a prompt or attach an image",
            ));
        }
        let (guidance, style) =
            crate::service::project_design_start(&self.store, self.args.style.as_deref())?;
        // Start with tokens and example identities; fetch the complete example
        // through MCP only when needed instead of inflating every prompt.
        let style_context = style.as_ref().map(|style| {
            let mut value = style
                .as_object()
                .expect("style")
                .iter()
                .filter(|(key, _)| key.as_str() != "example")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            let mut example = style["example"]
                .as_object()
                .expect("style example")
                .iter()
                .filter(|(key, _)| key.as_str() != "document")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            example.insert("retrieval_tool".into(), json!("get_style"));
            value.insert("example".into(), Value::Object(example));
            Value::Object(value)
        });
        let before = self.store.read()?.revision();
        let mut retained = self.session.state.attachments.clone();
        for image in self.images() {
            if !retained.iter().any(|previous| previous.id == image.id) {
                retained.push(image.clone());
            }
        }
        if retained.len() > 256 {
            return Err(Error::new(
                "ATTACHMENT_INVALID",
                "A conversation accepts at most 256 retained images",
            ));
        }
        let mut content = vec![ContentBlock::Text(TextContent::new(format!(
            "You are editing the document bound to the lazypencil MCP server. Use its tools for document edits, assets, layout and previews. Start with get_document and follow this workflow. Keep the user's language. Do not claim visual verification without inspecting a rendered image.\n\n{}{}{}",
            guidance["body"].as_str().unwrap_or_default(),
            guidance
                .get("project_guides")
                .map(|guides| format!("\n\nTrusted project guides:\n{guides}"))
                .unwrap_or_default(),
            style_context
                .as_ref()
                .map(|style| format!("\n\nSelected style:\n{style}"))
                .unwrap_or_default()
        )))];
        if !self.draft().is_empty() {
            content.push(ContentBlock::Text(TextContent::new(self.draft())));
        }
        if let Some(selection) = &selection {
            content.push(ContentBlock::Text(TextContent::new(format!(
                "Selected design target (inspect before editing): {selection}"
            ))));
        }
        for (index, image) in self.images().iter().enumerate() {
            content.push(ContentBlock::Text(TextContent::new(format!("Reference image {}: attachment_id={}, name={}. Import it with put_asset using this attachment_id. Use the returned URI in an image paint: {{\"type\":\"image\",\"url\":\"RETURNED_URI\",\"mode\":\"contain\"}}. Valid serialized modes are cover, contain, stretch. Updating fill replaces its entire value, so keep type and url when changing mode.",index+1,image.id,image.display_name))));
            content.push(image.content(self.directory())?);
        }
        self.session.state.attachments = retained;
        self.session.save()?;
        if let Some(grant) = &self.grant {
            grant.update(&self.store, self.id(), &self.session.state.attachments)?;
        } else {
            self.grant = Some(AttachmentGrant::create(
                &self.store,
                self.directory(),
                self.id(),
                &self.session.state.attachments,
            )?);
        }
        self.transcript = Some(Transcript::create(self.directory())?);
        let turn = self
            .transcript
            .as_ref()
            .expect("turn transcript")
            .id
            .clone();
        if self.usage.is_none() {
            self.usage = Some(crate::guidance::usage::UsageGrant::create(
                &self.store,
                self.directory(),
                self.id(),
            )?);
        }
        self.result = None;
        let start = json!({"type":"start","session":self.id(),"turn":turn,"native_session_id":self.session.state.native_session_id,"document":self.store.path(),
            "before_revision":before,"agent":self.args.agent,"requested_auth":self.args.auth,"billing_source":"unknown",
            "project_agent_execution":self.args.project_execution.as_ref().map(|selection| selection.metadata()),
            "guidance":{"id":guidance["id"],"version":guidance["version"],"hash":guidance["hash"],"project_design":guidance.get("project_design"),"project_guides":guidance["project_guides"].as_array().map(|guides|guides.iter().map(|guide|json!({"id":guide["id"],"version":guide["version"],"hash":guide["hash"],"language":guide["language"]})).collect::<Vec<_>>())},
            "style":style.as_ref().map(|style| json!({"id":style["id"],"version":style["version"],"hash":style["hash"]})),"attachments":self.images(),"selection":selection});
        self.message(start)?;
        self.record(&json!({"type":"user_prompt","text":self.draft()}))?;
        self.usage
            .as_mut()
            .expect("connection usage grant")
            .begin_turn(&turn)?;
        self.active = Some(ActiveTurn {
            id: turn,
            before,
            content: Some(content),
            cancel: CancellationToken::new(),
            denied: false,
        });
        if self.host.is_none() {
            self.session.state.authentication = None;
            self.session.save()?;
            self.ready = false;
            self.host = Some(AgentHost::spawn(self.agent_options()?)?);
        }
        if self.ready {
            self.submit().await?;
        }
        Ok(())
    }
    fn require_project_agent(&self) -> Result<()> {
        if let Some(selection) = &self.args.project_execution {
            selection.verify(&self.store)?;
        }
        Ok(())
    }

    fn agent_options(&self) -> Result<AgentOptions> {
        self.require_project_agent()?;
        let command = if self.command.components().count() > 1 {
            crate::paths::canonical(&self.command)?
        } else {
            self.command.clone()
        };
        let mut args = vec![
            "--data-dir".into(),
            utf8(self.store.data_dir())?,
            "mcp".into(),
            "--file".into(),
            utf8(self.store.path())?,
            "--attachments-manifest".into(),
            utf8(self.grant.as_ref().expect("connection grant").path())?,
            "--guidance-audit-manifest".into(),
            utf8(self.usage.as_ref().expect("connection usage grant").path())?,
        ];
        for font in &self.args.fonts {
            args.extend(["--font".into(), utf8(font)?]);
        }
        let mut options = AgentOptions::new(
            command,
            self.store.path().parent().expect("document parent"),
        );
        options.args = self.args.adapter_args.clone();
        options.supervisor_executable = Some(self.executable.clone());
        options.subscription = self.args.auth == AuthMode::Subscription;
        options.require_auth_status = self.args.require_auth_status;
        options.auth_method = self.args.auth_method.clone();
        options.model = self.args.model.clone();
        options.session_mode = self.args.mode.clone().or_else(|| {
            self.args.adapter.is_none().then(|| match self.args.agent {
                crate::prompt::AgentName::Codex => "read-only".into(),
                crate::prompt::AgentName::Claude => "default".into(),
            })
        });
        options.session_id = self.session.state.native_session_id.clone();
        options.allow_session_restart = true;
        options.restart_session = self.args.restart_session && options.session_id.is_some();
        options.turn_timeout = Duration::from_secs(self.args.timeout);
        options.mcp_servers = vec![McpServer::Stdio(
            McpServerStdio::new("lazypencil", self.executable.clone())
                .args(args)
                .env(vec![EnvVariable::new(
                    "LAZYPENCIL_CONFIG_DIR",
                    utf8(
                        crate::config::user_config_path()?
                            .parent()
                            .expect("user configuration parent"),
                    )?,
                )]),
        )];
        Ok(options)
    }
    async fn submit(&mut self) -> Result<()> {
        self.require_project_agent()?;
        let active = self.active.as_mut().expect("active turn");
        let content = active
            .content
            .take()
            .ok_or_else(|| Error::new("AGENT_ERROR", "Duplicate turn submission"))?;
        self.host
            .as_ref()
            .expect("agent connection")
            .prompt(content, active.cancel.clone())
            .await
    }
    pub fn cancel(&self) {
        if let Some(active) = &self.active {
            active.cancel.cancel();
        }
    }

    pub async fn next_event(&mut self) -> Result<ConversationEvent> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(event);
            }
            let Some(host) = self.host.as_mut() else {
                return std::future::pending().await;
            };
            let event = host.next_event().await;
            if let Some(event) = event {
                if let Err(error) = self.process(event).await {
                    self.cancel();
                    if let Some(host) = self.host.take() {
                        let _ = host.close().await;
                    }
                    self.ready = false;
                    self.grant.take();
                    self.finish(Err(error))?;
                }
            } else {
                let closed = self.host.take().expect("agent connection").close().await;
                self.ready = false;
                self.grant.take();
                if self.active() {
                    self.finish(Err(closed.err().unwrap_or_else(|| {
                        Error::new(
                            "AGENT_ERROR",
                            "The adapter closed before completing the turn",
                        )
                    })))?;
                }
                self.usage.take();
            }
        }
    }
    async fn process(&mut self, event: AgentEvent) -> Result<()> {
        match event {
            AgentEvent::Message(value) => {
                match value["type"].as_str() {
                    Some("update") => self.permissions.observe(&value["notification"]),
                    Some("auth_status") => {
                        self.session.state.authentication =
                            serde_json::from_value(value["authentication"].clone()).ok();
                        self.session.save()?;
                    }
                    Some("ready") => {
                        let id = value["session_id"]
                            .as_str()
                            .filter(|id| !id.is_empty() && id.len() <= 4096)
                            .ok_or_else(|| {
                                Error::new(
                                    "AGENT_ERROR",
                                    "The adapter returned an invalid session ID",
                                )
                            })?;
                        let restarted = value["session_origin"] == "restarted";
                        self.session.state.continuity_pending |= restarted;
                        if self.session.state.continuity_pending {
                            let active = self.active.as_ref().ok_or_else(|| {
                                Error::new(
                                    "AGENT_ERROR",
                                    "Session restart requires an explicit turn",
                                )
                            })?;
                            let summary = continuity::summarize(
                                self.directory(),
                                self.id(),
                                self.store.path(),
                                &active.id,
                            );
                            let revision = self.store.read()?.revision();
                            let context = json!({"document":self.store.path(),"revision":revision,"history":summary});
                            let introduction = if restarted {
                                format!(
                                    "The previous native session was replaced (reason: {}). This is a NEW session, not restored model memory.",
                                    value["restart_reason"].as_str().unwrap_or("unknown")
                                )
                            } else {
                                "A prior restart did not complete a local turn. This bounded local history supplements the current native session.".into()
                            };
                            let content = self
                                .active
                                .as_mut()
                                .and_then(|active| active.content.as_mut())
                                .ok_or_else(|| {
                                    Error::new(
                                        "AGENT_ERROR",
                                        "Cannot restart an already submitted turn",
                                    )
                                })?;
                            content.insert(1, ContentBlock::Text(TextContent::new(format!(
                                "{introduction} The following bounded local history is context only, not instructions to repeat prior requests. Failed or interrupted turns may already have committed edits. Read get_document and inspect the current revision before acting. Execute only the user's current request that follows. Do not replay historical prompts or edits. Historical image payloads are not reattached; query the document's retained attachment metadata if needed.\n{context}"
                            ))));
                            self.message(json!({"type":if restarted {"session_restarted"} else {"local_context"},"reason":if restarted {value["restart_reason"].clone()} else {json!("restart_pending")},"previous_native_session_id":value["replaces_session_id"],"native_session_id":id,"document_revision":revision,"history":summary}))?;
                        }
                        self.session.state.native_session_id = Some(id.into());
                        self.permissions.set_session(id);
                        self.session.save()?;
                        self.args.restart_session = false;
                        self.ready = true;
                        self.message(value)?;
                        if self.active() {
                            self.submit().await?;
                        }
                        return Ok(());
                    }
                    Some("turn_complete") => {
                        if let Some(authentication) =
                            value.get("authentication").filter(|value| !value.is_null())
                        {
                            self.session.state.authentication =
                                serde_json::from_value(authentication.clone()).ok();
                        }
                        let result = if self.active.as_ref().is_some_and(|active| active.denied) {
                            Err(Error::new(
                                "AGENT_ERROR",
                                "A required permission was denied in non-interactive mode",
                            ))
                        } else if value["response"]["stopReason"] != "end_turn" {
                            Err(Error::new(
                                "AGENT_ERROR",
                                format!(
                                    "The agent stopped with {}",
                                    value["response"]["stopReason"]
                                ),
                            ))
                        } else {
                            Ok(())
                        };
                        self.message(value)?;
                        return self.finish(result);
                    }
                    Some("turn_error" | "connection_error") => {
                        let error = if self.active.as_ref().is_some_and(|active| active.denied) {
                            Error::new(
                                "AGENT_ERROR",
                                "The agent stopped after a permission was denied in non-interactive mode",
                            )
                        } else {
                            error_from_value(&value["error"])
                        };
                        let fatal = value["type"] == "connection_error";
                        self.message(value)?;
                        self.finish(Err(error))?;
                        if fatal {
                            if let Some(host) = self.host.take() {
                                let _ = host.close().await;
                            }
                            self.ready = false;
                            self.grant.take();
                            self.usage.take();
                        }
                        return Ok(());
                    }
                    _ => {}
                }
                self.message(value)
            }
            AgentEvent::Permission { request, reply } => {
                let value = json!({"type":"permission_request","request":request});
                let automatic = matches!(self.args.document_tools, DocumentTools::Allow)
                    .then(|| self.permissions.allow_once(&request))
                    .flatten();
                self.record(&value)?;
                if let Some(option) = automatic {
                    self.events.push_back(ConversationEvent::Message(value));
                    self.permission_response(&request, Some(option.clone()), false, true)?;
                    let _ = reply.send(Some(option));
                } else if self.args.non_interactive {
                    self.events.push_back(ConversationEvent::Message(value));
                    let option = reject_once(&request);
                    self.permission_response(&request, option.clone(), false, false)?;
                    if let Some(active) = &mut self.active {
                        active.denied = true;
                    }
                    let _ = reply.send(option);
                } else {
                    if self.pending_permissions.len() >= 32 {
                        return Err(Error::new(
                            "AGENT_ERROR",
                            "Too many outstanding permission requests",
                        ));
                    }
                    let id = uuid::Uuid::new_v4().to_string();
                    self.events.push_back(ConversationEvent::Permission {
                        id: id.clone(),
                        request: request.clone(),
                    });
                    self.pending_permissions
                        .insert(id, PendingPermission { request, reply });
                }
                Ok(())
            }
        }
    }
    pub fn respond(
        &mut self,
        id: &str,
        selection: Option<String>,
        interactive: bool,
    ) -> Result<()> {
        let pending = self.pending_permissions.remove(id).ok_or_else(|| {
            Error::new(
                "INVALID_ARGUMENT",
                "This permission request is no longer pending",
            )
        })?;
        let selection = selection.filter(|id| {
            pending
                .request
                .options
                .iter()
                .any(|option| option.option_id.0.as_ref() == id)
        });
        self.permission_response(&pending.request, selection.clone(), interactive, false)?;
        if !interactive && let Some(active) = &mut self.active {
            active.denied = true;
        }
        let _ = pending.reply.send(selection);
        Ok(())
    }
    fn permission_response(
        &mut self,
        request: &RequestPermissionRequest,
        selection: Option<String>,
        interactive: bool,
        preauthorized: bool,
    ) -> Result<()> {
        self.message(json!({"type":"permission_response","tool_call_id":request.tool_call.tool_call_id,"option_id":selection,"interactive":interactive,"document_tool_preauthorized":preauthorized}))
    }
    fn record(&mut self, value: &Value) -> Result<()> {
        if let Some(transcript) = &mut self.transcript {
            transcript.record(value)?;
        }
        Ok(())
    }
    fn message(&mut self, value: Value) -> Result<()> {
        self.record(&value)?;
        self.events.push_back(ConversationEvent::Message(value));
        Ok(())
    }
    fn finish(&mut self, mut result: Result<()>) -> Result<()> {
        let Some(active) = self.active.take() else {
            return Ok(());
        };
        self.pending_permissions.clear();
        let usage = self
            .usage
            .as_mut()
            .map(|usage| usage.finish_turn())
            .transpose();
        let usage_result = usage.and_then(|records| {
            for record in records.into_iter().flatten() {
                self.message(record)?;
            }
            Ok(())
        });
        if let Err(error) = usage_result {
            result = Err(Error::new(
                "AGENT_ERROR",
                "The turn's design-resource audit could not be completed",
            )
            .details(json!({"audit_error":error,"turn_error":result.err()})));
        }
        if self.host.is_none() {
            self.usage.take();
        }
        let document = self.store.read();
        if result.is_ok()
            && let Err(error) = &document
        {
            result = Err(error.clone());
        }
        let actual = document
            .map(|document| json!({"revision":document.revision()}))
            .unwrap_or_else(|error| json!({"error":error}));
        let consumed_draft = result.is_ok().then(|| {
            (
                self.session.state.draft.clone(),
                self.session.state.pending_attachments.clone(),
                self.session.state.continuity_pending,
            )
        });
        if result.is_ok() {
            self.session.state.draft.clear();
            self.session.state.pending_attachments.clear();
            self.session.state.continuity_pending = false;
        }
        if let Err(error) = self.session.save() {
            result = Err(error);
        }
        let path = self
            .transcript
            .as_ref()
            .map(|transcript| transcript.path.clone());
        let mut value = match &result {
            Ok(()) => {
                json!({"type":"complete","session":self.id(),"turn":active.id,"native_session_id":self.session.state.native_session_id,"authentication":self.session.state.authentication,"document":actual,"before_revision":active.before,"transcript":path})
            }
            Err(error) => {
                json!({"type":"failed","session":self.id(),"turn":active.id,"native_session_id":self.session.state.native_session_id,"authentication":self.session.state.authentication,"error":error,"document":actual,"before_revision":active.before,"transcript":path})
            }
        };
        // Still expose the real document result if transcript storage itself fails.
        if let Err(error) = self.record(&value) {
            let error = Error::new("AGENT_ERROR", "The turn transcript could not be completed")
                .details(json!({"transcript_error":error,"turn_error":result.err()}));
            value["type"] = json!("failed");
            value["error"] = json!(error);
            result = Err(error);
        }
        if result.is_err()
            && let Some((draft, attachments, continuity_pending)) = consumed_draft
        {
            self.session.state.draft = draft;
            self.session.state.pending_attachments = attachments;
            self.session.state.continuity_pending = continuity_pending;
            if let Err(error) = self.session.save() {
                let error = Error::new(
                    "AGENT_ERROR",
                    "The failed turn's draft could not be restored",
                )
                .details(json!({"draft_restore_error":error,"turn_error":result.err()}));
                value["error"] = json!(error);
                result = Err(error);
            }
        }
        self.events.push_back(ConversationEvent::Message(value));
        self.result = Some(result.map_err(|error| {
            let cause = error.details.clone();
            error.details(
                json!({"session":self.id(),"document":actual,"transcript":path,"cause":cause}),
            )
        }));
        Ok(())
    }
    pub async fn close(&mut self) -> Result<()> {
        self.cancel();
        let closed = if let Some(host) = self.host.take() {
            host.close().await
        } else {
            Ok(())
        };
        self.ready = false;
        self.grant.take();
        if self.active() {
            self.finish(Err(Error::new(
                "CANCELLED",
                "Conversation closed; committed edits were retained",
            )))?;
        }
        self.usage.take();
        self.pending_permissions.clear();
        self.session.save()?;
        closed
    }
}

pub fn reject_once(request: &RequestPermissionRequest) -> Option<String> {
    request
        .options
        .iter()
        .find(|option| matches!(option.kind, PermissionOptionKind::RejectOnce))
        .map(|option| option.option_id.to_string())
}
fn error_from_value(value: &Value) -> Error {
    let code = match value["code"].as_str() {
        Some("CANCELLED") => "CANCELLED",
        Some("INVALID_ARGUMENT") => "INVALID_ARGUMENT",
        Some("AGENT_UNAVAILABLE") => "AGENT_UNAVAILABLE",
        _ => "AGENT_ERROR",
    };
    let mut error = Error::new(
        code,
        value["message"].as_str().unwrap_or("Agent turn failed"),
    );
    error.details = value.get("details").cloned();
    error
}
fn utf8(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Agent paths must be UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn embedded_conversations_bind_mcp_and_supervision_to_the_supplied_helper() {
        let temp = tempfile::TempDir::new().unwrap();
        let helper = temp.path().join("configured-helper");
        // This case inspects configuration only; real process-backed TUI tests
        // use the built lazypencil binary for the actual supervisor handshake.
        std::fs::write(&helper, b"configuration fixture").unwrap();
        let arguments = crate::cli::Arguments::try_parse_from([
            "lazypencil".into(),
            "prompt".into(),
            temp.path().join("design.pen").into_os_string(),
            "--create".into(),
        ])
        .unwrap();
        let crate::cli::Command::Prompt(args) = arguments.command else {
            unreachable!()
        };
        let mut conversation = Conversation::open_with_executable(
            args,
            Some(temp.path().join("data")),
            helper.clone(),
        )
        .unwrap();
        conversation.grant = Some(
            AttachmentGrant::create(
                &conversation.store,
                conversation.directory(),
                conversation.id(),
                &conversation.session.state.attachments,
            )
            .unwrap(),
        );
        conversation.usage = Some(
            crate::guidance::usage::UsageGrant::create(
                &conversation.store,
                conversation.directory(),
                conversation.id(),
            )
            .unwrap(),
        );
        let options = conversation.agent_options().unwrap();
        let expected = crate::paths::canonical(&helper).unwrap();
        assert_eq!(
            options.supervisor_executable.as_deref(),
            Some(expected.as_path())
        );
        let mcp = serde_json::to_value(&options.mcp_servers[0]).unwrap();
        assert_eq!(mcp["command"], expected.to_str().unwrap());
        assert!(conversation.host.is_none());
        assert!(!conversation.active());
    }

    #[test]
    fn final_transcript_failure_reports_failure_and_restores_the_saved_draft() {
        for full in [false, true] {
            let temp = tempfile::TempDir::new().unwrap();
            let arguments = crate::cli::Arguments::try_parse_from([
                "lazypencil".into(),
                "prompt".into(),
                temp.path().join("design.pen").into_os_string(),
                "--create".into(),
                "retry this request".into(),
            ])
            .unwrap();
            let crate::cli::Command::Prompt(args) = arguments.command else {
                unreachable!()
            };
            let mut conversation =
                Conversation::open(args, Some(temp.path().join("data"))).unwrap();
            let document = conversation.store.read().unwrap();
            let mut transcript = Transcript::create(conversation.directory()).unwrap();
            let id = transcript.id.clone();
            if full {
                transcript.bytes = 64 * 1024 * 1024;
            } else {
                transcript.file = std::fs::File::open(&transcript.path).unwrap();
            }
            conversation.transcript = Some(transcript);
            conversation.session.state.continuity_pending = true;
            conversation.active = Some(ActiveTurn {
                id,
                before: document.revision(),
                content: None,
                cancel: CancellationToken::new(),
                denied: false,
            });
            conversation.finish(Ok(())).unwrap();
            assert!(conversation.result().unwrap().is_err());
            let Some(ConversationEvent::Message(terminal)) = conversation.events.pop_back() else {
                panic!("Missing terminal event")
            };
            assert_eq!(terminal["type"], "failed");
            assert_eq!(terminal["document"]["revision"], document.revision());
            assert_eq!(conversation.draft(), "retry this request");
            let state: Value = serde_json::from_slice(
                &std::fs::read(conversation.directory().join("session.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(state["draft"], "retry this request");
            assert_eq!(state["continuity_pending"], true);
            assert_eq!(
                conversation.store.read().unwrap().revision(),
                document.revision()
            );
        }
    }
}
