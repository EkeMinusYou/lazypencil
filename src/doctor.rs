//! Actionable environment diagnostics without document edits or model prompts.
use crate::{Error, Result, prompt::AgentName};
use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};
use std::{fmt::Write, path::PathBuf};

mod agents;
mod environment;
mod preview;

#[derive(Debug, Args)]
pub struct Options {
    /// Check this agent instead of both installed adapters.
    #[arg(long, value_enum)]
    pub agent: Option<AgentName>,
    /// Maximum initialization/account-reporting time per adapter.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=30))]
    pub timeout_seconds: u64,
    /// Exit with status 1 when the report contains errors, while still printing it.
    #[arg(long)]
    pub strict: bool,
    /// Select the protocol for the automatic terminal-image test.
    #[cfg(feature = "tui")]
    #[arg(long, value_enum, default_value = "auto")]
    pub image_protocol: crate::tui::ImageProtocol,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Ok,
    Warning,
    Error,
    NotChecked,
}

#[derive(Serialize)]
struct Check {
    id: String,
    status: Status,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remediation: Option<String>,
    details: Value,
}

#[derive(Default)]
struct Report {
    checks: Vec<Check>,
}

impl Report {
    fn add(
        &mut self,
        id: impl Into<String>,
        status: Status,
        message: impl Into<String>,
        remediation: Option<String>,
        details: Value,
    ) {
        self.checks.push(Check {
            id: id.into(),
            status,
            message: message.into(),
            remediation,
            details,
        });
    }
    fn finish(self, mut value: Value) -> Value {
        let count = |status| {
            self.checks
                .iter()
                .filter(|check| {
                    std::mem::discriminant(&check.status) == std::mem::discriminant(&status)
                })
                .count()
        };
        let errors = count(Status::Error);
        let warnings = count(Status::Warning);
        value["summary"] = json!({"status":if errors > 0 {"error"} else if warnings > 0 {"warning"} else {"ok"},
            "errors":errors,"warnings":warnings,"ok":count(Status::Ok),"not_checked":count(Status::NotChecked)});
        value["checks"] = json!(self.checks);
        value
    }
}

pub fn run(options: &Options, data_override: Option<PathBuf>, json_output: bool) -> Result<Value> {
    let mut report = Report::default();
    let release: Value = serde_json::from_str(include_str!("../resources/release.json"))
        .map_err(|error| Error::new("INTERNAL_ERROR", error.to_string()))?;
    let release_ready = cfg!(all(
        target_os = "macos",
        target_arch = "aarch64",
        feature = "rendering",
        feature = "tui",
        not(debug_assertions)
    )) && release["format_version"] == 1
        && release["target"] == "aarch64-apple-darwin"
        && release["channel"] == "stable"
        && release["version"] == env!("CARGO_PKG_VERSION");
    report.add("build", Status::Ok, format!("lazypencil {} ({}/{})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH), None,
        json!({"rendering":cfg!(feature="rendering"),"tui":cfg!(feature="tui"),"release_ready":release_ready}));
    environment::platform(&mut report, &release);
    report.add(
        "rendering",
        if cfg!(feature = "rendering") {
            Status::Ok
        } else {
            Status::Warning
        },
        if cfg!(feature = "rendering") {
            "Skia rendering is compiled in"
        } else {
            "This build cannot render previews"
        },
        (!cfg!(feature = "rendering")).then(|| "Install a build with rendering support.".into()),
        json!({"compiled":cfg!(feature="rendering")}),
    );
    let data = environment::directory(
        &mut report,
        "data_directory",
        crate::store::data_directory(data_override),
    );
    let locks = environment::directory(
        &mut report,
        "lock_directory",
        crate::store::default_lock_dir(),
    );
    let settings = crate::config::diagnostic_settings();
    let configuration = match &settings {
        Ok(settings) => {
            report.add(
                "user_configuration",
                Status::Ok,
                if settings.source["exists"] == true {
                    "User configuration is valid"
                } else {
                    "No user configuration; using built-in defaults"
                },
                None,
                settings.source.clone(),
            );
            environment::fonts(&mut report);
            settings.source.clone()
        }
        Err(error) => {
            let path = crate::config::user_config_path().ok();
            report.add("user_configuration", Status::Error, "Cannot load or validate user configuration",
                Some("Inspect the selected config.toml and run lazypencil config show --user for details.".into()),
                json!({"path":path,"error_code":error.code}));
            json!({"path":path,"valid":false,"error_code":error.code})
        }
    };
    let agents = agents::diagnose(options, settings.as_ref().ok(), &mut report)?;
    let chromium = data
        .as_deref()
        .map(crate::runtime::status)
        .unwrap_or_else(|| json!({"available":false,"startup":"not_checked"}));
    report.add("chromium", if chromium["available"] == true {Status::Ok} else {Status::Warning},
        if chromium["available"] == true {"Optional Chromium runtime is present; browser startup is not tested"} else {"Optional Chromium runtime is unavailable; browser nodes and shaders need it"},
        (chromium["available"] != true).then(|| "Run lazypencil runtime setup chromium; inspect lazypencil --json runtime status for details.".into()), chromium.clone());
    environment::terminal(&mut report);
    preview::diagnose(&mut report, options, json_output)?;
    Ok(report.finish(json!({
        "version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "document_version":crate::document::DOCUMENT_VERSION,"supported_document_versions":crate::document::SUPPORTED_DOCUMENT_VERSIONS,
        "data_directory":data,"lock_directory":locks,"user_configuration":configuration,
        "skia_compiled":cfg!(feature="rendering"),"chromium":chromium,"release_ready":release_ready,"release":release,
        "agents":agents,"agent_probe_timeout_seconds":options.timeout_seconds,
    })))
}

fn clean(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            if character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                character
                    .escape_debug()
                    .to_string()
                    .chars()
                    .collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

pub fn format_report(value: &Value) -> String {
    let mut output = format!(
        "lazypencil doctor {}\n\n",
        clean(value["version"].as_str().unwrap_or("unknown"))
    );
    if let Some(checks) = value["checks"].as_array() {
        for check in checks {
            let label = match check["status"].as_str() {
                Some("ok") => "OK",
                Some("warning") => "WARN",
                Some("error") => "FAIL",
                _ => "SKIP",
            };
            let _ = writeln!(
                output,
                "[{label}] {}: {}",
                clean(check["id"].as_str().unwrap_or("check")),
                clean(check["message"].as_str().unwrap_or(""))
            );
            for key in ["path", "executable", "version", "protocol"] {
                if let Some(text) = check["details"][key].as_str() {
                    let _ = writeln!(output, "       {key}: {}", clean(text));
                }
            }
            if let Some(reason) = check["details"]["error"]["message"].as_str() {
                let _ = writeln!(output, "       reason: {}", clean(reason));
            }
            if let Some(hint) = check["remediation"].as_str() {
                let _ = writeln!(output, "       {}", clean(hint));
            }
        }
    }
    let _ = write!(
        output,
        "\n{} error(s), {} warning(s). Use --json for details; --strict fails on errors.",
        value["summary"]["errors"], value["summary"]["warnings"]
    );
    output
}
