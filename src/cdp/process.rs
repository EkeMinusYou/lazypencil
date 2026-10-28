//! Ownership of the browser launcher and its platform lifetime boundary.

use super::Budget;
#[cfg(not(any(windows, unix)))]
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
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::serve_worker;

pub(super) struct Owned {
    #[cfg(not(unix))]
    child: Child,
    #[cfg(unix)]
    child: Option<Child>,
    #[cfg(unix)]
    profile: std::path::PathBuf,
    #[cfg(any(windows, unix))]
    browser_pid: u32,
    #[cfg(windows)]
    job: Option<windows::Job>,
}

impl Owned {
    pub(super) fn spawn(
        command: Command,
        supervisor: Option<&Path>,
        profile: Option<&Path>,
        persistent: Option<(&Path, &str)>,
        budget: &Budget<'_>,
    ) -> Result<Self> {
        budget.check()?;
        if persistent.is_some() && !cfg!(unix) {
            return Err(crate::Error::new(
                "UNSUPPORTED_FEATURE",
                "Dedicated browser profiles require a supported Unix host",
            ));
        }
        #[cfg(windows)]
        {
            windows::spawn(
                command,
                supervisor,
                profile.expect("host browser profile"),
                budget,
            )
        }
        #[cfg(unix)]
        {
            debug_assert!(profile.is_none());
            unix::spawn(command, supervisor, persistent, budget)
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = (supervisor, profile);
            let mut command = command;
            let child = command
                .spawn()
                .map_err(|e| failed(format!("Cannot launch Chromium: {e}")))?;
            Ok(Self { child })
        }
    }

    #[cfg(unix)]
    pub(super) fn profile_path(&self) -> &Path {
        &self.profile
    }

    pub(super) fn id(&self) -> u32 {
        #[cfg(any(windows, unix))]
        {
            self.browser_pid
        }
        #[cfg(not(any(windows, unix)))]
        {
            self.child.id()
        }
    }

    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        #[cfg(unix)]
        {
            match &mut self.child {
                Some(child) => child.try_wait(),
                None => Ok(None),
            }
        }
        #[cfg(not(unix))]
        {
            self.child.try_wait()
        }
    }

    pub(super) fn stop(&mut self) {
        #[cfg(windows)]
        self.job.take();
        #[cfg(unix)]
        if let Some(child) = self.child.take() {
            unix::stop(child);
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        self.stop();
    }
}
