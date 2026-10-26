//! Real X11 emulator pixels and keyboard input from an isolated Xvfb display.
use super::raster::{self, Region};
use crate::{dist, fixtures::oracles::Raster, process, util};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const COLORS: [[u8; 4]; 3] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];

fn command(program: &str, args: &[&str]) -> Result<process::Output> {
    process::output_limited(
        Command::new(program).args(args),
        Duration::from_secs(10),
        65536,
    )
}

fn xdo(args: &[&str]) -> Result<()> {
    let result = command("xdotool", args)?;
    ensure!(
        !result.timed_out && result.status.success(),
        "X11 input failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

fn markers(frame: &Raster) -> Option<Vec<Region>> {
    COLORS
        .iter()
        .map(|color| {
            let mut regions = raster::regions(frame, *color, 1024);
            (regions.len() == 1).then(|| regions.remove(0))
        })
        .collect()
}

fn same(a: &[Region], b: &[Region]) -> bool {
    a.iter().zip(b).all(|(a, b)| {
        a.left.abs_diff(b.left) <= 1
            && a.top.abs_diff(b.top) <= 1
            && a.right.abs_diff(b.right) <= 1
            && a.bottom.abs_diff(b.bottom) <= 1
    })
}

struct Emulator {
    child: Child,
    root: PathBuf,
    window: String,
}

impl Drop for Emulator {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let group = -(self.child.id() as i32);
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

impl Emulator {
    fn start(protocol: &str, binary: &Path, helper: &Path, root: &Path) -> Result<Self> {
        fs::create_dir(root)?;
        fs::write(root.join("lazypencil.toml"), b"version=1\n")?;
        let document = json!({"version":"2.21","children":[{
            "id":"canvas","type":"frame","layout":"none","width":400,"height":300,"fill":"#f0f0f0",
            "children":[
                {"id":"red","type":"rectangle","x":100,"y":100,"width":60,"height":60,"fill":"#ff0000"},
                {"id":"green","type":"rectangle","x":240,"y":100,"width":60,"height":60,"fill":"#00ff00"},
                {"id":"blue","type":"rectangle","x":120,"y":220,"width":160,"height":20,"fill":"#0000ff"}
            ]
        }]});
        fs::write(root.join("design.pen"), util::encode(&document)?)?;
        let title = format!("lazypencil-owned-{protocol}-{}", std::process::id());
        let mut launch = if protocol.ends_with("kitty") {
            let config = root.join("kitty.conf");
            fs::write(
                &config,
                "linux_display_server x11\nfont_size 11\ninitial_window_width 120c\ninitial_window_height 40c\nwindow_padding_width 0\nbackground_opacity 1\nsync_to_monitor no\n",
            )?;
            let mut c = Command::new("kitty");
            c.arg("--debug-rendering")
                .arg("--title")
                .arg(&title)
                .arg("--config")
                .arg(config);
            c
        } else {
            let mut c = Command::new("xterm");
            c.arg("-title").arg(&title).args([
                "-geometry",
                "120x40",
                "-ti",
                "vt340",
                "-xrm",
                "*decGraphicsID:340",
                "-xrm",
                "*numColorRegisters:256",
                "-fa",
                "monospace",
                "-fs",
                "11",
                "-bg",
                "#101010",
                "-fg",
                "#eeeeee",
                "-e",
            ]);
            c
        };
        launch
            .args(["strace", "-f", "-s", "256", "-e", "trace=read", "-o"])
            .arg(root.join("input.strace.log"))
            .arg(binary)
            .arg("--data-dir")
            .arg(root.join("data"))
            .arg("tui")
            .arg(root.join("design.pen"))
            .arg("--adapter")
            .arg(helper)
            .args([
                "--adapter-arg=acp",
                "--adapter-arg=image-inspect",
                "--image-protocol",
                if protocol.starts_with("auto-") {
                    "auto"
                } else {
                    protocol
                },
            ])
            .current_dir(root)
            .env("LAZYPENCIL_CONFIG_DIR", root.join("user"))
            .stdin(Stdio::null())
            .stdout(fs::File::create(root.join("emulator.stdout.log"))?)
            .stderr(fs::File::create(root.join("emulator.stderr.log"))?);
        unsafe {
            launch.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut state = Self {
            child: launch.spawn()?,
            root: root.into(),
            window: String::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            ensure!(
                state.child.try_wait()?.is_none(),
                "Emulator exited before opening its window"
            );
            let result = command(
                "xdotool",
                &["search", "--onlyvisible", "--name", &format!("^{title}$")],
            )?;
            if result.status.success() {
                let windows = std::str::from_utf8(&result.stdout)?
                    .lines()
                    .collect::<Vec<_>>();
                ensure!(
                    windows.len() == 1 && windows[0].bytes().all(|b| b.is_ascii_digit()),
                    "Ambiguous test window"
                );
                state.window = windows[0].into();
                break;
            }
            ensure!(Instant::now() < deadline, "Emulator did not map its window");
            thread::sleep(Duration::from_millis(50));
        }
        xdo(&["windowfocus", "--sync", &state.window])?;
        Ok(state)
    }

    fn key(&mut self, key: &str) -> Result<()> {
        xdo(&["key", "--clearmodifiers", key])
    }

    fn phase(
        &mut self,
        name: &str,
        accept: impl Fn(&[Region], &Raster) -> bool,
    ) -> Result<(Vec<Region>, u32, u32)> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut previous = None;
        loop {
            ensure!(
                self.child.try_wait()?.is_none(),
                "Emulator exited during {name}"
            );
            let path = self.root.join("latest.xwd");
            let result = process::output_limited(
                Command::new("xwd")
                    .args(["-silent", "-nobdrs", "-id", &self.window, "-out"])
                    .arg(&path),
                Duration::from_secs(10),
                65536,
            )?;
            ensure!(
                !result.timed_out && result.status.success(),
                "Screenshot capture failed"
            );
            let frame = raster::decode(&util::read(&path, 20 * 1024 * 1024)?)?;
            let current = markers(&frame);
            if let Some(regions) = current.as_ref().filter(|r| accept(r, &frame)) {
                if previous.as_ref() == Some(regions) {
                    let png = raster::png(&frame)?;
                    fs::write(self.root.join(format!("{name}.png")), &png)?;
                    util::publish(
                        &self.root.join(format!("{name}.json")),
                        &util::encode(&json!({
                            "width":frame.width,"height":frame.height,"markers":regions,
                            "png_sha256":util::digest(&png)
                        }))?,
                        false,
                    )?;
                    return Ok((regions.clone(), frame.width, frame.height));
                }
                previous = Some(regions.clone());
            } else {
                previous = None;
            }
            if Instant::now() >= deadline {
                fs::write(self.root.join("failure.png"), raster::png(&frame)?)?;
                bail!("{name} did not produce the expected rendered marker geometry: {current:?}");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn verify(mut self) -> Result<Value> {
        self.phase("loaded", |_, _| true)?;
        self.key("Tab")?;
        let (initial, width, height) = self.phase("initial", |_, _| true)?;
        self.key("plus")?;
        let (zoomed, _, _) = self.phase("zoomed", |r, _| {
            r.iter().zip(&initial).all(|(a, b)| {
                a.right - a.left > b.right - b.left + 2 && a.bottom - a.top > b.bottom - b.top + 2
            })
        })?;
        self.key("Right")?;
        self.phase("panned-right", |r, _| {
            r.iter().zip(&zoomed).all(|(a, b)| {
                b.left > a.left + 8
                    && b.left < a.left + 128
                    && a.top.abs_diff(b.top) <= 1
                    && (a.right - a.left).abs_diff(b.right - b.left) <= 1
                    && (a.bottom - a.top).abs_diff(b.bottom - b.top) <= 1
            })
        })?;
        self.key("Left")?;
        self.phase("pan-restored", |r, _| same(r, &zoomed))?;
        self.key("minus")?;
        self.phase("zoom-restored", |r, _| same(r, &initial))?;
        xdo(&[
            "windowsize",
            &self.window,
            &(width + 160).to_string(),
            &(height + 96).to_string(),
        ])?;
        self.phase("resized", |r, f| {
            f.width > width && f.height > height && !same(r, &initial)
        })?;
        xdo(&[
            "windowsize",
            &self.window,
            &width.to_string(),
            &height.to_string(),
        ])?;
        self.phase("size-restored", |r, f| {
            f.width == width && f.height == height && same(r, &initial)
        })?;
        self.key("q")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait()? {
                ensure!(
                    status.success(),
                    "Emulator or application failed on normal exit"
                );
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Normal keyboard exit did not close the emulator"
            );
            thread::sleep(Duration::from_millis(20));
        }
        fn only_directory(path: &Path) -> Result<PathBuf> {
            let directories = fs::read_dir(path)?
                .map(|entry| -> Result<Option<PathBuf>> {
                    let entry = entry?;
                    Ok(entry.file_type()?.is_dir().then(|| entry.path()))
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            ensure!(directories.len() == 1, "Ambiguous owned test session");
            Ok(directories[0].clone())
        }
        let document = only_directory(&self.root.join("data/documents"))?;
        let session = only_directory(&document.join("conversations"))?;
        let session: Value =
            serde_json::from_slice(&util::read(&session.join("session.json"), 65536)?)?;
        ensure!(
            session["draft"] == "",
            "Terminal replies or navigation polluted the prompt"
        );
        ensure!(
            session["native_session_id"].is_null(),
            "Viewing unexpectedly started an agent"
        );
        Ok(
            json!({"screen_capture":"XWD v7 TrueColor from real X11 window",
            "document_sha256":util::file_digest(&self.root.join("design.pen"))?,
            "markers":"three connected primary-color regions, at least 1024 pixels each",
            "phases":["loaded","initial","zoomed","panned-right","pan-restored","zoom-restored","resized","size-restored"],
            "keyboard_exit":"success","prompt_draft_empty":true,"agent_started":false,
            "physical_display_or_ime_acceptance":false,"latency_benchmark":false}),
        )
    }
}

pub fn run(candidate: Option<&Path>, output: &Path) -> Result<()> {
    ensure!(
        std::env::var_os("DISPLAY").is_some(),
        "Missing isolated X11 display"
    );
    let temp = tempfile::tempdir_in(output)?;
    let extracted = temp.path().join("candidate");
    let revision = std::env::var("GITHUB_SHA").context("Missing source identity")?;
    let (binary, verification) = if let Some(candidate) = candidate {
        (
            extracted.join("bin/lazypencil"),
            dist::binary::verify(candidate, Some(&extracted))?,
        )
    } else {
        let build = crate::ci::native::Session::hosted()?;
        let binary = build.executable("debug");
        let verification = super::development_identity(&build.root, &binary, &revision)?;
        (binary, verification)
    };
    ensure!(
        verification["source_revision"] == revision,
        "Candidate belongs to another source"
    );
    let target = match std::env::consts::ARCH {
        "x86_64" => "x86_64-unknown-linux-gnu",
        "aarch64" => "aarch64-unknown-linux-gnu",
        _ => bail!("Unsupported initial-release emulator architecture"),
    };
    ensure!(
        verification["target"] == target,
        "Candidate belongs to another architecture"
    );
    let helper = std::env::current_exe()?.with_file_name("lazypencil-test-helper");
    ensure!(helper.is_file(), "Missing independent ACP test helper");
    let mut results = json!({});
    let mut failed = Vec::new();
    for (protocol, program) in [
        ("kitty", "kitty"),
        ("auto-kitty", "kitty"),
        ("sixel", "xterm"),
        ("auto-sixel", "xterm"),
    ] {
        let version = command(
            program,
            &[if program == "kitty" {
                "--version"
            } else {
                "-version"
            }],
        )?;
        ensure!(version.status.success(), "Cannot identify emulator version");
        let identity = format!(
            "{}{}",
            String::from_utf8_lossy(&version.stdout),
            String::from_utf8_lossy(&version.stderr)
        );
        ensure!(
            !identity.trim().is_empty(),
            "Empty emulator version identity"
        );
        // Each emulator owns its window and session. Keep the other reproductions
        // after a failure, but run sequentially because XTest uses display focus.
        let mut report = match Emulator::start(protocol, &binary, &helper, &output.join(protocol))
            .and_then(Emulator::verify)
        {
            Ok(mut report) => {
                report["status"] = json!("passed");
                report
            }
            Err(error) => {
                failed.push(protocol);
                json!({"status":"failed","error":format!("{error:#}")})
            }
        };
        report["emulator_version"] = json!(identity.trim());
        if program == "xterm" {
            report["emulator_configuration"] = json!({"decTerminalID":340,"decGraphicsID":340,"numColorRegisters":256,"font":"monospace","font_size":11});
        }
        results[protocol] = report;
    }
    util::publish(
        &output.join("report.json"),
        &util::encode(&json!({
            "format_version":1,"source_commit":revision,
            "candidate":candidate.is_some().then_some(&verification),
            "development_build":candidate.is_none().then_some(&verification),"target":target,
            "display":"Xvfb 1600x1000x24; LIBGL_ALWAYS_SOFTWARE=1","protocols":results,"failed":&failed,
            "scope":"real-emulator rendered pixels and X11 keyboard/resize; not physical-monitor, IME, clipboard, tmux/SSH or performance acceptance"
        }))?,
        false,
    )?;
    ensure!(
        failed.is_empty(),
        "Real-emulator reproductions failed: {}",
        failed.join(", ")
    );
    Ok(())
}
