use super::super::AgentOptions;
use crate::{Error, Result};
use serde_json::json;
use std::{io, process::Stdio, time::Duration};
use tokio::process::Child;
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

const PREFIX: &str = "Local\\lazypencil-agent-";
const MAGIC: &[u8; 8] = b"LPAP\x01\0\r\n";
const STARTUP: super::startup::Protocol = super::startup::Protocol::new(
    MAGIC,
    &["job_join", "adapter_spawn", "stderr_relay"],
    "Invalid adapter supervisor success status",
);

pub(super) struct Job(crate::process_job::Job);

impl Job {
    pub(super) fn new() -> Result<Self> {
        crate::process_job::Job::new(PREFIX)
            .map(Self)
            .map_err(|error| failure(error.phase, error.error))
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
        let mut command = tokio::process::Command::new(helper);
        command
            .arg("__agent_process_worker")
            .arg(self.0.name())
            .arg("--")
            .arg(&options.command)
            .args(&options.args)
            // Keep Ctrl+C with the host, which sends ACP cancellation first.
            .creation_flags(CREATE_NEW_PROCESS_GROUP);
        Ok(command)
    }
}

pub(super) async fn confirm_startup(child: &mut Child, limit: Duration) -> Result<()> {
    STARTUP.confirm(child, limit, failure).await
}

pub(super) fn serve_worker() -> Result<bool> {
    let result = worker();
    if let Err(error) = &result {
        STARTUP.report_error(error);
    }
    result
}

fn worker() -> Result<bool> {
    let mut args = std::env::args_os().skip(2);
    let name = args
        .next()
        .and_then(|value| value.into_string().ok())
        .filter(|value| {
            value
                .strip_prefix(PREFIX)
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        })
        .ok_or_else(|| failure("job_join", io::Error::from(io::ErrorKind::InvalidInput)))?;
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(failure(
            "job_join",
            io::Error::from(io::ErrorKind::InvalidInput),
        ));
    }
    let command = args.next().ok_or_else(|| {
        failure(
            "adapter_spawn",
            io::Error::from(io::ErrorKind::InvalidInput),
        )
    })?;
    crate::process_job::join(&name).map_err(|error| failure("job_join", error))?;

    let mut child = std::process::Command::new(command)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| failure("adapter_spawn", error))?;
    let stderr = child.stderr.take().expect("piped adapter stderr");
    let (release, relay) = match super::startup::relay(stderr) {
        Ok(relay) => relay,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure("stderr_relay", error));
        }
    };
    // No adapter stderr can precede this fixed private header. The host reads
    // exactly 13 bytes, then resumes its existing bounded, credential-free drain.
    if let Err(error) = STARTUP.write(0, 0) {
        drop(release);
        let _ = child.kill();
        let _ = child.wait();
        return Err(failure("stderr_relay", error));
    }
    let _ = release.send(());
    let exit = child
        .wait()
        .map_err(|error| failure("adapter_wait", error))?;
    // Descendants may still hold stderr. Never wait for their pipe at normal exit;
    // the host owns termination of the complete job.
    drop(relay);
    Ok(exit.success())
}

fn failure(phase: &str, error: io::Error) -> Error {
    Error::new("AGENT_UNAVAILABLE", "Cannot start or supervise ACP adapter")
        .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error(),"io_kind":format!("{:?}",error.kind())}))
}
