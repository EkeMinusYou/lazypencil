//! Hosted real-emulator verification; independent of the app and Skia crates.
pub mod raster;

use crate::{process, util};
use anyhow::{Result, ensure};
use clap::Args;
use std::{path::PathBuf, process::Command, time::Duration};

#[cfg(target_os = "linux")]
mod hosted;

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    pub candidate: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long, hide = true)]
    pub under_display: bool,
}

impl Arguments {
    pub fn run(self) -> Result<()> {
        ensure!(
            cfg!(target_os = "linux")
                && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                && std::env::var("RUNNER_OS").as_deref() == Ok("Linux"),
            "Real-emulator verification requires an isolated Linux GHA runner"
        );
        let task = Self {
            candidate: std::path::absolute(self.candidate)?,
            output: std::path::absolute(self.output)?,
            under_display: self.under_display,
        };
        task.run_hosted()
    }

    fn run_hosted(self) -> Result<()> {
        std::fs::create_dir_all(&self.output)?;
        if !self.under_display {
            let result = process::output_limited(
                Command::new("xvfb-run")
                    .args(["-a", "-s", "-screen 0 1600x1000x24 -nolisten tcp"])
                    .arg(std::env::current_exe()?)
                    .arg("terminal")
                    .arg("--candidate")
                    .arg(&self.candidate)
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
                &serde_json::json!({"real_emulators":"passed","protocols":["kitty","sixel"]}),
            );
        }
        #[cfg(target_os = "linux")]
        {
            hosted::run(&self.candidate, &self.output)
        }
        #[cfg(not(target_os = "linux"))]
        {
            unreachable!("Linux guard above")
        }
    }
}
