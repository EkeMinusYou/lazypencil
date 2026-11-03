use super::*;
use crate::{
    agent::{
        AgentHost, AgentInspection, AgentOptions,
        auth::{AuthPolicy, AuthSource},
    },
    config::{DiagnosticSettings, agents::AgentProfile},
    prompt::AuthMode,
};
use std::{path::Path, time::Duration};

fn name(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Codex => "codex",
        AgentName::Claude => "claude",
    }
}

fn install(agent: AgentName) -> String {
    format!(
        "Run npm install -g @agentclientprotocol/{}-acp@latest, then ensure {} is on PATH (check npm prefix -g and your PATH order).",
        name(agent),
        agent.command()
    )
}

pub(super) fn diagnose(
    options: &Options,
    settings: Option<&DiagnosticSettings>,
    report: &mut Report,
) -> Result<Vec<Value>> {
    let selected = options
        .agent
        .or_else(|| settings.and_then(|settings| settings.settings.agents.default))
        .unwrap_or(AgentName::Codex);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    let cwd = std::env::current_dir().map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    let executable =
        std::env::current_exe().map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    let mut agents = Vec::new();
    for agent in [AgentName::Codex, AgentName::Claude] {
        if options.agent.is_some_and(|selected| selected != agent) {
            continue;
        }
        let required = agent == selected;
        let failure = if required {
            Status::Error
        } else {
            Status::Warning
        };
        let id = format!("agents.{}", name(agent));
        let profile = settings
            .map(|settings| match agent {
                AgentName::Codex => &settings.settings.agents.codex,
                AgentName::Claude => &settings.settings.agents.claude,
            })
            .cloned()
            .unwrap_or_default();
        let command = profile
            .adapter
            .clone()
            .unwrap_or_else(|| agent.command().into());
        let paths = environment::executables(&command);
        let mut entry = json!({"agent":name(agent),"selected":required,"command":command,
            "path":paths.first(),"candidates":paths,"authentication":{"status":"not_checked"}});
        if settings.is_none() {
            report.add(
                &id,
                Status::NotChecked,
                "Agent checks require valid user configuration",
                Some("Fix the configuration errors and run doctor again.".into()),
                json!({"command":command}),
            );
            entry["status"] = json!("configuration_invalid");
            agents.push(entry);
            continue;
        }
        let Some(path) = paths.first() else {
            let exists = command.components().count() > 1 && command.exists();
            report.add(format!("{id}.executable"), if required {failure} else {Status::NotChecked},
                if exists {format!("{} exists but cannot be executed", command.display())} else {format!("{} is not installed or not on PATH", command.display())},
                Some(if exists {"Check executable permissions and the configured adapter path.".into()} else if profile.adapter.is_some() {
                    "Fix the adapter path in config.toml or install that adapter and add it to PATH.".into()
                } else {install(agent)}), json!({"command":command,"selected":required}));
            entry["status"] = json!(if exists { "not_executable" } else { "missing" });
            agents.push(entry);
            continue;
        };
        report.add(format!("{id}.executable"), if paths.len() > 1 {Status::Warning} else {Status::Ok},
            if paths.len() > 1 {"Multiple adapter executables found; the first PATH match is used"} else {"Adapter executable found"},
            (paths.len() > 1).then(|| "Check PATH order when upgrading so an older copy does not shadow the new adapter.".into()),
            json!({"path":path,"resolved_path":path.canonicalize().ok(),"candidates":paths}));
        if environment::needs_node(path) && environment::executables(Path::new("node")).is_empty() {
            report.add(
                format!("{id}.node"),
                failure,
                "This adapter requires Node.js, but node is not on PATH",
                Some("Install Node.js and npm, add node to PATH, then run doctor again.".into()),
                json!({"executable":path}),
            );
            entry["status"] = json!("node_missing");
            agents.push(entry);
            continue;
        }
        let mut probe = AgentOptions::new(path, &cwd);
        probe.args = profile.adapter_args.clone();
        probe.supervisor_executable = Some(executable.clone());
        probe.subscription =
            profile.auth.unwrap_or(AuthMode::Subscription) == AuthMode::Subscription;
        probe.require_auth_status = profile.require_auth_status.unwrap_or(false);
        probe.setup_timeout = Duration::from_secs(options.timeout_seconds);
        probe.auth_status_timeout = probe.setup_timeout;
        probe.cancel_grace = Duration::from_secs(1);
        let inspection = runtime.block_on(AgentHost::inspect(probe, crate::cancel::token()));
        match inspection {
            Ok(inspection) => {
                inspect_result(&id, &profile, inspection, failure, report, &mut entry)
            }
            Err(error) if error.code == "CANCELLED" => return Err(error),
            Err(error) => {
                report.add(format!("{id}.communication"), failure, "Cannot initialize the ACP adapter",
                    Some("Check the adapter command, its arguments and runtime dependencies; update the adapter, then retry.".into()),
                    json!({"error_code":error.code,"timeout_seconds":options.timeout_seconds}));
                entry["status"] = json!("connection_failed");
            }
        }
        agents.push(entry);
    }
    Ok(agents)
}

fn inspect_result(
    id: &str,
    profile: &AgentProfile,
    inspection: AgentInspection,
    failure: Status,
    report: &mut Report,
    entry: &mut Value,
) {
    entry["protocol_version"] = json!(inspection.protocol_version);
    entry["implementation"] = json!({"name":inspection.name,"version":inspection.version});
    entry["steering_supported"] = json!(inspection.steering_supported);
    if inspection.protocol_version.is_none() {
        report.add(format!("{id}.communication"), failure, "ACP initialization failed or timed out",
            Some("Check the adapter and its runtime dependencies; update it or increase --timeout-seconds for a slow startup.".into()),
            json!({"error":inspection.failure}));
        entry["status"] = json!("connection_failed");
        return;
    }
    report.add(
        format!("{id}.communication"),
        Status::Ok,
        "ACP protocol v1 connection succeeded",
        None,
        json!({"version":inspection.version,"name":inspection.name,"protocol_version":1}),
    );
    if inspection.failure.is_some() && inspection.authentication.is_some() {
        report.add(format!("{id}.inspection"), failure, "The adapter check did not finish cleanly",
            Some("Check the adapter runtime and retry; increase --timeout-seconds if startup is slow.".into()),
            json!({"error":inspection.failure}));
        entry["probe_error"] = json!(true);
    }
    let hint = if profile.auth == Some(AuthMode::Api) {
        "Configure API authentication in the selected runtime, then run doctor again."
    } else {
        "Sign in using the selected Codex or Claude runtime, then run doctor again; doctor never opens a login flow."
    };
    match inspection.authentication {
        Some(authentication) if authentication.source == AuthSource::NotAuthenticated => {
            report.add(
                format!("{id}.authentication"),
                failure,
                "The runtime reports that it is not signed in",
                Some(hint.into()),
                json!({"source":"not_authenticated"}),
            );
            entry["authentication"] =
                json!({"status":"not_authenticated","source":"not_authenticated"});
            entry["status"] = json!("not_authenticated");
        }
        Some(authentication) if authentication.source != AuthSource::Unknown => {
            let policy = AuthPolicy {
                subscription: profile.auth.unwrap_or(AuthMode::Subscription)
                    == AuthMode::Subscription,
                require_report: true,
            };
            let error = policy.check(Some(&authentication)).err();
            report.add(
                format!("{id}.authentication"),
                if error.is_some() { failure } else { Status::Ok },
                error
                    .as_ref()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| {
                        "The runtime reports an authenticated route matching the configuration"
                            .into()
                    }),
                error.as_ref().map(|_| {
                    "Check agents.<name>.auth and the runtime's authentication settings.".into()
                }),
                json!({"source":authentication.source}),
            );
            entry["authentication"] = json!({"status":if error.is_some() {"mismatch"} else {"authenticated"},"source":authentication.source});
            entry["status"] = json!(if error.is_some() {
                "authentication_mismatch"
            } else {
                "ready"
            });
        }
        _ => {
            report.add(format!("{id}.authentication"), if inspection.failure.is_some() || inspection.authentication_supported || profile.require_auth_status.unwrap_or(false) {failure} else {Status::Warning},
                if inspection.failure.is_some() {"Login-state reporting failed or timed out"} else {"The adapter did not report login state; authentication is unverified"},
                Some("Update the adapter or verify authentication in its runtime; increase --timeout-seconds if startup is slow.".into()),
                json!({"supported":inspection.authentication_supported,"error":inspection.failure}));
            entry["authentication"] = json!({"status":"not_reported"});
            entry["status"] = json!("authentication_unknown");
        }
    }
    if entry["probe_error"] == true {
        entry["status"] = json!("inspection_failed");
    }
    report.add(format!("{id}.steering"), if inspection.steering_supported {Status::Ok} else {Status::Warning},
        if inspection.steering_supported {"Mid-turn input is supported"} else {"Mid-turn input is unavailable; follow-ups use the queue"},
        (!inspection.steering_supported).then(|| "Update codex-acp to 2.3.0+ or claude-agent-acp to 0.100.0+; the adapter must advertise steering.".into()),
        json!({"supported":inspection.steering_supported,"version":inspection.version}));
}
