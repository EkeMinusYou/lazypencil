use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Command, ExitStatus, Stdio},
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

pub fn output(command: &mut Command, timeout: Duration) -> Result<Output> {
    output_limited(command, timeout, 64 * 1024 * 1024)
}

pub fn output_limited(command: &mut Command, timeout: Duration, limit: u64) -> Result<Output> {
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
    let mut child = command
        .spawn()
        .with_context(|| format!("Starting {command:?}"))?;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            #[cfg(not(unix))]
            {
                let _ = child.kill();
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline && child.try_wait()?.is_none() {
                thread::sleep(Duration::from_millis(10));
            }
            // Kill the group even when the direct child has already exited.
            // Descendants can otherwise retain pipes or continue native builds.
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            timed_out = true;
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.seek(SeekFrom::Start(0))?;
    stderr.seek(SeekFrom::Start(0))?;
    stdout.take(limit + 1).read_to_end(&mut out)?;
    stderr.take(limit + 1).read_to_end(&mut err)?;
    anyhow::ensure!(
        out.len() as u64 <= limit && err.len() as u64 <= limit,
        "Command output exceeded its byte limit"
    );
    Ok(Output {
        status,
        stdout: out,
        stderr: err,
        elapsed: started.elapsed(),
        timed_out,
    })
}

pub fn checked(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    let result = output(command, timeout)?;
    anyhow::ensure!(
        !result.timed_out,
        "Command exceeded {timeout:?}: {command:?}"
    );
    if !result.status.success() {
        bail!(
            "Command failed ({}) {command:?}\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(result.stdout)
}

pub fn text(command: &mut Command) -> Result<String> {
    Ok(String::from_utf8(checked(
        command,
        Duration::from_secs(45),
    )?)?)
}

pub fn logged(command: &mut Command, timeout: Duration, log: &Path, append: bool) -> Result<()> {
    eprintln!("Running {command:?}");
    let result = output(command, timeout)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(log)?;
    for bytes in [&result.stdout, &result.stderr] {
        file.write_all(bytes)?;
        std::io::stdout().write_all(bytes)?;
    }
    eprintln!(
        "Command finished in {:.3}s: {}",
        result.elapsed.as_secs_f64(),
        result.status
    );
    anyhow::ensure!(
        !result.timed_out,
        "Command exceeded {timeout:?}: {command:?}; see {}",
        log.display()
    );
    anyhow::ensure!(
        result.status.success(),
        "Command failed ({}) {command:?}; see {}",
        result.status,
        log.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_exit_code_and_separate_output() {
        let result = output(Command::new("git").arg("--version"), Duration::from_secs(5)).unwrap();
        assert!(result.status.success());
        assert!(result.stdout.starts_with(b"git version "));
        let result = output(
            Command::new("git").arg("definitely-not-a-git-command"),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
    }
}
