//! Bootstrap and maintain the small tool workspace without touching application artifacts.
use crate::{process, util};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::Command as Process,
    time::{Duration, UNIX_EPOCH},
};
#[derive(Subcommand)]
pub enum Command {
    Prepare,
    Verify,
    Stage {
        #[arg(long)]
        output: PathBuf,
    },
}
impl Command {
    pub fn run(self) -> Result<()> {
        let root = util::root();
        let workspace = root.join("xtask");
        let target = workspace.join("target");
        match self {
            Self::Stage { output } => {
                let executable = std::env::current_exe()?;
                let directory = executable
                    .parent()
                    .context("Tool executable lacks a directory")?;
                let names = if cfg!(windows) {
                    ["xtask.exe", "lazypencil-test-helper.exe"]
                } else {
                    ["xtask", "lazypencil-test-helper"]
                };
                let mut sums = String::new();
                for name in names {
                    let bytes = util::read(&directory.join(name), 256 * 1024 * 1024)?;
                    sums.push_str(&format!("{}  {name}\n", util::digest(&bytes)));
                    util::publish(&output.join(name), &bytes, false)?;
                }
                util::publish(&output.join("SHA256SUMS"), sums.as_bytes(), false)
            }
            Self::Prepare => {
                ensure!(
                    std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
                    "Hosted tool preparation required"
                );
                // An exact Actions cache key covers every tool source and fixture. Dependency
                // fallback caches never contain this package's outputs.
                let times =
                    fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(946684800));
                for path in util::walk_filtered(&workspace, |p| p.starts_with(&target))? {
                    fs::File::open(path)?.set_times(times)?;
                }
                let temporary =
                    PathBuf::from(std::env::var_os("RUNNER_TEMP").context("RUNNER_TEMP required")?);
                let helper = target.join("debug").join(if cfg!(windows) {
                    "lazypencil-test-helper.exe"
                } else {
                    "lazypencil-test-helper"
                });
                ensure!(helper.is_file(), "Native test helper was not built");
                let values = BTreeMap::from([
                    (
                        "CARGO_TARGET_DIR".into(),
                        temporary
                            .join("lazypencil-target")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    (
                        "LAZYPENCIL_CONFIG_DIR".into(),
                        temporary
                            .join("lazypencil-test-settings")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    (
                        "LAZYPENCIL_TEST_TUI_ARTIFACTS".into(),
                        temporary
                            .join("tui-artifacts")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    (
                        "LAZYPENCIL_TEST_HELPER".into(),
                        helper.to_string_lossy().into_owned(),
                    ),
                    (
                        "LAZYPENCIL_TEST_CODEC_DIR".into(),
                        temporary
                            .join("codec-reference")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    (
                        "LAZYPENCIL_TEST_RETAINED_DIR".into(),
                        temporary.to_string_lossy().into_owned(),
                    ),
                    (
                        "LAZYPENCIL_TEST_NATIVE_SHADER_DIR".into(),
                        temporary.to_string_lossy().into_owned(),
                    ),
                    (
                        "LAZYPENCIL_TEST_SUPERVISION_DIR".into(),
                        temporary.to_string_lossy().into_owned(),
                    ),
                ]);
                util::github_output(
                    std::env::var_os("GITHUB_ENV")
                        .as_deref()
                        .map(std::path::Path::new),
                    &values,
                )
            }
            Self::Verify => {
                crate::notices::rust::verify(&workspace, &workspace.join("notices"), None)?;
                let mut failures = Vec::new();
                for arguments in [
                    vec!["fmt", "--all", "--", "--check"],
                    vec![
                        "test",
                        "-vv",
                        "--locked",
                        "--all-targets",
                        "--no-fail-fast",
                        "--",
                        "--test-threads=2",
                    ],
                    vec![
                        "clippy",
                        "-vv",
                        "--locked",
                        "--all-targets",
                        "--",
                        "-D",
                        "warnings",
                    ],
                ] {
                    let mut command = Process::new("cargo");
                    command
                        .current_dir(&workspace)
                        .args(&arguments)
                        .env_remove(super::commands::TEST_PROFILE_ENV)
                        .env("CARGO_TARGET_DIR", &target);
                    if let Err(error) = process::logged(
                        &mut command,
                        Duration::from_secs(600),
                        &root.join("xtask-verification.log"),
                        true,
                    ) {
                        eprintln!("{error:#}");
                        failures.push(arguments);
                    }
                }
                ensure!(
                    failures.is_empty(),
                    "Tool verification failed: {failures:?}"
                );
                Ok(())
            }
        }
    }
}
