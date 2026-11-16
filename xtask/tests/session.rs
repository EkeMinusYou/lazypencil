#![cfg(unix)]

use lazypencil_xtask::process::Session;
use serde_json::json;
use std::{
    fs,
    os::fd::FromRawFd,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn fixture(table: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil-test-helper"));
    command
        .arg("fixture-command")
        .arg(table)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn ready(path: &std::path::Path) {
    let end = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < end, "Session fixture did not start");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn owned_sessions_reap_both_term_responsive_and_term_ignoring_children() {
    for pty in [false, true] {
        for ignore_term in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let marker = root.path().join("ready");
            let table = root.path().join("response.json");
            fs::write(
                &table,
                json!({"*":{"marker_file":marker,"ignore_term":ignore_term,"sleep_ms":30000}})
                    .to_string(),
            )
            .unwrap();
            let mut command = fixture(&table);
            let _master = if pty {
                let (mut master, mut slave) = (-1, -1);
                assert_eq!(
                    unsafe {
                        libc::openpty(
                            &mut master,
                            &mut slave,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                let master = unsafe { fs::File::from_raw_fd(master) };
                command.stdin(unsafe { fs::File::from_raw_fd(slave) });
                Some(master)
            } else {
                None
            };
            let child = Session::spawn(&mut command, pty.then_some(0)).unwrap();
            ready(&marker);
            let pid = child.id() as i32;
            assert_eq!(unsafe { libc::getsid(pid) }, pid);
            assert_eq!(unsafe { libc::getpgid(pid) }, pid);
            assert_ne!(pid, unsafe { libc::getpgrp() });
            let started = Instant::now();
            drop(child);
            assert!(started.elapsed() < Duration::from_secs(8));
            if ignore_term {
                assert!(started.elapsed() >= Duration::from_secs(2));
            }
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
    }
}

#[test]
fn an_already_reaped_session_retains_its_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let table = root.path().join("response.json");
    fs::write(&table, json!({"*":{"status":7}}).to_string()).unwrap();
    let mut child = Session::spawn(&mut fixture(&table), None).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(7));
    assert_eq!(child.try_wait().unwrap(), Some(status));
    drop(child);
}

#[test]
fn a_controlling_terminal_setup_failure_does_not_run_the_command() {
    let root = tempfile::tempdir().unwrap();
    let table = root.path().join("response.json");
    let marker = root.path().join("ready");
    fs::write(&table, json!({"*":{"marker_file":marker}}).to_string()).unwrap();
    assert!(Session::spawn(&mut fixture(&table), Some(0)).is_err());
    assert!(!marker.exists());
}
