//! Source-bound PTY observations; physical terminal presentation is measured separately.
use anyhow::{Result, ensure};
use clap::Args;
use std::path::PathBuf;

#[cfg(unix)]
#[path = "../test_support/terminal.rs"]
#[allow(dead_code)]
mod observer;

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    pub candidate: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(2..=200))]
    pub samples: u32,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(30..=300))]
    pub idle_seconds: u32,
    /// Observe real page/shader Chromium trees after the original trials.
    #[arg(long)]
    pub chromium_resources: bool,
}

/// Bounded, content-free worker observations from the explicitly owned child.
pub fn preview_profiles(bytes: &[u8]) -> Result<Vec<serde_json::Value>> {
    use anyhow::Context;
    let legacy_phases = [
        "snapshot_ms",
        "render_png_ms",
        "verification_ms",
        "decode_png_ms",
        "terminal_encode_ms",
        "metadata_ms",
    ];
    let phases = [
        "snapshot_ms",
        "render_raster_ms",
        "render_png_encode_ms",
        "verification_ms",
        "decode_png_ms",
        "terminal_encode_ms",
        "metadata_ms",
    ];
    let pixel_phases = [
        "snapshot_ms",
        "render_raster_ms",
        "render_pixel_copy_ms",
        "verification_ms",
        "terminal_encode_ms",
        "metadata_ms",
    ];
    ensure!(bytes.len() <= 64 * 1024, "Unbounded preview profile log");
    let mut reports = Vec::new();
    let mut previous = None;
    for line in std::str::from_utf8(bytes)?.lines() {
        let Some(line) = line.strip_prefix("LAZYPENCIL_PREVIEW_TIMING ") else {
            continue;
        };
        ensure!(reports.len() < 128, "Too many preview profiles");
        let report: serde_json::Value = serde_json::from_str(line)?;
        let phases: &[&str] = match report["format_version"].as_u64() {
            Some(1) => &legacy_phases,
            Some(2) => &phases,
            Some(3) => &pixel_phases,
            _ => anyhow::bail!("Unsupported preview profile version"),
        };
        let generation = report["generation"]
            .as_u64()
            .context("Missing preview generation")?;
        let fields = report["phases_ms"]
            .as_object()
            .context("Missing preview phases")?;
        ensure!(
            report.as_object().is_some_and(|fields| fields.len() == 4)
                && previous.is_none_or(|previous| generation > previous)
                && fields.len() == phases.len(),
            "Invalid preview profile version, order or phase membership"
        );
        let mut sum = 0.0;
        for &phase in phases {
            let value = fields
                .get(phase)
                .and_then(serde_json::Value::as_f64)
                .context("Missing preview phase duration")?;
            ensure!(value.is_finite() && value >= 0.0, "Invalid phase duration");
            sum += value;
        }
        let total = report["worker_total_ms"]
            .as_f64()
            .context("Missing preview total duration")?;
        ensure!(
            total.is_finite() && total >= 0.0 && sum <= total + 0.001,
            "Preview phases exceed the worker interval"
        );
        previous = Some(generation);
        reports.push(report);
    }
    ensure!(!reports.is_empty(), "Missing owned native preview profiles");
    Ok(reports)
}

impl Arguments {
    pub fn run(self) -> Result<()> {
        ensure!(
            (2..=200).contains(&self.samples) && (30..=300).contains(&self.idle_seconds),
            "Expected 2..200 startup samples and 30..300 idle seconds"
        );
        ensure!(
            std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                && cfg!(any(target_os = "linux", target_os = "macos")),
            "TUI measurement requires an isolated initial-platform GHA runner"
        );
        #[cfg(unix)]
        {
            hosted::run(self)
        }
        #[cfg(not(unix))]
        {
            anyhow::bail!("TUI measurement is unavailable for this target")
        }
    }
}

#[cfg(unix)]
mod hosted {
    use super::*;
    use crate::{dist, evidence, fixtures, performance::resources, util};
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

    #[derive(Clone, Copy, Default)]
    struct ObserverTime {
        screen_ms: f64,
        graphics_ms: f64,
        hash_ms: f64,
        poll_ms: f64,
    }
    impl ObserverTime {
        fn since(self, before: Self) -> Value {
            json!({"screen_ms":self.screen_ms-before.screen_ms,"graphics_ms":self.graphics_ms-before.graphics_ms,"hash_ms":self.hash_ms-before.hash_ms,"poll_ms":self.poll_ms-before.poll_ms})
        }
    }

    struct Terminal {
        child: Child,
        master: fs::File,
        _slave: fs::File,
        screen: observer::Screen,
        frames: observer::KittyFrames,
        received: u64,
        started: Instant,
        deadline: Instant,
        last_frame: Option<(Instant, String, observer::Frame, ObserverTime)>,
        last_camera: Option<(Instant, String, observer::Frame, ObserverTime)>,
        camera_interactions: Vec<Value>,
        observer: ObserverTime,
        activity: Option<resources::ActiveTrace>,
    }

    impl Drop for Terminal {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
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
        fn start(binary: &Path, helper: &Path, root: &Path, deadline: Instant) -> Result<Self> {
            Self::start_mode(binary, helper, root, deadline, "image-inspect")
        }

        fn start_mode(
            binary: &Path,
            helper: &Path,
            root: &Path,
            deadline: Instant,
            mode: &str,
        ) -> Result<Self> {
            Self::start_runtime(binary, helper, root, deadline, mode, None)
        }

        fn start_runtime(
            binary: &Path,
            helper: &Path,
            root: &Path,
            deadline: Instant,
            mode: &str,
            chromium: Option<&Path>,
        ) -> Result<Self> {
            let (mut master, mut slave) = (-1, -1);
            let mut size = libc::winsize {
                ws_row: 54,
                ws_col: 160,
                ws_xpixel: 1920,
                ws_ypixel: 1080,
            };
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
                "Cannot open owned measurement PTY"
            );
            let master = unsafe { fs::File::from_raw_fd(master) };
            let slave = unsafe { fs::File::from_raw_fd(slave) };
            let mut command = Command::new(binary);
            command
                .current_dir(root)
                .env("TERM", "xterm-kitty")
                .env("COLORTERM", "truecolor")
                .env("LAZYPENCIL_PROFILE_PREVIEW", "owned-gha-measurement")
                .env_remove("TERM_PROGRAM")
                .env_remove("TMUX")
                .env_remove("WEZTERM_EXECUTABLE")
                .env_remove("KONSOLE_VERSION")
                .env("LAZYPENCIL_CONFIG_DIR", root.join("user"))
                .arg("--data-dir")
                .arg(root.join("data"))
                .arg("tui")
                .arg(root.join("dashboard.pen"))
                .arg("--adapter")
                .arg(helper)
                .args([
                    "--adapter-arg=acp",
                    &format!("--adapter-arg={mode}"),
                    "--image-protocol",
                    "kitty",
                ])
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(fs::File::create(root.join("stderr.log"))?));
            if let Some(chromium) = chromium {
                command.env("LAZYPENCIL_CHROMIUM", chromium);
            }
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
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
                screen: observer::Screen::new(160, 54),
                frames: observer::KittyFrames::for_lazypencil(),
                received: 0,
                started,
                deadline,
                last_frame: None,
                last_camera: None,
                camera_interactions: Vec::new(),
                observer: ObserverTime::default(),
                activity: None,
            })
        }

        fn pump(&mut self) -> Result<()> {
            ensure!(
                Instant::now() < self.deadline,
                "TUI measurement exceeded ten minutes"
            );
            ensure!(
                self.child.try_wait()?.is_none(),
                "TUI exited during measurement"
            );
            if let Some(activity) = &mut self.activity {
                activity.observe_if_due()?;
            }
            let mut descriptor = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let poll_started = Instant::now();
            let ready = unsafe { libc::poll(&mut descriptor, 1, 20) };
            self.observer.poll_ms += poll_started.elapsed().as_secs_f64() * 1000.0;
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    return Ok(());
                }
                return Err(error.into());
            }
            ensure!(
                descriptor.revents & (libc::POLLERR | libc::POLLNVAL) == 0,
                "Measurement PTY failed"
            );
            if descriptor.revents & libc::POLLIN != 0 {
                let mut bytes = [0; 65536];
                let count = self.master.read(&mut bytes)?;
                ensure!(count > 0, "Measurement PTY closed");
                let received = Instant::now();
                let observer_at_receipt = self.observer;
                self.received += count as u64;
                ensure!(
                    self.received <= 512 * 1024 * 1024,
                    "Unbounded measured terminal output"
                );
                let screen_started = Instant::now();
                self.screen.feed(&bytes[..count]);
                self.observer.screen_ms += screen_started.elapsed().as_secs_f64() * 1000.0;
                let graphics_started = Instant::now();
                self.frames
                    .feed(&bytes[..count])
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                self.observer.graphics_ms += graphics_started.elapsed().as_secs_f64() * 1000.0;
                if let Some(frame) = self.frames.last_camera.take() {
                    let hash_started = Instant::now();
                    self.last_camera = Some((
                        received,
                        util::digest(frame.rgba()),
                        frame,
                        observer_at_receipt,
                    ));
                    self.observer.hash_ms += hash_started.elapsed().as_secs_f64() * 1000.0;
                }
                if let Some(frame) = self.frames.images.pop() {
                    let hash_started = Instant::now();
                    self.last_frame = Some((
                        received,
                        util::digest(frame.rgba()),
                        frame,
                        observer_at_receipt,
                    ));
                    self.observer.hash_ms += hash_started.elapsed().as_secs_f64() * 1000.0;
                    self.frames.images.clear();
                }
            }
            Ok(())
        }

        fn wait(&mut self, label: &str, mut predicate: impl FnMut(&Self) -> bool) -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !predicate(self) {
                ensure!(Instant::now() < deadline, "Timed out waiting for {label}");
                self.pump()?;
            }
            Ok(())
        }

        fn key(&mut self, bytes: &[u8]) -> Result<Instant> {
            let sent = Instant::now();
            self.master.write_all(bytes)?;
            Ok(sent)
        }

        fn changed_frame(&mut self, key: &[u8], previous: &str) -> Result<(f64, String, Value)> {
            let before = self.observer;
            let sent = self.key(key)?;
            self.wait("changed preview frame", |t| {
                t.last_frame
                    .as_ref()
                    .is_some_and(|(when, hash, _, _)| *when >= sent && hash != previous)
            })?;
            let (received, hash, _, at_receipt) = self
                .last_frame
                .as_ref()
                .context("Missing changed preview")?;
            self.camera_interactions.push(
                self.last_camera
                    .as_ref()
                    .filter(|(when, _, _, _)| *when >= sent && *when <= *received)
                    .map_or(Value::Null, |(when, hash, frame, _)| {
                        json!({
                            "elapsed_ms":when.duration_since(sent).as_secs_f64()*1000.0,
                            "rgba_sha256":hash,"width":frame.width,"height":frame.height,
                            "scope":"Temporary cached camera output, not the complete render"
                        })
                    }),
            );
            Ok((
                received.duration_since(sent).as_secs_f64() * 1000.0,
                hash.clone(),
                json!({"before_output_receipt":at_receipt.since(before),"through_complete_parse":self.observer.since(before),"scope":"Wall-time counters before complete output receipt versus after parsing its final chunk; the latter includes work outside the measured receipt latency"}),
            ))
        }

        fn capture(&self, root: &Path, name: &str) -> Result<()> {
            self.capture_frame(root, name, &self.last_frame)
        }

        fn capture_camera(&self, root: &Path, name: &str) -> Result<()> {
            self.capture_frame(root, name, &self.last_camera)
        }

        fn capture_frame(
            &self,
            root: &Path,
            name: &str,
            frame: &Option<(Instant, String, observer::Frame, ObserverTime)>,
        ) -> Result<()> {
            let (_, hash, frame, _) = frame.as_ref().context("Missing diagnostic frame")?;
            let raster = fixtures::oracles::Raster {
                width: frame.width as u32,
                height: frame.height as u32,
                pixels: frame
                    .rgba()
                    .chunks_exact(4)
                    .map(|pixel| pixel.try_into().unwrap())
                    .collect(),
            };
            let png = crate::terminal::raster::png(&raster)?;
            util::publish(&root.join(format!("{name}.png")), &png, false)?;
            util::publish(
                &root.join(format!("{name}.json")),
                &util::encode(
                    &json!({"width":frame.width,"height":frame.height,"rgba_sha256":hash,"png_sha256":util::digest(&png)}),
                )?,
                false,
            )?;
            util::publish(
                &root.join(format!("{name}.screen.txt")),
                self.screen.text().as_bytes(),
                false,
            )?;
            Ok(())
        }

        fn finish(&mut self) -> Result<()> {
            self.key(b"q")?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = self.child.try_wait()? {
                    ensure!(status.success(), "TUI failed on normal exit");
                    return Ok(());
                }
                ensure!(Instant::now() < deadline, "TUI did not exit normally");
                // Drain the PTY so terminal restoration cannot block behind output.
                let mut descriptor = libc::pollfd {
                    fd: self.master.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut descriptor, 1, 10) } > 0
                    && descriptor.revents & libc::POLLIN != 0
                {
                    let _ = self.master.read(&mut [0; 65536]);
                }
            }
        }
    }

    fn verify_empty_conversation(root: &Path) -> Result<()> {
        let files = util::walk(&root.join("data"))?;
        let sessions = files
            .iter()
            .filter(|path| path.file_name().is_some_and(|name| name == "session.json"))
            .collect::<Vec<_>>();
        ensure!(sessions.len() == 1, "Ambiguous measurement conversation");
        let session: Value = util::json(sessions[0])?;
        ensure!(
            session["draft"] == "" && session["native_session_id"].is_null(),
            "Measurement left prompt text or started an agent"
        );
        ensure!(
            !files.iter().any(|path| path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("turn-"))),
            "Measurement unexpectedly created a conversation turn"
        );
        Ok(())
    }

    fn active_resources_trial(
        binary: &Path,
        helper: &Path,
        output: &Path,
        deadline: Instant,
    ) -> Result<Value> {
        let root = output.join("resource-observation");
        let fixture = fixtures::performance(&root, "dashboard")?;
        fs::write(root.join("lazypencil.toml"), b"version=1\n")?;
        let mut terminal = Terminal::start(binary, helper, &root, deadline)?;
        let sampling_start_delay_ms = terminal.started.elapsed().as_secs_f64() * 1000.0;
        terminal.activity = Some(resources::ActiveTrace::new(terminal.child.id())?);
        terminal.wait("resource trial prompt", |t| {
            t.screen.text().contains("Prompt")
        })?;
        terminal.wait("resource trial complete preview", |t| {
            t.last_frame.is_some()
        })?;
        let initial = terminal.last_frame.as_ref().unwrap().1.clone();
        terminal.key(b"\t")?;
        let (_, zoomed, _) = terminal.changed_frame(b"+", &initial)?;
        let (_, unzoomed, _) = terminal.changed_frame(b"-", &zoomed)?;
        ensure!(
            unzoomed == initial,
            "Resource trial zoom changed original pixels"
        );
        let (_, panned, _) = terminal.changed_frame(b"\x1b[C", &initial)?;
        let (_, unpanned, _) = terminal.changed_frame(b"\x1b[D", &panned)?;
        ensure!(
            unpanned == initial,
            "Resource trial pan changed original pixels"
        );
        let trace = terminal
            .activity
            .take()
            .context("Missing activity trace")?
            .finish()?;
        terminal.finish()?;
        verify_empty_conversation(&root)?;
        let profiles = super::preview_profiles(&util::read(&root.join("stderr.log"), 64 * 1024)?)?;
        ensure!(
            profiles.len() >= 5,
            "Resource trial lacks complete worker profiles"
        );
        Ok(
            json!({"fixture":fixture,"sampling_start_delay_ms":sampling_start_delay_ms,
            "trace":trace,"initial_rgba_sha256":initial,"restored_rgba_sha256":unpanned,
            "native_preview_worker_profiles":profiles,
            "complete_interactions":4,"normal_exit":true,"agent_started":false,
            "scope":"Additional resource-only run after all original startup/interaction/idle trials; not added to latency statistics"}),
        )
    }

    fn bound_resources_trial(
        binary: &Path,
        helper: &Path,
        output: &Path,
        deadline: Instant,
        static_hash: Option<&str>,
        chromium: Option<&Path>,
    ) -> Result<Value> {
        let root = output.join(if chromium.is_some() {
            "chromium-resource-observation"
        } else {
            "bound-resource-observation"
        });
        let fixture = if chromium.is_some() {
            fixtures::performance_with_dynamic(&root)?
        } else {
            fixtures::performance_with_script(&root)?
        };
        fs::write(root.join("lazypencil.toml"), b"version=1\n")?;
        let mut terminal = Terminal::start_runtime(
            binary,
            helper,
            &root,
            deadline,
            if chromium.is_some() {
                "dynamic-resource-observation"
            } else {
                "resource-observation"
            },
            chromium,
        )?;
        terminal.activity = Some(resources::ActiveTrace::new(terminal.child.id())?);
        let result = (|| -> Result<Value> {
            terminal.wait("script dashboard preview", |t| t.last_frame.is_some())?;
            let initial_hash = terminal.last_frame.as_ref().unwrap().1.clone();
            if let Some(static_hash) = static_hash {
                ensure!(
                    initial_hash == static_hash,
                    "Script header changed dashboard pixels"
                );
            }
            let static_hash = initial_hash.as_str();
            terminal.capture(&root, "initial")?;
            let before = resources::snapshot(terminal.child.id())?;
            let children = before
                .processes
                .values()
                .filter(|process| process.parent == terminal.child.id())
                .collect::<Vec<_>>();
            let tui_script = if chromium.is_some() {
                let scripts = children
                    .iter()
                    .filter(|process| resources::require_script_worker(process.pid, binary).is_ok())
                    .collect::<Vec<_>>();
                ensure!(scripts.len() == 1, "Ambiguous viewer script worker");
                (**scripts[0]).clone()
            } else {
                ensure!(
                    children.len() == 1 && before.processes.len() == 2,
                    "Unexpected viewer children before the agent request"
                );
                let script = (*children[0]).clone();
                resources::require_script_worker(script.pid, binary)?;
                script
            };
            terminal.key(b"Measure this owned document through MCP.\x13")?;
            terminal.wait("bound MCP render", |_| {
                root.join("bound-render-0.json").is_file()
            })?;
            let first: Value = util::json(&root.join("bound-render-0.json"))?;
            let pid = |key: &str| -> Result<u32> {
                Ok(u32::try_from(
                    first[key].as_u64().context("Missing resource PID")?,
                )?)
            };
            let snapshot = resources::snapshot(terminal.child.id())?;
            let roles = if let Some(chromium) = chromium {
                resources::BoundRoles::capture_chromium(
                    &snapshot,
                    terminal.child.id(),
                    pid("adapter_pid")?,
                    pid("mcp_pid")?,
                    tui_script.pid,
                    binary,
                    chromium,
                )?
            } else {
                resources::BoundRoles::capture(
                    &snapshot,
                    terminal.child.id(),
                    pid("adapter_pid")?,
                    pid("mcp_pid")?,
                    tui_script.pid,
                )?
            };
            ensure!(
                roles.processes["tui_script"].start_ticks == tui_script.start_ticks,
                "Viewer script worker changed identity"
            );
            resources::require_script_worker(roles.processes["mcp_script"].pid, binary)?;
            util::publish(
                &root.join("roles.json"),
                &util::encode(&roles.processes)?,
                false,
            )?;
            terminal.activity.as_mut().unwrap().observe_now()?;
            fs::write(root.join("bound-render-again"), b"observe")?;
            terminal.key(b"\t")?;
            let (_, zoomed, _) = terminal.changed_frame(b"+", static_hash)?;
            terminal.capture(&root, "zoomed")?;
            let (_, restored, _) = terminal.changed_frame(b"-", &zoomed)?;
            ensure!(
                restored == static_hash,
                "Bound resource zoom changed dashboard pixels"
            );
            terminal.capture(&root, "zoom-restored")?;
            let (_, panned, _) = terminal.changed_frame(b"\x1b[C", static_hash)?;
            terminal.capture(&root, "panned-right")?;
            let (_, restored, _) = terminal.changed_frame(b"\x1b[D", &panned)?;
            ensure!(
                restored == static_hash,
                "Bound resource pan changed dashboard pixels"
            );
            terminal.capture(&root, "pan-restored")?;
            terminal.wait("second bound MCP render", |_| {
                root.join("bound-render-1.json").is_file()
            })?;
            terminal.activity.as_mut().unwrap().observe_now()?;
            let second: Value = util::json(&root.join("bound-render-1.json"))?;
            ensure!(
                first["adapter_pid"] == second["adapter_pid"]
                    && first["mcp_pid"] == second["mcp_pid"],
                "Bound processes were replaced"
            );
            for (round, metadata) in [&first, &second].into_iter().enumerate() {
                let png = util::read(
                    &root.join(format!("bound-render-{round}.png")),
                    8 * 1024 * 1024,
                )?;
                ensure!(
                    metadata["render"]["png_hash"] == util::digest(png)
                        && metadata["render"]["node_count"] == 1000
                        && metadata["render"]["script_executions"]
                            .as_array()
                            .is_some_and(|items| items.len() == 1),
                    "Incomplete script MCP render evidence"
                );
            }
            ensure!(
                first["render"]["png_hash"] == second["render"]["png_hash"],
                "Repeated bound render changed pixels"
            );
            if chromium.is_some() {
                for metadata in [&first, &second] {
                    let artifacts = metadata["render"]["dynamic_artifacts"]
                        .as_array()
                        .context("Missing Chromium artifacts")?;
                    ensure!(
                        artifacts.len() == 2,
                        "Missing real browser or shader acquisition"
                    );
                    ensure!(
                        artifacts
                            .iter()
                            .any(|artifact| artifact["node_id"] == "header/$generated/4"
                                && artifact["acquisition"]["kind"] == "browser")
                            && artifacts
                                .iter()
                                .any(|artifact| artifact["node_id"] == "header/$generated/7"
                                    && artifact["acquisition"]["kind"] == "shader"),
                        "Unexpected dynamic node identities"
                    );
                }
                for artifact in first["render"]["dynamic_artifacts"].as_array().unwrap() {
                    if artifact["acquisition"]["kind"] == "browser" {
                        let refreshed = second["render"]["dynamic_artifacts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|entry| entry["acquisition"]["kind"] == "browser")
                            .unwrap();
                        ensure!(
                            artifact["acquisition"]["acquisition_id"]
                                != refreshed["acquisition"]["acquisition_id"],
                            "Second MCP page render reused its acquisition"
                        );
                    }
                }
            }
            let idle_before = resources::snapshot(terminal.child.id())?;
            let idle_started = Instant::now();
            let idle_frame = terminal.last_frame.as_ref().unwrap().0;
            let mut idle_checkpoints = Vec::new();
            let mut next_checkpoint = Instant::now() + Duration::from_secs(1);
            while idle_started.elapsed() < Duration::from_secs(30) {
                terminal.pump()?;
                if Instant::now() >= next_checkpoint {
                    idle_checkpoints.push(resources::snapshot(terminal.child.id())?);
                    next_checkpoint = Instant::now() + Duration::from_secs(1);
                }
            }
            let idle_after = resources::snapshot(terminal.child.id())?;
            let idle_elapsed = idle_started.elapsed();
            let raw_cpu = idle_before.idle_cpu_upper_percent(&idle_after, idle_elapsed)?;
            let adapter = roles.processes["adapter"].pid;
            let mut app_before = idle_before.clone();
            let mut app_after = idle_after.clone();
            app_before.processes.remove(&adapter);
            app_after.processes.remove(&adapter);
            let app_cpu = app_before.idle_cpu_upper_percent(&app_after, idle_elapsed)?;
            let adapter_cpu = (idle_after.processes[&adapter].cpu_ms
                - idle_before.processes[&adapter].cpu_ms
                + idle_before.cpu_quantum_ms)
                / idle_elapsed.as_secs_f64()
                / 10.0;
            let mut idle_maximum = 0;
            for snapshot in std::iter::once(&idle_before)
                .chain(&idle_checkpoints)
                .chain(std::iter::once(&idle_after))
            {
                idle_before.idle_cpu_upper_percent(snapshot, idle_elapsed)?;
                idle_maximum = idle_maximum.max(
                    roles.account(snapshot)?["application_rss_bytes"]
                        .as_u64()
                        .unwrap(),
                );
            }
            if chromium.is_some() {
                ensure!(
                    util::file_digest(&root.join("resource-nav.glsl"))? == fixture["shader_sha256"],
                    "Resource shader changed during the turn"
                );
            }
            ensure!(
                terminal.last_frame.as_ref().unwrap().0 == idle_frame,
                "Unchanged bound dashboard repainted during idle observation"
            );
            let idle = json!({"seconds":idle_elapsed.as_secs_f64(),"before":idle_before,"checkpoints":idle_checkpoints,"after":idle_after,
                "application_cpu_upper_percent":app_cpu,"adapter_cpu_upper_percent":adapter_cpu,"raw_owned_cpu_upper_percent":raw_cpu,
                "maximum_sampled_application_rss_bytes":idle_maximum,"no_preview_image_update":true,
                "scope":"Separate 30-second idle interval with the bound MCP, both JavaScript workers and supervisor still alive; explicit ACP helper excluded only from application accounting"});
            let mut idle = idle;
            if chromium.is_some() {
                idle["scope"] = json!(
                    "Additional 30-second Chromium-inclusive idle interval; raw stable process membership and counters retained; only the identified external ACP helper excluded from application accounting"
                );
            }
            fs::write(root.join("bound-render-release"), b"complete")?;
            terminal.wait("completed resource turn", |t| {
                root.join("bound-render-closed.json").is_file()
                    && t.screen.text().contains("完了 1")
                    && t.screen
                        .text()
                        .lines()
                        .next()
                        .is_some_and(|line| line.contains("idle"))
            })?;
            let trace = terminal.activity.take().unwrap().finish_bound(&roles)?;
            let observed_pids = trace["points"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|point| point["snapshot"]["processes"].as_object().unwrap().values())
                .map(|process| process["pid"].as_u64().unwrap() as u32)
                .collect::<std::collections::BTreeSet<_>>();
            terminal.finish()?;
            let cleanup = Instant::now() + Duration::from_secs(10);
            while observed_pids
                .iter()
                .any(|&pid| unsafe { libc::kill(pid as i32, 0) } == 0)
            {
                ensure!(
                    Instant::now() < cleanup,
                    "Bound resource processes survived normal TUI exit"
                );
                thread::sleep(Duration::from_millis(10));
            }
            ensure!(
                util::file_digest(&root.join("dashboard.pen"))? == fixture["document_sha256"]
                    && util::file_digest(&root.join("resource-header.js"))?
                        == fixture["script_sha256"],
                "Resource fixture changed during the turn"
            );
            let files = util::walk(&root.join("data"))?;
            let sessions = files
                .iter()
                .filter(|path| path.file_name().is_some_and(|name| name == "session.json"))
                .collect::<Vec<_>>();
            ensure!(sessions.len() == 1, "Ambiguous bound resource conversation");
            let session: Value = util::json(sessions[0])?;
            ensure!(
                session["draft"] == "" && session["native_session_id"] == "fixture-native",
                "Resource request did not complete its own conversation"
            );
            let mut report = json!({"fixture":fixture,"trace":trace,"idle":idle,"mcp_renders":[first,second],
                "complete_interactions":4,"initial_rgba_sha256":static_hash,"restored_rgba_sha256":restored,
                "normal_exit":true,"owned_processes_gone":true,"observed_owned_pids":observed_pids,"document_and_script_unchanged":true,
                "scope":"Separate resource-only turn after all original trials; actual bound MCP and JavaScript, deterministic ACP helper only, no real provider request; excluded from original latency distributions"});
            if chromium.is_some() {
                report["scope"] = json!(
                    "Additional Chromium-inclusive resource turn after every original trial; actual offline page JavaScript, software shader, bound MCP and both script workers; only the explicit external ACP helper excluded; no provider or latency acceptance claim"
                );
            }
            Ok(report)
        })();
        if result.is_err() {
            let _ = fs::write(root.join("failure-screen.txt"), terminal.screen.text());
            if let Some(activity) = terminal.activity.take()
                && let Ok(mut trace) = activity.finish()
            {
                trace["scope"] = json!(
                    "Partial failed bound-resource trial; no acceptance or complete-process accounting claim"
                );
                let _ = util::encode(&trace).and_then(|bytes| {
                    util::publish(&root.join("failure-trace.json"), &bytes, false)
                });
            }
        }
        result
    }

    fn verified_chromium(binary: &Path) -> Result<(PathBuf, Value)> {
        let temporary = PathBuf::from(
            std::env::var_os("RUNNER_TEMP").context("Missing hosted temporary directory")?,
        );
        let data = temporary.join("chromium-test-data");
        let raw = crate::process::checked(
            Command::new(binary)
                .env_remove("LAZYPENCIL_CHROMIUM")
                .arg("--json")
                .arg("--data-dir")
                .arg(&data)
                .args(["runtime", "status"]),
            Duration::from_secs(300),
        )?;
        let status: Value = serde_json::from_slice(&raw)?;
        ensure!(
            status["available"] == true
                && status["managed"] == true
                && status["integrity_verified"] == true,
            "Chromium resource measurement requires a verified managed runtime"
        );
        let executable = PathBuf::from(
            status["executable"]
                .as_str()
                .context("Missing managed Chromium executable")?,
        )
        .canonicalize()?;
        let configured = PathBuf::from(
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
                .context("Install the hosted Chromium runtime before measuring it")?,
        )
        .canonicalize()?;
        ensure!(
            executable == configured
                && executable.starts_with(data.join("runtimes/chromium").canonicalize()?),
            "Chromium resource runtime differs from the verified setup"
        );
        let identity = json!({"status":status,"executable_sha256":util::file_digest(&executable)?,"scope":"Managed runtime integrity rechecked by the packaged executable immediately before the separate Chromium observation"});
        Ok((executable, identity))
    }

    pub(super) fn run(arguments: Arguments) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(600);
        let candidate = std::path::absolute(arguments.candidate)?;
        let output = std::path::absolute(arguments.output)?;
        fs::create_dir(&output)?;
        let temporary = tempfile::tempdir_in(&output)?;
        let extracted = temporary.path().join("candidate");
        let identity = dist::binary::verify(&candidate, Some(&extracted))?;
        ensure!(
            identity["source_revision"] == std::env::var("GITHUB_SHA")?
                && identity["target"] == evidence::linkage::native_host()?,
            "Candidate differs from hosted measurement source or target"
        );
        let binary = extracted.join("bin/lazypencil");
        let helper = std::env::current_exe()?.with_file_name("lazypencil-test-helper");
        let helper_identity =
            evidence::fingerprint(&helper, "measurement-helper", 64 * 1024 * 1024)?;
        let mut samples = Vec::new();
        let mut idle = Value::Null;
        for sample in 0..arguments.samples {
            ensure!(
                Instant::now() < deadline,
                "TUI measurement exceeded ten minutes"
            );
            let root = output.join(format!("sample-{sample}"));
            let fixture = fixtures::performance(&root, "dashboard")?;
            fs::write(root.join("lazypencil.toml"), b"version=1\n")?;
            let mut terminal = Terminal::start(&binary, &helper, &root, deadline)?;
            terminal.wait("prompt", |t| t.screen.text().contains("Prompt"))?;
            let marker = b"measurement-ready";
            terminal.key(marker)?;
            terminal.wait("prompt echo", |t| {
                t.screen.text().contains("measurement-ready")
            })?;
            let input_ms = terminal.started.elapsed().as_secs_f64() * 1000.0;
            terminal.key(&vec![0x7f; marker.len()])?;
            terminal.wait("cleared prompt", |t| {
                !t.screen.text().contains("measurement-ready")
            })?;
            terminal.wait("first complete preview", |t| t.last_frame.is_some())?;
            let preview_ms = terminal
                .last_frame
                .as_ref()
                .unwrap()
                .0
                .duration_since(terminal.started)
                .as_secs_f64()
                * 1000.0;
            terminal.key(b"\t")?;
            let initial = terminal.last_frame.as_ref().unwrap().1.clone();
            terminal.capture(&root, "initial")?;
            let (zoom_ms, zoomed, zoom_observer) = terminal.changed_frame(b"+", &initial)?;
            if sample == 0 {
                terminal.capture(&root, "zoomed")?;
                if terminal
                    .camera_interactions
                    .last()
                    .is_some_and(|frame| !frame.is_null())
                {
                    terminal.capture_camera(&root, "camera-zoomed")?;
                }
            }
            let (unzoom_ms, restored, unzoom_observer) = terminal.changed_frame(b"-", &zoomed)?;
            if sample == 0 || restored != initial {
                terminal.capture(&root, "zoom-restored")?;
                if sample == 0
                    && terminal
                        .camera_interactions
                        .last()
                        .is_some_and(|frame| !frame.is_null())
                {
                    terminal.capture_camera(&root, "camera-zoom-restored")?;
                }
            }
            ensure!(
                restored == initial,
                "Zoom did not restore the original preview pixels"
            );
            let (pan_ms, panned, pan_observer) = terminal.changed_frame(b"\x1b[C", &initial)?;
            if sample == 0 {
                terminal.capture(&root, "panned-right")?;
                if terminal
                    .camera_interactions
                    .last()
                    .is_some_and(|frame| !frame.is_null())
                {
                    terminal.capture_camera(&root, "camera-panned-right")?;
                }
            }
            let (unpan_ms, restored, unpan_observer) =
                terminal.changed_frame(b"\x1b[D", &panned)?;
            if sample == 0 || restored != initial {
                terminal.capture(&root, "pan-restored")?;
                if sample == 0
                    && terminal
                        .camera_interactions
                        .last()
                        .is_some_and(|frame| !frame.is_null())
                {
                    terminal.capture_camera(&root, "camera-pan-restored")?;
                }
            }
            ensure!(
                restored == initial,
                "Pan did not restore the original preview pixels"
            );
            let resources = resources::snapshot(terminal.child.id())?;
            samples.push(json!({"input_ready_ms":input_ms,"first_preview_ms":preview_ms,"interaction_ms":[zoom_ms,unzoom_ms,pan_ms,unpan_ms],"observer_wall_ms":[zoom_observer,unzoom_observer,pan_observer,unpan_observer],"processes":resources,"fixture":fixture,"terminal_bytes":terminal.received,"compressed_kitty_frames":terminal.frames.compressed_frames}));
            samples.last_mut().unwrap()["cached_camera_interaction_outputs"] =
                json!(terminal.camera_interactions);
            samples.last_mut().unwrap()["cached_camera_frames"] =
                json!(terminal.frames.camera_frames);
            if sample + 1 == arguments.samples {
                let before = resources::snapshot(terminal.child.id())?;
                let idle_frame = terminal.last_frame.as_ref().map(|frame| frame.0);
                let started = Instant::now();
                let mut checkpoints = Vec::new();
                let mut next_sample = Instant::now();
                while started.elapsed() < Duration::from_secs(arguments.idle_seconds.into()) {
                    terminal.pump()?;
                    if Instant::now() >= next_sample {
                        checkpoints.push(resources::snapshot(terminal.child.id())?);
                        next_sample = Instant::now() + Duration::from_secs(1);
                    }
                }
                let elapsed = started.elapsed();
                let after = resources::snapshot(terminal.child.id())?;
                ensure!(
                    terminal.last_frame.as_ref().map(|frame| frame.0) == idle_frame,
                    "Static preview retransmitted during idle measurement"
                );
                let cpu = before.idle_cpu_upper_percent(&after, elapsed)?;
                for checkpoint in &checkpoints {
                    before.idle_cpu_upper_percent(checkpoint, elapsed)?;
                }
                let rss = checkpoints
                    .iter()
                    .chain([&before, &after])
                    .map(resources::Snapshot::rss_bytes)
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .max()
                    .context("Missing idle RSS samples")?;
                idle = json!({"seconds":elapsed.as_secs_f64(),"cpu_upper_percent_including_clock_quantization":cpu,"maximum_sampled_process_tree_rss_bytes":rss,"before":before,"after":after,"checkpoints":checkpoints});
            }
            terminal.finish()?;
            verify_empty_conversation(&root)?;
            let sample = samples.last_mut().context("Missing measurement sample")?;
            sample["normal_exit"] = json!(true);
            sample["prompt_draft_empty"] = json!(true);
            sample["agent_started"] = json!(false);
            let profiles =
                super::preview_profiles(&util::read(&root.join("stderr.log"), 64 * 1024)?)?;
            ensure!(
                profiles.len() >= 5,
                "Missing startup or interaction profiles"
            );
            sample["native_preview_worker_profiles"] = json!(profiles);
            sample["native_preview_profile_scope"] = json!(
                "Completed worker requests before UI delivery; may include superseded results. No per-image transport/presentation correlation or application-only end-to-end latency claim."
            );
        }
        let input = samples
            .iter()
            .map(|s| s["input_ready_ms"].as_f64().unwrap())
            .collect::<Vec<_>>();
        let preview = samples
            .iter()
            .map(|s| s["first_preview_ms"].as_f64().unwrap())
            .collect::<Vec<_>>();
        let interactions = samples
            .iter()
            .flat_map(|s| {
                s["interaction_ms"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
            })
            .collect::<Vec<_>>();
        let input = crate::performance::sample_statistics(&input)?;
        let preview = crate::performance::sample_statistics(&preview)?;
        let interactions = crate::performance::sample_statistics(&interactions)?;
        let activity = active_resources_trial(&binary, &helper, &output, deadline)?;
        let bound_activity = bound_resources_trial(
            &binary,
            &helper,
            &output,
            deadline,
            Some(
                activity["initial_rgba_sha256"]
                    .as_str()
                    .context("Missing static resource pixels")?,
            ),
            None,
        )?;
        let report = json!({"format_version":1,"candidate":identity,"helper":helper_identity,"target":evidence::linkage::native_host()?,"pty":{"columns":160,"rows":54,"pixel_width":1920,"pixel_height":1080,"protocol":"kitty","display_emulator":false},"cache_condition":"Fresh app/data directories per sample; OS page/font caches uncontrolled, individual samples retained with first at index 0; no concurrent tasks in this job","samples":samples,"input_ready_ms":input,"first_preview_ms":preview,"interaction_output_ms":interactions,"idle":idle,"active_resources":activity,"scope":"PTY input echo and complete Kitty output receipt, including observer backpressure; sampled process trees can miss short-lived descendants. Not physical presentation, a controlled cold-start benchmark, transient peak RSS, external AI/adapter/terminal resource usage or full performance acceptance"});
        let mut report = report;
        report["bound_resources"] = bound_activity;
        // Publish the unchanged original observations before the opt-in phase,
        // so a Chromium failure cannot hide the completed baseline evidence.
        util::publish(&output.join("report.json"), &util::encode(&report)?, false)?;
        if arguments.chromium_resources {
            let (chromium, runtime) = verified_chromium(&binary)?;
            report["chromium_resources"] = bound_resources_trial(
                &binary,
                &helper,
                &output,
                Instant::now() + Duration::from_secs(300),
                None,
                Some(&chromium),
            )?;
            let (after, verified) = verified_chromium(&binary)?;
            ensure!(
                after == chromium && runtime == verified,
                "Managed Chromium changed during resource measurement"
            );
            report["chromium_runtime"] = runtime;
            util::publish(&output.join("report.json"), &util::encode(&report)?, true)?;
        }
        util::print(
            &json!({"tui_measurement":"recorded","samples":arguments.samples,"output":output}),
        )
    }
}
