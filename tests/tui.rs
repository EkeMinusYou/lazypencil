#![cfg(all(feature = "tui", unix))]

use std::{path::Path, process::Command};
use tempfile::TempDir;

#[path = "support/fixture.rs"]
mod fixture_support;
#[path = "support/terminal/mod.rs"]
mod terminal;

#[test]
fn real_pty_handles_japanese_input_live_turns_resize_and_image_protocols() {
    for case in [
        "ime-commit",
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
        "project-agent-config",
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

#[test]
fn real_pty_reviewed_deletion_remains_readable_at_minimum_terminal_size() {
    run_pty_case("session-deletion-small");
}

#[test]
fn real_pty_deletes_only_reviewed_inactive_local_conversations_and_preserves_current_state() {
    run_pty_case("session-deletion");
}

#[test]
fn real_pty_selects_saved_conversations_and_keeps_both_drafts_without_starting_an_agent() {
    run_pty_case("session-selection");
}

fn run_pty_case(case: &str) {
    let temp = TempDir::new().unwrap();
    let result = std::panic::catch_unwind(|| {
        terminal::run(
            Path::new(env!("CARGO_BIN_EXE_lazypencil")),
            temp.path(),
            case,
            &[],
        )
    });
    let success = matches!(&result, Ok(Ok(())));
    if let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS")
        && (!success
            || matches!(
                case,
                "ime-commit"
                    | "user-navigation-config"
                    | "saved-history"
                    | "session-selection"
                    | "session-deletion"
                    | "session-deletion-small"
            ))
    {
        let directory = std::path::PathBuf::from(directory).join(case);
        std::fs::create_dir_all(&directory).unwrap();
        for name in [
            "terminal-output.bin",
            "terminal-screen.txt",
            "terminal-errors.log",
            "ime-commit-report.json",
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
            "session-selection-report.json",
            "session-selection-first.json",
            "session-selection-second.json",
            "session-selection-first-screen.txt",
            "session-selection-second-screen.txt",
            "session-deletion-report.json",
            "session-deletion-review-screen.txt",
            "session-deletion-stale-screen.txt",
            "session-deletion-complete-screen.txt",
            "session-deletion-scrolled-screen.txt",
        ] {
            let source = temp.path().join(name);
            if source.is_file() {
                std::fs::copy(source, directory.join(name)).unwrap();
            }
        }
    }
    if !success {
        let artifacts = temp.keep();
        panic!(
            "case={case}, artifacts={}: {result:?}\n{}",
            artifacts.display(),
            std::fs::read_to_string(artifacts.join("terminal-errors.log")).unwrap_or_default()
        );
    }
}

#[test]
#[ignore = "Requires Chromium and real idle expiry in a native PTY"]
fn real_pty_dynamic_previews_survive_idle_cleanup_and_refresh_without_losing_drafts() {
    let temp = TempDir::new().unwrap();
    let result = terminal::run(
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
        temp.path(),
        "dynamic-idle",
        &[(
            "LAZYPENCIL_CHROMIUM".into(),
            std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").expect("Set LAZYPENCIL_TEST_CHROMIUM"),
        )],
    );
    assert!(
        result.is_ok(),
        "{result:?}\n{}\n{}",
        std::fs::read_to_string(temp.path().join("terminal-errors.log")).unwrap_or_default(),
        std::fs::read_to_string(temp.path().join("terminal-screen.txt")).unwrap_or_default()
    );
}

#[test]
#[ignore = "Writes only an explicitly owned isolated GHA clipboard"]
fn real_native_clipboard_ctrl_v_preserves_two_images_and_delivers_their_pixels() -> terminal::Result
{
    use arboard::{Clipboard, ImageData};
    use serde_json::json;
    use std::{borrow::Cow, collections::BTreeMap, fs};
    assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
    assert_eq!(
        std::env::var("LAZYPENCIL_TEST_NATIVE_CLIPBOARD").as_deref(),
        Ok("owned-runner-clipboard")
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        std::env::var("LAZYPENCIL_TEST_CLIPBOARD_DISPLAY").as_deref(),
        Ok("isolated-xvfb")
    );
    let parent = std::path::PathBuf::from(
        std::env::var_os("LAZYPENCIL_TEST_TUI_ARTIFACTS")
            .ok_or("Missing clipboard evidence directory")?,
    );
    assert!(parent.is_absolute());
    fs::create_dir_all(&parent)?;
    let root = parent.join("native-clipboard");
    fs::create_dir(&root)?;
    let document = root.join("design.pen");
    let design = serde_json::to_vec(
        &json!({"version":"2.21","children":[{"id":"red","type":"rectangle","width":180,"height":90,"fill":"#ff0000"}]}),
    )?;
    fs::write(&document, &design)?;
    fs::write(root.join("lazypencil.toml"), b"version=1\n")?;
    let mut environment = std::env::vars_os().collect::<BTreeMap<_, _>>();
    environment.insert(
        "LAZYPENCIL_CONFIG_DIR".into(),
        root.join("user").into_os_string(),
    );
    let binary = Path::new(env!("CARGO_BIN_EXE_lazypencil"));
    let mut h = terminal::Harness::start(binary, &root, &document, environment, "kitty", None)?;
    h.visible("Ctrl+V")?;
    let draft = "ネイティブ画像を二つ貼り付ける";
    h.paste(draft)?;
    h.draft(draft)?;
    let expected = [
        [255, 0, 0, 255].repeat(48 * 32),
        [0, 0, 255, 255].repeat(48 * 32),
    ];
    // Keep the X11 selection owner alive while the real image worker reads it.
    // No preexisting clipboard contents are read, retained or uploaded.
    let mut clipboard = Clipboard::new()?;
    for (index, pixels) in expected.iter().enumerate() {
        clipboard.set_image(ImageData {
            width: 48,
            height: 32,
            bytes: Cow::Borrowed(pixels),
        })?;
        h.key(b"\x16")?;
        h.wait("native Ctrl+V attachment", 15.0, |h| {
            h.session().is_some_and(|session| {
                session["pending_attachments"]
                    .as_array()
                    .is_some_and(|images| images.len() == index + 1)
            })
        })?;
        let session = h.session().ok_or("Missing clipboard conversation")?;
        assert_eq!(session["draft"], draft);
        assert!(session["native_session_id"].is_null());
        assert!(!root.join("sent-turn.txt").exists());
    }
    h.key(b"\x13")?;
    h.wait("native clipboard image delivery", 15.0, |h| {
        h.count("complete") == 1
    })?;
    let received: serde_json::Value = terminal::read_json(&root.join("received-turn-1.json"))
        .ok_or("Missing ACP image receipt")?;
    let images = received["images"]
        .as_array()
        .ok_or("Missing received image list")?;
    let mut matched = Vec::new();
    for name in images {
        let path = root.join(name.as_str().ok_or("Invalid image name")?);
        let pixels = image::load_from_memory(&fs::read(&path)?)?.to_rgba8();
        if pixels.dimensions() == (48, 32) {
            matched.push(pixels.into_raw());
        }
    }
    assert_eq!(matched, expected);
    assert!(received["text"].as_array().unwrap().contains(&json!(draft)));
    assert_eq!(h.session().unwrap()["draft"], "");
    assert_eq!(fs::read(&document)?, design);
    h.finish()?;
    clipboard.clear()?;
    terminal::write_json(
        &root.join("report.json"),
        &json!({"source_commit":std::env::var("GITHUB_SHA")?,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"native_backend":"arboard image producer -> app image worker -> TUI Ctrl+V -> ACP image blocks","images":2,"width":48,"height":32,"pixels_compared":48*32*2,"draft_preserved_until_send":true,"document_unchanged":true,"normal_exit_and_terminal_modes_restored":true,"scope":"Isolated runner clipboard and real PTY key dispatch; no physical keyboard/IME or real provider request"}),
    )?;
    Ok(())
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
    use std::time::{Duration, Instant};
    use tokio_util::sync::CancellationToken;
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("input-worker");
    fixture_support::install(&executable, &["input-worker".into()]);
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
