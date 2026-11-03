pub mod cache;
pub mod commands;
pub mod component;
pub mod focus;
pub mod native;
pub mod provenance;
pub mod setup;
pub mod toolchain;
pub mod tools;
pub mod verification;
pub mod watch;

use crate::{dist, util};
use anyhow::{Result, ensure};
use clap::Subcommand;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Subcommand)]
pub enum Command {
    Component {
        #[arg(value_enum)]
        stage: component::Stage,
    },
    Native(native::Arguments),
    Provenance(provenance::Arguments),
    Tools {
        #[command(subcommand)]
        command: tools::Command,
    },
    ToolchainEvidence {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        target: Option<String>,
    },
    LinuxDependencies,
    LinuxCoreDependencies,
    LinuxTerminalDependencies,
    MaterialFonts {
        #[arg(long)]
        output: PathBuf,
    },
    Plan {
        /// Require the complete graph, including when called from a PR workflow.
        #[arg(long)]
        full: bool,
        #[arg(long, default_value = "auto")]
        platform: String,
        #[arg(long, default_value = "all")]
        matrix: String,
    },
    Watch {
        #[arg(long)]
        run: u64,
        #[arg(long)]
        head: String,
        #[arg(long, default_value_t = 30)]
        interval: u64,
    },
    VerificationGate,
    Gate,
    Cache {
        #[command(subcommand)]
        command: cache::Command,
    },
    NormalizeSource {
        #[arg(long)]
        source: PathBuf,
    },
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Component { stage } => component::run(stage),
            Self::Native(arguments) => arguments.run(),
            Self::Provenance(arguments) => arguments.run(),
            Self::Tools { command } => command.run(),
            Self::ToolchainEvidence { output, target } => util::print(&toolchain::collect(
                &util::root(),
                &output,
                target.as_deref(),
            )?),
            Self::LinuxDependencies => setup::linux_dependencies(),
            Self::LinuxCoreDependencies => setup::linux_core_dependencies(),
            Self::LinuxTerminalDependencies => setup::linux_terminal_dependencies(),
            Self::MaterialFonts { output } => util::print(&setup::material_fonts(
                &util::root().join("resources/icons/catalogue.json"),
                &output,
            )?),
            Self::Plan {
                full,
                platform,
                matrix,
            } => {
                let root = util::root();
                let event = std::env::var("GITHUB_EVENT_NAME").unwrap_or_default();
                let paths = verification::changed_paths(&root, &event)?;
                let platform = focus::retry_platform(&platform);
                let mut plan = focus::select(&root, &paths, &platform)?;
                let (verification, reason) =
                    verification::select(if full { "workflow_call" } else { &event }, &paths);
                if verification == "docs" {
                    verification::check_documentation(&root)?;
                }
                if verification == "focused" && verification::presentation_only(&paths) {
                    plan.suite = "graphics".into();
                    plan.targets = ["tui", "agent", "user_config", "conversation_history"]
                        .map(str::to_owned)
                        .to_vec();
                    if paths
                        .iter()
                        .any(|path| path.starts_with("src/tui/documents"))
                    {
                        plan.targets.push("document_launch".into());
                    }
                }
                let platforms = focus::verification_platforms(&matrix)?;
                ensure!(
                    matrix != "all" || focus::RELEASE_PLATFORMS.contains(&plan.platform.as_str()),
                    "Release verification must focus on a release target"
                );
                let outputs = BTreeMap::from([
                    ("verification".into(), verification.into()),
                    ("verification_reason".into(), reason.into()),
                    ("platform".into(), plan.platform.clone()),
                    ("suite".into(), plan.suite.clone()),
                    ("targets".into(), serde_json::to_string(&plan.targets)?),
                    ("preprocessor".into(), plan.preprocessor.to_string()),
                    (
                        "runtime_parallel".into(),
                        (plan.suite != "normal-ci" && focus::runtime_scheduler_changed(&paths))
                            .to_string(),
                    ),
                    (
                        "native_clipboard".into(),
                        focus::native_clipboard_changed(&paths).to_string(),
                    ),
                    (
                        "terminal_reproduction".into(),
                        focus::terminal_reproduction_stages(&paths, &plan)
                            .0
                            .to_string(),
                    ),
                    (
                        "terminal_reproduction_full".into(),
                        focus::terminal_reproduction_stages(&paths, &plan)
                            .1
                            .to_string(),
                    ),
                    (
                        "measure_tui".into(),
                        focus::tui_measurement_changed(&paths).to_string(),
                    ),
                    ("matrix".into(), serde_json::to_string(&platforms)?),
                ]);
                util::print(&outputs)?;
                util::github_output(
                    std::env::var_os("GITHUB_OUTPUT")
                        .map(PathBuf::from)
                        .as_deref(),
                    &outputs,
                )
            }
            Self::Watch {
                run,
                head,
                interval,
            } => watch::run(&util::root(), run, &head, interval),
            Self::VerificationGate => {
                let mode = std::env::var("VERIFICATION_MODE")?;
                let results = serde_json::from_str(&std::env::var("VERIFICATION_RESULTS")?)?;
                util::print(&verification::gate(&mode, &results)?)
            }
            Self::Gate => {
                let value: Value = serde_json::from_str(&std::env::var("VERIFICATION_RESULTS")?)?;
                util::print(&require_success(&value)?)
            }
            Self::Cache { command } => command.run(),
            Self::NormalizeSource { source } => util::print(&normalize_source(&source)?),
        }
    }
}

pub fn require_success(value: &Value) -> Result<Value> {
    let results = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected verification results"))?;
    ensure!(
        results.len() == 3
            && ["native", "audit", "shader-preprocessor"]
                .iter()
                .all(|name| results.contains_key(*name)),
        "Expected exactly native, audit and shader-preprocessor results"
    );
    for name in ["native", "audit"] {
        ensure!(
            results[name]["result"] == "success",
            "Required {name} verification did not succeed"
        );
    }
    let shader = &results["shader-preprocessor"]["result"];
    ensure!(
        shader == "success" || shader == "skipped",
        "Required shader-preprocessor verification did not succeed"
    );
    Ok(serde_json::json!({"native":"success", "audit":"success", "shader-preprocessor":shader}))
}

pub fn normalize_source(source: &std::path::Path) -> Result<Value> {
    use std::{
        fs,
        fs::FileTimes,
        time::{Duration, UNIX_EPOCH},
    };
    let manifest = dist::verify_tree(source, None)?;
    let when = UNIX_EPOCH + Duration::from_secs(946684800);
    let times = FileTimes::new().set_accessed(when).set_modified(when);
    let mut directories = std::collections::BTreeSet::new();
    for name in manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .chain(std::iter::once(dist::MANIFEST))
    {
        let path = source.join(name);
        fs::File::open(&path)?.set_times(times)?;
        for parent in path
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(source))
        {
            directories.insert(parent.to_path_buf());
        }
    }
    for path in directories.iter().rev() {
        fs::File::open(path)?.set_times(times)?;
    }
    Ok(serde_json::json!({"files":manifest.files.len(),"source_mtime_ns":946684800000000000u64}))
}
