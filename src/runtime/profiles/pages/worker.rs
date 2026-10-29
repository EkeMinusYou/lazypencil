//! The profile lease remains in Chromium's existing guardian. This worker
//! serializes acquisitions and closes only a disconnected frontend's targets.
use super::*;
use std::{
    collections::HashMap,
    os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, net::UnixListener},
    },
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
};
use tokio_util::sync::CancellationToken;

struct Shared {
    connections: AtomicUsize,
    owners: AtomicUsize,
    bootstrap: AtomicBool,
    bytes: AtomicUsize,
    stopped: AtomicBool,
    clients: Mutex<BTreeMap<String, UnixStream>>,
    pool: Mutex<ResourcePool>,
    wake: Mutex<UnixStream>,
    queue: SyncSender<Job>,
}
impl Shared {
    fn wake(&self) {
        let _ = self
            .wake
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write_all(&[1]);
    }
    fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Reservation> {
        self.bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes)
                    .filter(|next| *next <= wire::MAX_RESOURCES)
            })
            .ok()?;
        Some(Reservation {
            state: self.clone(),
            bytes,
        })
    }
    fn shutdown(&self) {
        for stream in self
            .clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
        self.wake();
    }
    fn intern(&self, bytes: Vec<u8>) -> Result<Arc<[u8]>> {
        let hash = crate::document::hash(&bytes);
        let mut pool = self.pool.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(bytes) = pool.entries.get(&hash).and_then(|(_, weak)| weak.upgrade()) {
            return Ok(bytes);
        }
        if pool.bytes + bytes.len() > wire::MAX_RESOURCES || pool.entries.len() >= 65536 {
            pool.entries.retain(|_, (_, weak)| weak.strong_count() != 0);
            pool.bytes = pool.entries.values().map(|(bytes, _)| bytes).sum();
        }
        if let Some((previous, _)) = pool.entries.remove(&hash) {
            pool.bytes -= previous;
        }
        if pool.bytes + bytes.len() > wire::MAX_RESOURCES || pool.entries.len() >= 65536 {
            return Err(failure("profile_resource_limit"));
        }
        let bytes: Arc<[u8]> = bytes.into();
        pool.bytes += bytes.len();
        pool.entries
            .insert(hash, (bytes.len(), Arc::downgrade(&bytes)));
        Ok(bytes)
    }
}
#[derive(Default)]
struct ResourcePool {
    entries: HashMap<String, (usize, std::sync::Weak<[u8]>)>,
    bytes: usize,
}
struct Reservation {
    state: Arc<Shared>,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.state.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct Connection {
    state: Arc<Shared>,
    id: String,
    owner: Option<String>,
    cancel: CancellationToken,
    stream: UnixStream,
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.stream.shutdown(Shutdown::Read);
        if let Some(owner) = self.owner.take() {
            self.state.owners.fetch_sub(1, Ordering::AcqRel);
            let writer = self.stream.try_clone().ok();
            let _ = self.state.queue.send(Job::Close {
                owner,
                state: self.state.clone(),
                _writer: writer,
            });
        }
        self.state
            .clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
        self.state.connections.fetch_sub(1, Ordering::AcqRel);
        self.state.wake();
    }
}

struct EngineExit(std::sync::Weak<Shared>);
impl Drop for EngineExit {
    fn drop(&mut self) {
        if let Some(state) = self.0.upgrade() {
            state.stopped.store(true, Ordering::Release);
            state.shutdown();
        }
    }
}

enum Job {
    Request {
        owner: String,
        request: Request,
        writer: UnixStream,
        end: Instant,
        cancel: CancellationToken,
        in_flight: Arc<AtomicBool>,
        _reservation: Reservation,
    },
    Close {
        owner: String,
        state: Arc<Shared>,
        _writer: Option<UnixStream>,
    },
}

struct SocketCleanup {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.dev() == self.device && m.ino() == self.inode)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn serve() -> Result<()> {
    let mut args = std::env::args_os().skip(2);
    let profile = Profile::from_directory(&PathBuf::from(
        args.next()
            .ok_or_else(|| failure("profile_worker_arguments"))?,
    ))?;
    let browser = crate::paths::canonical(Path::new(
        &args
            .next()
            .ok_or_else(|| failure("profile_worker_arguments"))?,
    ))?;
    if args.next().is_some() {
        return Err(failure("profile_worker_arguments"));
    }
    let lock = super::super::private_file(&profile.directory().join(LOCK), true)?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {}
        Err(error) if crate::store::lock_contended(&error) => return Ok(()),
        Err(_) => return Err(failure("profile_channel_lock")),
    }
    let executable = std::env::current_exe().map_err(|_| failure("profile_engine"))?;
    let context = channel::context(
        &profile,
        &browser,
        &executable,
        &Budget::new(Duration::from_secs(60), None),
    )?;
    let endpoint = channel::Endpoint::new(&profile)?;
    match endpoint.check() {
        Ok(_) => match endpoint.connect(&Budget::new(Duration::from_secs(1), None)) {
            Ok(_) => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                fs::remove_file(&endpoint.path).map_err(|_| failure("profile_channel_cleanup"))?;
            }
            Err(_) => return Ok(()),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(failure("profile_channel_identity")),
    }
    // This is a fresh private worker, before any threads exist. The listener
    // and every subsequently created worker file must be private at creation.
    unsafe {
        libc::umask(0o077);
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
    let listener =
        UnixListener::bind(&endpoint.path).map_err(|_| failure("profile_channel_bind"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| failure("profile_channel_bind"))?;
    let metadata = endpoint
        .check()
        .map_err(|_| failure("profile_channel_identity"))?;
    let _cleanup = SocketCleanup {
        path: endpoint.path.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let (wake, mut awakened) = UnixStream::pair().map_err(|_| failure("profile_channel_wake"))?;
    wake.set_nonblocking(true)
        .map_err(|_| failure("profile_channel_wake"))?;
    awakened
        .set_nonblocking(true)
        .map_err(|_| failure("profile_channel_wake"))?;
    let (queue, receiver) = mpsc::sync_channel(3 * MAX_CLIENTS + 1);
    let state = Arc::new(Shared {
        connections: AtomicUsize::new(0),
        owners: AtomicUsize::new(0),
        bootstrap: AtomicBool::new(true),
        bytes: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
        clients: Mutex::new(BTreeMap::new()),
        pool: Mutex::new(ResourcePool::default()),
        wake: Mutex::new(wake),
        queue,
    });
    let watch = state.clone();
    std::thread::Builder::new()
        .name("profile-owner".into())
        .spawn(move || {
            // Only the initiating host owns this pipe's write end. Once its first
            // authenticated connection exists the pipe can close without shutdown.
            // Keep fd 0 open after EOF. Closing it here lets a later macOS
            // posix_spawn reuse it for a CLOEXEC pipe, losing the nested
            // guardian's stdin handshake. StdinLock only borrows standard IO.
            let input = io::stdin();
            let mut owner = input.lock();
            let mut byte = [0; 1];
            loop {
                match owner.read(&mut byte) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => break,
                }
            }
            watch.bootstrap.store(false, Ordering::Release);
            watch.wake();
        })
        .map_err(|_| failure("profile_owner_watch"))?;
    let exit = EngineExit(Arc::downgrade(&state));
    let engine = std::thread::Builder::new()
        .name("profile-pages".into())
        .spawn(move || {
            let _exit = exit;
            let mut worker = Engine {
                profile,
                browser_path: browser,
                executable,
                browser: None,
                policy: None,
            };
            while let Ok(job) = receiver.recv() {
                worker.run(job);
            }
        })
        .map_err(|_| failure("profile_worker_start"))?;
    let mut readers = Vec::new();
    let result = (|| {
        loop {
            if state.stopped.load(Ordering::Acquire) {
                return Err(failure("profile_worker_stopped"));
            }
            loop {
                let (stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => return Err(failure("profile_channel_accept")),
                };
                stream
                    .set_nonblocking(false)
                    .map_err(|_| failure("profile_channel_accept"))?;
                if channel::check_peer(&stream).is_err() {
                    continue;
                }
                let registered = stream.try_clone().map_err(|_| failure("profile_channel"))?;
                if state.connections.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                    state.connections.fetch_sub(1, Ordering::AcqRel);
                    continue;
                }
                let id = uuid::Uuid::new_v4().to_string();
                state
                    .clients
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(id.clone(), registered);
                let connection = Connection {
                    state: state.clone(),
                    id,
                    owner: None,
                    cancel: CancellationToken::new(),
                    stream,
                };
                let expected = context.clone();
                readers.push(
                    std::thread::Builder::new()
                        .name("profile-client".into())
                        .spawn(move || {
                            let _ = read_client(connection, &expected);
                        })
                        .map_err(|_| failure("profile_client_start"))?,
                );
                let mut index = 0;
                while index < readers.len() {
                    if readers[index].is_finished() {
                        let _ = readers.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
            }
            if state.connections.load(Ordering::Acquire) == 0
                && !state.bootstrap.load(Ordering::Acquire)
            {
                break;
            }
            let mut fds = [
                libc::pollfd {
                    fd: listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: awakened.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) };
            if ready < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(failure("profile_channel_wait"));
            }
            let mut bytes = [0; 256];
            while awakened.read(&mut bytes).is_ok_and(|count| count != 0) {}
        }
        Ok(())
    })();
    // Connections are gone before the engine's last sender is dropped. The
    // ordinary Browser/guardian cleanup completes before releasing startup lock.
    if result.is_err() {
        state.shutdown();
    }
    for reader in readers {
        let _ = reader.join();
    }
    drop(state);
    engine.join().map_err(|_| failure("profile_worker_join"))?;
    result
}

fn read_client(mut connection: Connection, expected: &str) -> Result<()> {
    let initial = Budget::new(Duration::from_secs(60), Some(&connection.cancel));
    let mut framed = LocalStream::new(
        connection
            .stream
            .try_clone()
            .map_err(|_| failure("profile_channel"))?,
        &initial,
    );
    let hello = wire::read_json::<Hello>(&mut framed, None)?;
    if hello.context != expected {
        wire::write_json(&mut framed, &Reply::error(failure("profile_context")))?;
        return Err(failure("profile_context"));
    }
    let owner = uuid::Uuid::new_v4().to_string();
    connection.owner = Some(owner.clone());
    connection.state.owners.fetch_add(1, Ordering::AcqRel);
    wire::write_json(
        &mut framed,
        &Reply::Hello {
            context: expected.into(),
            owner: owner.clone(),
        },
    )?;
    let in_flight = Arc::new(AtomicBool::new(false));
    loop {
        connection
            .stream
            .set_read_timeout(None)
            .map_err(|_| failure("profile_channel"))?;
        let mut first = [0; 1];
        match connection.stream.read(&mut first) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(failure("profile_channel_read")),
        }
        if in_flight.swap(true, Ordering::AcqRel) {
            return Err(failure("profile_request_overlap"));
        }
        let started = Instant::now();
        let header_budget = Budget::new(Duration::from_secs(300), Some(&connection.cancel));
        framed.set_budget(&header_budget);
        let mut request = wire::read_json::<Request>(&mut framed, Some(first[0]))?;
        let timeout = request.timeout_ms();
        let bytes = request.validate()?;
        let end = started + Duration::from_millis(timeout);
        let remaining = end.checked_duration_since(Instant::now()).ok_or_else(|| {
            Error::new("RENDER_TIMEOUT", "Browser operation exceeded its deadline")
        })?;
        let cancel = connection.cancel.child_token();
        let budget = Budget::new(remaining, Some(&cancel));
        framed.set_budget(&budget);
        let Some(reservation) = connection.state.reserve(bytes) else {
            wire::write_json(&mut framed, &Reply::error(failure("profile_queue_limit")))?;
            return Ok(());
        };
        wire::write_json(&mut framed, &Reply::Ready)?;
        if let Request::Page { page, files, .. } = &mut request
            && !files.is_empty()
        {
            let mut resources = Vec::with_capacity(files.len());
            for file in files {
                let mut bytes = vec![0; file.bytes];
                framed
                    .read_exact(&mut bytes)
                    .map_err(|_| failure("profile_channel_read"))?;
                resources.push((
                    file.url.clone(),
                    file.mime.clone(),
                    connection.state.intern(bytes)?,
                ));
            }
            page.resources = Some(Arc::new(crate::cdp::LocalFiles::with_optional_mime(
                resources,
            )?));
        }
        let writer = connection
            .stream
            .try_clone()
            .map_err(|_| failure("profile_channel"))?;
        connection
            .state
            .queue
            .send(Job::Request {
                owner: owner.clone(),
                request,
                writer,
                end,
                cancel,
                in_flight: in_flight.clone(),
                _reservation: reservation,
            })
            .map_err(|_| failure("profile_worker_stopped"))?;
    }
}

struct Engine {
    profile: Profile,
    browser_path: PathBuf,
    executable: PathBuf,
    browser: Option<Browser>,
    policy: Option<crate::config::runtime::PageNetworkPolicy>,
}
impl Engine {
    fn close_owner(&mut self, owner: &str) {
        if let Some(browser) = self.browser.as_mut()
            && browser.leave_profile_owner(owner).is_err()
        {
            self.browser.take();
            self.policy = None;
        }
    }
    fn run(&mut self, job: Job) {
        let Job::Request {
            owner,
            mut request,
            writer,
            end,
            cancel,
            in_flight,
            _reservation,
        } = job
        else {
            if let Job::Close {
                owner,
                state,
                _writer,
            } = job
            {
                self.close_owner(&owner);
                if state.owners.load(Ordering::Acquire) == 0 {
                    self.browser.take();
                    self.policy = None;
                }
            }
            return;
        };
        let budget = Budget::new(end.saturating_duration_since(Instant::now()), Some(&cancel));
        let result = (|| {
            budget.check()?;
            if Profile::from_directory(self.profile.directory())?.identity()
                != self.profile.identity()
            {
                return Err(failure("profile_identity"));
            }
            match &mut request {
                Request::Page { page, .. } => {
                    page.validate()?;
                    if self.policy != Some(page.settings.network_policy) {
                        self.browser.take();
                        self.policy = None;
                    }
                    if self.browser.is_none() {
                        let launch = Budget::new(
                            budget
                                .remaining()?
                                .min(Duration::from_millis(page.settings.launch_timeout_ms)),
                            Some(&cancel),
                        );
                        loop {
                            launch.check()?;
                            match self.profile.lease() {
                                Ok(lease) => {
                                    drop(lease);
                                    break;
                                }
                                Err(error)
                                    if error
                                        .details
                                        .as_ref()
                                        .is_some_and(|d| d["stage"] == "profile_lock") =>
                                {
                                    std::thread::sleep(Duration::from_millis(10))
                                }
                                Err(error) => return Err(error),
                            }
                        }
                        let installed = crate::runtime::InstalledBrowser {
                            executable: self.browser_path.clone(),
                            version: "shared-profile".into(),
                            platform: crate::runtime::platform()?.into(),
                        };
                        let mut browser = Browser::launch_for_runtime(
                            &installed,
                            Purpose::Page,
                            page.settings.network_policy,
                            &launch,
                            Some(&self.executable),
                            Some(&self.profile),
                        )?;
                        // Discover popup ownership even if it appears while a
                        // different frontend's acquisition is being serviced.
                        browser.command(
                            None,
                            "Target.setDiscoverTargets",
                            serde_json::json!({"discover":true}),
                            &launch,
                        )?;
                        self.browser = Some(browser);
                        self.policy = Some(page.settings.network_policy);
                    }
                    let browser = self.browser.as_mut().expect("profile browser");
                    browser.enter_profile_owner(&owner, &budget)?;
                    if let Some((key, _)) = &mut page.retained {
                        *key = format!("{owner}:{key}");
                    }
                    let raster = page.acquire(browser, &budget)?;
                    Ok((
                        Reply::Raster {
                            metadata: raster.metadata,
                            bytes: raster.png.len(),
                            pid: browser.process_id(),
                            retained: browser.profile_page_count(&owner),
                        },
                        Some(raster.png),
                    ))
                }
                Request::Retain { signatures, .. } => {
                    if let Some(browser) = &mut self.browser {
                        browser.retain_profile_pages(&owner, signatures, &budget)?;
                    }
                    Ok((
                        Reply::Done {
                            pid: self.browser.as_ref().map_or(0, Browser::process_id),
                            retained: self
                                .browser
                                .as_ref()
                                .map_or(0, |b| b.profile_page_count(&owner)),
                        },
                        None,
                    ))
                }
            }
        })()
        .and_then(|reply| {
            budget.check()?;
            Ok(reply)
        });
        let mut output = LocalStream::new(writer, &budget);
        // Allow the next header once the result is committed, before its last
        // byte can reach the caller. Otherwise a fast sequential caller races
        // the writer's final store. At most one successor can be queued.
        in_flight.store(false, Ordering::Release);
        match result {
            Ok((reply, png)) => {
                let sent = wire::write_json(&mut output, &reply).and_then(|()| {
                    if let Some(png) = png {
                        output
                            .write_all(&png)
                            .map_err(|_| failure("profile_channel_write"))?;
                    }
                    Ok(())
                });
                if sent.is_err() {
                    self.close_owner(&owner);
                }
            }
            Err(error) => {
                self.close_owner(&owner);
                let error = budget.check().err().unwrap_or(error);
                // A timed-out/closed caller need not receive an error body. Its
                // connection watcher already cancels and releases this owner.
                let _ = wire::write_json(&mut output, &Reply::error(error));
            }
        }
    }
}
