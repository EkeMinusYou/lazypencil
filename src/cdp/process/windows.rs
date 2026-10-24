//! The host alone owns the kill-on-close job. A worker joins it before spawning Chromium.

use super::{Budget, Owned};
use crate::cdp::failed;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
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

const PREFIX: &str = "Local\\lazypencil-browser-";
const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x0001;
const MAX_STATUS: u64 = 4096;

pub(super) struct Job {
    _handle: OwnedHandle,
    name: String,
}

impl Job {
    fn new() -> Result<Self> {
        let name = format!("{PREFIX}{}", uuid::Uuid::new_v4());
        let wide = wide(&name);
        // SAFETY: The NUL-terminated name is valid, and null security attributes
        // create a non-inheritable handle. Only this host retains the owner.
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
        // SAFETY: CreateJobObjectW returned one owned handle.
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
        // SAFETY: The live handle and pointer/size match the declared class.
        // Breakaway and job-handle inheritance are deliberately not enabled.
        if unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(failure("job_limits", io::Error::last_os_error()));
        }
        Ok(Self {
            _handle: handle,
            name,
        })
    }
}

pub(super) fn spawn(
    command: Command,
    supervisor: Option<&Path>,
    profile: &Path,
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
    let job = Job::new()?;
    let status = profile.join(format!("lazypencil-process-{}.json", uuid::Uuid::new_v4()));
    let child = Command::new(helper)
        .arg("__browser_process_worker")
        .arg(&job.name)
        .arg(&status)
        .arg("--")
        .arg(command.get_program())
        .args(command.get_args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| failure("supervisor_spawn", e))?;
    let mut owned = Owned {
        child,
        browser_pid: 0,
        job: Some(job),
    };
    loop {
        budget.check()?;
        match fs::File::open(&status) {
            Ok(file) => {
                let mut data = Vec::new();
                file.take(MAX_STATUS + 1)
                    .read_to_end(&mut data)
                    .map_err(|e| failure("supervisor_status", e))?;
                if data.len() as u64 > MAX_STATUS {
                    return Err(failed("Oversized browser supervisor status"));
                }
                let value: Value = serde_json::from_slice(&data)
                    .map_err(|_| failed("Invalid browser supervisor status"))?;
                if let Some(pid) = value["pid"]
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .filter(|pid| *pid != 0)
                {
                    owned.browser_pid = pid;
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
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(failure("supervisor_status", error)),
        }
        if owned
            .try_wait()
            .map_err(|e| failure("supervisor_wait", e))?
            .is_some()
        {
            // Recheck the atomic status before treating a fast failed worker as
            // missing: it may have published the error between open and wait.
            if status.exists() {
                continue;
            }
            return Err(failed("Browser supervisor exited before startup"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn serve_worker() -> Result<bool> {
    let mut args = std::env::args_os().skip(2);
    let name = args
        .next()
        .and_then(|name| name.into_string().ok())
        .filter(|name| {
            name.strip_prefix(PREFIX)
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        })
        .ok_or_else(|| failed("Invalid browser job name"))?;
    let status = args
        .next()
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| failed("Invalid browser status path"))?;
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--")) {
        return Err(failed("Invalid browser worker arguments"));
    }
    let command = args
        .next()
        .ok_or_else(|| failed("Missing browser executable"))?;
    let result = (|| {
        let wide = wide(&name);
        // SAFETY: Only self-assignment rights are requested. This temporary
        // non-inheritable handle must close before Chromium can execute.
        let raw = unsafe { OpenJobObjectW(JOB_OBJECT_ASSIGN_PROCESS, 0, wide.as_ptr()) };
        if raw.is_null() {
            return Err(failure("job_join", io::Error::last_os_error()));
        }
        // SAFETY: OpenJobObjectW returned a new owned kernel handle.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: Both handles are live; the process pseudo handle is borrowed.
        if unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) } == 0 {
            return Err(failure("job_join", io::Error::last_os_error()));
        }
        // If the host died during startup, closing the last owner terminates
        // this worker here, before it can start a browser outside supervision.
        drop(job);
        let mut browser = Command::new(command)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| failure("browser_spawn", e))?;
        publish(&status, &json!({"pid":browser.id()}))?;
        let exit = browser.wait().map_err(|e| failure("browser_wait", e))?;
        Ok(exit.success())
    })();
    if let Err(error) = &result {
        let details = error.details.as_ref();
        let _ = publish(
            &status,
            &json!({"phase":details.and_then(|d| d["supervisor_phase"].as_str()).unwrap_or("supervisor_status"),"os_error":details.and_then(|d| d["os_error"].as_i64()).unwrap_or(0)}),
        );
    }
    result
}

fn publish(path: &Path, value: &Value) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| failed("Missing browser status directory"))?,
    )
    .map_err(|e| failure("supervisor_status", e))?;
    file.write_all(value.to_string().as_bytes())
        .map_err(|e| failure("supervisor_status", e))?;
    file.persist_noclobber(path)
        .map_err(|e| failure("supervisor_status", e.error))?;
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn failure(phase: &str, error: io::Error) -> Error {
    failed(format!("Cannot supervise Chromium ({phase}): {error}"))
        .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error()}))
}
