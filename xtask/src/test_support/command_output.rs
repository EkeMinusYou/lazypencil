//! Bounded command observation shared by CI tools and application integration tests.
use std::{
    io::{self, Read, Seek, SeekFrom},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
    pub timed_out: bool,
}
fn terminate(child: &mut Child) -> io::Result<ExitStatus> {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end && child.try_wait()?.is_none() {
        thread::sleep(Duration::from_millis(10));
    }
    // The direct child may exit before a descendant closes an inherited stream.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    child.wait()
}
pub fn output(command: &mut Command, timeout: Duration, limit: u64) -> io::Result<Output> {
    output_observed(command, timeout, limit, None::<fn(&[u8]) -> io::Result<()>>)
}

/// Observe newly written stdout without changing the child's shared file offset.
/// Observers must not block: command ownership and deadlines stay in this loop.
pub fn output_observed(
    command: &mut Command,
    timeout: Duration,
    limit: u64,
    mut observer: Option<impl FnMut(&[u8]) -> io::Result<()>>,
) -> io::Result<Output> {
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = command.spawn()?;
    let mut timed_out = false;
    let mut oversized = false;
    let mut observed = 0;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            observe(&stdout, &mut observed, limit, &mut observer)?;
            break status;
        }
        oversized = stdout.metadata()?.len() > limit || stderr.metadata()?.len() > limit;
        if started.elapsed() >= timeout || oversized {
            timed_out = !oversized;
            break terminate(&mut child)?;
        }
        if let Err(error) = observe(&stdout, &mut observed, limit, &mut observer) {
            let _ = terminate(&mut child);
            return Err(error);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.seek(SeekFrom::Start(0))?;
    stderr.seek(SeekFrom::Start(0))?;
    stdout.take(limit.saturating_add(1)).read_to_end(&mut out)?;
    stderr.take(limit.saturating_add(1)).read_to_end(&mut err)?;
    if oversized || out.len() as u64 > limit || err.len() as u64 > limit {
        return Err(io::Error::other("Command output exceeded its byte limit"));
    }
    Ok(Output {
        status,
        stdout: out,
        stderr: err,
        elapsed: started.elapsed(),
        timed_out,
    })
}

fn observe(
    file: &std::fs::File,
    position: &mut u64,
    limit: u64,
    observer: &mut Option<impl FnMut(&[u8]) -> io::Result<()>>,
) -> io::Result<()> {
    let Some(observer) = observer else {
        return Ok(());
    };
    let end = file.metadata()?.len();
    if end > limit {
        return Err(io::Error::other("Command output exceeded its byte limit"));
    }
    let mut buffer = [0; 65536];
    while *position < end {
        let length = usize::try_from((end - *position).min(buffer.len() as u64)).unwrap();
        #[cfg(unix)]
        let read = {
            use std::os::unix::fs::FileExt;
            file.read_at(&mut buffer[..length], *position)?
        };
        #[cfg(windows)]
        let read = {
            use std::os::windows::fs::FileExt;
            file.seek_read(&mut buffer[..length], *position)?
        };
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Command output was truncated",
            ));
        }
        observer(&buffer[..read])?;
        *position += read as u64;
    }
    Ok(())
}
