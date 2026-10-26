use lazypencil_xtask::util;
#[cfg(unix)]
use lazypencil_xtask::{dist, terminal};
#[cfg(unix)]
use serde_json::json;
use std::{fs, process::Command};

#[cfg(unix)]
const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
#[cfg(unix)]
fn development_reproduction_requires_exact_sources_and_a_regular_executable() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    fs::create_dir_all(source.join("src")).unwrap();
    let mut files = Vec::new();
    for (name, bytes) in [
        (
            "Cargo.toml",
            "[package]\nname='lazypencil'\nversion='0.1.0-dev'\n",
        ),
        ("Cargo.lock", "version=4\n"),
        ("LICENSE", "MIT fixture\n"),
        ("src/main.rs", "fn main() {}\n"),
    ] {
        let path = source.join(name);
        fs::write(&path, bytes).unwrap();
        files.push(dist::Entry {
            path: name.into(),
            sha256: util::digest(bytes),
            bytes: bytes.len() as u64,
            mode: util::mode(&path).unwrap(),
        });
    }
    fs::write(
        source.join(dist::MANIFEST),
        util::encode(&dist::Manifest {
            format_version: 1,
            package: json!({"name":"lazypencil","version":"0.1.0-dev"}),
            source_date_epoch: 0,
            git: json!({"revision":REVISION,"dirty":false}),
            files,
        })
        .unwrap(),
    )
    .unwrap();
    let binary = temporary.path().join("lazypencil");
    fs::write(&binary, b"independent executable fixture").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let report = terminal::development_identity(&source, &binary, REVISION).unwrap();
    assert_eq!(report["kind"], "focused-development-reproduction");
    assert_eq!(report["release_ready"], false);
    assert_eq!(report["source"]["files"], 4);
    assert_eq!(report["source_revision"], REVISION);
    assert_eq!(
        report["binary_sha256"],
        util::digest(b"independent executable fixture")
    );
    assert!(terminal::development_identity(&source, &binary, &"b".repeat(40)).is_err());
    fs::write(source.join("src/main.rs"), "fn main() { }\n").unwrap();
    assert!(terminal::development_identity(&source, &binary, REVISION).is_err());
    fs::write(source.join("src/main.rs"), "fn main() {}\n").unwrap();
    let linked = temporary.path().join("linked");
    symlink(&binary, &linked).unwrap();
    assert!(terminal::development_identity(&source, &linked, REVISION).is_err());
    assert!(terminal::development_identity(&source, temporary.path(), REVISION).is_err());
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(terminal::development_identity(&source, &binary, REVISION).is_err());
    fs::write(&binary, []).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(terminal::development_identity(&source, &binary, REVISION).is_err());
}

#[test]
fn terminal_inputs_are_exclusive_and_never_launch_outside_hosted_linux() {
    for (arguments, code) in [
        (vec!["terminal", "--output", "unused"], 2),
        (
            vec![
                "terminal",
                "--candidate",
                "unused",
                "--development-build",
                "--output",
                "unused",
            ],
            2,
        ),
        (
            vec!["terminal", "--candidate", "unused", "--output", "unused"],
            1,
        ),
        (
            vec!["terminal", "--development-build", "--output", "unused"],
            1,
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(arguments)
            .env("GITHUB_ACTIONS", "false")
            .env_remove("RUNNER_OS")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(code));
        if code == 1 {
            assert!(String::from_utf8_lossy(&result.stderr).contains("isolated Linux GHA runner"));
        }
    }
    let workflow = fs::read_to_string(util::root().join(".github/workflows/native.yml")).unwrap();
    let position = |text| workflow.find(text).unwrap();
    assert!(position("native focus-prepare") < position("terminal --development-build"));
    assert!(position("terminal --development-build") < position("native focus-release"));
    assert!(position("native candidate\n") < position("terminal --candidate"));
    assert_eq!(
        workflow.matches("ci linux-terminal-dependencies").count(),
        1
    );
}
