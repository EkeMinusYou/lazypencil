//! PID-pinned process-group cleanup inside private Unix guardian processes.

#[cfg(target_os = "linux")]
use std::{fs, time::Duration};
use std::{
    io,
    process::{Child, ExitStatus},
    sync::{Arc, Mutex},
};

/// Apply child-wait policy only in a freshly started private guardian.
pub(crate) fn prepare() -> io::Result<()> {
    // SAFETY: This fresh worker has not spawned threads or children. Ignored
    // SIGCHLD would defeat the unreaped PID ownership used below.
    if unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) } == libc::SIG_ERR {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_os = "linux")]
    enable_subreaper()?;
    Ok(())
}

pub(crate) type GroupControl = Arc<Mutex<Option<libc::pid_t>>>;

pub(crate) fn stop_group(control: &GroupControl) {
    let mut group = control.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(group) = group.take() {
        // SAFETY: The guardian owns this unreaped group leader. The same lock
        // clears its ID before reaping, so the owner watcher cannot signal a
        // recycled PID even when shutdown races normal browser exit.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

pub(crate) struct Group {
    pub(crate) child: Child,
    pub(crate) control: GroupControl,
    pub(crate) reaped: bool,
}

impl Group {
    pub(crate) fn wait_for_root(&mut self) -> io::Result<()> {
        loop {
            // SAFETY: Zero is a valid empty C status. This private guardian
            // waits only for its own children; WNOWAIT pins each reported PID.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result =
                unsafe { libc::waitid(libc::P_ALL, 0, &mut info, libc::WEXITED | libc::WNOWAIT) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.control
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take();
                    self.reaped = true;
                }
                return Err(error);
            }
            #[cfg(target_os = "linux")]
            let pid = unsafe { info.si_pid() };
            #[cfg(not(target_os = "linux"))]
            let pid = info.si_pid;
            if info.si_signo == 0 || pid <= 0 {
                return Err(io::Error::other("Invalid owned process wait status"));
            }
            if pid as u32 == self.child.id() {
                return Ok(());
            }
            // Linux adopts orphaned descendants while the root is still live.
            // Reap exited descendants immediately rather than retain zombies
            // until idle shutdown. The root's group remains pinned throughout.
            loop {
                let waited = unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
                if waited == pid {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }

    pub(crate) fn finish(&mut self) -> io::Result<ExitStatus> {
        // Clear the shared group ID before consuming the leader's status.
        stop_group(&self.control);
        let status = self.child.wait().inspect_err(|error| {
            if error.raw_os_error() == Some(libc::ECHILD) {
                self.reaped = true;
            }
        })?;
        self.reaped = true;
        #[cfg(target_os = "linux")]
        drain_adopted_children()?;
        Ok(status)
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.finish();
        }
    }
}

#[cfg(target_os = "linux")]
fn children_path() -> std::path::PathBuf {
    format!("/proc/self/task/{}/children", std::process::id()).into()
}

#[cfg(target_os = "linux")]
fn enable_subreaper() -> io::Result<()> {
    // Validate the cleanup interface before creating any child. This policy is
    // local to the private guardian, never the embedding application's process.
    fs::read_to_string(children_path())?;
    // SAFETY: PR_SET_CHILD_SUBREAPER takes a boolean integer and no pointers.
    if unsafe {
        libc::prctl(
            libc::PR_SET_CHILD_SUBREAPER,
            1 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn drain_adopted_children() -> io::Result<()> {
    loop {
        let children = fs::read_to_string(children_path())?;
        if children.trim().is_empty() {
            return Ok(());
        }
        // No thread in this worker reaps children while the list is being
        // signalled. Live children or their zombies therefore pin these PIDs.
        for child in children.split_ascii_whitespace() {
            let pid: libc::pid_t = child
                .parse()
                .map_err(|_| io::Error::other("Invalid owned child PID"))?;
            if pid <= 0 {
                return Err(io::Error::other("Invalid owned child PID"));
            }
            // SAFETY: procfs enumerates this worker's own unreaped children.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        loop {
            // SAFETY: This private subreaper owns all children it waits for.
            let reaped = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
            if reaped > 0 {
                continue;
            }
            if reaped == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ECHILD) {
                    return Ok(());
                }
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            break;
        }
        // Keep ownership until all adopted descendants are reaped. The host's
        // bounded stop can hand this wait to its reaper thread if the OS stalls.
        std::thread::sleep(Duration::from_millis(5));
    }
}
