//! ACP v1 client shared by command-line and terminal frontends.

use crate::{Error, Result};
use acp::schema::{ProtocolVersion, v1::*};
use agent_client_protocol as acp;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc, oneshot, watch},
};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LinesCodec},
    sync::CancellationToken,
};

const MAX_LINE: usize = 96 * 1024 * 1024;

pub mod auth;
mod diagnostics;
pub mod permissions;
mod preparation;
mod process;
mod settings;
pub(crate) use settings::normalize_modes;
mod steering;
use auth::{AuthPolicy, AuthReport};
use process::AgentProcess;

#[derive(Debug, Clone)]
pub struct AgentOptions {
    pub command: PathBuf,
    pub args: Vec<String>,
    /// Host binary implementing the private process supervisor entry point.
    /// Defaults to this executable; library hosts must supply the lazypencil binary.
    pub supervisor_executable: Option<PathBuf>,
    pub cwd: PathBuf,
    pub mcp_servers: Vec<McpServer>,
    pub session_id: Option<String>,
    /// Allow a new native session when the adapter explicitly cannot load sessions.
    /// Frontends must supply current-document context and disclose the restart.
    pub allow_session_restart: bool,
    /// Explicitly replace a saved native session, retaining frontend history.
    pub restart_session: bool,
    pub auth_method: Option<String>,
    /// Remove inherited API-key overrides for subscription requests.
    pub subscription: bool,
    /// Refuse to send prompts unless the adapter reports a matching authentication route.
    pub require_auth_status: bool,
    pub auth_status_timeout: Duration,
    pub session_mode: Option<String>,
    pub model: Option<String>,
    /// Override the advertised Fast mode selector; None preserves adapter defaults.
    pub fast: Option<bool>,
    pub setup_timeout: Duration,
    pub turn_timeout: Duration,
    pub cancel_grace: Duration,
    /// Existing private directory for detailed ACP error files. None disables logging.
    pub diagnostic_directory: Option<PathBuf>,
}

impl AgentOptions {
    pub fn new(command: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: vec![],
            supervisor_executable: std::env::current_exe().ok(),
            cwd: cwd.into(),
            mcp_servers: vec![],
            session_id: None,
            allow_session_restart: false,
            restart_session: false,
            auth_method: None,
            subscription: true,
            require_auth_status: false,
            auth_status_timeout: Duration::from_secs(5),
            session_mode: None,
            model: None,
            fast: None,
            setup_timeout: Duration::from_secs(60),
            turn_timeout: Duration::from_secs(600),
            cancel_grace: Duration::from_secs(5),
            diagnostic_directory: None,
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

/// Initialization-only diagnostics. No session, authentication flow or prompt is started.
#[derive(Debug, Default)]
pub struct AgentInspection {
    pub protocol_version: Option<u16>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub authentication_supported: bool,
    pub steering_supported: bool,
    pub authentication: Option<AuthReport>,
    pub failure: Option<Error>,
}

struct Turn {
    content: Vec<ContentBlock>,
    cancel: CancellationToken,
    generation: u64,
}

struct Commands {
    turns: mpsc::Receiver<Turn>,
    modes: mpsc::Receiver<String>,
    steering: mpsc::Receiver<steering::Request>,
    steering_supported: Arc<AtomicBool>,
    steering_turn: Arc<Mutex<Option<u64>>>,
}

/// Owns one adapter connection and one session. Sequential prompts reuse both.
pub struct AgentHost {
    commands: Option<mpsc::Sender<Turn>>,
    modes: mpsc::Sender<String>,
    steering: mpsc::Sender<steering::Request>,
    steering_supported: Arc<AtomicBool>,
    steering_turn: Arc<Mutex<Option<u64>>>,
    generation: AtomicU64,
    events: mpsc::Receiver<AgentEvent>,
    shutdown: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    authentication: watch::Receiver<Option<AuthReport>>,
    deferred_options: Option<oneshot::Sender<AgentOptions>>,
    prepared_options: Option<AgentOptions>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Conversation,
    Authentication,
}

impl AgentHost {
    pub fn spawn(options: AgentOptions) -> Result<Self> {
        Self::spawn_for(options, Purpose::Conversation)
    }

    fn spawn_for(options: AgentOptions, purpose: Purpose) -> Result<Self> {
        Self::spawn_task(options, purpose, None, None)
    }

    fn spawn_task(
        options: AgentOptions,
        purpose: Purpose,
        deferred: Option<oneshot::Receiver<AgentOptions>>,
        finished: Option<oneshot::Sender<()>>,
    ) -> Result<Self> {
        if options.restart_session && options.session_id.is_none() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Restarting requires a saved native session ID",
            ));
        }
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
        let (modes, mode_receiver) = mpsc::channel(1);
        let (steering, steering_receiver) = mpsc::channel(1);
        let steering_supported = Arc::new(AtomicBool::new(false));
        let steering_turn = Arc::new(Mutex::new(None));
        let controls = Commands {
            turns: receiver,
            modes: mode_receiver,
            steering: steering_receiver,
            steering_supported: steering_supported.clone(),
            steering_turn: steering_turn.clone(),
        };
        let (sender, events) = mpsc::channel(32);
        let shutdown = CancellationToken::new();
        let stop = shutdown.clone();
        let (auth_sender, authentication) = watch::channel(None);
        let task = tokio::spawn(async move {
            let reporting = sender.clone();
            let result = run(
                options,
                controls,
                sender,
                stop,
                auth_sender,
                purpose,
                deferred,
            )
            .await;
            if let Err(error) = &result {
                let _ = reporting
                    .send(AgentEvent::Message(
                        json!({"type":"connection_error","error":error}),
                    ))
                    .await;
            }
            if let Some(finished) = finished {
                let _ = finished.send(());
            }
            result
        });
        Ok(Self {
            commands: Some(commands),
            modes,
            steering,
            steering_supported,
            steering_turn,
            generation: AtomicU64::new(0),
            events,
            shutdown,
            task: Some(task),
            authentication,
            deferred_options: None,
            prepared_options: None,
        })
    }

    pub fn authentication(&self) -> Option<AuthReport> {
        self.authentication.borrow().clone()
    }

    /// Initialize the selected runtime without a session, tools, login or prompt.
    pub async fn inspect_authentication(
        options: AgentOptions,
        cancel: CancellationToken,
    ) -> Result<Option<AuthReport>> {
        let inspection = Self::inspect(options, cancel).await?;
        match inspection.failure {
            Some(error) => Err(error),
            None => Ok(inspection.authentication),
        }
    }

    /// Retain negotiated facts even when account reporting fails or times out.
    pub async fn inspect(
        mut options: AgentOptions,
        cancel: CancellationToken,
    ) -> Result<AgentInspection> {
        options.mcp_servers.clear();
        options.session_id = None;
        options.restart_session = false;
        options.auth_method = None;
        let mut host = Self::spawn_for(options, Purpose::Authentication)?;
        let mut inspection = AgentInspection::default();
        let result = loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break Err(Error::new("CANCELLED", "Account check cancelled")),
                event = host.next_event() => match event {
                    Some(AgentEvent::Message(value)) if value["type"] == "connection_error" => {
                        inspection.failure = Some(agent_error(value["error"]["message"].as_str()
                            .unwrap_or("Account check failed")));
                        break Ok(());
                    },
                    Some(AgentEvent::Message(value)) if value["type"] == "inspection" => {
                        inspection.protocol_version = Some(1);
                        inspection.name = value["name"].as_str().map(str::to_owned);
                        inspection.version = value["version"].as_str().map(str::to_owned);
                        inspection.authentication_supported = value["authentication_supported"] == true;
                        inspection.steering_supported = value["steering_supported"] == true;
                    },
                    Some(AgentEvent::Permission { reply, .. }) => { let _ = reply.send(None); },
                    Some(_) => {},
                    None => break Ok(()),
                }
            }
        };
        inspection.authentication = host.authentication();
        let closed = host.close().await;
        result?;
        if inspection.failure.is_none() {
            inspection.failure = closed.err();
        }
        Ok(inspection)
    }

    pub async fn next_event(&mut self) -> Option<AgentEvent> {
        self.events.recv().await
    }

    /// Changes are consumed only between prompts, never inside an active turn.
    pub fn set_mode(&self, mode: String) -> Result<()> {
        self.modes
            .try_send(mode)
            .map_err(|_| agent_error("A mode change is pending or the agent is closed"))
    }

    pub async fn prompt(
        &self,
        content: Vec<ContentBlock>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let (commands, turn) = self.prepare_turn(content, cancel)?;
        commands
            .send(turn)
            .await
            .map_err(|_| agent_error("Agent connection has closed"))
    }
    fn prepare_turn(
        &self,
        content: Vec<ContentBlock>,
        cancel: CancellationToken,
    ) -> Result<(mpsc::Sender<Turn>, Turn)> {
        if self.deferred_options.is_some() {
            return Err(agent_error("Choose a document before submitting a prompt"));
        }
        if content.is_empty() || content.len() > 32 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "A prompt must contain 1..32 content blocks",
            ));
        }
        let commands = self
            .commands
            .as_ref()
            .ok_or_else(|| agent_error("Agent is closed"))?
            .clone();
        Ok((
            commands,
            Turn {
                content,
                cancel,
                generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
            },
        ))
    }

    /// Retain submission across frontend select cancellation while waiting for
    /// an external durability prerequisite. Cancellation still prevents delivery.
    pub(crate) fn prompt_after(
        &self,
        content: Vec<ContentBlock>,
        cancel: CancellationToken,
        ready: oneshot::Receiver<Result<()>>,
    ) -> Result<oneshot::Receiver<Result<()>>> {
        let (commands, turn) = self.prepare_turn(content, cancel.clone())?;
        let (reply, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let sent = async {
                ready
                    .await
                    .map_err(|_| agent_error("Prompt durability worker stopped"))??;
                commands
                    .send(turn)
                    .await
                    .map_err(|_| agent_error("Agent connection has closed"))
            };
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::new("CANCELLED", "Prompt cancelled before delivery")),
                result = sent => result,
            };
            let _ = reply.send(result);
        });
        Ok(receiver)
    }

    /// Whether this adapter supports steering with a host-owned idle fallback.
    pub fn supports_steering(&self) -> bool {
        self.steering_supported.load(Ordering::Relaxed)
    }

    pub fn can_steer(&self) -> bool {
        self.supports_steering() && self.steering_turn.lock().expect("steering lock").is_some()
    }

    /// Enqueue mid-turn input without blocking event or permission handling.
    pub fn steer(&self, id: String, content: Vec<ContentBlock>) -> Result<()> {
        if !self.supports_steering() {
            return Err(agent_error(
                "This adapter does not support safe mid-turn input",
            ));
        }
        if content.is_empty() || content.len() > 32 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "A prompt must contain 1..32 content blocks",
            ));
        }
        let turn = self.steering_turn.lock().expect("steering lock");
        let generation = turn.ok_or_else(|| agent_error("The active turn has already finished"))?;
        self.steering
            .try_send(steering::Request {
                id,
                content,
                generation,
            })
            .map_err(|_| {
                agent_error("Another mid-turn input is pending or the agent connection has closed")
            })
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
    commands: Commands,
    events: mpsc::Sender<AgentEvent>,
    shutdown: CancellationToken,
    auth_sender: watch::Sender<Option<AuthReport>>,
    purpose: Purpose,
    deferred: Option<oneshot::Receiver<AgentOptions>>,
) -> Result<()> {
    let setup_started = std::time::Instant::now();
    let mut process = tokio::select! {
        result = AgentProcess::spawn(&options) => result?,
        _ = shutdown.cancelled() => return Ok(()),
    };
    let mut options = options;
    let diagnostic_directory = Arc::new(Mutex::new(options.diagnostic_directory.clone()));
    let connection_diagnostics = diagnostic_directory.clone();
    options.setup_timeout = options
        .setup_timeout
        .saturating_sub(setup_started.elapsed());
    let child = process.child_mut();
    let process_id = child.id();
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
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
    let mode_updates = Arc::new(Mutex::new(Value::Null));
    let notification_modes = mode_updates.clone();
    let permission_shutdown = shutdown.clone();
    let auth_receiver = auth_sender.subscribe();
    // Host errors cross the SDK boundary out of band. Adapter-controlled error
    // data must never impersonate a host error or become public diagnostics.
    let host_error = Arc::new(Mutex::new(None));
    let captured_error = host_error.clone();
    let result = acp::Client.builder()
        .on_receive_notification(async move |notification: acp::UntypedMessage, _cx| {
            // Preserve new, non-breaking update variants and extension notifications.
            let value = if notification.method == "_auth/status_update" {
                let report = AuthReport::from_notification(&notification.params);
                auth_sender.send_replace(Some(report.clone()));
                // Serialize route metadata only. Display identity stays in the
                // live watch channel; credentials and vendor extras are discarded.
                json!({"type":"auth_status","authentication":report})
            } else if notification.method == "session/update" {
                let update=&notification.params["update"];
                if matches!(update["sessionUpdate"].as_str(),Some("config_option_update"|"current_mode_update")) {
                    let mut snapshot=notification_modes.lock().expect("mode updates");
                    if snapshot["sessionId"]!=notification.params["sessionId"] { *snapshot=json!({"sessionId":notification.params["sessionId"]}); }
                    if update["sessionUpdate"]=="config_option_update" {snapshot["configOptions"]=update["configOptions"].clone();}
                    else {snapshot["currentModeId"]=update["currentModeId"].clone();}
                }
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
            let result = async {
                let options = if let Some(deferred) = deferred {
                    emit(&events, json!({"type":"agent_prepared","process_id":process_id})).await?;
                    tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => return Ok(()),
                        options = deferred => match options { Ok(options) => options, Err(_) => return Ok(()) },
                    }
                } else { options };
                *connection_diagnostics.lock().expect("diagnostic directory lock") = options.diagnostic_directory.clone();
                conversation(&connection, &options, ConversationChannels {
                    commands,
                    events: &events,
                    shutdown: &shutdown,
                    current_cancel,
                    authentication: auth_receiver,
                    mode_updates,
                }, purpose).await
            }.await;
            result.map_err(|error| {
                *captured_error.lock().expect("host error lock") = Some(error);
                acp::Error::internal_error()
            })
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
        let mut error = host_error
            .lock()
            .expect("host error lock")
            .take()
            .unwrap_or_else(|| {
                protocol_error(
                    "connection",
                    error,
                    diagnostic_directory
                        .lock()
                        .expect("diagnostic directory lock")
                        .as_deref(),
                )
            });
        let mut details = error.details.take().unwrap_or_else(|| json!({}));
        if !details.is_object() {
            details = json!({"cause":details});
        }
        details["adapter_stderr_bytes"] = json!(stderr_bytes);
        error.details = Some(details);
        error
    })
}

struct ConversationChannels<'a> {
    commands: Commands,
    events: &'a mpsc::Sender<AgentEvent>,
    shutdown: &'a CancellationToken,
    current_cancel: Arc<Mutex<Option<CancellationToken>>>,
    authentication: watch::Receiver<Option<AuthReport>>,
    mode_updates: Arc<Mutex<Value>>,
}

async fn conversation(
    connection: &acp::ConnectionTo<acp::Agent>,
    options: &AgentOptions,
    channels: ConversationChannels<'_>,
    purpose: Purpose,
) -> Result<()> {
    let ConversationChannels {
        commands,
        events,
        shutdown,
        current_cancel,
        mut authentication,
        mode_updates,
    } = channels;
    let Commands {
        mut turns,
        mut modes,
        mut steering,
        steering_supported,
        steering_turn,
    } = commands;
    let setup = async {
        let initialized = connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(Implementation::new("lazypencil", env!("CARGO_PKG_VERSION"))),
            )
            .block_task()
            .await
            .map_err(|error| {
                protocol_error("initialize", error, options.diagnostic_directory.as_deref())
            })?;
        if initialized.protocol_version != ProtocolVersion::V1 {
            return Err(agent_error("The adapter did not negotiate ACP v1"));
        }
        steering_supported.store(steering::supported(&initialized), Ordering::Relaxed);
        let auth_supported = initialized
            .agent_capabilities
            .meta
            .as_ref()
            .is_some_and(|meta| meta.contains_key("authStatus"));
        if purpose == Purpose::Authentication {
            // Export only bounded implementation facts, never vendor metadata.
            let info = initialized.agent_info.as_ref();
            emit(
                events,
                json!({"type":"inspection",
                    "name":info.map(|info| info.name.chars().take(256).collect::<String>()),
                    "version":info.map(|info| info.version.chars().take(128).collect::<String>()),
                    "authentication_supported":auth_supported,
                    "steering_supported":steering::supported(&initialized),
                }),
            )
            .await?;
            if auth_supported && authentication.borrow().is_none() {
                tokio::time::timeout(options.auth_status_timeout, async {
                    while authentication.borrow().is_none() {
                        authentication
                            .changed()
                            .await
                            .map_err(|_| agent_error("Account reporting ended"))?;
                    }
                    Ok::<_, Error>(())
                })
                .await
                .map_err(|_| {
                    agent_error("Account status was not reported before the deadline")
                })??;
            }
            return Ok(None);
        }
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
                .map_err(|error| {
                    protocol_error(
                        "authenticate",
                        error,
                        options.diagnostic_directory.as_deref(),
                    )
                })?;
        }
        let restart = options.session_id.is_some()
            && (options.restart_session
                || (!initialized.agent_capabilities.load_session && options.allow_session_restart));
        let (session, response) = if let Some(id) = &options.session_id
            && !restart
        {
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
                .map_err(|error| {
                    protocol_error(
                        "session/load",
                        error,
                        options.diagnostic_directory.as_deref(),
                    )
                })?;
            (SessionId::new(id.clone()), serde_json::to_value(response))
        } else {
            let response = connection
                .send_request(
                    NewSessionRequest::new(options.cwd.clone())
                        .mcp_servers(options.mcp_servers.clone()),
                )
                .block_task()
                .await
                .map_err(|error| {
                    protocol_error(
                        "session/new",
                        error,
                        options.diagnostic_directory.as_deref(),
                    )
                })?;
            (response.session_id.clone(), serde_json::to_value(response))
        };
        let mut response = response.map_err(|e| agent_error(e.to_string()))?;
        if let Err(error) = settings::apply(
            connection,
            &session,
            &mut response,
            options.session_mode.as_deref(),
            options.model.as_deref(),
            options.fast,
            options.diagnostic_directory.as_deref(),
        )
        .await
        {
            emit(events, json!({"type":"session_modes","response":response})).await?;
            return Err(error);
        }
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
        emit(events, json!({"type":"ready","session_id":session,"response":response,"authentication":reported_auth,"auth_status_required":policy.require_report,
            "session_origin":if restart {"restarted"} else if options.session_id.is_some() {"loaded"} else {"new"},
            "replaces_session_id":if restart {options.session_id.as_deref()} else {None},
            "restart_reason":if restart {Some(if options.restart_session {"user_requested"} else {"load_session_unsupported"})} else {None}})).await?;
        Ok(Some((
            session,
            initialized.agent_capabilities.prompt_capabilities.image,
            policy,
            response,
        )))
    };
    let outcome = tokio::select! {
        biased;
        _ = shutdown.cancelled() => return Ok(()),
        result = tokio::time::timeout(options.setup_timeout, setup) => result.map_err(|_| agent_error("ACP initialization or session setup timed out"))??,
    };
    let Some((session, images, auth_policy, mut configuration)) = outcome else {
        return Ok(());
    };
    loop {
        let mut turn = tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Ok(()),
            mode = modes.recv() => {
                let Some(mode) = mode else { return Ok(()); };
                settings::merge_updates(&mut configuration,&session,&mode_updates);
                let result = tokio::select! {
                    _ = shutdown.cancelled() => return Ok(()),
                    result = tokio::time::timeout(options.setup_timeout,
                        settings::apply(connection, &session, &mut configuration, Some(&mode), None, None, options.diagnostic_directory.as_deref())) => result,
                };
                match result {
                    Ok(Ok(())) => {
                        settings::merge_updates(&mut configuration,&session,&mode_updates);
                        if configuration["modes"]["currentModeId"]!=mode {
                            emit(events,json!({"type":"mode_error","mode":mode,"response":configuration,"error":agent_error("The adapter reported a different permission mode")})).await?;
                        } else {emit(events, json!({"type":"mode_changed","mode":mode,"response":configuration})).await?;}
                    },
                    Ok(Err(error)) => emit(events, json!({"type":"mode_error","mode":mode,"error":error,"response":configuration})).await?,
                    Err(_) => return Err(agent_error("Permission mode change timed out; reconnect before sending another prompt")),
                }
                continue;
            },
            input = steering.recv() => {
                if let Some(input) = input {
                    emit(events, json!({"type":"steering_result", "id":input.id, "outcome":"promptRequired"})).await?;
                }
                continue;
            },
            turn = turns.recv() => match turn { Some(turn) => turn, None => return Ok(()) },
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
            .send_request(PromptRequest::new(
                session.clone(),
                std::mem::take(&mut turn.content),
            ))
            .block_task();
        tokio::pin!(request);
        *steering_turn.lock().expect("steering lock") = Some(turn.generation);
        let result = steering::run_prompt(
            connection,
            &session,
            options,
            &turn,
            &mut request,
            steering::Channels {
                events,
                shutdown,
                authentication: &mut authentication,
                policy: auth_policy,
                inputs: &mut steering,
            },
        )
        .await;
        let unsent = {
            // Serialize admission with completion. Every accepted input gets a
            // receipt before the prompt's terminal event, even at this boundary.
            let mut active = steering_turn.lock().expect("steering lock");
            *active = None;
            let mut unsent = Vec::new();
            while let Ok(input) = steering.try_recv() {
                unsent.push(input.id);
            }
            unsent
        };
        for id in unsent {
            emit(
                events,
                json!({"type":"steering_result","id":id,"outcome":"promptRequired"}),
            )
            .await?;
        }
        let result = result?;
        let result = match result {
            Ok(result) => result
                .map_err(|error| {
                    protocol_error(
                        "session/prompt",
                        error,
                        options.diagnostic_directory.as_deref(),
                    )
                })
                .and_then(|response| {
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
                        .map_err(|error| {
                            protocol_error(
                                "session/cancel",
                                error,
                                options.diagnostic_directory.as_deref(),
                            )
                        })?;
                    (&mut request).await.map_err(|error| {
                        protocol_error(
                            "session/prompt",
                            error,
                            options.diagnostic_directory.as_deref(),
                        )
                    })
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
fn protocol_error(
    phase: &'static str,
    error: acp::Error,
    directory: Option<&std::path::Path>,
) -> Error {
    let code = serde_json::to_value(&error)
        .ok()
        .and_then(|value| value["code"].as_i64());
    let mut message = match code {
        Some(code) => format!("ACP {phase} failed (code {code})"),
        None => format!("ACP {phase} failed"),
    };
    // Detailed payloads stay in private files, never in frontend events or transcripts.
    let mut details = json!({"phase":phase,"acp_error_code":code});
    if let Some(directory) = directory {
        match diagnostics::write(directory, phase, &error) {
            Ok(path) => {
                message.push_str(&format!("; diagnostic: {}", path.display()));
                details["adapter_diagnostic_path"] = json!(path);
            }
            Err(error) => {
                let kind = error.kind().to_string();
                message.push_str(&format!("; diagnostic log write failed: {kind}"));
                details["adapter_diagnostic_write_error"] = json!(kind);
            }
        }
    }
    agent_error(message).details(details)
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

#[cfg(any(windows, unix))]
#[doc(hidden)]
pub fn serve_process_worker() -> Result<bool> {
    process::serve_worker()
}
