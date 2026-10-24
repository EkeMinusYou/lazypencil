#![cfg(all(feature = "tui", unix))]

use std::{path::Path, process::Command};
use tempfile::TempDir;

#[test]
fn real_pty_handles_japanese_input_live_turns_resize_and_image_protocols() {
    for case in [
        "basic",
        "kitty",
        "sixel",
        "auto-none",
        "auto-kitty",
        "auto-sixel",
        "restore",
        "restore-conflict",
        "font-config",
        "user-font-config",
        "user-agent-config",
        "user-key-config",
        "script-config",
    ] {
        run_pty_case(case);
    }
}

#[test]
fn real_pty_custom_navigation_preserves_draft_and_changes_preview_pixels() {
    run_pty_case("user-navigation-config");
}

#[test]
fn real_pty_saved_logs_page_format_refresh_and_keep_the_unsent_draft_without_starting_an_agent() {
    run_pty_case("saved-history");
}

fn run_pty_case(case: &str) {
    let temp = TempDir::new().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let result = Command::new(
        std::env::var_os("LAZYPENCIL_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
    )
    .arg(root.join("tests/support/tui_pty.py"))
    .arg(env!("CARGO_BIN_EXE_lazypencil"))
    .arg(temp.path())
    .arg(root.join("tests/support/acp_fixture.py"))
    .arg(case)
    .output()
    .unwrap();
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS")
        && (!result.status.success() || matches!(case, "user-navigation-config" | "saved-history"))
    {
        let directory = std::path::PathBuf::from(directory).join(case);
        std::fs::create_dir_all(&directory).unwrap();
        for name in [
            "terminal-output.bin",
            "terminal-screen.txt",
            "terminal-errors.log",
            "navigation-report.json",
            "saved-history-report.json",
            "saved-history-source.jsonl",
            "saved-history-expected.txt",
            "saved-history-formatted-screen.txt",
            "saved-history-end-screen.txt",
            "saved-history-raw-screen.txt",
            "saved-history-refreshed-screen.txt",
            "saved-history-session.json",
            "saved-history-design.pen",
        ] {
            let source = temp.path().join(name);
            if source.is_file() {
                std::fs::copy(source, directory.join(name)).unwrap();
            }
        }
    }
    if !result.status.success() {
        let artifacts = temp.keep();
        panic!(
            "case={case}, artifacts={}: {}\n{}\n{}",
            artifacts.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
            std::fs::read_to_string(artifacts.join("terminal-errors.log")).unwrap_or_default()
        );
    }
}

#[test]
#[ignore = "Requires Chromium and real idle expiry in a native PTY"]
fn real_pty_dynamic_previews_survive_idle_cleanup_and_refresh_without_losing_drafts() {
    let temp = TempDir::new().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let result = Command::new(
        std::env::var_os("LAZYPENCIL_TEST_PYTHON").unwrap_or_else(|| "python3".into()),
    )
    .arg(root.join("tests/support/tui_pty.py"))
    .arg(env!("CARGO_BIN_EXE_lazypencil"))
    .arg(temp.path())
    .arg(root.join("tests/support/acp_fixture.py"))
    .arg("dynamic-idle")
    .env(
        "LAZYPENCIL_CHROMIUM",
        std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
    )
    .output()
    .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
        std::fs::read_to_string(temp.path().join("terminal-errors.log")).unwrap_or_default(),
        std::fs::read_to_string(temp.path().join("terminal-screen.txt")).unwrap_or_default()
    );
}

#[test]
fn file_input_worker_normalizes_without_clipboard_access_and_rejects_non_images() {
    use lazypencil::clipboard::{self, InputContent};
    use tokio_util::sync::CancellationToken;
    let executable = Path::new(env!("CARGO_BIN_EXE_lazypencil"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = TempDir::new().unwrap();
    let InputContent::Images { images } = clipboard::read_image(
        executable,
        &root.join("examples/assets/crop-source.png"),
        &CancellationToken::new(),
    )
    .unwrap() else {
        panic!("Expected images")
    };
    assert_eq!(images.len(), 1);
    let image = images
        .into_iter()
        .next()
        .unwrap()
        .store(temp.path())
        .unwrap();
    assert_eq!((image.width, image.height), (320, 160));
    let error = clipboard::read_image(
        executable,
        &root.join("Cargo.toml"),
        &CancellationToken::new(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "ATTACHMENT_INVALID");
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        clipboard::read_image(
            executable,
            &root.join("examples/assets/crop-source.png"),
            &cancel
        )
        .err()
        .unwrap()
        .code,
        "CANCELLED"
    );
}

#[test]
fn cancellation_reaps_input_worker_and_descendants_without_blocking_on_stdout() {
    assert_input_worker_cancellation(false);
}

#[test]
fn cancellation_remains_live_after_an_input_worker_exits_with_inherited_stdout() {
    assert_input_worker_cancellation(true);
}

fn assert_input_worker_cancellation(exited: bool) {
    use lazypencil::clipboard;
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    use tokio_util::sync::CancellationToken;
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("input-worker");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
for input_path in "$@"; do :; done
exec "${LAZYPENCIL_TEST_PYTHON:-python3}" -c '
import json, os, sys, time
child = os.fork()
if child == 0:
 time.sleep(60)
else:
 path = sys.argv[-1]
 with open(path + ".tmp", "w") as handle: json.dump([os.getpid(), child], handle)
 os.replace(path + ".tmp", path)
 if path.endswith("exited.json"): sys.exit(0)
 time.sleep(60)
' "$@" 2>"$input_path.stderr"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = temp.path().join(if exited {
        "exited.json"
    } else {
        "running.json"
    });
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let pid_path = path.clone();
    let cancelling = std::thread::spawn(move || {
        for _ in 0..200 {
            if pid_path.exists() {
                if exited {
                    std::thread::sleep(Duration::from_millis(100));
                }
                token.cancel();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        token.cancel();
    });
    let before = Instant::now();
    let error = clipboard::read_image(&executable, &path, &cancel)
        .err()
        .unwrap();
    cancelling.join().unwrap();
    let stderr = std::fs::read_to_string(format!("{}.stderr", path.display())).unwrap_or_default();
    assert!(
        path.exists(),
        "Input fixture did not publish process IDs: {error:?}; stderr: {stderr}"
    );
    assert_eq!(error.code, "CANCELLED", "{error:?}; stderr: {stderr}");
    assert!(before.elapsed() < Duration::from_secs(3));
    let pids: Vec<i32> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for pid in pids {
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "Input process {pid} survived cancellation"
        );
    }
}

#[test]
fn tui_rejects_machine_and_nonterminal_use_before_creating_a_document() {
    let temp = TempDir::new().unwrap();
    for json in [false, true] {
        let file = temp.path().join(if json {
            "machine.pen"
        } else {
            "nonterminal.pen"
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
        if json {
            command.arg("--json");
        }
        let output = command
            .arg("tui")
            .arg(&file)
            .arg("--create")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!file.exists());
        if json {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("requires a terminal"));
        }
    }
}
