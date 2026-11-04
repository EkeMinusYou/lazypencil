//! Prepare bounded exact transfer files away from interactive rendering.
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

const CAPACITY_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct LocalTransfer {
    sequence: AtomicU64,
    files: Arc<Mutex<Files>>,
    sender: Option<mpsc::SyncSender<Command>>,
    thread: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    pub(super) directory: Arc<tempfile::TempDir>,
}

#[derive(Default)]
struct Files {
    entries: VecDeque<Entry>,
    bytes: usize,
}

struct Entry {
    key: u64,
    file: tempfile::NamedTempFile,
    bytes: usize,
}

struct Job {
    key: u64,
    file: std::fs::File,
    length: usize,
    live: Weak<()>,
}

enum Command {
    Prepare(Job),
    #[cfg(test)]
    Barrier(mpsc::SyncSender<()>),
}

pub(super) struct LocalImage {
    key: u64,
    file: tempfile::NamedTempFile,
    transfer: Arc<LocalTransfer>,
    live: Option<Arc<()>>,
}

pub(super) struct Link {
    pub(super) path: PathBuf,
    pub(super) raw: bool,
}

impl LocalTransfer {
    pub(super) fn new() -> std::io::Result<Self> {
        Self::with_capacity(CAPACITY_BYTES)
    }

    fn with_capacity(capacity: usize) -> std::io::Result<Self> {
        let directory = Arc::new(
            tempfile::Builder::new()
                .prefix("tty-graphics-protocol-lazypencil-")
                .tempdir()?,
        );
        let files = Arc::new(Mutex::new(Files::default()));
        let stopping = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(8);
        let cache = files.clone();
        let storage = directory.clone();
        let stop = stopping.clone();
        let thread = std::thread::Builder::new()
            .name("lazypencil-transfer".into())
            .spawn(move || {
                while let Ok(command) = receiver.recv() {
                    match command {
                        Command::Prepare(job) => {
                            if !stop.load(Ordering::Acquire) && job.live.strong_count() != 0 {
                                let _ = prepare(job, &storage, &cache, capacity);
                            }
                        }
                        #[cfg(test)]
                        Command::Barrier(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })?;
        Ok(Self {
            sequence: AtomicU64::new(0),
            files,
            sender: Some(sender),
            thread: Some(thread),
            stopping,
            directory,
        })
    }

    pub(super) fn store(
        self: &Arc<Self>,
        payload: &[u8],
        raw_length: Option<usize>,
    ) -> Option<Arc<LocalImage>> {
        // The ready compressed file keeps the original new-view latency.
        let mut file = tempfile::NamedTempFile::new_in(self.directory.path()).ok()?;
        file.write_all(payload).ok()?;
        let key = self.sequence.fetch_add(1, Ordering::Relaxed);
        let live = Arc::new(());
        if let Some(length) = raw_length
            && let Ok(source) = file.reopen()
        {
            // A full queue never holds up the preview worker or input thread.
            let _ = self.sender.as_ref()?.try_send(Command::Prepare(Job {
                key,
                file: source,
                length,
                live: Arc::downgrade(&live),
            }));
        }
        Some(Arc::new(LocalImage {
            key,
            file,
            transfer: self.clone(),
            live: Some(live),
        }))
    }

    #[cfg(test)]
    pub(super) fn drain(&self) {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .unwrap()
            .send(Command::Barrier(sender))
            .unwrap();
        receiver.recv().unwrap();
    }
}

fn prepare(
    job: Job,
    directory: &tempfile::TempDir,
    files: &Mutex<Files>,
    capacity: usize,
) -> std::io::Result<()> {
    if job.length == 0 || job.length > capacity {
        return Ok(());
    }
    let mut pixels = vec![0; job.length];
    let mut decoder = flate2::read::ZlibDecoder::new(job.file);
    decoder.read_exact(&mut pixels)?;
    if decoder.read(&mut [0])? != 0 || job.live.strong_count() == 0 {
        return Ok(());
    }
    let mut file = tempfile::NamedTempFile::new_in(directory.path())?;
    file.write_all(&pixels)?;
    drop(pixels);
    let mut files = files.lock().unwrap();
    if job.live.strong_count() == 0 {
        return Ok(());
    }
    let mut evicted = Vec::new();
    while files.bytes + job.length > capacity {
        let entry = files.entries.pop_front().expect("Accounted transfer file");
        files.bytes -= entry.bytes;
        evicted.push(entry);
    }
    files.bytes += job.length;
    files.entries.push_back(Entry {
        key: job.key,
        file,
        bytes: job.length,
    });
    drop(files);
    drop(evicted);
    Ok(())
}

impl LocalImage {
    pub(super) fn link(&self) -> Option<Link> {
        let number = self.transfer.sequence.fetch_add(1, Ordering::Relaxed);
        let path = self
            .transfer
            .directory
            .path()
            .join(format!("upload-{number}"));
        let mut files = self.transfer.files.lock().unwrap();
        if let Some(index) = files.entries.iter().position(|entry| entry.key == self.key) {
            let entry = files.entries.remove(index)?;
            let result = std::fs::hard_link(entry.file.path(), &path);
            files.entries.push_back(entry);
            if result.is_ok() {
                return Some(Link { path, raw: true });
            }
        }
        drop(files);
        std::fs::hard_link(self.file.path(), &path).ok()?;
        Some(Link { path, raw: false })
    }
}

impl Drop for LocalImage {
    fn drop(&mut self) {
        self.live.take();
        let mut files = self.transfer.files.lock().unwrap();
        if let Some(index) = files.entries.iter().position(|entry| entry.key == self.key) {
            let entry = files.entries.remove(index).unwrap();
            files.bytes -= entry.bytes;
            drop(files);
            drop(entry);
        }
    }
}

impl Drop for LocalTransfer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(transfer: &Arc<LocalTransfer>, value: u8, length: usize) -> Arc<LocalImage> {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&vec![value; length]).unwrap();
        transfer
            .store(&encoder.finish().unwrap(), Some(length))
            .unwrap()
    }

    #[test]
    fn eviction_keeps_recent_files_and_outstanding_terminal_links_intact() {
        let transfer = Arc::new(LocalTransfer::with_capacity(8).unwrap());
        let first = store(&transfer, 1, 4);
        let second = store(&transfer, 2, 4);
        transfer.drain();
        let ticket = first.link().unwrap();
        assert!(ticket.raw);
        let third = store(&transfer, 3, 4);
        transfer.drain();
        let fallback = second.link().unwrap();
        assert!(!fallback.raw);
        std::fs::remove_file(fallback.path).unwrap();
        assert_eq!(transfer.files.lock().unwrap().bytes, 8);
        drop(first);
        assert_eq!(std::fs::read(&ticket.path).unwrap(), [1; 4]);
        std::fs::remove_file(ticket.path).unwrap();
        drop((second, third));
        assert_eq!(transfer.files.lock().unwrap().bytes, 0);
        assert_eq!(
            std::fs::read_dir(transfer.directory.path())
                .unwrap()
                .count(),
            0
        );
        let oversized = store(&transfer, 0, 9);
        transfer.drain();
        assert!(!oversized.link().unwrap().raw);
    }
}
