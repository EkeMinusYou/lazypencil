pub mod cache;
pub mod commands;
pub mod component;
pub mod focus;
pub mod native;
pub mod provenance;
pub mod setup;
pub mod toolchain;
pub mod tools;

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
    LinuxTerminalDependencies,
    MaterialFonts {
        #[arg(long)]
        output: PathBuf,
    },
    Plan {
        #[arg(long, default_value = "auto")]
        platform: String,
        #[arg(long, default_value = "all")]
        matrix: String,
    },
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
            Self::LinuxTerminalDependencies => setup::linux_terminal_dependencies(),
            Self::MaterialFonts { output } => util::print(&setup::material_fonts(
                &util::root().join("resources/icons/catalogue.json"),
                &output,
            )?),
            Self::Plan { platform, matrix } => {
                let root = util::root();
                let paths = focus::changed_paths(&root)?;
                let platform = focus::retry_platform(&root, &platform);
                let plan = focus::select(&root, &paths, &platform)?;
                ensure!(
                    matrix == "all" || focus::PLATFORMS.contains(&matrix.as_str()),
                    "Unsupported release platform"
                );
                let platforms = if matrix == "all" {
                    focus::PLATFORMS.iter().map(|s| s.to_string()).collect()
                } else {
                    vec![matrix]
                };
                let outputs = BTreeMap::from([
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
        results.len() == 2 && results.contains_key("native") && results.contains_key("audit"),
        "Expected exactly native and audit results"
    );
    for name in ["native", "audit"] {
        ensure!(
            results[name]["result"] == "success",
            "Required {name} verification did not succeed"
        );
    }
    Ok(serde_json::json!({"native":"success", "audit":"success"}))
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
