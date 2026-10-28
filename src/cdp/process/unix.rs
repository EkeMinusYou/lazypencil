//! A private guardian owns the Chromium process group and observes host EOF.

use super::{Budget, Owned};
use crate::{
    Error, Result,
    cdp::failed,
    process_owner::{self, Group, GroupControl, stop_group},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
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
    persistent: Option<(&Path, &str)>,
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
    let mut worker = Command::new(helper);
    worker.arg("__browser_process_worker");
    if let Some((profile, identity)) = persistent {
        worker
            .arg("--persistent-profile")
            .arg(profile)
            .arg("--profile-identity")
            .arg(identity);
    }
    let child = worker
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

pub fn serve_worker() -> Result<bool> {
    let mut args = std::env::args_os().skip(2);
    let first = args.next();
    let persistent = if first.as_deref() == Some(std::ffi::OsStr::new("--persistent-profile")) {
        let path = args
            .next()
            .ok_or_else(|| failed("Missing dedicated profile path"))?;
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--profile-identity")) {
            return Err(failed("Missing dedicated profile identity"));
        }
        let identity = args
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or_else(|| failed("Invalid dedicated profile identity"))?;
        if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
            return Err(failed("Invalid browser worker arguments"));
        }
        Some((std::path::PathBuf::from(path), identity))
    } else if first.as_deref() == Some(std::ffi::OsStr::new("--")) {
        None
    } else {
        return Err(failed("Invalid browser worker arguments"));
    };
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
        process_owner::prepare().map_err(|e| failure("child_wait_policy", e))?;
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
        let persistent = persistent
            .as_ref()
            .map(|(path, identity)| {
                let profile = crate::runtime::profiles::Profile::from_directory(path)?;
                if profile.identity() != identity {
                    return Err(Error::new(
                        "RUNTIME_UNAVAILABLE",
                        "Dedicated profile identity changed",
                    )
                    .details(json!({"stage":"profile_identity"})));
                }
                Ok(profile)
            })
            .transpose()?;
        let _lease = persistent
            .as_ref()
            .map(crate::runtime::profiles::Profile::lease)
            .transpose()?;
        // Only freshly created ephemeral directories are recursively removed.
        // A persistent profile and its lease outlive the complete browser group.
        let profile = if persistent.is_none() {
            Some(
                tempfile::Builder::new()
                    .prefix("lazypencil-chromium-")
                    .tempdir()
                    .map_err(|e| failure("browser_profile", e))?,
            )
        } else {
            None
        };
        let directory = persistent
            .as_ref()
            .map(crate::runtime::profiles::Profile::directory)
            .or_else(|| profile.as_ref().map(tempfile::TempDir::path))
            .expect("browser profile");
        if persistent.is_some() {
            match fs::remove_file(directory.join("DevToolsActivePort")) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(failure("profile_port_cleanup", error)),
            }
        }
        let child = Command::new(command)
            .arg(format!("--user-data-dir={}", directory.display()))
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
        publish(&json!({"pid":group.child.id(),"profile":directory}))?;
        announced = true;
        group
            .wait_for_root()
            .map_err(|e| failure("browser_wait", e))?;
        let status = group.finish().map_err(|e| failure("browser_wait", e))?;
        if let Some(profile) = profile {
            profile.close().map_err(|e| failure("profile_cleanup", e))?;
        }
        Ok(status.success())
    })();
    if !announced && let Err(error) = &result {
        let details = error.details.as_ref();
        let _ = publish(
            &json!({"phase":details.and_then(|d| d["supervisor_phase"].as_str().or_else(|| d["stage"].as_str())).unwrap_or("supervisor_status"),"os_error":details.and_then(|d| d["os_error"].as_i64()).unwrap_or(0)}),
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
