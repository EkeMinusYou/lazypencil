//! ACP v1 client shared by command-line and terminal frontends.

use crate::{Error, Result};
use acp::schema::{ProtocolVersion, v1::*};
use agent_client_protocol as acp;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LinesCodec},
    sync::CancellationToken,
};

const MAX_LINE: usize = 96 * 1024 * 1024;

pub mod auth;
pub mod permissions;
mod settings;
use auth::{AuthPolicy, AuthReport};

#[derive(Debug, Clone)]
pub struct AgentOptions {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub mcp_servers: Vec<McpServer>,
    pub session_id: Option<String>,
    pub auth_method: Option<String>,
    /// Remove inherited API-key overrides for subscription requests.
    pub subscription: bool,
    /// Refuse to send prompts unless the adapter reports a matching authentication route.
    pub require_auth_status: bool,
    pub auth_status_timeout: Duration,
    pub session_mode: Option<String>,
    pub model: Option<String>,
    pub setup_timeout: Duration,
    pub turn_timeout: Duration,
    pub cancel_grace: Duration,
}

impl AgentOptions {
    pub fn new(command: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: vec![],
            cwd: cwd.into(),
            mcp_servers: vec![],
            session_id: None,
            auth_method: None,
            subscription: true,
            require_auth_status: false,
            auth_status_timeout: Duration::from_secs(5),
            session_mode: None,
            model: None,
            setup_timeout: Duration::from_secs(60),
            turn_timeout: Duration::from_secs(600),
            cancel_grace: Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
pub enum AgentEvent {
    Message(Value),
    Permission {
        request: Box<RequestPermissionRequest>,
        reply: oneshot::Sender<Option<String>>,
    },
}

struct Turn {
    content: Vec<ContentBlock>,
    cancel: CancellationToken,
}

/// Owns one adapter connection and one session. Sequential prompts reuse both.
pub struct AgentHost {
    commands: Option<mpsc::Sender<Turn>>,
    events: mpsc::Receiver<AgentEvent>,
    shutdown: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl AgentHost {
    pub fn spawn(options: AgentOptions) -> Result<Self> {
        if !options.cwd.is_absolute() || !options.cwd.is_dir() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Agent cwd must be an existing absolute directory",
            ));
        }
        if options.setup_timeout.is_zero()
            || options.turn_timeout.is_zero()
            || options.cancel_grace.is_zero()
            || options.auth_status_timeout.is_zero()
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Agent time limits must be positive",
            ));
        }
        let (commands, receiver) = mpsc::channel(1);
        let (sender, events) = mpsc::channel(32);
        let shutdown = CancellationToken::new();
        let stop = shutdown.clone();
        let task = tokio::spawn(async move {
            let reporting = sender.clone();
            let result = run(options, receiver, sender, stop).await;
            if let Err(error) = &result {
                let _ = reporting
                    .send(AgentEvent::Message(
                        json!({"type":"connection_error","error":error}),
                    ))
                    .await;
            }
            result
        });
        Ok(Self {
            commands: Some(commands),
            events,
            shutdown,
            task: Some(task),
        })
    }

    pub async fn next_event(&mut self) -> Option<AgentEvent> {
        self.events.recv().await
    }

    pub async fn prompt(
        &self,
        content: Vec<ContentBlock>,
        cancel: CancellationToken,
    ) -> Result<()> {
        if content.is_empty() || content.len() > 32 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "A prompt must contain 1..32 content blocks",
            ));
        }
        self.commands
            .as_ref()
            .ok_or_else(|| agent_error("Agent is closed"))?
            .send(Turn { content, cancel })
            .await
            .map_err(|_| agent_error("Agent connection has closed"))
    }

    pub async fn close(mut self) -> Result<()> {
        self.commands.take();
        // A frontend may stop reading while closing; never block cleanup on its event queue.
        self.events.close();
        self.shutdown.cancel();
        self.task
            .take()
            .expect("live task")
            .await
            .map_err(|_| agent_error("Agent host terminated unexpectedly"))?
    }
}

impl Drop for AgentHost {
    fn drop(&mut self) {
        // Do not abort the task: its cancellation path must reap the adapter process.
        self.shutdown.cancel();
    }
}

async fn run(
    options: AgentOptions,
    commands: mpsc::Receiver<Turn>,
    events: mpsc::Sender<AgentEvent>,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut command = Command::new(&options.command);
    command
        .args(&options.args)
        .current_dir(&options.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if options.subscription {
        for key in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "DEFAULT_AUTH_REQUEST",
            "OPENAI_BASE_URL",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ] {
            command.env_remove(key);
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|error| {
        Error::new(
            "AGENT_UNAVAILABLE",
            format!(
                "Cannot start ACP adapter {}: {error}",
                options.command.display()
            ),
        )
    })?;
    let mut process = AgentProcess::new(child);
    let stdin = process.child.stdin.take().expect("piped stdin");
    let stdout = process.child.stdout.take().expect("piped stdout");
    let mut stderr = process.child.stderr.take().expect("piped stderr");
    // Drain continuously without retaining credentials or unbounded diagnostics from the adapter.
    let mut drain = tokio::spawn(async move {
        let mut count = 0u64;
        let mut buffer = [0u8; 8192];
        while let Ok(size) = stderr.read(&mut buffer).await {
            if size == 0 {
                break;
            }
            count = count.saturating_add(size as u64);
        }
        count
    });
    let transport = acp::Lines::new(
        SinkExt::<String>::sink_map_err(
            FramedWrite::new(stdin, LinesCodec::new_with_max_length(MAX_LINE)),
            std::io::Error::other,
        ),
        FramedRead::new(stdout, LinesCodec::new_with_max_length(16 * 1024 * 1024)).map(|line| {
            let line = line.map_err(std::io::Error::other)?;
            validate_frame(&line)?;
            Ok(line)
        }),
    );
    let current_cancel = Arc::new(Mutex::new(None::<CancellationToken>));
    let permission_cancel = current_cancel.clone();
    let permission_events = events.clone();
    let notification_events = events.clone();
    let permission_shutdown = shutdown.clone();
    let (auth_sender, auth_receiver) = watch::channel(None::<AuthReport>);
    let result = acp::Client.builder()
        .on_receive_notification(async move |notification: acp::UntypedMessage, _cx| {
            // Preserve new, non-breaking update variants and extension notifications.
            let value = if notification.method == "_auth/status_update" {
                let report = AuthReport::from_notification(&notification.params);
                auth_sender.send_replace(Some(report.clone()));
                // Only retain route metadata, never the account email, credentials, or vendor extras.
                json!({"type":"auth_status","authentication":report})
            } else if notification.method == "session/update" {
                json!({"type":"update","notification":notification.params})
            } else {
                json!({"type":"agent_notification","method":notification.method,"params":notification.params})
            };
            notification_events.send(AgentEvent::Message(value))
                .await.map_err(acp::Error::into_internal_error)?;
            Ok(())
        }, acp::on_receive_notification!())
        .on_receive_request(async move |request: RequestPermissionRequest, responder, _cx| {
            let cancel = permission_cancel.lock().expect("cancellation lock").clone();
            let (reply, answer) = oneshot::channel();
            let options = request.options.clone();
            let sent = permission_events.send(AgentEvent::Permission { request: Box::new(request), reply });
            let selected = tokio::select! {
                _ = permission_shutdown.cancelled() => None,
                _ = async { if let Some(cancel) = cancel { cancel.cancelled().await; } else { std::future::pending::<()>().await; } } => None,
                answer = async { sent.await.ok()?; answer.await.ok().flatten() } => answer,
            };
            let outcome = selected.and_then(|id| options.iter().find(|option| option.option_id.to_string() == id).map(|option| option.option_id.clone()))
                .map(|id| RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)))
                .unwrap_or(RequestPermissionOutcome::Cancelled);
            responder.respond(RequestPermissionResponse::new(outcome))
        }, acp::on_receive_request!())
        .connect_with(transport, async move |connection: acp::ConnectionTo<acp::Agent>| {
            conversation(&connection, &options, commands, &events, &shutdown, current_cancel, auth_receiver).await
                .map_err(|error| acp::Error::internal_error().data(serde_json::to_value(error).unwrap_or(Value::Null)))
        }).await;
    process.stop().await;
    let stderr_bytes = match tokio::time::timeout(Duration::from_secs(1), &mut drain).await {
        Ok(Ok(bytes)) => bytes,
        _ => {
            drain.abort();
            0
        }
    };
    result.map_err(|error| {
        // Preserve our structured error across the SDK's connection boundary.
        let encoded = serde_json::to_value(&error).unwrap_or(Value::Null);
        let data = &encoded["data"];
        let code = match data["code"].as_str() {
            Some("CANCELLED") => "CANCELLED",
            Some("AGENT_UNAVAILABLE") => "AGENT_UNAVAILABLE",
            Some("INVALID_ARGUMENT") => "INVALID_ARGUMENT",
            _ => "AGENT_ERROR",
        };
        Error::new(
            code,
            data["message"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("ACP connection failed: {error}")),
        )
        .details(json!({"adapter_stderr_bytes":stderr_bytes,"cause":data.get("details")}))
    })
}

#[allow(clippy::too_many_arguments)]
async fn conversation(
    connection: &acp::ConnectionTo<acp::Agent>,
    options: &AgentOptions,
    mut commands: mpsc::Receiver<Turn>,
    events: &mpsc::Sender<AgentEvent>,
    shutdown: &CancellationToken,
    current_cancel: Arc<Mutex<Option<CancellationToken>>>,
    mut authentication: watch::Receiver<Option<AuthReport>>,
) -> Result<()> {
    let setup = async {
        let initialized = connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(Implementation::new("lazypencil", env!("CARGO_PKG_VERSION"))),
            )
            .block_task()
            .await
            .map_err(protocol_error)?;
        if initialized.protocol_version != ProtocolVersion::V1 {
            return Err(agent_error("The adapter did not negotiate ACP v1"));
        }
        let auth_supported = initialized
            .agent_capabilities
            .meta
            .as_ref()
            .is_some_and(|meta| meta.contains_key("authStatus"));
        emit(
            events,
            json!({"type":"initialized", "response":initialized, "billing_source":"unknown"}),
        )
        .await?;
        if let Some(method) = &options.auth_method {
            let advertised = serde_json::to_value(&initialized.auth_methods)
                .map_err(|e| agent_error(e.to_string()))?;
            if !advertised.as_array().is_some_and(|methods| {
                methods
                    .iter()
                    .any(|candidate| candidate["id"] == *method && candidate["type"] != "terminal")
            }) {
                return Err(agent_error(
                    "The requested authentication method is not advertised by the adapter",
                ));
            }
            connection
                .send_request(AuthenticateRequest::new(method.clone()))
                .block_task()
                .await
                .map_err(protocol_error)?;
        }
        let (session, response) = if let Some(id) = &options.session_id {
            if !initialized.agent_capabilities.load_session {
                return Err(agent_error(
                    "This adapter cannot reload sessions; a local transcript is not a resumable agent session",
                ));
            }
            let response = connection
                .send_request(
                    LoadSessionRequest::new(id.clone(), options.cwd.clone())
                        .mcp_servers(options.mcp_servers.clone()),
                )
                .block_task()
                .await
                .map_err(protocol_error)?;
            (SessionId::new(id.clone()), serde_json::to_value(response))
        } else {
            let response = connection
                .send_request(
                    NewSessionRequest::new(options.cwd.clone())
                        .mcp_servers(options.mcp_servers.clone()),
                )
                .block_task()
                .await
                .map_err(protocol_error)?;
            (response.session_id.clone(), serde_json::to_value(response))
        };
        let mut response = response.map_err(|e| agent_error(e.to_string()))?;
        settings::apply(
            connection,
            &session,
            &mut response,
            options.session_mode.as_deref(),
            options.model.as_deref(),
        )
        .await?;
        if (auth_supported || options.require_auth_status) && authentication.borrow().is_none() {
            let _ = tokio::time::timeout(options.auth_status_timeout, async {
                while authentication.borrow().is_none() {
                    if authentication.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }
        let policy = AuthPolicy {
            subscription: options.subscription,
            require_report: options.require_auth_status || auth_supported,
        };
        let reported_auth = authentication.borrow().clone();
        emit(events, json!({"type":"ready","session_id":session,"response":response,"authentication":reported_auth,"auth_status_required":policy.require_report})).await?;
        Ok((
            session,
            initialized.agent_capabilities.prompt_capabilities.image,
            policy,
        ))
    };
    let (session, images, auth_policy) = tokio::select! {
        biased;
        _ = shutdown.cancelled() => return Ok(()),
        result = tokio::time::timeout(options.setup_timeout, setup) => result.map_err(|_| agent_error("ACP initialization or session setup timed out"))??,
    };
    loop {
        let turn = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Ok(()),
            turn = commands.recv() => match turn { Some(turn) => turn, None => return Ok(()) },
        };
        if turn.cancel.is_cancelled() {
            emit(events, json!({"type":"turn_error","error":Error::new("CANCELLED", "The queued prompt was cancelled before sending")})).await?;
            continue;
        }
        let reported_auth = authentication.borrow().clone();
        if let Err(error) = auth_policy.check(reported_auth.as_ref()) {
            emit(events, json!({"type":"turn_error","error":error})).await?;
            continue;
        }
        if turn
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Image(_)))
            && !images
        {
            emit(events, json!({"type":"turn_error","error":agent_error("The adapter does not support image prompts; no text-only fallback was sent")})).await?;
            continue;
        }
        *current_cancel.lock().expect("cancellation lock") = Some(turn.cancel.clone());
        let request = connection
            .send_request(PromptRequest::new(session.clone(), turn.content))
            .block_task();
        tokio::pin!(request);
        let result = tokio::select! {
            biased;
            _ = shutdown.cancelled() => Err(TurnStop::Cancelled),
            _ = turn.cancel.cancelled() => Err(TurnStop::Cancelled),
            _ = tokio::time::sleep(options.turn_timeout) => Err(TurnStop::TimedOut),
            error = auth::wait_for_conflict(&mut authentication, auth_policy) => Err(TurnStop::Authentication(error)),
            result = &mut request => Ok(result),
        };
        let result = match result {
            Ok(result) => result.map_err(protocol_error).and_then(|response| {
                if response.stop_reason == StopReason::Cancelled {
                    Err(Error::new("CANCELLED", "The agent cancelled this turn"))
                } else {
                    Ok(response)
                }
            }),
            Err(reason) => {
                turn.cancel.cancel();
                let cancellation = async {
                    connection
                        .send_notification(CancelNotification::new(session.clone()))
                        .map_err(protocol_error)?;
                    (&mut request).await.map_err(protocol_error)
                };
                if tokio::time::timeout(options.cancel_grace, cancellation)
                    .await
                    .is_err()
                {
                    return Err(if matches!(reason, TurnStop::TimedOut) {
                        agent_error(
                            "Agent turn timed out; the adapter did not finish after session/cancel and was terminated",
                        )
                    } else if let TurnStop::Authentication(error) = reason {
                        error
                    } else {
                        Error::new(
                            "CANCELLED",
                            "The adapter did not finish after session/cancel and was terminated; committed edits were retained",
                        )
                    });
                }
                Err(if matches!(reason, TurnStop::TimedOut) {
                    agent_error("Agent turn timed out; committed document edits were retained")
                } else if let TurnStop::Authentication(error) = reason {
                    error
                } else {
                    Error::new(
                        "CANCELLED",
                        "Agent turn stopped; committed document edits were retained",
                    )
                })
            }
        };
        *current_cancel.lock().expect("cancellation lock") = None;
        match result {
            Ok(response) => {
                let reported_auth = authentication.borrow().clone();
                match auth_policy.check(reported_auth.as_ref()) {
                    Ok(()) => emit(events, json!({"type":"turn_complete","response":response,"authentication":reported_auth})).await?,
                    Err(error) => emit(events, json!({"type":"turn_error","error":error})).await?,
                }
            }
            Err(error) => emit(events, json!({"type":"turn_error","error":error})).await?,
        }
    }
}

enum TurnStop {
    Cancelled,
    TimedOut,
    Authentication(Error),
}

async fn emit(events: &mpsc::Sender<AgentEvent>, value: Value) -> Result<()> {
    events
        .send(AgentEvent::Message(value))
        .await
        .map_err(|_| agent_error("The agent event receiver closed"))
}
fn agent_error(message: impl Into<String>) -> Error {
    Error::new("AGENT_ERROR", message)
}
fn protocol_error(error: acp::Error) -> Error {
    agent_error(format!("ACP request failed: {error}"))
}

fn validate_frame(line: &str) -> std::io::Result<()> {
    fn valid(value: &Value) -> bool {
        if let Some(batch) = value.as_array() {
            return !batch.is_empty()
                && batch.len() <= 128
                && batch
                    .iter()
                    .all(|message| message.is_object() && valid(message));
        }
        if !value.is_object() || value["jsonrpc"] != "2.0" {
            return false;
        }
        let id = value.get("id");
        let valid_id = id.is_some_and(|id| id.is_string() || id.is_i64() || id.is_u64());
        if value.get("method").is_some() {
            return value["method"].is_string()
                && (id.is_none() || valid_id)
                && value.get("result").is_none()
                && value.get("error").is_none();
        }
        match (value.get("result"), value.get("error")) {
            (Some(_), None) => valid_id,
            (None, Some(error)) => {
                (valid_id || id.is_some_and(Value::is_null))
                    && error["code"].is_i64()
                    && error["message"].is_string()
            }
            _ => false,
        }
    }
    let value: Value = serde_json::from_str(line).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Adapter stdout contained invalid JSON",
        )
    })?;
    if !valid(&value) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Adapter stdout contained an invalid JSON-RPC envelope",
        ));
    }
    Ok(())
}

struct AgentProcess {
    child: Child,
    group: Option<u32>,
}
impl AgentProcess {
    fn new(child: Child) -> Self {
        let group = child.id();
        Self { child, group }
    }
    async fn stop(&mut self) {
        self.kill_group();
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        self.group = None;
    }
    fn kill_group(&self) {
        #[cfg(unix)]
        if let Some(group) = self.group.and_then(|id| i32::try_from(id).ok()) {
            // The adapter starts in its own process group; never signal the parent's group.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
}
impl Drop for AgentProcess {
    fn drop(&mut self) {
        self.kill_group();
    }
}
