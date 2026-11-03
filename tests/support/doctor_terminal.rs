//! PTY observations: only protocol replies are supplied, never user confirmation.
#[path = "../../xtask/src/test_support/terminal.rs"]
#[allow(dead_code)]
mod observer;
use super::*;
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Stdio},
};

struct Pty {
    child: Child,
    master: fs::File,
    slave: fs::File,
    original: libc::termios,
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run(protocol: &str, receipt: &str, json: bool) -> (Vec<u8>, observer::KittyFrames) {
    let root = TempDir::new().unwrap();
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 800,
        ws_ypixel: 480,
    };
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        },
        0
    );
    let master = unsafe { fs::File::from_raw_fd(master) };
    let slave = unsafe { fs::File::from_raw_fd(slave) };
    let mut original = std::mem::MaybeUninit::uninit();
    assert_eq!(
        unsafe { libc::tcgetattr(slave.as_raw_fd(), original.as_mut_ptr()) },
        0
    );
    let original = unsafe { original.assume_init() };
    let mut command = command(root.path());
    if json {
        command.arg("--json");
    }
    command.args(["doctor", "--agent", "codex"]);
    if !protocol.starts_with("auto-") {
        command.args(["--image-protocol", protocol]);
    }
    command
        .env(
            "TERM",
            if matches!(protocol, "kitty" | "auto-direct-kitty") {
                "xterm-kitty"
            } else {
                "xterm-256color"
            },
        )
        .env_remove("WEZTERM_EXECUTABLE")
        .env_remove("KONSOLE_VERSION")
        .env_remove("TMUX")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    let mut pty = Pty {
        child,
        master,
        slave,
        original,
    };
    let mut output = Vec::new();
    let mut frames = observer::KittyFrames::for_lazypencil();
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut query_answered = false;
    let mut parsed = 0;
    let mut upload = None;
    let mut exit = None;
    loop {
        assert!(
            Instant::now() < deadline,
            "Doctor waited for user input or failed to finish: {}",
            String::from_utf8_lossy(&output[output.len().saturating_sub(4000)..])
        );
        let mut poll = libc::pollfd {
            fd: pty.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, 20) };
        let mut received = false;
        if ready > 0 {
            let mut buffer = [0u8; 65536];
            match pty.master.read(&mut buffer) {
                Ok(count) if count > 0 => {
                    received = true;
                    output.extend_from_slice(&buffer[..count]);
                    frames.feed(&buffer[..count]).unwrap();
                }
                Ok(_) => {}
                Err(error) if error.raw_os_error() == Some(libc::EIO) => {}
                Err(error) => panic!("{error}"),
            }
            if !query_answered && output.windows(4).any(|window| window == b"\x1b[5n") {
                if protocol == "auto-kitty" {
                    pty.master
                        .write_all(b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[?1;2c")
                        .unwrap();
                } else if protocol == "auto-sixel" {
                    pty.master.write_all(b"\x1b[?1;2;4c").unwrap();
                }
                pty.master
                    .write_all(b"\x1b]11;rgb:0000/0000/0000\x07\x1b[0n")
                    .unwrap();
                query_answered = true;
            }
            while let Some(start) = output[parsed..]
                .windows(3)
                .position(|window| window == b"\x1b_G")
            {
                let start = parsed + start + 3;
                let Some(end) = output[start..]
                    .windows(2)
                    .position(|window| window == b"\x1b\\")
                else {
                    break;
                };
                let end = start + end;
                parsed = end + 2;
                let packet = String::from_utf8_lossy(&output[start..end]);
                let header = packet.split(';').next().unwrap();
                let field = |key: &str| {
                    header
                        .split(',')
                        .find_map(|field| field.strip_prefix(key))
                        .map(str::to_owned)
                };
                if field("a=").as_deref() == Some("t") && field("q=").as_deref() == Some("0") {
                    upload = field("i=");
                }
                if field("m=").as_deref() == Some("0")
                    && let Some(id) = upload.take()
                    && receipt != "silent"
                {
                    let reply = if receipt == "reject" {
                        "EINVAL:test rejection"
                    } else {
                        "OK"
                    };
                    write!(pty.master, "\x1b_Gi={id};{reply}\x1b\\").unwrap();
                }
                if field("a=").as_deref() == Some("p")
                    && field("q=").as_deref() == Some("0")
                    && receipt != "silent"
                {
                    let id = field("i=").unwrap();
                    let reply = if receipt == "reject" {
                        "ENOENT:test rejection"
                    } else {
                        "OK"
                    };
                    write!(pty.master, "\x1b_Gi={id},p=1;{reply}\x1b\\").unwrap();
                }
            }
        }
        if exit.is_none() {
            exit = pty.child.try_wait().unwrap();
        }
        if exit.is_some() && !received {
            break;
        }
    }
    assert!(
        exit.unwrap().success(),
        "{}",
        String::from_utf8_lossy(&output)
    );
    let mut restored = std::mem::MaybeUninit::uninit();
    assert_eq!(
        unsafe { libc::tcgetattr(pty.slave.as_raw_fd(), restored.as_mut_ptr()) },
        0
    );
    let restored = unsafe { restored.assume_init() };
    let flags = |value: &libc::termios| {
        #[cfg(target_os = "macos")]
        {
            value.c_lflag & !libc::PENDIN
        }
        #[cfg(not(target_os = "macos"))]
        {
            value.c_lflag
        }
    };
    assert_eq!(
        (
            restored.c_iflag,
            restored.c_oflag,
            restored.c_cflag,
            flags(&restored),
            restored.c_cc
        ),
        (
            pty.original.c_iflag,
            pty.original.c_oflag,
            pty.original.c_cflag,
            flags(&pty.original),
            pty.original.c_cc
        ),
        "Terminal modes were not restored"
    );
    (output, frames)
}

#[test]
fn doctor_preview_automatically_verifies_renderer_upload_and_placement_without_confirmation() {
    for protocol in ["kitty", "auto-direct-kitty", "auto-kitty"] {
        let (output, frames) = run(protocol, "ok", false);
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("[OK] preview.renderer"), "{text}");
        assert!(text.contains("[OK] preview.terminal"), "{text}");
        assert!(text.contains("physical visibility is not verified"));
        assert!(!frames.images.is_empty());
        for color in [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
        ] {
            assert!(
                frames.images.iter().any(|frame| frame.has_color(color)),
                "Missing test-image color {color:?}"
            );
        }
    }
}

#[test]
fn doctor_preview_reports_rejection_and_silent_terminals_without_claiming_visibility() {
    for (receipt, expected) in [
        ("reject", "[FAIL] preview.terminal"),
        ("silent", "[WARN] preview.terminal"),
    ] {
        let (output, _) = run("kitty", receipt, false);
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains("[OK] preview.terminal"));
    }
}

#[test]
fn doctor_preview_checks_sixel_and_halfblocks_without_waiting_for_a_user_or_a_nonexistent_receipt()
{
    for protocol in ["sixel", "auto-sixel", "halfblocks"] {
        let (output, _) = run(protocol, "silent", false);
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("[WARN] preview.terminal"), "{text}");
        if protocol.contains("sixel") {
            let start = output
                .windows(2)
                .position(|bytes| bytes == b"\x1bP")
                .expect("Sixel DCS");
            let rest = &output[start + 2..];
            let header = rest
                .iter()
                .position(|byte| !byte.is_ascii_digit() && *byte != b';')
                .unwrap();
            assert_eq!(rest[header], b'q');
            let body = &rest[header + 1..];
            let end = body
                .windows(2)
                .position(|bytes| bytes == b"\x1b\\")
                .unwrap();
            let (red, total) = observer::sixel_red_pixels(&body[..end]).unwrap();
            assert!(
                red > 0 && red < total,
                "The Sixel test image must contain a red quadrant and other colors"
            );
        } else {
            assert!(text.contains('▀') || text.contains('▄'));
        }
    }
}

#[test]
fn doctor_json_in_a_terminal_is_plain_json_and_never_probes_or_writes_graphics() {
    let (output, frames) = run("kitty", "silent", true);
    assert!(!output.contains(&0x1b));
    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(check(&value, "preview.renderer")["status"], "ok");
    assert_eq!(check(&value, "preview.terminal")["status"], "not_checked");
    assert!(frames.images.is_empty());
}
