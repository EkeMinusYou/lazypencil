use super::AgentOptions;
use crate::{Error, Result};
use std::{process::Stdio, time::Duration};
use tokio::process::Child;
#[cfg(not(any(windows, unix)))]
use tokio::process::Command;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(super) fn serve_worker() -> Result<bool> {
    unix::serve_worker()
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) fn serve_worker() -> Result<bool> {
    windows::serve_worker()
}

pub(super) struct AgentProcess {
    child: Option<Child>,
    #[cfg(unix)]
    owner: Option<std::os::unix::net::UnixStream>,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl AgentProcess {
    pub(super) async fn spawn(options: &AgentOptions) -> Result<Self> {
        #[cfg(windows)]
        let job = windows::Job::new()?;
        #[cfg(windows)]
        let mut command = job.command(options)?;
        #[cfg(unix)]
        let owner = unix::Owner::new()?;
        #[cfg(unix)]
        let mut command = owner.command(options)?;
        #[cfg(not(any(windows, unix)))]
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
            // Unix guardians must survive long enough to reap their owned group.
            .kill_on_drop(!cfg!(unix));
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
            owner: Some(owner.start()?),
            child: Some(child),
            #[cfg(windows)]
            job: Some(job),
        };
        #[cfg(any(windows, unix))]
        let mut process = process;
        #[cfg(unix)]
        if let Err(error) = unix::confirm_startup(process.child_mut(), options.setup_timeout).await
        {
            process.stop().await;
            return Err(error);
        }
        #[cfg(windows)]
        if let Err(error) =
            windows::confirm_startup(process.child_mut(), options.setup_timeout).await
        {
            process.stop().await;
            return Err(error);
        }
        Ok(process)
    }

    pub(super) fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("live agent guardian")
    }

    pub(super) async fn stop(&mut self) {
        self.kill_tree();
        #[cfg(not(unix))]
        let _ = self.child_mut().start_kill();
        #[cfg(unix)]
        self.child_mut().stdin.take();
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child_mut().wait()).await;
    }

    fn kill_tree(&mut self) {
        #[cfg(unix)]
        {
            // ACP backpressure cannot hold this independent owner channel open.
            // EOF also covers a killed host or an abruptly dropped Tokio runtime.
            self.owner.take();
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
        #[cfg(unix)]
        if let Some(mut child) = self.child.take()
            && matches!(child.try_wait(), Ok(None))
        {
            // Tokio's best-effort orphan queue needs a running signal driver.
            // Keep this exact owned Child until it is reaped even when the
            // runtime itself has been dropped. Never wait on a recycled PID.
            std::thread::spawn(move || {
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) | Err(_) => break,
                        Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            });
        }
    }
}
