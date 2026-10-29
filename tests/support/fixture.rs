// Independent test targets share these roles but do not all invoke every role.
#![allow(dead_code)]

#[path = "../../xtask/src/test_support/install.rs"]
mod native;

pub fn helper() -> std::path::PathBuf {
    native::helper(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}

pub fn install(path: &std::path::Path, arguments: &[String]) {
    native::install(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        path,
        arguments,
    )
    .unwrap();
}

pub fn process(path: &std::path::Path, record: serde_json::Value) {
    let table = path.with_extension("response.json");
    std::fs::write(
        &table,
        serde_json::to_vec(&serde_json::json!({"*": record})).unwrap(),
    )
    .unwrap();
    install(
        path,
        &["fixture-command".into(), table.to_str().unwrap().into()],
    );
}

/// Disk-backed pipes avoid a blocked child, and the owner always kills/reaps it
/// after a failed assertion or deadline. Files are outside project fixtures.
pub fn output(
    command: &mut std::process::Command,
    timeout: std::time::Duration,
) -> std::process::Output {
    struct Owner(std::process::Child);
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let stdout = tempfile::NamedTempFile::new().unwrap();
    let stderr = tempfile::NamedTempFile::new().unwrap();
    let mut child = Owner(
        command
            .stdin(std::process::Stdio::null())
            .stdout(stdout.reopen().unwrap())
            .stderr(stderr.reopen().unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Fixture command exceeded its deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    std::process::Output {
        status,
        stdout: std::fs::read(stdout.path()).unwrap(),
        stderr: std::fs::read(stderr.path()).unwrap(),
    }
}
