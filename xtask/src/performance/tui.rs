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

    struct Terminal {
        child: Child,
        master: fs::File,
        _slave: fs::File,
        screen: observer::Screen,
        frames: observer::KittyFrames,
        received: u64,
        started: Instant,
        deadline: Instant,
        last_frame: Option<(Instant, String)>,
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
                    "--adapter-arg=image-inspect",
                    "--image-protocol",
                    "kitty",
                ])
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(fs::File::create(root.join("stderr.log"))?));
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
                frames: observer::KittyFrames::default(),
                received: 0,
                started,
                deadline,
                last_frame: None,
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
            let mut descriptor = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut descriptor, 1, 20) };
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
                self.received += count as u64;
                ensure!(
                    self.received <= 512 * 1024 * 1024,
                    "Unbounded measured terminal output"
                );
                self.screen.feed(&bytes[..count]);
                self.frames
                    .feed(&bytes[..count])
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                if let Some(frame) = self.frames.images.pop() {
                    self.last_frame = Some((received, util::digest(frame.rgba)));
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

        fn changed_frame(&mut self, key: &[u8], previous: &str) -> Result<(f64, String)> {
            let sent = self.key(key)?;
            self.wait("changed preview frame", |t| {
                t.last_frame
                    .as_ref()
                    .is_some_and(|(when, hash)| *when >= sent && hash != previous)
            })?;
            let (received, hash) = self
                .last_frame
                .as_ref()
                .context("Missing changed preview")?;
            Ok((
                received.duration_since(sent).as_secs_f64() * 1000.0,
                hash.clone(),
            ))
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
            let (zoom_ms, zoomed) = terminal.changed_frame(b"+", &initial)?;
            let (unzoom_ms, restored) = terminal.changed_frame(b"-", &zoomed)?;
            ensure!(
                restored == initial,
                "Zoom did not restore the original preview pixels"
            );
            let (pan_ms, panned) = terminal.changed_frame(b"\x1b[C", &initial)?;
            let (unpan_ms, restored) = terminal.changed_frame(b"\x1b[D", &panned)?;
            ensure!(
                restored == initial,
                "Pan did not restore the original preview pixels"
            );
            let resources = resources::snapshot(terminal.child.id())?;
            samples.push(json!({"input_ready_ms":input_ms,"first_preview_ms":preview_ms,"interaction_ms":[zoom_ms,unzoom_ms,pan_ms,unpan_ms],"processes":resources,"fixture":fixture,"terminal_bytes":terminal.received}));
            if sample + 1 == arguments.samples {
                let before = resources::snapshot(terminal.child.id())?;
                let idle_frame = terminal.last_frame.clone();
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
                    terminal.last_frame == idle_frame,
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
            let files = util::walk(&root.join("data"))?;
            let sessions = files
                .iter()
                .filter(|p| p.file_name().is_some_and(|n| n == "session.json"))
                .collect::<Vec<_>>();
            ensure!(sessions.len() == 1, "Ambiguous measurement conversation");
            let session: Value = util::json(sessions[0])?;
            ensure!(
                session["draft"] == "" && session["native_session_id"].is_null(),
                "Measurement left prompt text or started an agent"
            );
            ensure!(
                !files.iter().any(|p| p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("turn-"))),
                "Measurement unexpectedly created a conversation turn"
            );
            let sample = samples.last_mut().context("Missing measurement sample")?;
            sample["normal_exit"] = json!(true);
            sample["prompt_draft_empty"] = json!(true);
            sample["agent_started"] = json!(false);
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
        let report = json!({"format_version":1,"candidate":identity,"helper":helper_identity,"target":evidence::linkage::native_host()?,"pty":{"columns":160,"rows":54,"pixel_width":1920,"pixel_height":1080,"protocol":"kitty","display_emulator":false},"cache_condition":"Fresh app/data directories per sample; OS page/font caches uncontrolled, individual samples retained with first at index 0; no concurrent tasks in this job","samples":samples,"input_ready_ms":input,"first_preview_ms":preview,"interaction_output_ms":interactions,"idle":idle,"scope":"PTY input echo and complete Kitty output receipt, including observer backpressure; sampled persistent process trees can miss short-lived descendants. Not physical presentation, a controlled cold-start benchmark, transient peak RSS, external AI/adapter/terminal resource usage or full performance acceptance"});
        util::publish(&output.join("report.json"), &util::encode(&report)?, false)?;
        util::print(
            &json!({"tui_measurement":"recorded","samples":arguments.samples,"output":output}),
        )
    }
}
