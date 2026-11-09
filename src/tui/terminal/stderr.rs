//! Keep process-level diagnostics off the alternate screen, including native writes.
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, IsTerminal, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::net::UnixStream,
    },
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const RETAINED_BYTES: usize = 64 * 1024;

pub(super) struct Capture {
    original: File,
    target: RawFd,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<VecDeque<u8>>>,
}

impl Capture {
    pub(super) fn enter() -> io::Result<Option<Self>> {
        // Preserve explicit redirection, including live profiling to a file.
        if !io::stderr().is_terminal() {
            return Ok(None);
        }
        Self::redirect(libc::STDERR_FILENO).map(Some)
    }

    fn redirect(target: RawFd) -> io::Result<Self> {
        let original = unsafe { libc::fcntl(target, libc::F_DUPFD_CLOEXEC, 3) };
        if original < 0 {
            return Err(io::Error::last_os_error());
        }
        // Own only the duplicate; the standard descriptor is restored on drop.
        let original = unsafe { File::from_raw_fd(original) };
        let (reader, writer) = UnixStream::pair()?;
        reader.set_read_timeout(Some(Duration::from_millis(50)))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let reader = thread::Builder::new()
            .name("lazypencil-stderr".into())
            .spawn(move || drain(reader, stopping))?;
        let capture = Self {
            original,
            target,
            stop,
            reader: Some(reader),
        };
        flush(target);
        duplicate(writer.as_raw_fd(), target)?;
        Ok(capture)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        flush(self.target);
        let _ = duplicate(self.original.as_raw_fd(), self.target);
        self.stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take()
            && let Ok(mut bytes) = reader.join()
            && !bytes.is_empty()
        {
            let _ = self
                .original
                .write_all(b"\nDiagnostics captured while the TUI was active (up to 64 KiB):\n");
            let _ = self.original.write_all(bytes.make_contiguous());
            if bytes.back() != Some(&b'\n') {
                let _ = self.original.write_all(b"\n");
            }
        }
    }
}

fn duplicate(source: RawFd, target: RawFd) -> io::Result<()> {
    loop {
        if unsafe { libc::dup2(source, target) } >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn flush(target: RawFd) {
    if target == libc::STDERR_FILENO {
        let _ = io::stderr().flush();
        // AppKit bypasses Rust's writer; flush C stdio before switching descriptors.
        unsafe { libc::fflush(std::ptr::null_mut()) };
    }
}

fn drain(mut reader: UnixStream, stop: Arc<AtomicBool>) -> VecDeque<u8> {
    let mut tail = VecDeque::with_capacity(RETAINED_BYTES);
    let mut buffer = [0; 8192];
    // A descendant may retain the descriptor. Never wait for its EOF or let
    // continued writes hold up shutdown; drain at most 1 MiB already queued.
    let mut remaining: usize = 1024 * 1024;
    loop {
        let stopping = stop.load(Ordering::Acquire);
        if stopping && reader.set_nonblocking(true).is_err() {
            break;
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let excess = (tail.len() + count).saturating_sub(RETAINED_BYTES);
                tail.drain(..excess);
                tail.extend(&buffer[..count]);
                if stopping {
                    remaining = remaining.saturating_sub(count);
                    if remaining == 0 {
                        break;
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if stopping {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    tail
}

#[cfg(test)]
mod tests;
