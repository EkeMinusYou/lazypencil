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
    assert!(!data.exists());
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
