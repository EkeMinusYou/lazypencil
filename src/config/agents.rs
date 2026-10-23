//! User-owned ACP preferences. Project files never select executables.

use super::{UserSource, invalid, user_config_path};
use crate::{
    Result,
    prompt::{AgentName, AuthMode, DocumentTools, PromptArgs},
};
use clap::{ArgMatches, parser::ValueSource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(super) const TEMPLATE: &str = "\n# User-owned agent preferences; CLI options take precedence.\n# [agents]\n# default = \"codex\"\n# [agents.codex]\n# adapter = \"codex-acp\"\n# adapter_args = []\n# auth = \"subscription\"\n# require_auth_status = true\n# model = \"an-adapter-advertised-model\"\n# timeout = 600\n# [agents.claude]\n# adapter = \"claude-agent-acp\"\n# auth = \"subscription\"\n";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSettings {
    pub default: Option<AgentName>,
    pub codex: AgentProfile,
    pub claude: AgentProfile,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AgentProfile {
    /// A PATH command or a local path relative to the physical user configuration.
    pub adapter: Option<PathBuf>,
    #[schemars(length(max = 128))]
    pub adapter_args: Vec<String>,
    pub auth: Option<AuthMode>,
    #[schemars(length(min = 1, max = 256))]
    pub auth_method: Option<String>,
    pub require_auth_status: Option<bool>,
    #[schemars(length(min = 1, max = 512))]
    pub model: Option<String>,
    #[schemars(length(min = 1, max = 256))]
    pub mode: Option<String>,
    pub document_tools: Option<DocumentTools>,
    #[schemars(range(min = 1, max = 86400))]
    pub timeout: Option<u64>,
}

impl AgentSettings {
    pub(super) fn validate(&self) -> Result<()> {
        for profile in [&self.codex, &self.claude] {
            if let Some(adapter) = &profile.adapter {
                let text = adapter
                    .to_str()
                    .ok_or_else(|| invalid("Agent adapter paths must be UTF-8"))?;
                if text.is_empty()
                    || text.len() > 4096
                    || text.contains('\0')
                    || text.contains("://")
                {
                    return Err(invalid(
                        "Agent adapters must be nonempty local commands or paths up to 4096 bytes",
                    ));
                }
            }
            if profile.adapter_args.len() > 128
                || profile
                    .adapter_args
                    .iter()
                    .any(|value| value.len() > 4096 || value.contains('\0'))
                || profile.adapter_args.iter().map(String::len).sum::<usize>() > 64 * 1024
                || (!profile.adapter_args.is_empty() && profile.adapter.is_none())
            {
                return Err(invalid(
                    "Agent arguments require an adapter and are limited to 128 arguments, 4096 bytes each and 64 KiB total",
                ));
            }
            for (value, limit) in [
                (&profile.auth_method, 256),
                (&profile.model, 512),
                (&profile.mode, 256),
            ] {
                if value.as_ref().is_some_and(|value| {
                    value.trim().is_empty()
                        || value.len() > limit
                        || value.chars().any(char::is_control)
                }) {
                    return Err(invalid(
                        "Agent model, mode and authentication identifiers must be bounded nonempty text without control characters",
                    ));
                }
            }
            if profile
                .timeout
                .is_some_and(|value| !(1..=86400).contains(&value))
            {
                return Err(invalid("Agent timeout must be between 1 and 86400 seconds"));
            }
        }
        Ok(())
    }

    fn apply(&self, args: &mut PromptArgs, matches: &ArgMatches, base: Option<&Path>) {
        let inherited = |id| {
            matches!(
                matches.value_source(id),
                None | Some(ValueSource::DefaultValue)
            )
        };
        if inherited("agent")
            && let Some(agent) = self.default
        {
            args.agent = agent;
        }
        let profile = match args.agent {
            AgentName::Codex => &self.codex,
            AgentName::Claude => &self.claude,
        };
        // An explicitly selected executable starts with exactly its explicit argv.
        if inherited("adapter")
            && let Some(adapter) = &profile.adapter
        {
            args.adapter = Some(
                if adapter.is_relative()
                    && adapter
                        .file_name()
                        .is_none_or(|name| name != adapter.as_os_str())
                {
                    base.expect("Configured adapters have a physical configuration source")
                        .join(adapter)
                } else {
                    adapter.clone()
                },
            );
            args.adapter_args = profile.adapter_args.clone();
        }
        if inherited("auth")
            && let Some(value) = profile.auth
        {
            args.auth = value;
        }
        if inherited("auth_method")
            && let Some(value) = &profile.auth_method
        {
            args.auth_method = Some(value.clone());
        }
        if inherited("require_auth_status")
            && let Some(value) = profile.require_auth_status
        {
            args.require_auth_status = value;
        }
        if inherited("model")
            && let Some(value) = &profile.model
        {
            args.model = Some(value.clone());
        }
        if inherited("mode")
            && let Some(value) = &profile.mode
        {
            args.mode = Some(value.clone());
        }
        if inherited("document_tools")
            && let Some(value) = profile.document_tools
        {
            args.document_tools = value;
        }
        if inherited("timeout")
            && let Some(value) = profile.timeout
        {
            args.timeout = value;
        }
    }
}

/// Resolve preferences once, at the CLI boundary, before creating a conversation.
/// Programmatic callers already supply their complete `PromptArgs` explicitly.
pub fn resolve_arguments(
    arguments: &mut crate::cli::Arguments,
    matches: &ArgMatches,
) -> Result<()> {
    let args = match &mut arguments.command {
        crate::cli::Command::Prompt(args) => args,
        #[cfg(feature = "tui")]
        crate::cli::Command::Tui(args) => &mut args.prompt,
        _ => return Ok(()),
    };
    let source = UserSource::load(Some(user_config_path()?), true)?;
    source.settings.agents.apply(
        args,
        matches.subcommand().expect("Prompt subcommand").1,
        source.base(),
    );
    source.verify()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches};

    fn parsed(command: &str, options: &[&str]) -> (crate::cli::Arguments, ArgMatches) {
        let matches = crate::cli::Arguments::command()
            .try_get_matches_from(
                ["lazypencil", command, "design.pen"]
                    .into_iter()
                    .chain(options.iter().copied()),
            )
            .unwrap();
        (
            crate::cli::Arguments::from_arg_matches(&matches).unwrap(),
            matches,
        )
    }

    fn settings() -> AgentSettings {
        AgentSettings {
            default: Some(AgentName::Claude),
            claude: AgentProfile {
                timeout: Some(12),
                require_auth_status: Some(true),
                auth: Some(AuthMode::Api),
                document_tools: Some(DocumentTools::Ask),
                model: Some("user-model".into()),
                mode: Some("user-mode".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn value_provenance_keeps_explicit_builtin_values_and_individual_overrides() {
        let (mut arguments, matches) = parsed("prompt", &[]);
        let crate::cli::Command::Prompt(args) = &mut arguments.command else {
            unreachable!()
        };
        settings().apply(args, matches.subcommand().unwrap().1, None);
        assert_eq!(args.agent, AgentName::Claude);
        assert_eq!(args.timeout, 12);
        assert!(args.require_auth_status);
        assert_eq!(args.auth, AuthMode::Api);
        assert_eq!(args.document_tools, DocumentTools::Ask);
        assert_eq!(args.model.as_deref(), Some("user-model"));
        assert_eq!(args.mode.as_deref(), Some("user-mode"));

        let (mut arguments, matches) = parsed(
            "prompt",
            &[
                "--timeout",
                "600",
                "--auth",
                "subscription",
                "--require-auth-status=false",
                "--document-tools",
                "allow",
                "--model",
                "cli-model",
                "--mode",
                "cli-mode",
            ],
        );
        let crate::cli::Command::Prompt(args) = &mut arguments.command else {
            unreachable!()
        };
        settings().apply(args, matches.subcommand().unwrap().1, None);
        assert_eq!(args.agent, AgentName::Claude);
        assert_eq!(args.timeout, 600);
        assert!(!args.require_auth_status);
        assert_eq!(args.auth, AuthMode::Subscription);
        assert_eq!(args.document_tools, DocumentTools::Allow);
        assert_eq!(args.model.as_deref(), Some("cli-model"));
        assert_eq!(args.mode.as_deref(), Some("cli-mode"));

        let (mut arguments, matches) = parsed("prompt", &["--agent", "codex"]);
        let crate::cli::Command::Prompt(args) = &mut arguments.command else {
            unreachable!()
        };
        settings().apply(args, matches.subcommand().unwrap().1, None);
        assert_eq!(args.agent, AgentName::Codex);
        assert_eq!(args.timeout, 600);
        assert!(args.model.is_none());
        assert!(args.mode.is_none());
    }

    #[cfg(feature = "tui")]
    #[test]
    fn tui_uses_the_same_precedence_and_keeps_terminal_options_separate() {
        let (mut arguments, matches) = parsed(
            "tui",
            &[
                "--image-protocol",
                "sixel",
                "--timeout",
                "600",
                "--require-auth-status",
            ],
        );
        let crate::cli::Command::Tui(args) = &mut arguments.command else {
            unreachable!()
        };
        settings().apply(&mut args.prompt, matches.subcommand().unwrap().1, None);
        assert_eq!(args.prompt.agent, AgentName::Claude);
        assert_eq!(args.prompt.timeout, 600);
        assert!(args.prompt.require_auth_status);
        assert_eq!(args.image_protocol, crate::tui::ImageProtocol::Sixel);
    }
}
