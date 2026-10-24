use lazypencil::runtime;
use std::{fs, process::Command};
use tempfile::TempDir;

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
        "fake-browser.cmd"
    } else {
        "fake-browser"
    });
    fs::write(
        &browser,
        if cfg!(windows) {
            "@echo off\r\necho launched>\"%~dp0launched\"\r\nexit /b 91\r\n"
        } else {
            "#!/bin/sh\n: > \"${0%/*}/launched\"\nexit 91\n"
        },
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&browser, fs::Permissions::from_mode(0o700)).unwrap();
    }
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
            assert_eq!(value["release_ready"], false);
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
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    let temp = TempDir::new().unwrap();
    let executable = temp.path().join("exiting-browser");
    fs::write(&executable, "#!/bin/sh\necho private-value >&2\nexit 91\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
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
