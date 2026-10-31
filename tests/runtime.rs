use lazypencil::runtime;
use std::{fs, process::Command};
use tempfile::TempDir;
#[path = "support/fixture.rs"]
mod fixture_support;
#[cfg(unix)]
#[path = "support/lock_wait.rs"]
mod lock_wait;

#[test]
fn a_cancelled_runtime_setup_does_not_create_local_state() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("absent");
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        runtime::setup_with_cancellation(&data, None, Some(&cancel), None)
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert!(!data.exists());
}

#[test]
fn blocked_runtime_download_is_cancelled_reaped_and_never_activated() {
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };
    let temp = TempDir::new().unwrap();
    let executable = temp
        .path()
        .join(format!("download-fixture{}", std::env::consts::EXE_SUFFIX));
    let started = temp.path().join("download.pid");
    fixture_support::process(
        &executable,
        serde_json::json!({"pid_file": started, "stdout":"partial archive", "sleep_ms":60000}),
    );
    let data = temp.path().join("data");
    let cancel = tokio_util::sync::CancellationToken::new();
    let observed = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !started.is_file() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            observed.store(started.is_file(), Ordering::SeqCst);
            cancel.cancel();
        });
        runtime::setup_with_cancellation(&data, None, Some(&cancel), Some(&executable))
    });
    assert!(
        observed.load(Ordering::SeqCst),
        "The actual owned download process must have started"
    );
    assert_eq!(result.unwrap_err().code, "CANCELLED");
    #[cfg(unix)]
    {
        let pid = fs::read_to_string(&started)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "The owned download process was not reaped"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../resources/runtimes/chromium.json")).unwrap();
    let parent = data
        .join("runtimes/chromium")
        .join(manifest["version"].as_str().unwrap());
    assert!(!parent.join(runtime::platform().unwrap()).exists());
    assert_eq!(
        fs::read_dir(parent).unwrap().count(),
        1,
        "Only the installer lock may remain"
    );
}

#[cfg(unix)]
#[test]
fn ctrl_c_cancels_runtime_lock_wait_without_releasing_another_installer() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../resources/runtimes/chromium.json")).unwrap();
    let parent = data
        .join("runtimes/chromium")
        .join(manifest["version"].as_str().unwrap());
    fs::create_dir_all(&parent).unwrap();
    let path = parent.join(format!(".{}.lock", runtime::platform().unwrap()));
    let holder = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    fs2::FileExt::lock_exclusive(&holder).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .args(["--json", "--data-dir"])
        .arg(&data)
        .args(["runtime", "setup", "chromium", "--archive"])
        .arg(temp.path().join("unread.zip"))
        .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("configuration"));
    let waiter = lock_wait::WaitingProcess::spawn(&mut command);
    assert_eq!(unsafe { libc::kill(waiter.id() as i32, libc::SIGINT) }, 0);
    let output = waiter.wait_with_output();
    assert_eq!(output.status.code(), Some(130));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "CANCELLED");
    let probe = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    assert!(fs2::FileExt::try_lock_exclusive(&probe).is_err());
    assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn profile_commands_require_explicit_creation_and_never_launch_a_browser() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(&data)
            .args(["runtime", "profile"])
            .args(args)
            .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("config"))
            .output()
            .unwrap()
    };
    assert!(!invoke(&["path", "work"]).status.success());
    assert!(!data.exists());
    for _ in 0..2 {
        let output = invoke(&["create", "work"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["profile"], "work");
    }
    let output = invoke(&["path", "work"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        std::path::Path::new(value["directory"].as_str().unwrap()),
        fs::canonicalize(data.join("browser-profiles/work")).unwrap()
    );
    assert_eq!(
        fs::read_dir(data.join("browser-profiles/work"))
            .unwrap()
            .count(),
        1
    );
    for args in [
        vec!["create", "../outside"],
        vec![
            "login",
            "work",
            "--browser",
            "unused",
            "--url",
            "file:///secret",
        ],
        vec![
            "login",
            "work",
            "--browser",
            "unused",
            "--url",
            "https://example.invalid",
            "--timeout-seconds",
            "0",
        ],
    ] {
        assert!(!invoke(&args).status.success());
    }
    let browser = temp.path().join("login-browser");
    let acquisition = temp.path().join("acquisition-browser");
    for path in [&browser, &acquisition] {
        fixture_support::install(
            path,
            &[
                "mark".into(),
                temp.path().join("must-not-launch").to_str().unwrap().into(),
            ],
        );
    }
    let before = fs::read(data.join("browser-profiles/work/lazypencil-profile.json")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(&data)
        .args(["runtime", "profile", "login", "work", "--browser"])
        .arg(&browser)
        .args(["--url", "https://example.invalid", "--timeout-seconds", "1"])
        .env("LAZYPENCIL_CHROMIUM", &acquisition)
        .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("config"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        output["error"]["details"]["stage"],
        "profile_browser_identity"
    );
    assert!(!temp.path().join("must-not-launch").exists());
    assert_eq!(
        fs::read(data.join("browser-profiles/work/lazypencil-profile.json")).unwrap(),
        before
    );
}

#[cfg(unix)]
#[test]
fn local_runtime_archives_reject_nonregular_inputs_without_blocking_or_leaving_staging_files() {
    use std::{os::unix::fs::symlink, process::Stdio, time::Duration};
    let temp = TempDir::new().unwrap();
    let fifo = temp.path().join("archive.zip");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let alias = temp.path().join("archive-alias.zip");
    symlink(&fifo, &alias).unwrap();
    for (index, archive) in [fifo.as_path(), alias.as_path(), temp.path()]
        .into_iter()
        .enumerate()
    {
        let data = temp.path().join(format!("data-{index}"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(&data)
            .args(["runtime", "setup", "chromium", "--archive"])
            .arg(archive)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Runtime archive input blocked: {}", archive.display());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["code"], "RUNTIME_INTEGRITY");
        let version: serde_json::Value =
            serde_json::from_str(include_str!("../resources/runtimes/chromium.json")).unwrap();
        let parent = data
            .join("runtimes/chromium")
            .join(version["version"].as_str().unwrap());
        let remaining = fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(
            remaining.len(),
            1,
            "Partial runtime files survived rejection"
        );
        assert_eq!(
            remaining[0],
            std::ffi::OsString::from(format!(".{}.lock", runtime::platform().unwrap()))
        );
    }
}

#[test]
fn unknown_or_corrupted_archives_never_become_installed_runtimes() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("wrong.zip");
    fs::write(&archive, b"not a pinned archive").unwrap();
    let data = temp.path().join("data");
    let error = runtime::setup(&data, Some(&archive)).unwrap_err();
    assert_eq!(error.code, "RUNTIME_INTEGRITY");
    assert_eq!(runtime::status(&data)["available"], false);
    assert!(
        !data
            .join("runtimes/chromium/156.0.8078.47")
            .join(runtime::platform().unwrap())
            .exists()
    );
}

#[test]
fn runtime_status_does_not_create_a_missing_data_directory_or_download_files() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("not-created");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .args(["--json", "--data-dir"])
        .arg(&data)
        .args(["runtime", "status"])
        .env_remove("LAZYPENCIL_CHROMIUM")
        .output()
        .unwrap();
    assert!(output.status.success());
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["available"], false);
    assert_eq!(status["startup"], "not_checked");
    assert_eq!(status["host"]["os"], std::env::consts::OS);
    assert_eq!(status["host"]["sandbox_profile"], "not_checked");
    assert!(!data.exists());
}

#[test]
fn doctor_and_runtime_status_do_not_launch_an_explicit_browser_or_create_local_state() {
    let temp = TempDir::new().unwrap();
    let browser = temp.path().join(if cfg!(windows) {
        "fake-browser.exe"
    } else {
        "fake-browser"
    });
    fixture_support::process(
        &browser,
        serde_json::json!({"marker_file":temp.path().join("launched"),"status":91}),
    );
    let data = temp.path().join("absent-data");
    let settings = temp.path().join("absent-settings");
    let locks = temp.path().join("absent-locks");
    for arguments in [vec!["runtime", "status"], vec!["doctor"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .args(["--json", "--data-dir"])
            .arg(&data)
            .args(&arguments)
            .env("LAZYPENCIL_CHROMIUM", &browser)
            .env("LAZYPENCIL_CONFIG_DIR", &settings)
            .env("LAZYPENCIL_LOCK_DIR", &locks)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let mut value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if arguments[0] == "doctor" {
            assert_eq!(
                value["release_ready"],
                cfg!(all(
                    target_os = "macos",
                    target_arch = "aarch64",
                    feature = "rendering",
                    feature = "tui",
                    not(debug_assertions)
                ))
            );
            let release: serde_json::Value =
                serde_json::from_str(include_str!("../resources/release.json")).unwrap();
            assert_eq!(value["version"], release["version"]);
            assert_eq!(value["release"], release);
            value = value["chromium"].take();
        }
        assert_eq!(value["available"], true);
        assert_eq!(value["managed"], false);
        assert_eq!(value["integrity_verified"], false);
        assert_eq!(value["startup"], "not_checked");
        assert_eq!(value["host"]["browser_sandbox"], "required");
        assert_eq!(value["host"]["shared_libraries"], "not_checked");
        assert_eq!(value["host"]["sandbox_profile"], "not_checked");
        #[cfg(target_os = "linux")]
        {
            assert_eq!(value["host"]["linux"]["effective_uid"], unsafe {
                libc::geteuid()
            });
            assert!(
                value["host"]["linux"]
                    .get("apparmor_restrict_unprivileged_userns")
                    .is_some()
            );
        }
        assert!(!temp.path().join("launched").exists());
        assert!(!data.exists());
        assert!(!settings.exists());
        assert!(!locks.exists());
    }
}

#[test]
#[ignore = "Requires the pinned archive supplied through LAZYPENCIL_TEST_CHROMIUM_ARCHIVE"]
fn real_pinned_installation_is_reused_and_modification_is_detected() {
    let temp = TempDir::new().unwrap();
    let archive =
        std::env::var_os("LAZYPENCIL_TEST_CHROMIUM_ARCHIVE").expect("Set the pinned archive path");
    let result = runtime::setup(temp.path(), Some(std::path::Path::new(&archive))).unwrap();
    assert_eq!(result["reused"], false);
    assert_eq!(
        runtime::setup(temp.path(), Some(std::path::Path::new(&archive))).unwrap()["reused"],
        true
    );
    let executable = result["executable"].as_str().unwrap();
    fs::write(executable, b"modified runtime").unwrap();
    assert_eq!(
        runtime::installed(temp.path()).err().unwrap().code,
        "RUNTIME_INTEGRITY"
    );
    assert_eq!(
        runtime::setup(temp.path(), Some(std::path::Path::new(&archive)))
            .unwrap_err()
            .code,
        "RUNTIME_INTEGRITY"
    );
    assert_eq!(fs::read(executable).unwrap(), b"modified runtime");
}

#[cfg(unix)]
#[test]
fn startup_failures_report_the_stage_and_host_facts_without_raw_child_output() {
    use lazypencil::cdp::{Browser, Budget, Purpose};
    use std::time::Duration;
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("exiting-browser");
    fixture_support::process(
        &executable,
        serde_json::json!({"stderr":"private-value\n","status":91}),
    );
    let installed = runtime::InstalledBrowser {
        executable,
        version: "fixture".into(),
        platform: runtime::platform().unwrap().into(),
    };
    let error = Browser::launch_with_supervisor(
        &installed,
        Purpose::Page,
        &Budget::new(Duration::from_secs(5), None),
        std::path::Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .err()
    .expect("The fixture exits before CDP startup");
    assert_eq!(error.code, "RUNTIME_UNAVAILABLE");
    assert!(error.message.contains("lazypencil --json doctor"));
    let encoded = serde_json::to_value(error).unwrap();
    assert_eq!(encoded["details"]["stage"], "browser_startup");
    assert_eq!(encoded["details"]["host"]["os"], std::env::consts::OS);
    assert_eq!(encoded["details"]["host"]["sandbox_profile"], "not_checked");
    assert!(!encoded.to_string().contains("private-value"));
}
