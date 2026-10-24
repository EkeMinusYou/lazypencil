//! A guardian observes a separate owner socket, independently of ACP traffic.

use super::super::AgentOptions;
use crate::{
    Error, Result,
    process_owner::{self, Group, GroupControl, stop_group},
};
use serde_json::json;
use std::{
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Child};

const MAGIC: &[u8; 8] = b"LPAP\x02\0\r\n";
const STATUS_BYTES: usize = 13;

pub(super) struct Owner {
    parent: UnixStream,
    worker: UnixStream,
}

impl Owner {
    pub(super) fn new() -> Result<Self> {
        let (parent, worker) = UnixStream::pair().map_err(|e| failure("owner_channel", e))?;
        Ok(Self { parent, worker })
    }

    pub(super) fn command(&self, options: &AgentOptions) -> Result<tokio::process::Command> {
        let helper = options
            .supervisor_executable
            .as_ref()
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                Error::new(
                    "AGENT_UNAVAILABLE",
                    "An absolute lazypencil supervisor executable is required",
                )
            })?;
        let fd = self.worker.as_raw_fd();
        let mut command = tokio::process::Command::new(helper);
        command
            .arg("__agent_process_worker")
            .arg(fd.to_string())
            .arg("--")
            .arg(&options.command)
            .args(&options.args);
        // SAFETY: Only async-signal-safe fcntl calls run after fork. The Owner
        // keeps this exact descriptor alive until spawn returns. No parent FD
        // flags change; the host end remains close-on-exec in every child.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(command)
    }

    pub(super) fn start(mut self) -> Result<UnixStream> {
        self.parent
            .write_all(MAGIC)
            .map_err(|e| failure("owner_start", e))?;
        Ok(self.parent)
    }
}

pub(super) async fn confirm_startup(child: &mut Child, limit: Duration) -> Result<()> {
    let mut frame = [0u8; STATUS_BYTES];
    tokio::time::timeout(
        limit,
        child
            .stderr
            .as_mut()
            .expect("piped stderr")
            .read_exact(&mut frame),
    )
    .await
    .map_err(|_| Error::new("AGENT_ERROR", "Agent initialization timed out"))?
    .map_err(|e| failure("supervisor_status", e))?;
    if &frame[..8] != MAGIC || (frame[8] == 0 && frame[9..] != [0; 4]) {
        return Err(Error::new(
            "AGENT_UNAVAILABLE",
            "Invalid adapter supervisor startup status",
        ));
    }
    if frame[8] != 0 {
        let phase = match frame[8] {
            1 => "owner_start",
            2 => "adapter_spawn",
            3 => "stderr_relay",
            4 => "child_wait_policy",
            _ => "supervisor_startup",
        };
        let code = i32::from_le_bytes(frame[9..].try_into().expect("fixed status size"));
        return Err(failure(
            phase,
            if code == 0 {
                io::Error::other("Supervisor startup failed")
            } else {
                io::Error::from_raw_os_error(code)
            },
        ));
    }
    Ok(())
}

pub(super) fn serve_worker() -> Result<bool> {
    let mut announced = false;
    let result = worker(&mut announced);
    if !announced && let Err(error) = &result {
        let details = error.details.as_ref();
        let phase = match details.and_then(|value| value["supervisor_phase"].as_str()) {
            Some("adapter_spawn") => 2,
            Some("stderr_relay") => 3,
            Some("child_wait_policy") => 4,
            _ => 1,
        };
        let code = details
            .and_then(|value| value["os_error"].as_i64())
            .unwrap_or(0) as i32;
        let _ = status(phase, code);
    }
    result
}

fn worker(announced: &mut bool) -> Result<bool> {
    let mut args = std::env::args_os().skip(2);
    let fd = args
        .next()
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<libc::c_int>().ok())
        .filter(|fd| *fd > libc::STDERR_FILENO)
        .ok_or_else(|| failure("owner_start", io::Error::from(io::ErrorKind::InvalidInput)))?;
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(failure("owner_start", io::ErrorKind::InvalidInput.into()));
    }
    let command = args
        .next()
        .ok_or_else(|| failure("adapter_spawn", io::ErrorKind::InvalidInput.into()))?;
    // Validate the inherited FD before taking ownership, and make it private
    // before spawning any adapter. A missing owner cannot launch an adapter.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(failure("owner_start", io::Error::last_os_error()));
    }
    // SAFETY: The private parent passes exactly this live, inherited endpoint.
    // This fresh worker has not opened another resource at its descriptor.
    let mut owner = unsafe { UnixStream::from_raw_fd(fd) };
    let mut magic = [0u8; MAGIC.len()];
    owner
        .read_exact(&mut magic)
        .map_err(|e| failure("owner_start", e))?;
    if &magic != MAGIC {
        return Err(failure("owner_start", io::ErrorKind::InvalidInput.into()));
    }
    process_owner::prepare().map_err(|e| failure("child_wait_policy", e))?;
    let lost = Arc::new(AtomicBool::new(false));
    let watch = lost.clone();
    let control: GroupControl = Arc::new(Mutex::new(None));
    let watched_group = control.clone();
    std::thread::Builder::new()
        .name("adapter-owner".into())
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
        .map_err(|e| failure("owner_start", e))?;
    if lost.load(Ordering::Acquire) {
        return Err(failure("owner_start", io::ErrorKind::BrokenPipe.into()));
    }
    let child = Command::new(command)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| failure("adapter_spawn", e))?;
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
    let mut stderr = group.child.stderr.take().expect("piped adapter stderr");
    let (release, gate) = std::sync::mpsc::channel();
    let relay = std::thread::Builder::new()
        .name("adapter-stderr".into())
        .spawn(move || {
            if gate.recv().is_ok() {
                let mut buffer = [0u8; 8192];
                let mut output = io::stderr();
                while let Ok(size) = stderr.read(&mut buffer) {
                    if size == 0 || output.write_all(&buffer[..size]).is_err() {
                        break;
                    }
                }
            }
        })
        .map_err(|e| failure("stderr_relay", e))?;
    // The host consumes this bounded private frame before draining stderr.
    // Adapter output cannot race it, and ACP stdout is never modified.
    status(0, 0).map_err(|e| failure("stderr_relay", e))?;
    *announced = true;
    let _ = release.send(());
    group
        .wait_for_root()
        .map_err(|e| failure("adapter_wait", e))?;
    let exit = group.finish().map_err(|e| failure("adapter_wait", e))?;
    drop(relay);
    Ok(exit.success())
}

fn status(phase: u8, code: i32) -> io::Result<()> {
    let mut frame = [0u8; STATUS_BYTES];
    frame[..8].copy_from_slice(MAGIC);
    frame[8] = phase;
    frame[9..].copy_from_slice(&code.to_le_bytes());
    let mut output = io::stderr().lock();
    output.write_all(&frame)?;
    output.flush()
}

fn failure(phase: &str, error: io::Error) -> Error {
    Error::new(
        "AGENT_UNAVAILABLE",
        format!("Cannot supervise ACP adapter ({phase}): {error}"),
    )
    .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error()}))
}
