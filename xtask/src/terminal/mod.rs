//! Hosted real-emulator verification; independent of the app and Skia crates.
pub mod raster;

use crate::{evidence, process, util};
use anyhow::{Result, ensure};
use clap::Args;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[cfg(target_os = "linux")]
mod hosted;

#[derive(Args)]
pub struct Arguments {
    #[arg(
        long,
        required_unless_present = "development_build",
        conflicts_with = "development_build"
    )]
    pub candidate: Option<PathBuf>,
    /// Reuse the debug executable already built by this hosted focused job.
    #[arg(long)]
    pub development_build: bool,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long, hide = true)]
    pub under_display: bool,
}

impl Arguments {
    pub fn run(self) -> Result<()> {
        ensure!(
            self.candidate.is_some() != self.development_build,
            "Select one terminal verification input"
        );
        ensure!(
            cfg!(target_os = "linux")
                && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                && std::env::var("RUNNER_OS").as_deref() == Ok("Linux"),
            "Real-emulator verification requires an isolated Linux GHA runner"
        );
        let task = Self {
            candidate: self.candidate.map(std::path::absolute).transpose()?,
            development_build: self.development_build,
            output: std::path::absolute(self.output)?,
            under_display: self.under_display,
        };
        task.run_hosted()
    }

    fn run_hosted(self) -> Result<()> {
        std::fs::create_dir_all(&self.output)?;
        if !self.under_display {
            let mut command = Command::new("xvfb-run");
            // Terminals are sequential clients of one owned display. Do not
            // reset the server between closing one and connecting the next.
            command
                .args(["-a", "-s", "-screen 0 1600x1000x24 -nolisten tcp -noreset"])
                .arg(std::env::current_exe()?)
                .arg("terminal");
            if let Some(candidate) = &self.candidate {
                command.arg("--candidate").arg(candidate);
            } else {
                command.arg("--development-build");
            }
            let result = process::output_limited(
                command
                    .arg("--output")
                    .arg(&self.output)
                    .arg("--under-display")
                    .env("LIBGL_ALWAYS_SOFTWARE", "1"),
                Duration::from_secs(300),
                1024 * 1024,
            )?;
            std::fs::write(self.output.join("display.stdout.log"), &result.stdout)?;
            std::fs::write(self.output.join("display.stderr.log"), &result.stderr)?;
            ensure!(
                !result.timed_out && result.status.success(),
                "Emulator verification failed; see {}",
                self.output.display()
            );
            return util::print(
                &serde_json::json!({"real_emulators":"passed","protocols":["kitty","auto-kitty","sixel","auto-sixel"]}),
            );
        }
        #[cfg(target_os = "linux")]
        {
            hosted::run(self.candidate.as_deref(), &self.output)
        }
        #[cfg(not(target_os = "linux"))]
        {
            unreachable!("Linux guard above")
        }
    }
}

/// This is an early reproduction of the current job's development executable.
/// Packaged release provenance, notices and isolation are checked separately.
pub fn development_identity(
    source: &Path,
    binary: &Path,
    revision: &str,
) -> Result<serde_json::Value> {
    let source = evidence::source_identity(source, revision)?;
    let executable = evidence::fingerprint(binary, "debug/lazypencil", 512 * 1024 * 1024)?;
    ensure!(
        !cfg!(unix) || util::mode(binary)? == 0o755,
        "Development binary is not executable"
    );
    ensure!(
        executable["bytes"].as_u64().is_some_and(|n| n > 0),
        "Empty development executable"
    );
    Ok(serde_json::json!({
        "kind":"focused-development-reproduction", "release_ready":false,
        "source_revision":revision, "source":source,
        "target":evidence::linkage::native_host()?,
        "binary_sha256":executable["sha256"], "binary_bytes":executable["bytes"],
        "scope":"Executable already compiled by this focused CI job; not packaged release acceptance"
    }))
}
