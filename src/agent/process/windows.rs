use super::super::AgentOptions;
use crate::{Error, Result};
use serde_json::json;
use std::{
    io::{self, Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Child};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError, SetLastError},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, OpenJobObjectW, SetInformationJobObject,
        },
        Threading::{CREATE_NEW_PROCESS_GROUP, GetCurrentProcess},
    },
};

const PREFIX: &str = "Local\\lazypencil-agent-";
const MAGIC: &[u8; 8] = b"LPAP\x01\0\r\n";
const STATUS_BYTES: usize = 13;
// The worker can assign itself, but cannot change limits or terminate the job.
const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x0001;

pub(super) struct Job {
    _handle: OwnedHandle,
    name: String,
}

impl Job {
    pub(super) fn new() -> Result<Self> {
        let name = format!("{PREFIX}{}", uuid::Uuid::new_v4());
        let wide = wide(&name);
        // SAFETY: The name is NUL-terminated; null security attributes make the
        // handle non-inheritable. Only this host retains the owning handle.
        let raw = unsafe {
            SetLastError(0);
            CreateJobObjectW(std::ptr::null(), wide.as_ptr())
        };
        let error = unsafe { GetLastError() };
        if raw.is_null() {
            return Err(failure(
                "job_create",
                io::Error::from_raw_os_error(error as i32),
            ));
        }
        // SAFETY: A successful CreateJobObjectW returns one owned kernel handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if error == ERROR_ALREADY_EXISTS {
            return Err(failure(
                "job_create",
                io::Error::from(io::ErrorKind::AlreadyExists),
            ));
        }
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: The handle is live and the pointer/size describe the declared
        // Windows information class. No breakaway or handle inheritance is enabled.
        let success = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if success == 0 {
            return Err(failure("job_limits", io::Error::last_os_error()));
        }
        Ok(Self {
            _handle: handle,
            name,
        })
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
            .arg(&self.name)
            .arg("--")
            .arg(&options.command)
            .args(&options.args)
            // Keep Ctrl+C with the host, which sends ACP cancellation first.
            .creation_flags(CREATE_NEW_PROCESS_GROUP);
        Ok(command)
    }
}

pub(super) async fn confirm_startup(child: &mut Child, limit: Duration) -> Result<()> {
    let mut status = [0u8; STATUS_BYTES];
    tokio::time::timeout(
        limit,
        child
            .stderr
            .as_mut()
            .expect("piped stderr")
            .read_exact(&mut status),
    )
    .await
    .map_err(|_| Error::new("AGENT_ERROR", "Agent initialization timed out"))?
    .map_err(|error| failure("supervisor_status", error))?;
    if &status[..8] != MAGIC {
        return Err(Error::new(
            "AGENT_UNAVAILABLE",
            "Invalid adapter supervisor startup status",
        ));
    }
    if status[8] != 0 {
        let phase = match status[8] {
            1 => "job_join",
            2 => "adapter_spawn",
            3 => "stderr_relay",
            _ => "supervisor_startup",
        };
        let code = i32::from_le_bytes(status[9..13].try_into().expect("fixed status size"));
        let error = if code == 0 {
            io::Error::other("Supervisor startup failed")
        } else {
            io::Error::from_raw_os_error(code)
        };
        return Err(failure(phase, error));
    }
    if status[9..13] != [0; 4] {
        return Err(Error::new(
            "AGENT_UNAVAILABLE",
            "Invalid adapter supervisor success status",
        ));
    }
    Ok(())
}

pub(super) fn serve_worker() -> Result<bool> {
    let result = worker();
    if let Err(error) = &result {
        let details = error.details.as_ref();
        let phase = match details.and_then(|value| value["supervisor_phase"].as_str()) {
            Some("adapter_spawn") => 2,
            Some("stderr_relay") => 3,
            _ => 1,
        };
        let code = details
            .and_then(|value| value["os_error"].as_i64())
            .unwrap_or(0) as i32;
        let _ = status(phase, code);
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
    let wide = wide(&name);
    // SAFETY: The bounded private name is NUL-terminated. The temporary handle
    // is explicitly non-inheritable and requests only self-assignment rights.
    let raw = unsafe { OpenJobObjectW(JOB_OBJECT_ASSIGN_PROCESS, 0, wide.as_ptr()) };
    if raw.is_null() {
        return Err(failure("job_join", io::Error::last_os_error()));
    }
    // SAFETY: OpenJobObjectW returned a new owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: Both handles are live; the current-process pseudo handle is borrowed.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) } == 0 {
        return Err(failure("job_join", io::Error::last_os_error()));
    }
    // This must precede adapter creation: the parent is the sole long-lived
    // owner. If it already died, closing this last handle terminates us here.
    drop(job);

    let mut child = std::process::Command::new(command)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| failure("adapter_spawn", error))?;
    let mut stderr = child.stderr.take().expect("piped adapter stderr");
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
        });
    let relay = match relay {
        Ok(relay) => relay,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure("stderr_relay", error));
        }
    };
    // No adapter stderr can precede this fixed private header. The host reads
    // exactly 13 bytes, then resumes its existing bounded, credential-free drain.
    if let Err(error) = status(0, 0) {
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

fn status(phase: u8, code: i32) -> io::Result<()> {
    let mut frame = [0u8; STATUS_BYTES];
    frame[..8].copy_from_slice(MAGIC);
    frame[8] = phase;
    frame[9..13].copy_from_slice(&code.to_le_bytes());
    let mut stderr = io::stderr().lock();
    stderr.write_all(&frame)?;
    stderr.flush()
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn failure(phase: &str, error: io::Error) -> Error {
    Error::new("AGENT_UNAVAILABLE", "Cannot start or supervise ACP adapter")
        .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error(),"io_kind":format!("{:?}",error.kind())}))
}
