//! Explicit developer checks with isolated settings and the same native fixture executable as CI.
use crate::{process, util};
use anyhow::{Result, ensure};
use clap::Args;
use std::{process::Command, time::Duration};
#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    pub core: bool,
}
impl Arguments {
    pub fn run(self) -> Result<()> {
        let root = util::root();
        let configuration = tempfile::tempdir()?;
        process::checked(
            Command::new("cargo").current_dir(&root).args([
                "build",
                "--locked",
                "--manifest-path",
                "xtask/Cargo.toml",
                "--target-dir",
                "xtask/target",
                "--bin",
                "lazypencil-test-helper",
            ]),
            Duration::from_secs(600),
        )?;
        let helper = root.join("xtask/target/debug").join(if cfg!(windows) {
            "lazypencil-test-helper.exe"
        } else {
            "lazypencil-test-helper"
        });
        let mut checks = vec![vec![
            "test",
            "--locked",
            "--no-default-features",
            "--no-fail-fast",
            "--",
            "--test-threads=2",
        ]];
        if !self.core {
            checks.push(vec![
                "test",
                "--locked",
                "--no-fail-fast",
                "--",
                "--test-threads=2",
            ]);
            checks.push(vec![
                "clippy",
                "--locked",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]);
        }
        checks.push(vec![
            "clippy",
            "--locked",
            "--no-default-features",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]);
        checks.push(if self.core {
            vec!["build", "--locked", "--no-default-features"]
        } else {
            vec!["build", "--locked"]
        });
        let mut failures = Vec::new();
        for arguments in checks {
            let mut command = Command::new("cargo");
            command
                .current_dir(&root)
                .env("LAZYPENCIL_CONFIG_DIR", configuration.path().join("user"))
                .env("LAZYPENCIL_TEST_HELPER", &helper)
                .args(&arguments);
            if let Err(error) = process::logged(
                &mut command,
                Duration::from_secs(3600),
                &configuration.path().join("checks.log"),
                true,
            ) {
                eprintln!("{error:#}");
                failures.push(arguments);
            }
        }
        ensure!(failures.is_empty(), "Developer checks failed: {failures:?}");
        Ok(())
    }
}
