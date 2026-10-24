//! Ownership of the browser launcher and its platform lifetime boundary.

use super::Budget;
#[cfg(not(windows))]
use super::failed;
use crate::Result;
use std::{
    path::Path,
    process::{Child, Command, ExitStatus},
};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::serve_worker;

pub(super) struct Owned {
    child: Child,
    #[cfg(windows)]
    browser_pid: u32,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl Owned {
    pub(super) fn spawn(
        command: Command,
        supervisor: Option<&Path>,
        profile: &Path,
        budget: &Budget<'_>,
    ) -> Result<Self> {
        budget.check()?;
        #[cfg(windows)]
        {
            windows::spawn(command, supervisor, profile, budget)
        }
        #[cfg(not(windows))]
        {
            let _ = (supervisor, profile);
            let mut command = command;
            let child = command
                .spawn()
                .map_err(|e| failed(format!("Cannot launch Chromium: {e}")))?;
            Ok(Self { child })
        }
    }

    pub(super) fn id(&self) -> u32 {
        #[cfg(windows)]
        {
            self.browser_pid
        }
        #[cfg(not(windows))]
        {
            self.child.id()
        }
    }

    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(super) fn stop(&mut self) {
        #[cfg(windows)]
        self.job.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        self.stop();
    }
}
