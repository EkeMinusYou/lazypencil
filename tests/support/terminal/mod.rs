//! Real PTY observations shared by frontend and retained-browser integration tests.
mod cases;
#[path = "../../../xtask/src/test_support/command_output.rs"]
#[allow(dead_code)] // CI also consumes elapsed time and separate stderr.
mod command_output;
#[path = "../../../xtask/src/test_support/terminal.rs"]
#[allow(dead_code)] // Observer methods are also exercised by independent xtask tests.
pub mod observer;
pub use cases::run;

use observer::{KittyFrames, Screen};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Harness {
    pub binary: PathBuf,
    pub root: PathBuf,
    pub document: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
    pub case: String,
    pub image_case: String,
    pub configuration: Option<PathBuf>,
    pub output: Vec<u8>,
    pub frames: KittyFrames,
    pub screen: Screen,
    pub child: Child,
    master: fs::File,
    slave: fs::File,
    original: libc::termios,
    probe_answered: bool,
}

pub fn read_json(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}
pub fn write_json(path: &Path, value: &Value) -> Result {
    fs::write(path, serde_json::to_vec(value)?)?;
    Ok(())
}

fn run_cli(
    binary: &Path,
    root: &Path,
    environment: &BTreeMap<OsString, OsString>,
    arguments: &[&std::ffi::OsStr],
) -> Result<Value> {
    let output = command_output::output(
        Command::new(binary)
            .current_dir(root)
            .envs(environment)
            .arg("--data-dir")
            .arg(root.join("data"))
            .arg("--json")
            .args(arguments),
        Duration::from_secs(300),
        64 * 1024 * 1024,
    )?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "CLI failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub fn files(root: &Path, name: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    fn visit(root: &Path, predicate: &impl Fn(&str) -> bool, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit(&path, predicate, out);
                } else if predicate(&entry.file_name().to_string_lossy()) {
                    out.push(path);
                }
            }
        }
    }
    let mut result = Vec::new();
    visit(root, &name, &mut result);
    result.sort();
    result
}

fn ready(fd: libc::c_int, milliseconds: i32) -> bool {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        libc::poll(&mut descriptor, 1, milliseconds) > 0 && descriptor.revents & libc::POLLIN != 0
    }
}

impl Harness {
    pub fn start(
        binary: &Path,
        root: &Path,
        document: &Path,
        environment: BTreeMap<OsString, OsString>,
        case: &str,
        configuration: Option<PathBuf>,
    ) -> Result<Self> {
        let image_case = if [
            "dynamic-idle",
            "page-playback",
            "font-config",
            "user-font-config",
            "script-config",
            "user-navigation-config",
        ]
        .contains(&case)
        {
            "kitty"
        } else {
            case.strip_prefix("auto-").unwrap_or(case)
        };
        let protocol = if case.starts_with("auto-") {
            "auto"
        } else if ["kitty", "sixel"].contains(&image_case) {
            image_case
        } else {
            "halfblocks"
        };
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize {
            ws_row: 34,
            ws_col: 130,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let master = unsafe { fs::File::from_raw_fd(master) };
        let slave = unsafe { fs::File::from_raw_fd(slave) };
        let mut original = std::mem::MaybeUninit::<libc::termios>::zeroed();
        if unsafe { libc::tcgetattr(slave.as_raw_fd(), original.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let original = unsafe { original.assume_init() };
        let errors = fs::File::create(root.join("terminal-errors.log"))?;
        let mut command = Command::new(binary);
        command
            .current_dir(root)
            .envs(&environment)
            .arg("--data-dir")
            .arg(root.join("data"))
            .arg("tui")
            .arg(document);
        if case != "user-agent-config" {
            command
                .arg("--adapter")
                .arg(crate::fixture_support::helper())
                .arg("--adapter-arg=acp")
                .arg(if case == "basic" {
                    "--adapter-arg=image-inspect-audit"
                } else {
                    "--adapter-arg=image-inspect"
                });
        }
        command
            .args(["--image-protocol", protocol])
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(errors));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn()?;
        Ok(Self {
            binary: binary.into(),
            root: root.into(),
            document: document.into(),
            environment,
            case: case.into(),
            image_case: image_case.into(),
            configuration,
            output: Vec::new(),
            frames: KittyFrames::for_lazypencil(),
            screen: Screen::new(130, 34),
            child,
            master,
            slave,
            original,
            probe_answered: false,
        })
    }

    pub fn pump(&mut self) -> Result {
        self.pump_with_limit(65536)
    }

    fn pump_with_limit(&mut self, limit: usize) -> Result {
        if ready(self.master.as_raw_fd(), 20) {
            let mut bytes = [0; 65536];
            let count = match self.master.read(&mut bytes[..limit]) {
                Ok(n) => n,
                Err(e) if e.raw_os_error() == Some(libc::EIO) => 0,
                Err(e) => return Err(e.into()),
            };
            let bytes = &bytes[..count];
            self.output.extend_from_slice(bytes);
            self.frames.feed(bytes)?;
            self.screen.feed(bytes);
            if ["auto-kitty", "auto-sixel"].contains(&self.case.as_str())
                && !self.probe_answered
                && self.contains_bytes(b"\x1b[5n")
            {
                let mut reply = if self.image_case == "kitty" {
                    b"\x1b_Gi=31;OK\x1b\\\x1b[?1;2c".to_vec()
                } else {
                    b"\x1b[?1;2;4c".to_vec()
                };
                reply.extend(b"\x1b[6;20;10t\x1b[0n");
                self.key(&reply)?;
                self.probe_answered = true;
            }
            if self.output.len() >= 64 * 1024 * 1024 {
                return Err("Unbounded terminal output".into());
            }
        }
        Ok(())
    }
    pub fn key(&mut self, bytes: &[u8]) -> Result {
        self.master.write_all(bytes)?;
        Ok(())
    }
    pub fn paste(&mut self, text: &str) -> Result {
        self.key(b"\x1b[200~")?;
        self.key(text.as_bytes())?;
        self.key(b"\x1b[201~")
    }
    pub fn contains_bytes(&self, bytes: &[u8]) -> bool {
        self.output.windows(bytes.len()).any(|p| p == bytes)
    }
    pub fn wait(
        &mut self,
        label: &str,
        seconds: f64,
        mut predicate: impl FnMut(&Self) -> bool,
    ) -> Result {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < end {
            self.pump()?;
            if predicate(self) {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait()? {
                return Err(format!(
                    "TUI exited before {label}: {status}\n{}",
                    self.screen.text()
                )
                .into());
            }
        }
        Err(format!("Timed out: {label}\n{}", self.screen.text()).into())
    }
    pub fn visible(&mut self, text: &str) -> Result {
        self.wait(text, 15.0, |h| h.screen.text().contains(text))
    }
    pub fn hidden(&mut self, text: &str) -> Result {
        self.wait(text, 15.0, |h| !h.screen.text().contains(text))
    }
    pub fn draft(&mut self, text: &str) -> Result {
        self.wait("saved draft", 15.0, |h| {
            h.session().is_some_and(|s| s["draft"] == text)
        })
    }
    pub fn pump_for(&mut self, seconds: f64) -> Result {
        let end = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < end {
            self.pump()?;
        }
        Ok(())
    }
    pub fn sessions(&self) -> Vec<PathBuf> {
        files(&self.root.join("data"), |s| s == "session.json")
    }
    pub fn session(&self) -> Option<Value> {
        self.sessions().first().and_then(|p| read_json(p))
    }
    pub fn turns(&self) -> Vec<Value> {
        files(&self.root.join("data"), |s| {
            s.starts_with("turn-") && s.ends_with(".jsonl")
        })
        .into_iter()
        .flat_map(|p| {
            fs::read_to_string(p)
                .unwrap_or_default()
                .lines()
                .filter_map(|s| serde_json::from_str(s).ok())
                .collect::<Vec<Value>>()
        })
        .collect()
    }
    pub fn count(&self, kind: &str) -> usize {
        self.turns().iter().filter(|v| v["type"] == kind).count()
    }
    pub fn last_color(&self, color: [u8; 4]) -> bool {
        self.frames
            .images
            .last()
            .is_some_and(|f| f.has_color(color))
    }
    pub fn cli(&self, arguments: &[&std::ffi::OsStr]) -> Result<Value> {
        run_cli(&self.binary, &self.root, &self.environment, arguments)
    }
    pub fn resize(&mut self, width: u16, height: u16) -> Result {
        let size = libc::winsize {
            ws_row: height,
            ws_col: width,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe { libc::ioctl(self.slave.as_raw_fd(), libc::TIOCSWINSZ, &size) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        self.screen.resize(width, height);
        if unsafe { libc::kill(self.child.id() as i32, libc::SIGWINCH) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    pub fn save_screen(&self, name: &str) -> Result {
        fs::write(self.root.join(name), self.screen.text())?;
        Ok(())
    }
    pub fn finish(&mut self) -> Result {
        self.key(if self.case == "basic" {
            b"q"
        } else if self.case == "user-navigation-config" {
            b"x"
        } else {
            b"\x03"
        })?;
        let end = Instant::now() + Duration::from_secs(10);
        let mut status = None;
        while Instant::now() < end {
            if let Some(value) = self.child.try_wait()? {
                status = Some(value);
                break;
            }
            self.pump()?;
        }
        if !status.is_some_and(|s| s.success()) {
            return Err(format!("TUI did not exit cleanly: {status:?}").into());
        }
        while ready(self.master.as_raw_fd(), 50) {
            self.pump()?;
        }
        let mut restored = std::mem::MaybeUninit::<libc::termios>::zeroed();
        if unsafe { libc::tcgetattr(self.slave.as_raw_fd(), restored.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let restored = unsafe { restored.assume_init() };
        let flags = |t: &libc::termios| {
            #[cfg(target_os = "macos")]
            {
                t.c_lflag & !libc::PENDIN
            }
            #[cfg(not(target_os = "macos"))]
            {
                t.c_lflag
            }
        };
        let before = &self.original;
        assert_eq!(
            (
                before.c_iflag,
                before.c_oflag,
                before.c_cflag,
                flags(before),
                before.c_cc
            ),
            (
                restored.c_iflag,
                restored.c_oflag,
                restored.c_cflag,
                flags(&restored),
                restored.c_cc
            ),
            "Terminal modes must be restored"
        );
        assert_eq!(unsafe { libc::cfgetispeed(before) }, unsafe {
            libc::cfgetispeed(&restored)
        });
        assert_eq!(unsafe { libc::cfgetospeed(before) }, unsafe {
            libc::cfgetospeed(&restored)
        });
        let start = self.output.len();
        self.key(b"mode-restored")?;
        assert!(
            !ready(self.slave.as_raw_fd(), 0),
            "Canonical input must wait for Enter"
        );
        self.key(b"\r")?;
        assert!(ready(self.slave.as_raw_fd(), 1000));
        let mut input = [0; 1024];
        let count = self.slave.read(&mut input)?;
        assert_eq!(&input[..count], b"mode-restored\n");
        // PTYs may deliver the echoed text before its final CRLF. Exercise
        // fragmented reads deliberately while retaining the exact echo check.
        let echo = b"mode-restored\r\n";
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end && !self.output[start..].windows(echo.len()).any(|b| b == echo) {
            self.pump_with_limit(1)?;
        }
        assert!(
            self.output[start..].windows(echo.len()).any(|b| b == echo),
            "Terminal echo must include the restored CRLF: {:?}",
            &self.output[start..]
        );
        assert!(
            self.contains_bytes(b"\x1b[?1049l"),
            "Alternate screen must be restored"
        );
        if self.image_case == "kitty" {
            assert!(
                !self.frames.deleted_resources.is_empty(),
                "Delete this application's Kitty resource"
            );
            assert!(!self.contains_bytes(b"d=A"));
            assert!(
                self.frames
                    .transmitted_resources
                    .is_subset(&self.frames.deleted_resources),
                "Delete every transmitted exact and temporary image resource"
            );
        }
        Ok(())
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
        let _ = fs::write(self.root.join("terminal-output.bin"), &self.output);
        let _ = fs::write(self.root.join("terminal-screen.txt"), self.screen.text());
    }
}
