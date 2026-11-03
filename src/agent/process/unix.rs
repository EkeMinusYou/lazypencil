//! A guardian observes a separate owner socket, independently of ACP traffic.

use super::super::AgentOptions;
use crate::{
    Error, Result,
    process_owner::{self, OwnerWatch},
};
use serde_json::json;
use std::{
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Command, Stdio},
    time::Duration,
};
use tokio::process::Child;

const MAGIC: &[u8; 8] = b"LPAP\x02\0\r\n";
const STARTUP: super::startup::Protocol = super::startup::Protocol::new(
    MAGIC,
    &[
        "owner_start",
        "adapter_spawn",
        "stderr_relay",
        "child_wait_policy",
    ],
    "Invalid adapter supervisor startup status",
);

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
    STARTUP.confirm(child, limit, failure).await
}

pub(super) fn serve_worker() -> Result<bool> {
    let mut announced = false;
    let result = worker(&mut announced);
    if !announced && let Err(error) = &result {
        STARTUP.report_error(error);
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
    let watch = OwnerWatch::start(owner, "adapter-owner").map_err(|e| failure("owner_start", e))?;
    if watch.is_lost() {
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
    let mut group = watch.supervise(child);
    let stderr = group.child.stderr.take().expect("piped adapter stderr");
    let (release, relay) = super::startup::relay(stderr).map_err(|e| failure("stderr_relay", e))?;
    // The host consumes this bounded private frame before draining stderr.
    // Adapter output cannot race it, and ACP stdout is never modified.
    STARTUP
        .write(0, 0)
        .map_err(|e| failure("stderr_relay", e))?;
    *announced = true;
    let _ = release.send(());
    group
        .wait_for_root()
        .map_err(|e| failure("adapter_wait", e))?;
    let exit = group.finish().map_err(|e| failure("adapter_wait", e))?;
    drop(relay);
    Ok(exit.success())
}

fn failure(phase: &str, error: io::Error) -> Error {
    Error::new(
        "AGENT_UNAVAILABLE",
        format!("Cannot supervise ACP adapter ({phase}): {error}"),
    )
    .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error()}))
}
