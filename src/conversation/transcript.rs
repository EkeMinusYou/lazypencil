//! Ordered transcript I/O with bounded buffering and explicit durability barriers.
use crate::{Error, Result};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const QUEUED_BYTES: usize = 8 * 1024 * 1024;
const SYNC_INTERVAL: Duration = Duration::from_millis(100);
const RECORD_OVERHEAD: usize = 128;

enum Command {
    Record(Vec<u8>, usize),
    Barrier(oneshot::Sender<Result<()>>),
}

pub(super) struct Writer {
    sender: mpsc::Sender<Command>,
    queued: Arc<AtomicUsize>,
    error: Arc<Mutex<Option<Error>>>,
    failed: Arc<tokio::sync::Notify>,
}

trait Sink: Write + Send {
    fn sync(&mut self) -> std::io::Result<()>;
}
impl Sink for File {
    fn sync(&mut self) -> std::io::Result<()> {
        self.sync_data()
    }
}

impl Writer {
    pub(super) fn create(path: &Path) -> Result<Self> {
        let path = path.to_owned();
        Self::start(path.clone(), move || {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(path)
        })
    }
    fn start<S: Sink + 'static>(
        path: PathBuf,
        open: impl FnOnce() -> std::io::Result<S> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let error = Arc::new(Mutex::new(None));
        let worker_queued = queued.clone();
        let worker_error = error.clone();
        let failed = Arc::new(tokio::sync::Notify::new());
        let worker_failed = failed.clone();
        std::thread::Builder::new()
            .name("lazypencil-transcript".into())
            .spawn(move || {
                let mut sink = open();
                let mut dirty = false;
                let mut deadline = Instant::now() + SYNC_INTERVAL;
                let status = |error: &std::io::Error| {
                    let mut failure = worker_error.lock().unwrap_or_else(|p| p.into_inner());
                    failure.get_or_insert_with(|| {
                        worker_failed.notify_one();
                        Error::io(&path, std::io::Error::new(error.kind(), error.to_string()))
                    });
                };
                if let Err(error) = &sink {
                    status(error);
                }
                loop {
                    let command = if dirty {
                        receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    } else {
                        receiver
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    };
                    let mut barrier = None;
                    match command {
                        Ok(Command::Record(bytes, reserved)) => {
                            let healthy = worker_error
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .is_none();
                            if healthy && let Ok(sink) = &mut sink {
                                match sink.write_all(&bytes) {
                                    Ok(()) => {
                                        if !dirty {
                                            deadline = Instant::now() + SYNC_INTERVAL;
                                        }
                                        dirty = true;
                                    }
                                    Err(error) => status(&error),
                                }
                            }
                            worker_queued.fetch_sub(reserved, Ordering::Relaxed);
                        }
                        Ok(Command::Barrier(reply)) => barrier = Some(reply),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            if dirty
                                && let Ok(sink) = &mut sink
                                && let Err(error) = sink.sync()
                            {
                                status(&error);
                            }
                            break;
                        }
                    }
                    if barrier.is_some() || Instant::now() >= deadline {
                        if dirty
                            && let Ok(sink) = &mut sink
                            && let Err(error) = sink.sync()
                        {
                            status(&error);
                        }
                        dirty = false;
                        deadline = Instant::now() + SYNC_INTERVAL;
                        if let Some(reply) = barrier {
                            let result = worker_error
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .clone()
                                .map_or(Ok(()), Err);
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .map_err(|error| {
                Error::new(
                    "IO_ERROR",
                    format!("Cannot start transcript writer: {error}"),
                )
            })?;
        Ok(Self {
            sender,
            queued,
            error,
            failed,
        })
    }
    #[cfg(test)]
    pub(super) fn from_file(path: PathBuf, file: File) -> Result<Self> {
        Self::start(path, move || Ok(file))
    }
    pub(super) async fn failure(&self) -> Error {
        loop {
            let notified = self.failed.notified();
            if let Err(error) = self.check() {
                return error;
            }
            notified.await;
        }
    }
    pub(super) fn check(&self) -> Result<()> {
        self.error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .map_or(Ok(()), Err)
    }
    pub(super) fn record(&self, bytes: Vec<u8>) -> Result<()> {
        self.check()?;
        let reserved = bytes.len().saturating_add(RECORD_OVERHEAD);
        let mut used = self.queued.load(Ordering::Relaxed);
        loop {
            let next = used
                .checked_add(reserved)
                .filter(|next| *next <= QUEUED_BYTES)
                .ok_or_else(|| {
                    Error::new(
                        "AGENT_ERROR",
                        "Transcript storage is too slow; its bounded write queue is full",
                    )
                })?;
            match self.queued.compare_exchange_weak(
                used,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => used = actual,
            }
        }
        if self.sender.send(Command::Record(bytes, reserved)).is_err() {
            self.queued.fetch_sub(reserved, Ordering::Relaxed);
            return Err(Error::new("IO_ERROR", "Transcript writer stopped"));
        }
        Ok(())
    }
    pub(super) fn barrier(&self) -> Result<oneshot::Receiver<Result<()>>> {
        self.check()?;
        let (reply, receiver) = oneshot::channel();
        self.sender
            .send(Command::Barrier(reply))
            .map_err(|_| Error::new("IO_ERROR", "Transcript writer stopped"))?;
        Ok(receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Condvar;

    #[derive(Default)]
    struct State {
        bytes: Vec<u8>,
        syncs: usize,
    }
    struct Capture(Arc<Mutex<State>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().bytes.extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Sink for Capture {
        fn sync(&mut self) -> std::io::Result<()> {
            self.0.lock().unwrap().syncs += 1;
            Ok(())
        }
    }
    struct Release(Arc<(Mutex<bool>, Condvar)>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }
    fn blocked_writer() -> (Writer, Arc<Mutex<State>>, Release) {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let release = Release(gate.clone());
        let state = Arc::new(Mutex::new(State::default()));
        let output = state.clone();
        let writer = Writer::start(PathBuf::from("test-transcript"), move || {
            let mut ready = gate.0.lock().unwrap();
            while !*ready {
                ready = gate.1.wait(ready).unwrap();
            }
            Ok(Capture(output))
        })
        .unwrap();
        (writer, state, release)
    }

    #[tokio::test]
    async fn blocked_storage_preserves_order_and_async_responsiveness_and_batches_syncs() {
        let (writer, state, release) = blocked_writer();
        let mut expected: Vec<u8> = Vec::new();
        for i in 0..32 {
            let bytes = format!("{i}\n").into_bytes();
            expected.extend(&bytes);
            writer.record(bytes).unwrap();
        }
        let mut barrier = writer.barrier().unwrap();
        // Repeatedly dropping the polling future must retain this same barrier.
        for _ in 0..3 {
            tokio::select! {
                result = &mut barrier => panic!("Blocked storage acknowledged durability: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(5)) => {},
            }
        }
        assert!(state.lock().unwrap().bytes.is_empty());
        drop(release);
        tokio::time::timeout(Duration::from_secs(2), barrier)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.bytes, expected);
        assert!(
            state.syncs < 32,
            "Chunk updates must share a durability sync"
        );
    }

    #[tokio::test]
    async fn blocked_storage_has_a_byte_bound_and_recovers_its_queue_reservations() {
        let (writer, state, release) = blocked_writer();
        let bytes = vec![1; QUEUED_BYTES - RECORD_OVERHEAD];
        writer.record(bytes).unwrap();
        assert_eq!(writer.queued.load(Ordering::Relaxed), QUEUED_BYTES);
        assert_eq!(writer.record(vec![2]).unwrap_err().code, "AGENT_ERROR");
        assert_eq!(writer.queued.load(Ordering::Relaxed), QUEUED_BYTES);
        let barrier = writer.barrier().unwrap();
        drop(release);
        tokio::time::timeout(Duration::from_secs(2), barrier)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(writer.queued.load(Ordering::Relaxed), 0);
        writer.record(vec![3]).unwrap();
        writer.barrier().unwrap().await.unwrap().unwrap();
        assert_eq!(state.lock().unwrap().bytes.last(), Some(&3));
    }

    #[tokio::test]
    async fn filesystem_failures_wake_the_host_and_fail_all_durability_barriers() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("transcript.jsonl");
        std::fs::write(&path, "").unwrap();
        let writer = Writer::from_file(path.clone(), File::open(&path).unwrap()).unwrap();
        let failure = writer.failure();
        writer.record(b"record\n".to_vec()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), failure)
                .await
                .unwrap()
                .code,
            "IO_ERROR"
        );
        assert!(writer.barrier().is_err());
        assert!(writer.record(b"later\n".to_vec()).is_err());
        assert!(std::fs::read(path).unwrap().is_empty());
    }
}
