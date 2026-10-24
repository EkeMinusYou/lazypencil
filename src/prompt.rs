//! Streaming CLI and durable metadata for ACP conversations.

use crate::{
    Error, Result,
    conversation::{Conversation, ConversationEvent},
};
use agent_client_protocol::schema::v1::RequestPermissionRequest;
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
    time::Duration,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum AgentName {
    Codex,
    Claude,
}
impl AgentName {
    pub(crate) fn command(self) -> &'static str {
        match self {
            Self::Codex => "codex-acp",
            Self::Claude => "claude-agent-acp",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    Subscription,
    Api,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DocumentTools {
    Allow,
    Ask,
}

#[derive(Debug, Clone, Args)]
pub struct PromptArgs {
    pub file: PathBuf,
    /// User request, in the language intended for the design.
    #[arg(default_value = "")]
    pub prompt: String,
    #[arg(long, value_enum, default_value = "codex")]
    pub agent: AgentName,
    #[arg(long, conflicts_with = "session")]
    pub create: bool,
    /// Resume a local lazypencil conversation ID printed by a previous request.
    #[arg(long)]
    pub session: Option<String>,
    /// Start a new native session with local history instead of loading the saved one.
    #[arg(long, requires = "session")]
    pub restart_session: bool,
    #[arg(long = "image")]
    pub images: Vec<PathBuf>,
    #[arg(long = "font")]
    pub fonts: Vec<PathBuf>,
    #[arg(long)]
    pub style: Option<String>,
    /// Explicit ACP executable override; no shell parsing or automatic installation.
    #[arg(long)]
    pub adapter: Option<PathBuf>,
    #[arg(long = "adapter-arg", allow_hyphen_values = true, requires = "adapter")]
    pub adapter_args: Vec<String>,
    #[arg(long, value_enum, default_value = "subscription")]
    pub auth: AuthMode,
    /// Explicit adapter-advertised ACP authentication method. Credentials stay with the adapter.
    #[arg(long)]
    pub auth_method: Option<String>,
    /// Require an adapter-reported authentication route before sending a prompt.
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1,
        require_equals = true, default_missing_value = "true", default_value = "false")]
    pub require_auth_status: bool,
    /// Select an adapter-advertised model through ACP.
    #[arg(long)]
    pub model: Option<String>,
    /// Select an adapter-advertised permission mode through ACP.
    #[arg(long)]
    pub mode: Option<String>,
    /// Preauthorize only tools on this document's lazypencil MCP server.
    #[arg(long, value_enum, default_value = "allow")]
    pub document_tools: DocumentTools,
    /// Deny permission requests instead of reading from the terminal.
    #[arg(long)]
    pub non_interactive: bool,
    #[arg(long, default_value_t=600, value_parser=clap::value_parser!(u64).range(1..=86400))]
    pub timeout: u64,
}

pub fn run_cli(args: &PromptArgs, data_dir: Option<PathBuf>, json_output: bool) -> Result<()> {
    if (args.prompt.trim().is_empty() && args.images.is_empty()) || args.prompt.len() > 1024 * 1024
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Enter up to 1 MiB of prompt text or attach an image",
        ));
    }
    let mut conversation = Conversation::open(args.clone(), data_dir)?;
    // A CLI invocation uses only explicitly supplied attachments, never a saved draft's images.
    conversation.clear_images()?;
    conversation.set_draft(args.prompt.clone())?;
    for path in &args.images {
        conversation.attach_file(path)?;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    let result = runtime.block_on(async {
        let sent = conversation.send(None).await;
        if let Err(error) = sent {
            let _ = conversation.close().await;
            return Err(error);
        }
        let cancel = crate::cancel::token();
        let mut closing = false;
        let mut interrupted = false;
        loop {
            let event = tokio::select! {
                biased;
                _ = cancel.cancelled(), if !closing => {
                    interrupted = true;
                    closing = true;
                    conversation.cancel();
                    let _ = conversation.close().await;
                    continue;
                }
                event = conversation.next_event() => event,
            }?;
            match event {
                ConversationEvent::Message(value) => {
                    output(&value, json_output)?;
                    if matches!(value["type"].as_str(), Some("complete" | "failed")) {
                        let result = conversation.result().cloned().unwrap_or(Ok(()));
                        let closed = conversation.close().await;
                        return if interrupted {
                            Err(Error::new(
                                "CANCELLED",
                                "Agent request interrupted; committed edits were retained",
                            ))
                        } else {
                            result.and(closed)
                        };
                    }
                }
                ConversationEvent::Permission { id, request } => {
                    output(
                        &json!({"type":"permission_request","request":request}),
                        json_output,
                    )?;
                    let interactive = !args.non_interactive
                        && std::io::stdin().is_terminal()
                        && std::io::stderr().is_terminal();
                    let selection = if interactive {
                        read_permission(&request).await?
                    } else {
                        crate::conversation::reject_once(&request)
                    };
                    conversation.respond(&id, selection, interactive)?;
                }
            }
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

async fn read_permission(request: &RequestPermissionRequest) -> Result<Option<String>> {
    eprintln!(
        "\nACP permission: {}",
        serde_json::to_string(&request.tool_call).unwrap_or_default()
    );
    for (index, option) in request.options.iter().enumerate() {
        eprintln!(
            "  {}. {} ({:?})",
            index + 1,
            printable(&option.name),
            option.kind
        );
    }
    eprint!("Choose a number, or Enter to deny: ");
    std::io::stderr()
        .flush()
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let (sender, answer) = tokio::sync::oneshot::channel();
    // A detached OS thread avoids Tokio's non-cancellable blocking-stdin shutdown wait.
    std::thread::Builder::new()
        .name("lazypencil-permission".into())
        .spawn(move || {
            let mut line = String::new();
            let result = std::io::stdin().read_line(&mut line).map(|_| line);
            let _ = sender.send(result);
        })
        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    let cancel = crate::cancel::token();
    let answer = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        answer = answer => answer.map_err(|_| Error::new("IO_ERROR", "Permission input closed"))?.map_err(|e| Error::new("IO_ERROR", e.to_string()))?,
    };
    Ok(answer
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .and_then(|index| request.options.get(index))
        .map(|option| option.option_id.to_string()))
}

fn printable(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}
fn output(value: &Value, json_output: bool) -> Result<()> {
    if json_output {
        let value = crate::conversation::sanitize_event(value);
        writeln!(std::io::stdout().lock(), "{value}")
            .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
    } else {
        match value["type"].as_str() {
            Some("start") => eprintln!(
                "Conversation: {} (billing source unverified)",
                value["session"].as_str().unwrap_or_default()
            ),
            Some("auth_status") => eprintln!(
                "Authentication: {} (adapter report)",
                value["authentication"]["source"]
                    .as_str()
                    .unwrap_or("unknown")
            ),
            Some("session_restarted") => eprintln!(
                "Started a new native session with current document context and bounded local history; previous model memory was not restored."
            ),
            Some("local_context") => eprintln!(
                "Added current document context and bounded local history after an interrupted restart."
            ),
            Some("update")
                if value["notification"]["update"]["sessionUpdate"] == "agent_message_chunk" =>
            {
                if let Some(text) = value["notification"]["update"]["content"]["text"].as_str() {
                    print!("{}", printable(text));
                    std::io::stdout()
                        .flush()
                        .map_err(|e| Error::new("IO_ERROR", e.to_string()))?;
                }
            }
            Some("complete") => eprintln!(
                "\nSaved conversation: {}",
                value["session"].as_str().unwrap_or_default()
            ),
            _ => {}
        }
    }
    Ok(())
}
