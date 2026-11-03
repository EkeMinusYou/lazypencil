//! The host alone owns the kill-on-close job. A worker joins it before spawning Chromium.

use super::{Budget, Owned};
use crate::cdp::failed;
pub(super) use crate::process_job::Job;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP;

const PREFIX: &str = "Local\\lazypencil-browser-";
const MAX_STATUS: u64 = 4096;

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
    let job = Job::new(PREFIX).map_err(|error| failure(error.phase, error.error))?;
    let status = profile.join(format!("lazypencil-process-{}.json", uuid::Uuid::new_v4()));
    let child = Command::new(helper)
        .arg("__browser_process_worker")
        .arg(job.name())
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
        crate::process_job::join(&name).map_err(|error| failure("job_join", error))?;
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

fn failure(phase: &str, error: io::Error) -> Error {
    failed(format!("Cannot supervise Chromium ({phase}): {error}"))
        .details(json!({"supervisor_phase":phase,"os_error":error.raw_os_error()}))
}
