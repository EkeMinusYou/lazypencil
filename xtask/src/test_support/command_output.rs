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
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        oversized = stdout.metadata()?.len() > limit || stderr.metadata()?.len() > limit;
        if started.elapsed() >= timeout || oversized {
            timed_out = !oversized;
            break terminate(&mut child)?;
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
