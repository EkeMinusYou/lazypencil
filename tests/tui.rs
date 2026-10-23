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
        "script-config",
    ] {
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
    use lazypencil::clipboard;
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    use tokio_util::sync::CancellationToken;
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("input-worker");
    std::fs::write(&executable, "#!/usr/bin/env python3\nimport json, os, sys, time\nchild = os.fork()\nif child == 0:\n time.sleep(60)\nelse:\n path = sys.argv[-1]\n with open(path + '.tmp', 'w') as handle: json.dump([os.getpid(), child], handle)\n os.replace(path + '.tmp', path)\n time.sleep(60)\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = temp.path().join("pids.json");
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let pid_path = path.clone();
    let cancelling = std::thread::spawn(move || {
        for _ in 0..200 {
            if pid_path.exists() {
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
    assert_eq!(error.code, "CANCELLED");
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
