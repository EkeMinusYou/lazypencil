//! Developer checks with isolated settings and the same native fixture as CI.
use crate::{process, util};
use anyhow::{Context, Result, ensure};
use clap::Args;
use std::{io::Write, path::Path, process::Command, time::Duration};

pub fn verify_release() -> Result<()> {
    let invocation = crate::evidence::link_map::verification_cargo_arguments();
    eprintln!("Running cargo {}", invocation[1..].join(" "));
    let result = process::output(
        Command::new(&invocation[0])
            .args(&invocation[1..])
            .current_dir(util::root()),
        Duration::from_secs(3600),
    )?;
    std::io::stdout().write_all(&result.stdout)?;
    std::io::stderr().write_all(&result.stderr)?;
    ensure!(
        !result.timed_out && result.status.success(),
        "Optimized functional-verification build failed"
    );
    Ok(())
}

#[derive(Args)]
pub struct Arguments {
    /// Check the configuration without rendering and TUI features.
    #[arg(long)]
    pub core: bool,
    /// Run library tests only, or combine with selected integration targets.
    #[arg(long)]
    pub lib: bool,
    /// Run an integration test target (repeatable).
    #[arg(long = "test")]
    pub tests: Vec<String>,
    /// Select test names within each requested target; zero executed tests fail.
    #[arg(long)]
    pub filter: Option<String>,
    /// Also lint the selected targets, rather than every integration target.
    #[arg(long)]
    pub lint: bool,
}
impl Arguments {
    pub fn run(self) -> Result<()> {
        self.run_in(&util::root())
    }

    pub fn run_in(self, root: &Path) -> Result<()> {
        let focused = self.lib || !self.tests.is_empty();
        ensure!(
            focused || (self.filter.is_none() && !self.lint),
            "--filter and --lint require --lib or --test"
        );
        ensure!(
            self.tests.iter().all(|s| !s.is_empty()
                && !s.starts_with('-')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')),
            "Invalid integration test target"
        );
        ensure!(
            self.filter
                .as_ref()
                .is_none_or(|filter| !filter.is_empty() && !filter.starts_with('-')),
            "Expected a nonempty test-name filter, not a Cargo option"
        );
        let configuration = tempfile::tempdir()?;
        let helper = if let Some(path) = std::env::var_os("LAZYPENCIL_TEST_HELPER") {
            let path = std::path::PathBuf::from(path);
            ensure!(path.is_file(), "Configured test helper is missing");
            path.canonicalize()?
        } else {
            let output = process::checked(
                Command::new("cargo").current_dir(root).args([
                    "build",
                    "--locked",
                    "--jobs",
                    "2",
                    "--manifest-path",
                    "xtask/Cargo.toml",
                    "--bin",
                    "lazypencil-test-helper",
                    "--message-format=json",
                ]),
                Duration::from_secs(600),
            )?;
            String::from_utf8(output)?
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find_map(|artifact| {
                    (artifact["reason"] == "compiler-artifact"
                        && artifact["target"]["name"] == "lazypencil-test-helper")
                        .then(|| {
                            artifact["executable"]
                                .as_str()
                                .map(std::path::PathBuf::from)
                        })
                        .flatten()
                })
                .context("Cargo did not report the native test helper executable")?
        };
        let mut checks: Vec<Vec<String>> = Vec::new();
        let words = |args: &[&str]| args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        if focused {
            let targets = self.lib.then(|| words(&["--lib"])).into_iter().chain(
                self.tests
                    .iter()
                    .map(|name| vec!["--test".into(), name.clone()]),
            );
            for target in targets {
                let mut args = words(&["test", "--locked", "--jobs", "2"]);
                if self.core {
                    args.push("--no-default-features".into());
                }
                args.extend(target);
                if let Some(filter) = &self.filter {
                    args.push(filter.clone());
                }
                args.extend(words(&["--", "--test-threads=2"]));
                checks.push(args);
            }
            if self.lint {
                let mut args = words(&["clippy", "--locked", "--jobs", "2"]);
                if self.core {
                    args.push("--no-default-features".into());
                }
                if self.lib {
                    args.push("--lib".into());
                }
                for target in &self.tests {
                    args.extend(["--test".into(), target.clone()]);
                }
                args.extend(words(&["--", "-D", "warnings"]));
                checks.push(args);
            }
        } else {
            checks.push(words(&[
                "test",
                "--locked",
                "--no-default-features",
                "--no-fail-fast",
                "--",
                "--test-threads=2",
            ]));
            if !self.core {
                checks.push(words(&[
                    "test",
                    "--locked",
                    "--no-fail-fast",
                    "--",
                    "--test-threads=2",
                ]));
                checks.push(words(&[
                    "clippy",
                    "--locked",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ]));
            }
            checks.push(words(&[
                "clippy",
                "--locked",
                "--no-default-features",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ]));
            checks.push(if self.core {
                words(&["build", "--locked", "--no-default-features"])
            } else {
                words(&["build", "--locked"])
            });
        }
        let mut failures = Vec::new();
        for args in checks {
            eprintln!("Running cargo {}", args.join(" "));
            let result = process::output(
                Command::new("cargo")
                    .current_dir(root)
                    .env("LAZYPENCIL_CONFIG_DIR", configuration.path().join("user"))
                    .env("LAZYPENCIL_TEST_HELPER", &helper)
                    .args(&args),
                Duration::from_secs(3600),
            )?;
            std::io::stdout().write_all(&result.stdout)?;
            std::io::stderr().write_all(&result.stderr)?;
            let output = String::from_utf8_lossy(&result.stdout);
            let executed = output.lines().any(|line| {
                line.starts_with("test result: ok. ")
                    && line
                        .split_whitespace()
                        .nth(3)
                        .and_then(|n| n.parse::<usize>().ok())
                        .is_some_and(|n| n > 0)
            });
            if result.timed_out
                || !result.status.success()
                || (focused && args[0] == "test" && !executed)
            {
                failures.push(args);
                if focused {
                    break;
                }
            }
            eprintln!("Finished in {:.3}s", result.elapsed.as_secs_f64());
        }
        ensure!(
            failures.is_empty(),
            "Developer checks failed or selected zero executed tests: {failures:?}"
        );
        Ok(())
    }
}
