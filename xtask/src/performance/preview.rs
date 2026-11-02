//! Local, owned-PTY preview measurements for an explicitly supplied document.
use anyhow::{Result, ensure};
use clap::{Args, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry([u16; 4]);

fn geometry(value: &str) -> std::result::Result<Geometry, String> {
    let values = value
        .split(',')
        .map(str::parse::<u16>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| {
            "Expected rows,columns,pixel_width,pixel_height as positive u16 values".to_owned()
        })?;
    let values: [u16; 4] = values
        .try_into()
        .map_err(|_| "Expected four geometry values".to_owned())?;
    if values.contains(&0) {
        return Err("Geometry values must be positive".into());
    }
    Ok(Geometry(values))
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum KittyTransfer {
    #[default]
    Zlib,
    Png,
}

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    document: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "measurement")]
    label: String,
    #[arg(long, default_value = "54,160,1920,1080", value_parser = geometry)]
    geometry: Geometry,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(2..=200))]
    samples: u32,
    /// Number of Tab presses needed to focus Preview in the measured UI version.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    focus_tabs: u8,
    /// Include down/up round trips in addition to zoom and horizontal movement.
    #[arg(long)]
    all_directions: bool,
    /// Simulate direct Kitty (ZLIB) or an unprobed compatible terminal (PNG).
    #[arg(long, value_enum, default_value_t = KittyTransfer::Zlib)]
    kitty_transfer: KittyTransfer,
    #[arg(long)]
    median_limit_ms: Option<f64>,
    #[arg(long)]
    reference_report: Option<PathBuf>,
    #[arg(long)]
    bursts: bool,
    #[arg(long)]
    worker_profile: bool,
    #[arg(long)]
    profile: bool,
    #[arg(long)]
    capture: bool,
}

impl Arguments {
    fn actions(&self) -> Vec<(&'static str, &'static [u8])> {
        let mut actions = vec![
            ("zoom_in", b"+".as_slice()),
            ("zoom_out", b"-".as_slice()),
            ("pan_right", b"\x1b[C".as_slice()),
            ("pan_left", b"\x1b[D".as_slice()),
        ];
        if self.all_directions {
            actions.extend([
                ("pan_down", b"\x1b[B".as_slice()),
                ("pan_up", b"\x1b[A".as_slice()),
            ]);
        }
        actions
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            !self.label.is_empty()
                && self.label != "."
                && self.label != ".."
                && !self.label.contains(['/', '\\']),
            "Label must be one file name"
        );
        if let Some(limit) = self.median_limit_ms {
            ensure!(
                limit.is_finite()
                    && limit > 0.0
                    && !self.worker_profile
                    && !self.profile
                    && !self.capture,
                "A finite positive median limit requires an unprofiled run without capture"
            );
        }
        ensure!(
            !self.profile || cfg!(target_os = "macos"),
            "Native sampling requires macOS"
        );
        Ok(())
    }

    pub fn run(self) -> Result<()> {
        self.validate()?;
        #[cfg(unix)]
        {
            owned::run(self)
        }
        #[cfg(not(unix))]
        {
            anyhow::bail!("Local preview measurement requires a Unix PTY")
        }
    }
}

#[cfg(unix)]
use super::observer as decoder;

#[cfg(unix)]
mod owned {
    use super::*;
    use crate::{performance::sample_statistics, util};
    use anyhow::Context;
    use serde_json::{Value, json};
    use std::{
        fs,
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        },
        path::Path,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    const SYNC_END: &[u8] = b"\x1b[?2026l";
    const DSR: &[u8] = b"\x1b[6n";

    #[derive(Debug, PartialEq)]
    enum ControlEvent {
        Data(Vec<u8>),
        Sync,
        CursorPosition,
    }

    fn control_events(buffer: &mut Vec<u8>, bytes: &[u8]) -> Vec<ControlEvent> {
        buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        loop {
            let event = [SYNC_END, DSR]
                .into_iter()
                .enumerate()
                .filter_map(|(kind, marker)| {
                    buffer
                        .windows(marker.len())
                        .position(|part| part == marker)
                        .map(|at| (at, kind, marker.len()))
                })
                .min_by_key(|event| event.0);
            let Some((at, kind, length)) = event else {
                break;
            };
            if at > 0 {
                events.push(ControlEvent::Data(buffer.drain(..at).collect()));
            }
            buffer.drain(..length);
            events.push(if kind == 0 {
                ControlEvent::Sync
            } else {
                ControlEvent::CursorPosition
            });
        }
        // Preserve incomplete markers across arbitrary PTY read boundaries.
        let count = buffer.len().saturating_sub(SYNC_END.len() - 1);
        if count > 0 {
            events.push(ControlEvent::Data(buffer.drain(..count).collect()));
        }
        events
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn controls_preserve_image_bytes_and_order_across_every_read_boundary() {
            let wire = b"image-one\x1b[?2026limage-two\x1b[6nimage-three\x1b[?2026l";
            let check = |chunks: Vec<&[u8]>| {
                let mut buffer = Vec::new();
                let mut images = Vec::new();
                let mut current = Vec::new();
                let mut cursor_requests = 0;
                for chunk in chunks {
                    for event in control_events(&mut buffer, chunk) {
                        match event {
                            ControlEvent::Data(bytes) => current.extend(bytes),
                            ControlEvent::Sync => images.push(std::mem::take(&mut current)),
                            ControlEvent::CursorPosition => cursor_requests += 1,
                        }
                    }
                }
                assert_eq!(
                    images,
                    [b"image-one".to_vec(), b"image-twoimage-three".to_vec()]
                );
                assert_eq!(cursor_requests, 1);
                assert!(buffer.is_empty());
            };
            for split in 0..=wire.len() {
                check(vec![&wire[..split], &wire[split..]]);
            }
            check(wire.chunks(1).collect());
        }
    }

    #[derive(Clone)]
    struct Frame {
        exact: bool,
        received: Instant,
        synced: Option<Instant>,
        hash: String,
        width: usize,
        height: usize,
    }

    struct Sampler(Child);

    impl Drop for Sampler {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    struct Terminal {
        child: Child,
        master: fs::File,
        _slave: fs::File,
        decoder: decoder::KittyFrames,
        control: Vec<u8>,
        frames: Vec<Frame>,
        pending: Vec<usize>,
        started: Instant,
        bytes: u64,
        observer_ms: f64,
        root: PathBuf,
        capture: bool,
    }

    impl Drop for Terminal {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            // The child owns a new session; never signal the user's terminal.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGTERM);
            }
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
    }

    impl Terminal {
        fn start(args: &Arguments, binary: &Path, document: &Path, root: &Path) -> Result<Self> {
            let config = root.join("config");
            fs::create_dir(&config)?;
            fs::write(
                config.join("config.toml"),
                "[fonts]\nfallbacks=[\"Noto Sans JP\",\"Apple Color Emoji\",\"Apple Symbols\"]\n",
            )?;
            let [rows, columns, width, height] = args.geometry.0;
            let mut size = libc::winsize {
                ws_row: rows,
                ws_col: columns,
                ws_xpixel: width,
                ws_ypixel: height,
            };
            let (mut master, mut slave) = (-1, -1);
            ensure!(
                unsafe {
                    libc::openpty(
                        &mut master,
                        &mut slave,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &raw mut size,
                    )
                } == 0,
                "Cannot open measurement PTY"
            );
            let master = unsafe { fs::File::from_raw_fd(master) };
            let slave = unsafe { fs::File::from_raw_fd(slave) };
            let mut command = Command::new(binary);
            command
                .current_dir(document.parent().context("Document has no parent")?)
                .env(
                    "TERM",
                    match args.kitty_transfer {
                        KittyTransfer::Zlib => "xterm-kitty",
                        KittyTransfer::Png => "xterm-256color",
                    },
                )
                .env("COLORTERM", "truecolor")
                .env("LAZYPENCIL_CONFIG_DIR", &config)
                .args(["--data-dir"])
                .arg(root.join("data"))
                .arg("tui")
                .arg(document)
                .args(["--adapter", "/usr/bin/false", "--image-protocol", "kitty"])
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(fs::File::create(root.join("stderr.log"))?));
            for key in [
                "TMUX",
                "TERM_PROGRAM",
                "WEZTERM_EXECUTABLE",
                "KONSOLE_VERSION",
                "GITHUB_ACTIONS",
                "LAZYPENCIL_PROFILE_PREVIEW",
            ] {
                command.env_remove(key);
            }
            if args.worker_profile {
                command.env("LAZYPENCIL_PROFILE_PREVIEW", "owned-local-measurement");
            }
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let started = Instant::now();
            let child = command.spawn()?;
            Ok(Self {
                child,
                master,
                _slave: slave,
                decoder: decoder::KittyFrames::for_lazypencil(),
                control: Vec::new(),
                frames: Vec::new(),
                pending: Vec::new(),
                started,
                bytes: 0,
                observer_ms: 0.0,
                root: root.to_owned(),
                capture: args.capture,
            })
        }

        fn record(&mut self, frame: decoder::Frame, exact: bool, received: Instant) -> Result<()> {
            ensure!(self.frames.len() < 2048, "Too many preview frames");
            let hash = util::digest(frame.rgba());
            if self.capture {
                fs::write(
                    self.root.join(format!(
                        "frame-{}-{}-{}x{}.rgba",
                        self.frames.len() + 1,
                        if exact { "exact" } else { "cached" },
                        frame.width,
                        frame.height
                    )),
                    frame.rgba(),
                )?;
            }
            self.pending.push(self.frames.len());
            self.frames.push(Frame {
                exact,
                received,
                synced: None,
                hash,
                width: frame.width,
                height: frame.height,
            });
            Ok(())
        }

        fn feed(&mut self, data: &[u8], received: Instant) -> Result<()> {
            self.decoder
                .feed(data)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let exact = std::mem::take(&mut self.decoder.images);
            for frame in exact {
                self.record(frame, true, received)?;
            }
            if let Some(frame) = self.decoder.last_camera.take() {
                self.record(frame, false, received)?;
            }
            Ok(())
        }

        fn pump(&mut self, wait: Duration) -> Result<()> {
            ensure!(
                self.child.try_wait()?.is_none(),
                "Preview process exited during measurement"
            );
            let mut poll = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, wait.as_millis().min(20) as i32) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    return Ok(());
                }
                return Err(error.into());
            }
            if ready == 0 {
                return Ok(());
            }
            ensure!(
                poll.revents & (libc::POLLERR | libc::POLLNVAL) == 0,
                "Measurement PTY failed"
            );
            let mut bytes = [0; 65536];
            let size = self.master.read(&mut bytes)?;
            let received = Instant::now();
            ensure!(size > 0, "Measurement PTY closed");
            self.bytes += size as u64;
            ensure!(
                self.bytes <= 512 * 1024 * 1024,
                "Measurement output exceeded its bound"
            );
            let begin = Instant::now();
            for event in control_events(&mut self.control, &bytes[..size]) {
                match event {
                    ControlEvent::Data(bytes) => self.feed(&bytes, received)?,
                    ControlEvent::Sync => {
                        for index in self.pending.drain(..) {
                            self.frames[index].synced = Some(received);
                        }
                    }
                    ControlEvent::CursorPosition => self.master.write_all(b"\x1b[1;1R")?,
                }
            }
            self.observer_ms += begin.elapsed().as_secs_f64() * 1000.0;
            Ok(())
        }

        fn drain(&mut self, duration: Duration) -> Result<()> {
            let until = Instant::now() + duration;
            while Instant::now() < until {
                self.pump(
                    until
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(20)),
                )?;
            }
            Ok(())
        }

        fn key(&mut self, key: &[u8]) -> Result<Instant> {
            let sent = Instant::now();
            self.master.write_all(key)?;
            Ok(sent)
        }

        fn wait_exact(&mut self, after: Instant, expected: Option<&str>) -> Result<Frame> {
            let until = Instant::now() + Duration::from_secs(45);
            loop {
                if let Some(frame) =
                    self.frames.iter().rev().find(|frame| {
                        frame.exact && frame.received >= after && frame.synced.is_some()
                    })
                    && expected.is_none_or(|hash| frame.hash == hash)
                {
                    return Ok(frame.clone());
                }
                ensure!(
                    Instant::now() < until,
                    "Timed out waiting for a complete preview image"
                );
                self.pump(Duration::from_millis(20))?;
            }
        }

        fn interaction(&mut self, action: &str, key: &[u8]) -> Result<Value> {
            self.drain(Duration::from_millis(30))?;
            let previous = self
                .frames
                .iter()
                .rev()
                .find(|frame| frame.exact)
                .context("Missing previous frame")?
                .hash
                .clone();
            let (bytes, observer) = (self.bytes, self.observer_ms);
            let sent = self.key(key)?;
            let frame = self.wait_exact(sent, None)?;
            let cached = self.frames.iter().find(|f| {
                !f.exact && f.received >= sent && f.received <= frame.received && f.synced.is_some()
            });
            let sync = frame.synced.context("Missing synchronized update end")?;
            let feedback = cached.map_or(sync, |cached| cached.synced.unwrap().min(sync));
            Ok(
                json!({"action":action,"complete_ms":millis(frame.received, sent),"complete_sync_ms":millis(sync,sent),"feedback_ms":millis(feedback,sent),"cached_ms":cached.map(|f| millis(f.synced.unwrap(), sent)),"bytes":self.bytes-bytes,"changed":frame.hash != previous,"hash":frame.hash,"observer_ms":self.observer_ms-observer}),
            )
        }

        fn close(&mut self) -> Result<i32> {
            let _ = self.key(b"\x03");
            let until = Instant::now() + Duration::from_secs(4);
            while Instant::now() < until {
                if let Some(status) = self.child.try_wait()? {
                    return Ok(status.code().unwrap_or(-1));
                }
                let _ = self.pump(Duration::from_millis(20));
            }
            anyhow::bail!("Preview process did not exit normally")
        }
    }

    fn millis(after: Instant, before: Instant) -> f64 {
        after.duration_since(before).as_secs_f64() * 1000.0
    }

    fn stats(values: &[f64]) -> Result<Value> {
        let mut result = sample_statistics(values)?;
        let fields = result.as_object_mut().unwrap();
        let p95 = fields.remove("p95_nearest_rank").unwrap();
        fields.insert("p95".into(), p95);
        fields.insert("n".into(), json!(values.len()));
        Ok(result)
    }

    pub(super) fn run(args: Arguments) -> Result<()> {
        let binary = fs::canonicalize(&args.binary)?;
        let document = fs::canonicalize(&args.document)?;
        let document_hash = util::file_digest(&document)?;
        let binary_hash = util::file_digest(&binary)?;
        let [rows, columns, width, height] = args.geometry.0;
        let geometry =
            json!({"rows":rows,"columns":columns,"pixel_width":width,"pixel_height":height});
        let actions = args.actions();
        let reference: Option<Value> = args
            .reference_report
            .as_ref()
            .map(|path| util::json(path))
            .transpose()?;
        if let Some(reference) = &reference {
            ensure!(
                reference["document_sha256"] == document_hash
                    && reference["geometry"] == geometry
                    && reference["protocol"] == "kitty",
                "Reference document, geometry or protocol differs"
            );
            ensure!(
                reference["samples"]
                    .as_array()
                    .is_some_and(|s| s.len() == args.samples as usize * actions.len()),
                "Reference sample count differs"
            );
        }
        if let Some(parent) = args.output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&args.output).context("Measurement output directory must be new")?;
        let output = fs::canonicalize(&args.output)?;
        let root = output.join(&args.label);
        fs::create_dir(&root)?;
        let mut terminal = Terminal::start(&args, &binary, &document, &root)?;
        let mut report = json!({"format_version":1,"observer":"Rust owned PTY / Kitty","binary":binary,"binary_sha256":binary_hash,"document_sha256":document_hash,"document_bytes":fs::metadata(&document)?.len(),"protocol":"kitty","geometry":geometry,"platform":format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH),"scope":"Key write to full image receipt and synchronized update end in an owned PTY; no physical display timing. False adapter prevents agent/account work. OS caches and load uncontrolled.","samples":[]});
        report["kitty_transfer"] = json!(match args.kitty_transfer {
            KittyTransfer::Zlib => "zlib",
            KittyTransfer::Png => "png",
        });
        report["actions"] = json!(
            actions
                .iter()
                .map(|(action, _)| *action)
                .collect::<Vec<_>>()
        );
        let result = (|| -> Result<()> {
            let first = terminal.wait_exact(terminal.started, None)?;
            report["first_preview_ms"] = json!(millis(first.synced.unwrap(), terminal.started));
            report["frame"] = json!({"width":first.width,"height":first.height,"compressed":terminal.decoder.compressed_frames > 0});
            report["initial_rgba_sha256"] = json!(first.hash);
            terminal.key(&vec![b'\t'; usize::from(args.focus_tabs)])?;
            terminal.drain(Duration::from_millis(200))?;
            let mut profiler = if args.profile {
                Some(Sampler(
                    Command::new("/usr/bin/sample")
                        .args([
                            terminal.child.id().to_string(),
                            "5".into(),
                            "1".into(),
                            "-file".into(),
                        ])
                        .arg(root.join("sample.txt"))
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()?,
                ))
            } else {
                None
            };
            for round in 0..args.samples {
                let mut trial = Vec::new();
                for (action, key) in &actions {
                    trial.push(terminal.interaction(action, key)?);
                }
                ensure!(
                    trial.iter().all(|s| s["changed"] == true)
                        && trial
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .all(|pair| pair[1]["hash"] == first.hash),
                    "Camera round trip changed or failed to restore pixels"
                );
                if let Some(reference) = &reference {
                    for (offset, sample) in trial.iter().enumerate() {
                        let expected =
                            &reference["samples"][round as usize * actions.len() + offset];
                        ensure!(
                            sample["hash"] == expected["hash"]
                                && sample["action"] == expected["action"],
                            "Reference image pixels differ"
                        );
                    }
                }
                println!(
                    "{}",
                    json!({"trial":round,"complete_ms":trial.iter().map(|s|s["complete_ms"].clone()).collect::<Vec<_>>()})
                );
                report["samples"].as_array_mut().unwrap().extend(trial);
            }
            let mut statistics = json!({});
            for &(action, _) in &actions {
                for field in [
                    "complete_ms",
                    "complete_sync_ms",
                    "feedback_ms",
                    "bytes",
                    "observer_ms",
                ] {
                    let values = report["samples"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|sample| sample["action"] == action)
                        .map(|sample| sample[field].as_f64().context("Missing sample value"))
                        .collect::<Result<Vec<_>>>()?;
                    statistics[action][field] = stats(&values)?;
                }
            }
            report["statistics"] = statistics;
            if let Some(profiler) = &mut profiler {
                let until = Instant::now() + Duration::from_secs(15);
                loop {
                    if let Some(status) = profiler.0.try_wait()? {
                        report["profiler_exit_code"] = json!(status.code());
                        break;
                    }
                    if Instant::now() >= until {
                        let _ = profiler.0.kill();
                        let _ = profiler.0.wait();
                        anyhow::bail!("Native sampler exceeded deadline");
                    }
                    terminal.pump(Duration::from_millis(20))?;
                }
            }
            report["pixel_restoration_verified"] = json!(true);
            report["reference_pixels_verified"] = json!(reference.is_some());
            if args.bursts {
                let mut bursts = Vec::new();
                let burst_actions = if args.all_directions {
                    vec!["pan", "pan_vertical", "zoom"]
                } else {
                    vec!["pan", "zoom"]
                };
                for action in burst_actions {
                    terminal.drain(Duration::from_millis(200))?;
                    let before = terminal.frames.len();
                    let begin = Instant::now();
                    let mut sent = Vec::new();
                    for i in 0..40 {
                        let due = begin + Duration::from_millis(i * 50);
                        while Instant::now() < due {
                            terminal.pump(
                                due.saturating_duration_since(Instant::now())
                                    .min(Duration::from_millis(5)),
                            )?;
                        }
                        let key = if action == "pan_vertical" {
                            if i < 20 {
                                b"\x1b[B".as_slice()
                            } else {
                                b"\x1b[A".as_slice()
                            }
                        } else if action == "pan" {
                            if i < 20 {
                                b"\x1b[C".as_slice()
                            } else {
                                b"\x1b[D".as_slice()
                            }
                        } else if i % 2 == 0 {
                            b"+".as_slice()
                        } else {
                            b"-".as_slice()
                        };
                        sent.push(terminal.key(key)?);
                    }
                    terminal.wait_exact(sent[39], Some(&first.hash))?;
                    terminal.drain(Duration::from_millis(100))?;
                    let last = terminal.wait_exact(sent[39], Some(&first.hash))?;
                    bursts.push(json!({"action":action,"keys":40,"requested_interval_ms":50,"actual_key_interval_ms":stats(&sent.windows(2).map(|p|millis(p[1],p[0])).collect::<Vec<_>>())?,"cached_frames":terminal.frames[before..].iter().filter(|f|!f.exact).count(),"exact_frames":terminal.frames[before..].iter().filter(|f|f.exact).count(),"last_input_to_exact_sync_ms":millis(last.synced.unwrap(),sent[39]),"pixel_restoration_verified":last.hash==first.hash}));
                }
                report["bursts"] = json!(bursts);
            }
            Ok(())
        })();
        let exit = terminal.close();
        report["exit_code"] = json!(exit.as_ref().ok());
        report["document_unchanged"] =
            json!(util::file_digest(&document).is_ok_and(|hash| hash == document_hash));
        report["binary_unchanged"] =
            json!(util::file_digest(&binary).is_ok_and(|hash| hash == binary_hash));
        let stderr = util::read(&root.join("stderr.log"), 1024 * 1024)?;
        let profiles = String::from_utf8_lossy(&stderr)
            .lines()
            .filter_map(|line| line.strip_prefix("LAZYPENCIL_PREVIEW_TIMING "))
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        report["worker_profiles"] = json!(profiles);
        let report_path = output.join(format!("{}.json", args.label));
        fs::write(&report_path, util::encode(&report)?)?;
        result?;
        ensure!(
            exit? == 0
                && report["document_unchanged"] == true
                && report["binary_unchanged"] == true,
            "Measurement changed inputs or failed to exit normally"
        );
        if let Some(limit) = args.median_limit_ms {
            ensure!(
                report["statistics"].as_object().unwrap().values().all(|v| {
                    v["complete_ms"]["median"]
                        .as_f64()
                        .is_some_and(|median| median <= limit)
                }),
                "Complete-image median exceeds limit; all observations retained"
            );
        }
        util::print(
            &json!({"report":report_path,"statistics":report["statistics"],"exit_code":report["exit_code"]}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Arguments,
    }

    #[test]
    fn geometry_requires_four_positive_bounded_values() {
        assert_eq!(
            geometry("57,224,2688,1140").unwrap(),
            Geometry([57, 224, 2688, 1140])
        );
        for value in ["1,2,3", "0,2,3,4", "65536,2,3,4", "1,2,no,4"] {
            assert!(geometry(value).is_err());
        }
    }

    #[test]
    fn acceptance_rejects_instrumentation_and_path_labels() {
        for extra in [
            vec!["--capture"],
            vec!["--worker-profile"],
            vec!["--profile"],
            vec!["--label", "../escape"],
        ] {
            let mut argv = vec![
                "preview",
                "--binary",
                "app",
                "--document",
                "scene.pen",
                "--output",
                "new",
                "--median-limit-ms",
                "30",
            ];
            argv.extend(extra);
            let parsed = Cli::try_parse_from(argv).unwrap();
            assert!(parsed.args.validate().is_err());
        }
        let parsed = Cli::try_parse_from([
            "preview",
            "--binary",
            "app",
            "--document",
            "scene.pen",
            "--output",
            "new",
            "--median-limit-ms",
            "30",
        ])
        .unwrap();
        assert!(parsed.args.validate().is_ok());
    }
}
