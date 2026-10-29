//! One profile browser, with bounded, independently owned frontend requests.
use super::{Profile, failure};
use crate::{
    Error, Result,
    cdp::{Browser, Budget, BudgetStream, Purpose},
    dynamic::{Raster, acquisition::PageAcquisition},
};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

mod channel;
mod wire;
mod worker;
use wire::{FileHeader, Hello, Reply, Request};
type LocalStream = BudgetStream<UnixStream>;
const LOCK: &str = "lazypencil-page-broker.lock";
const MAX_CLIENTS: usize = 16;

pub(crate) struct Client {
    owner: UnixStream,
    stream: LocalStream,
    pid: u32,
    retained: usize,
    keys: VecDeque<String>,
}

struct Starting(Option<Child>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            child.stdin.take();
            // The worker's lifetime belongs to all connected owners. Reaping
            // the starter must never terminate another frontend's browser.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

impl Client {
    pub(crate) fn connect(
        profile: &Profile,
        browser: &crate::runtime::InstalledBrowser,
        supervisor: Option<&Path>,
        budget: &Budget<'_>,
    ) -> Result<Self> {
        let executable = match supervisor {
            Some(path) => crate::paths::canonical(path)?,
            None => std::env::current_exe().map_err(|_| failure("profile_engine"))?,
        };
        let browser = crate::paths::canonical(&browser.executable)?;
        let context = channel::context(profile, &browser, &executable, budget)?;
        let endpoint = channel::Endpoint::new(profile)?;
        let mut starting = Starting(None);
        loop {
            budget.check()?;
            match endpoint.connect(budget) {
                Ok(owner) => {
                    let mut stream = LocalStream::new(
                        owner.try_clone().map_err(|_| failure("profile_channel"))?,
                        budget,
                    );
                    let handshake = (|| {
                        wire::write_json(
                            &mut stream,
                            &Hello {
                                context: context.clone(),
                            },
                        )?;
                        match wire::read_json::<Reply>(&mut stream, None)?.checked()? {
                            Reply::Hello {
                                context: actual,
                                owner,
                            } if actual == context && uuid::Uuid::parse_str(&owner).is_ok() => {
                                Ok(())
                            }
                            _ => Err(failure("profile_protocol")),
                        }
                    })();
                    match handshake {
                        Ok(()) => {
                            return Ok(Self {
                                owner,
                                stream,
                                pid: 0,
                                retained: 0,
                                keys: VecDeque::new(),
                            });
                        }
                        Err(error) => {
                            let _ = owner.shutdown(Shutdown::Both);
                            budget.check()?;
                            if !matches!(
                                error.details.as_ref().and_then(|d| d["stage"].as_str()),
                                Some("profile_channel_read" | "profile_channel_write")
                            ) {
                                return Err(error);
                            }
                        }
                    }
                }
                Err(error) => {
                    budget.check()?;
                    if !matches!(
                        error.kind(),
                        io::ErrorKind::NotFound
                            | io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) {
                        return Err(failure("profile_channel_identity"));
                    }
                }
            }
            if starting
                .0
                .as_mut()
                .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some()))
            {
                starting = Starting(None);
            }
            if starting.0.is_none() {
                let lock = super::private_file(&profile.directory().join(LOCK), true)?;
                match fs2::FileExt::try_lock_exclusive(&lock) {
                    Ok(()) => {
                        drop(lock);
                        let child = Command::new(&executable)
                            .arg("__profile_page_worker")
                            .arg(profile.directory())
                            .arg(&browser)
                            .stdin(Stdio::piped())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .process_group(0)
                            .spawn()
                            .map_err(|_| failure("profile_worker_start"))?;
                        starting = Starting(Some(child));
                    }
                    Err(error) if crate::store::lock_contended(&error) => {}
                    Err(_) => return Err(failure("profile_channel_lock")),
                }
            }
            std::thread::sleep(Duration::from_millis(10).min(budget.remaining()?));
        }
    }

    pub(crate) fn page(&mut self, page: PageAcquisition, budget: &Budget<'_>) -> Result<Raster> {
        page.validate()?;
        let retained_key = page.retained.as_ref().map(|(key, _)| key.clone());
        let files = page
            .resources
            .as_ref()
            .map(|resources| {
                resources
                    .entries()
                    .map(|(url, mime, bytes)| FileHeader {
                        url: url.into(),
                        mime: mime.map(str::to_owned),
                        bytes: bytes.len(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let request = Request::Page {
            timeout_ms: milliseconds(budget)?,
            page: Box::new(page),
            files,
        };
        let result = (|| {
            self.stream.set_budget(budget);
            wire::write_json(&mut self.stream, &request)?;
            if !matches!(
                wire::read_json::<Reply>(&mut self.stream, None)?.checked()?,
                Reply::Ready
            ) {
                return Err(failure("profile_protocol"));
            }
            if let Request::Page { page, .. } = &request
                && let Some(resources) = &page.resources
            {
                for (_, _, bytes) in resources.entries() {
                    self.stream
                        .write_all(bytes)
                        .map_err(|_| failure("profile_channel_write"))?;
                }
            }
            match wire::read_json::<Reply>(&mut self.stream, None)?.checked()? {
                Reply::Raster {
                    metadata,
                    bytes,
                    pid,
                    retained,
                } if bytes <= wire::MAX_PNG
                    && pid != 0
                    && retained <= crate::cdp::MAX_RETAINED_PAGES =>
                {
                    let mut png = vec![0; bytes];
                    self.stream
                        .read_exact(&mut png)
                        .map_err(|_| failure("profile_channel_read"))?;
                    budget.check()?;
                    if metadata["png_hash"] != crate::document::hash(&png) {
                        return Err(failure("profile_image_integrity"));
                    }
                    let raster = crate::dynamic::checked_png(png, metadata)?;
                    self.pid = pid;
                    self.retained = retained;
                    Ok(raster)
                }
                _ => Err(failure("profile_protocol")),
            }
        })()
        .map_err(|error| budget.check().err().unwrap_or(error));
        if result.is_ok()
            && let Some(key) = retained_key
        {
            self.keys.retain(|previous| previous != &key);
            self.keys.push_back(key);
            while self.keys.len() > crate::cdp::MAX_RETAINED_PAGES {
                self.keys.pop_front();
            }
        }
        result
    }

    pub(crate) fn retain(
        &mut self,
        signatures: &BTreeMap<String, String>,
        budget: &Budget<'_>,
    ) -> Result<()> {
        let kept = self
            .keys
            .iter()
            .filter_map(|key| {
                signatures
                    .get(key)
                    .map(|signature| (key.clone(), signature.clone()))
            })
            .collect();
        let result = (|| {
            self.stream.set_budget(budget);
            wire::write_json(
                &mut self.stream,
                &Request::Retain {
                    timeout_ms: milliseconds(budget)?,
                    signatures: kept,
                },
            )?;
            if !matches!(
                wire::read_json::<Reply>(&mut self.stream, None)?.checked()?,
                Reply::Ready
            ) {
                return Err(failure("profile_protocol"));
            }
            match wire::read_json::<Reply>(&mut self.stream, None)?.checked()? {
                Reply::Done { pid, retained } if retained <= crate::cdp::MAX_RETAINED_PAGES => {
                    self.pid = pid;
                    self.retained = retained;
                    Ok(())
                }
                _ => Err(failure("profile_protocol")),
            }
        })()
        .map_err(|error| budget.check().err().unwrap_or(error));
        if result.is_ok() {
            self.keys.retain(|key| signatures.contains_key(key));
        }
        result
    }

    pub(crate) fn process_id(&self) -> Option<u32> {
        (self.pid != 0).then_some(self.pid)
    }
    pub(crate) fn retained_count(&self) -> usize {
        self.retained
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.owner.shutdown(Shutdown::Write);
        // Preserve the existing browser's graceful-close allowance. EOF is
        // acknowledged after this owner's targets, and the last browser, close.
        let end = Instant::now() + Duration::from_secs(6);
        let mut bytes = [0; 1024];
        while let Some(remaining) = end.checked_duration_since(Instant::now()) {
            if self
                .owner
                .set_read_timeout(Some(remaining.min(Duration::from_millis(50))))
                .is_err()
            {
                break;
            }
            match self.owner.read(&mut bytes) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => break,
            }
        }
        let _ = self.owner.shutdown(Shutdown::Both);
    }
}

fn milliseconds(budget: &Budget<'_>) -> Result<u64> {
    Ok(budget.remaining()?.as_millis().clamp(1, 300000) as u64)
}

pub(super) fn serve_worker() -> Result<()> {
    worker::serve()
}
