//! Timestamp PTY reads independently of image decoding, hashing and capture I/O.
use std::{
    fs::File,
    io::{self, Read},
    os::fd::AsRawFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct Chunk {
    pub bytes: Vec<u8>,
    pub received: Instant,
}

pub(super) struct Reader {
    receiver: mpsc::Receiver<io::Result<Chunk>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    blocked: Arc<AtomicBool>,
}

impl Reader {
    pub(super) fn start(mut source: File) -> io::Result<Self> {
        // Bound queued wire data to 2 MiB. Validation still enforces decoded
        // geometry, checksums, image counts and the total observation limit.
        let (sender, receiver) = mpsc::sync_channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        #[cfg(test)]
        let blocked = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let thread_blocked = blocked.clone();
        let thread = thread::Builder::new()
            .name("preview-pty-observer".into())
            .spawn(move || {
                let mut bytes = [0; 65536];
                while !stopping.load(Ordering::Acquire) {
                    let mut poll = libc::pollfd {
                        fd: source.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let ready = unsafe { libc::poll(&mut poll, 1, 20) };
                    if ready == 0 {
                        continue;
                    }
                    let mut result = if ready < 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() == io::ErrorKind::Interrupted {
                            continue;
                        }
                        Err(error)
                    } else if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                        Err(io::Error::other("Measurement PTY failed"))
                    } else {
                        match source.read(&mut bytes) {
                            Ok(0) => Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "Measurement PTY closed",
                            )),
                            Ok(size) => {
                                let received = Instant::now();
                                Ok(Chunk {
                                    bytes: bytes[..size].to_vec(),
                                    received,
                                })
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            Err(error) => Err(error),
                        }
                    };
                    let finished = result.is_err();
                    loop {
                        if stopping.load(Ordering::Acquire) {
                            return;
                        }
                        match sender.try_send(result) {
                            Ok(()) => break,
                            Err(mpsc::TrySendError::Disconnected(_)) => return,
                            Err(mpsc::TrySendError::Full(pending)) => {
                                #[cfg(test)]
                                thread_blocked.store(true, Ordering::Release);
                                result = pending;
                                thread::park_timeout(Duration::from_millis(1));
                            }
                        }
                    }
                    if finished {
                        return;
                    }
                }
            })?;
        Ok(Self {
            receiver,
            stop,
            thread: Some(thread),
            #[cfg(test)]
            blocked,
        })
    }

    pub(super) fn receive(&self, wait: Duration) -> io::Result<Option<Chunk>> {
        match self.receiver.recv_timeout(wait) {
            Ok(result) => result.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "PTY reader stopped",
            )),
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::{fd::OwnedFd, unix::net::UnixStream},
    };

    #[test]
    fn drains_before_validation_and_stops_even_with_a_full_queue() {
        let (source, mut writer) = UnixStream::pair().unwrap();
        let buffer: libc::c_int = 4096;
        for (fd, option) in [
            (source.as_raw_fd(), libc::SO_RCVBUF),
            (writer.as_raw_fd(), libc::SO_SNDBUF),
        ] {
            assert_eq!(
                unsafe {
                    libc::setsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        option,
                        (&buffer as *const libc::c_int).cast(),
                        std::mem::size_of_val(&buffer) as libc::socklen_t,
                    )
                },
                0
            );
        }
        let reader = Reader::start(File::from(OwnedFd::from(source))).unwrap();
        let (done, completed) = mpsc::channel();
        let sent = Instant::now();
        let writer = thread::spawn(move || {
            writer.write_all(&vec![91; 64 * 1024]).unwrap();
            done.send(()).unwrap();
            writer
        });
        // No validation/drain on this thread until the producer has completed.
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut count = 0;
        while count < 64 * 1024 {
            let chunk = reader.receive(Duration::from_secs(5)).unwrap().unwrap();
            assert!(chunk.received >= sent && chunk.received <= Instant::now());
            assert!(chunk.bytes.iter().all(|byte| *byte == 91));
            count += chunk.bytes.len();
        }
        let mut writer = writer.join().unwrap();
        let writing = thread::spawn(move || {
            let _ = writer.write_all(&vec![93; 8 * 1024 * 1024]);
        });
        let until = Instant::now() + Duration::from_secs(5);
        while !reader.blocked.load(Ordering::Acquire) {
            assert!(
                Instant::now() < until,
                "Reader must reach its bounded queue limit"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let (done, completed) = mpsc::channel();
        thread::spawn(move || {
            drop(reader);
            done.send(()).unwrap();
        });
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        writing.join().unwrap();
    }
}
