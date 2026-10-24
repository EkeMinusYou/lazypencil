//! A private guardian owns the Chromium process group and observes host EOF.

use super::{Budget, Owned};
use crate::{Error, Result, cdp::failed};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAGIC: &[u8; 8] = b"LPBO\x02\0\r\n";
const MAX_STATUS: usize = 4096;

pub(super) fn spawn(
    command: Command,
    supervisor: Option<&Path>,
    budget: &Budget<'_>,
) -> Result<Owned> {
    let current;
    let helper = match supervisor {
        Some(path) => path,
        None => {
            current = std::env::current_exe().map_err(|e| failure("supervisor_path", e))?;
            &current
        }
    };
    if !helper.is_absolute() {
        return Err(failed(
            "An absolute lazypencil browser supervisor executable is required",
        ));
    }
    let child = Command::new(helper)
        .arg("__browser_process_worker")
        .arg("--")
        .arg(command.get_program())
        .args(command.get_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| failure("supervisor_spawn", e))?;
    let mut owned = Owned {
        child: Some(child),
        browser_pid: 0,
        profile: std::path::PathBuf::new(),
    };
    let mut output = owned
        .child
        .as_mut()
        .expect("live guardian")
        .stdout
        .take()
        .expect("status pipe");
    let fd = output.as_raw_fd();
    // SAFETY: This is our owned pipe descriptor. Only status reads use it.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(failure("supervisor_status", io::Error::last_os_error()));
    }
    budget.check()?;
    owned
        .child
        .as_mut()
        .expect("live guardian")
        .stdin
        .as_mut()
        .expect("owner pipe")
        .write_all(MAGIC)
        .map_err(|e| failure("owner_start", e))?;
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        budget.check()?;
        match output.read(&mut chunk) {
            Ok(0) => {
                return Err(failed(
                    "Browser supervisor exited before publishing its status",
                ));
            }
            Ok(count) => {
                data.extend_from_slice(&chunk[..count]);
                if data.len() > MAX_STATUS {
                    return Err(failed("Oversized browser supervisor status"));
                }
                if let Some(end) = data.iter().position(|byte| *byte == b'\n') {
                    if end + 1 != data.len() {
                        return Err(failed("Unexpected trailing browser supervisor status"));
                    }
                    let value: Value = serde_json::from_slice(&data[..end])
                        .map_err(|_| failed("Invalid browser supervisor status"))?;
                    budget.check()?;
                    if let Some(pid) = value["pid"]
                        .as_u64()
                        .and_then(|pid| u32::try_from(pid).ok())
                        .filter(|pid| *pid != 0)
                    {
                        let profile = value["profile"]
                            .as_str()
                            .map(std::path::PathBuf::from)
                            .filter(|path| path.is_absolute())
                            .ok_or_else(|| failed("Invalid browser supervisor profile"))?;
                        owned.browser_pid = pid;
                        owned.profile = profile;
                        return Ok(owned);
                    }
                    let phase = value["phase"].as_str().unwrap_or("supervisor_status");
                    let code = value["os_error"].as_i64().unwrap_or(0) as i32;
                    return Err(failure(
                        phase,
                        if code == 0 {
                            io::Error::other("Browser supervisor failed")
                        } else {
                            io::Error::from_raw_os_error(code)
                        },
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(failure("supervisor_status", error)),
        }
    }
}

pub(super) fn stop(mut child: Child) {
    // Only the host has this write end; EOF also covers SIGKILL/abnormal exit.
    child.stdin.take();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Err(_) => break,
            Ok(None) => {}
        }
        if Instant::now() >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Keep the guardian (and its group ownership) alive until cleanup finishes.
    // A stalled OS wait must not block the application's UI or lose its waiter.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

type GroupControl = Arc<Mutex<Option<libc::pid_t>>>;

fn stop_group(control: &GroupControl) {
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

struct Group {
    child: Child,
    control: GroupControl,
    reaped: bool,
}

impl Group {
    fn wait_for_root(&mut self) -> io::Result<()> {
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
                return Err(io::Error::other("Invalid browser wait status"));
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

    fn finish(&mut self) -> io::Result<ExitStatus> {
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

pub fn serve_worker() -> Result<bool> {
    let mut args = std::env::args_os().skip(2);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(failed("Invalid browser worker arguments"));
    }
    let command = args
        .next()
        .ok_or_else(|| failed("Missing browser executable"))?;
    let mut announced = false;
    let result = (|| {
        // SAFETY: The private worker owns stdin, which is the host's pipe.
        let mut owner = unsafe { fs::File::from_raw_fd(libc::STDIN_FILENO) };
        let mut magic = [0u8; MAGIC.len()];
        owner
            .read_exact(&mut magic)
            .map_err(|e| failure("owner_start", e))?;
        if &magic != MAGIC {
            return Err(failed("Invalid browser owner handshake"));
        }
        // SAFETY: Reset only this fresh worker's inherited disposition, before
        // it spawns threads/children. Ignored SIGCHLD would defeat PID pinning.
        if unsafe { libc::signal(libc::SIGCHLD, libc::SIG_DFL) } == libc::SIG_ERR {
            return Err(failure("child_wait_policy", io::Error::last_os_error()));
        }
        #[cfg(target_os = "linux")]
        enable_subreaper().map_err(|e| failure("child_wait_policy", e))?;
        let lost = Arc::new(AtomicBool::new(false));
        let watch = lost.clone();
        let control: GroupControl = Arc::new(Mutex::new(None));
        let watched_group = control.clone();
        std::thread::Builder::new()
            .name("browser-owner".into())
            .spawn(move || {
                let mut byte = [0u8; 1];
                loop {
                    match owner.read(&mut byte) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        _ => {
                            watch.store(true, Ordering::Release);
                            stop_group(&watched_group);
                            break;
                        }
                    }
                }
            })
            .map_err(|e| failure("owner_watch", e))?;
        if lost.load(Ordering::Acquire) {
            return Err(failure(
                "owner_start",
                io::Error::from(io::ErrorKind::BrokenPipe),
            ));
        }
        // This worker creates and owns the directory it will remove. No
        // caller-supplied path is ever adopted for recursive cleanup.
        let profile = tempfile::Builder::new()
            .prefix("lazypencil-chromium-")
            .tempdir()
            .map_err(|e| failure("browser_profile", e))?;
        let child = Command::new(command)
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|e| failure("browser_spawn", e))?;
        let mut group = Group {
            child,
            control,
            reaped: false,
        };
        *group
            .control
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(group.child.id() as libc::pid_t);
        if lost.load(Ordering::Acquire) {
            stop_group(&group.control);
        }
        publish(&json!({"pid":group.child.id(),"profile":profile.path()}))?;
        announced = true;
        group
            .wait_for_root()
            .map_err(|e| failure("browser_wait", e))?;
        let status = group.finish().map_err(|e| failure("browser_wait", e))?;
        profile.close().map_err(|e| failure("profile_cleanup", e))?;
        Ok(status.success())
    })();
    if !announced && let Err(error) = &result {
        let details = error.details.as_ref();
        let _ = publish(
            &json!({"phase":details.and_then(|d| d["supervisor_phase"].as_str()).unwrap_or("supervisor_status"),"os_error":details.and_then(|d| d["os_error"].as_i64()).unwrap_or(0)}),
        );
    }
    result
}

fn publish(value: &Value) -> Result<()> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|_| failed("Cannot encode browser supervisor status"))?;
    bytes.push(b'\n');
    if bytes.len() > MAX_STATUS {
        return Err(failed("Oversized browser supervisor status"));
    }
    // The fresh status pipe has room for this bounded message. Only this
    // worker writes it; the host also handles partial reads and premature EOF.
    io::stdout()
        .lock()
        .write_all(&bytes)
        .map_err(|e| failure("supervisor_status", e))
}

fn failure(phase: &str, error: io::Error) -> Error {
    failed(format!("Cannot supervise Chromium ({phase}): {error}"))
        .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error()}))
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
