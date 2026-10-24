use super::AgentOptions;
use crate::{Error, Result};
use std::{process::Stdio, time::Duration};
use tokio::process::Child;
#[cfg(not(windows))]
use tokio::process::Command;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) fn serve_worker() -> Result<bool> {
    windows::serve_worker()
}

pub(super) struct AgentProcess {
    pub(super) child: Child,
    #[cfg(unix)]
    group: Option<u32>,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl AgentProcess {
    pub(super) async fn spawn(options: &AgentOptions) -> Result<Self> {
        #[cfg(windows)]
        let job = windows::Job::new()?;
        #[cfg(windows)]
        let mut command = job.command(options)?;
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new(&options.command);
            command.args(&options.args);
            command
        };
        command
            .current_dir(&options.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if options.subscription {
            for key in [
                "OPENAI_API_KEY",
                "CODEX_API_KEY",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "DEFAULT_AUTH_REQUEST",
                "OPENAI_BASE_URL",
                "ANTHROPIC_BASE_URL",
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
            ] {
                command.env_remove(key);
            }
        }
        #[cfg(unix)]
        command.process_group(0);
        let child = command.spawn().map_err(|error| {
            Error::new(
                "AGENT_UNAVAILABLE",
                format!(
                    "Cannot start ACP adapter {}: {error}",
                    options.command.display()
                ),
            )
        })?;
        let process = Self {
            #[cfg(unix)]
            group: child.id(),
            child,
            #[cfg(windows)]
            job: Some(job),
        };
        #[cfg(windows)]
        let mut process = process;
        #[cfg(windows)]
        if let Err(error) =
            windows::confirm_startup(&mut process.child, options.setup_timeout).await
        {
            process.stop().await;
            return Err(error);
        }
        Ok(process)
    }

    pub(super) async fn stop(&mut self) {
        self.kill_tree();
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        #[cfg(unix)]
        {
            self.group = None;
        }
    }

    fn kill_tree(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group.and_then(|id| i32::try_from(id).ok()) {
            // The adapter starts in its own process group; never signal the parent's group.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            // Closing the host's last handle also covers abnormal parent exit.
            // The worker closes its temporary handle before launching the adapter.
            self.job.take();
        }
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        self.kill_tree();
    }
}
