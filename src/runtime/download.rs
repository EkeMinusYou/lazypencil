//! Keep blocking DNS/TLS/HTTP reads inside a cancellable owned process.
use super::{Artifact, copy_checked, integrity, unavailable};
use crate::{Error, Result};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const DOWNLOAD_SECONDS: u64 = 180;
const MAGIC: &[u8; 8] = b"LPDL\x01\0\r\n";

pub(super) fn watch_owner() -> Result<()> {
    let mut input = std::io::stdin();
    let mut magic = [0u8; 8];
    input
        .read_exact(&mut magic)
        .map_err(|_| unavailable("Runtime download owner is unavailable"))?;
    if &magic != MAGIC {
        return Err(unavailable("Invalid runtime download owner"));
    }
    // This private worker owns no installed files or descendant processes.
    // EOF independently terminates blocked DNS/TLS/HTTP even if its host died.
    std::thread::Builder::new()
        .name("runtime-download-owner".into())
        .spawn(move || {
            let mut byte = [0u8; 1];
            loop {
                match input.read(&mut byte) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    _ => std::process::exit(1),
                }
            }
        })
        .map_err(|error| unavailable(format!("Cannot observe runtime download owner: {error}")))?;
    Ok(())
}

pub(super) fn direct(artifact: &Artifact, writer: &mut impl Write) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(DOWNLOAD_SECONDS)))
        .build()
        .into();
    let mut response = agent
        .get(&artifact.url)
        .call()
        .map_err(|error| unavailable(format!("Chromium download failed: {error}")))?;
    let copied = copy_checked(
        response.body_mut().as_reader(),
        writer,
        Path::new("Chromium download"),
        artifact.bytes,
        None,
    )
    .map_err(|error| {
        if error.code == "IO_ERROR" {
            unavailable(format!("Chromium download failed: {}", error.message))
        } else {
            error
        }
    })?;
    if copied != artifact.bytes {
        return Err(integrity("Incomplete Chromium download"));
    }
    writer
        .flush()
        .map_err(|error| unavailable(format!("Cannot write Chromium download: {error}")))
}

struct Transfer {
    child: Child,
    reader: Option<JoinHandle<()>>,
    completed: bool,
    owner: Option<ChildStdin>,
}
impl Drop for Transfer {
    fn drop(&mut self) {
        self.owner.take();
        if !self.completed {
            // The downloader creates threads, never child processes. Child's
            // owned wait state avoids signalling a recycled process-group ID
            // after try_wait has already consumed an early worker exit.
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn run(
    executable: &Path,
    archive: &tempfile::NamedTempFile,
    expected_bytes: u64,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    crate::cancel::check(cancel)?;
    if !executable.is_absolute() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Runtime download workers require an absolute executable path",
        ));
    }
    let mut output = archive
        .reopen()
        .map_err(|error| Error::io(archive.path(), error))?;
    let path = archive.path().to_owned();
    let mut command = Command::new(executable);
    command
        .arg("__runtime_download_worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| unavailable(format!("Cannot start runtime download worker: {error}")))?;
    let stdout = child.stdout.take().expect("piped worker output");
    let owner = child.stdin.take().expect("owner pipe");
    let (sender, receiver) = mpsc::sync_channel(1);
    let mut transfer = Transfer {
        child,
        reader: None,
        completed: false,
        owner: Some(owner),
    };
    transfer
        .owner
        .as_mut()
        .expect("live owner pipe")
        .write_all(MAGIC)
        .map_err(|error| unavailable(format!("Cannot start runtime download owner: {error}")))?;
    transfer.reader = Some(std::thread::spawn(move || {
        let result = copy_checked(stdout, &mut output, &path, expected_bytes, None);
        let _ = sender.send(result);
    }));
    // Keep the HTTP budget unchanged and allow one second to drain/reap the
    // worker. Cancellation does not wait for an individual blocking HTTP call.
    let deadline = Instant::now() + Duration::from_secs(DOWNLOAD_SECONDS + 1);
    let mut copied = None;
    loop {
        crate::cancel::check(cancel)?;
        if Instant::now() >= deadline {
            return Err(unavailable(
                "Chromium download worker exceeded its transfer deadline",
            ));
        }
        if copied.is_none() {
            match receiver.try_recv() {
                Ok(result) => copied = Some(result?),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(unavailable(
                        "Runtime download output reader stopped unexpectedly",
                    ));
                }
            }
        }
        if let Some(status) = transfer.child.try_wait().map_err(|error| {
            unavailable(format!("Cannot wait for runtime download worker: {error}"))
        })? {
            if !status.success() {
                return Err(unavailable("Chromium download worker failed"));
            }
            if let Some(copied) = copied {
                crate::cancel::check(cancel)?;
                if copied != expected_bytes {
                    return Err(integrity("Incomplete Chromium download"));
                }
                transfer.completed = true;
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}
