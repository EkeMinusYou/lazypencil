//! Windows jobs whose sole long-lived handle belongs to the application host.
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError, SetLastError},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, OpenJobObjectW, SetInformationJobObject,
        },
        Threading::GetCurrentProcess,
    },
};

pub(crate) struct JobError {
    pub phase: &'static str,
    pub error: io::Error,
}

pub(crate) struct Job {
    _handle: OwnedHandle,
    name: String,
}

impl Job {
    pub(crate) fn new(prefix: &str) -> Result<Self, JobError> {
        let name = format!("{prefix}{}", uuid::Uuid::new_v4());
        let wide = wide(&name);
        // SAFETY: The NUL-terminated name is valid. Null security attributes
        // create a non-inheritable handle owned only by this host.
        let raw = unsafe {
            SetLastError(0);
            CreateJobObjectW(std::ptr::null(), wide.as_ptr())
        };
        let error = unsafe { GetLastError() };
        if raw.is_null() {
            return Err(JobError {
                phase: "job_create",
                error: io::Error::from_raw_os_error(error as i32),
            });
        }
        // SAFETY: CreateJobObjectW returned one owned kernel handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if error == ERROR_ALREADY_EXISTS {
            return Err(JobError {
                phase: "job_create",
                error: io::ErrorKind::AlreadyExists.into(),
            });
        }
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: The pointer and size match the declared information class.
        // No breakaway rights or handle inheritance are enabled.
        let success = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if success == 0 {
            return Err(JobError {
                phase: "job_limits",
                error: io::Error::last_os_error(),
            });
        }
        Ok(Self {
            _handle: handle,
            name,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

/// Join before spawning any child, then relinquish the worker's last handle.
/// If the host already died, closing that last handle terminates this worker.
pub(crate) fn join(name: &str) -> io::Result<()> {
    // The worker can assign itself, but cannot change limits or terminate the job.
    const JOB_OBJECT_ASSIGN_PROCESS: u32 = 0x0001;
    let wide = wide(name);
    // SAFETY: The caller validated its private bounded name. The temporary
    // handle is non-inheritable and requests only self-assignment rights.
    let raw = unsafe { OpenJobObjectW(JOB_OBJECT_ASSIGN_PROCESS, 0, wide.as_ptr()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenJobObjectW returned a new owned handle.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: Both handles are live; the process pseudo handle is borrowed.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), GetCurrentProcess()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    drop(job);
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
