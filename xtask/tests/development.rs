use lazypencil_xtask::process;
use std::{fs, path::Path, process::Command, time::Duration};

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/bin")).unwrap();
    fs::create_dir(dir.path().join("tests")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='lazypencil'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        r#"
#[test] fn selected_behavior() {
    let config = std::path::PathBuf::from(std::env::var_os("LAZYPENCIL_CONFIG_DIR").unwrap());
    assert!(!config.join("config.toml").exists());
    assert!(std::path::Path::new(&std::env::var_os("LAZYPENCIL_TEST_HELPER").unwrap()).is_file());
}
#[test] fn real_failure() { panic!("behavior regression"); }
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("tests/chosen.rs"),
        "#[test] fn selected_behavior() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("tests/untouched.rs"),
        "compile_error!(\"unrelated target must not compile\");\n",
    )
    .unwrap();
    process::checked(
        Command::new("cargo")
            .current_dir(dir.path())
            .args(["generate-lockfile", "--offline"]),
        Duration::from_secs(60),
    )
    .unwrap();
    dir
}
fn check(root: &Path, args: &[&str]) -> process::Output {
    process::output(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(root)
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("LAZYPENCIL_TEST_HELPER", std::env::current_exe().unwrap())
            .arg("test")
            .args(args),
        Duration::from_secs(60),
    )
    .unwrap()
}

#[test]
fn focused_checks_execute_only_selected_targets_and_reject_zero_tests_and_regressions() {
    let dir = fixture();
    let result = check(
        dir.path(),
        &["--lib", "--test", "chosen", "--filter", "selected_behavior"],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !check(dir.path(), &["--lib", "--filter", "misspelled_test"])
            .status
            .success()
    );
    assert!(
        !check(dir.path(), &["--lib", "--filter", "real_failure"])
            .status
            .success()
    );
    assert!(
        !check(dir.path(), &["--filter", "selected_behavior"])
            .status
            .success()
    );
    assert!(
        !check(dir.path(), &["--test", "--all-targets"])
            .status
            .success()
    );
    assert!(
        !check(dir.path(), &["--lib", "--filter=--release"])
            .status
            .success()
    );
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("helper"), "fixture").unwrap();
    let result = process::output(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(&nested)
            .env("CARGO_TARGET_DIR", dir.path().join("target"))
            .env("LAZYPENCIL_TEST_HELPER", "helper")
            .args(["test", "--lib", "--filter", "selected_behavior"]),
        Duration::from_secs(60),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn watcher_pins_the_run_identity_and_propagates_failure_without_redispatch() {
    let dir = fixture();
    fs::write(
        dir.path().join("src/bin/gh.rs"),
        r#"
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["run", "view", "7", "--json", "databaseId,headSha"] {
        println!("{{\"databaseId\":7,\"headSha\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}");
    } else if args == ["run", "watch", "7", "--exit-status", "--interval", "30", "--compact"] {
        std::fs::write("watched", "yes").unwrap();
        std::process::exit(std::env::var("WATCH_RESULT").unwrap().parse().unwrap());
    } else { panic!("Unexpected gh command: {args:?}"); }
}
"#,
    )
    .unwrap();
    process::checked(
        Command::new("cargo")
            .current_dir(dir.path())
            .env("CARGO_TARGET_DIR", dir.path().join("target"))
            .args(["build", "--locked", "--jobs", "2", "--bin", "gh"]),
        Duration::from_secs(60),
    )
    .unwrap();
    let path = std::env::join_paths(
        std::iter::once(dir.path().join("target/debug"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let watch = |sha: &str, status: &str| {
        process::output(
            Command::new(env!("CARGO_BIN_EXE_xtask"))
                .current_dir(dir.path())
                .env("PATH", &path)
                .env("WATCH_RESULT", status)
                .args(["ci", "watch", "--run", "7", "--head", sha]),
            Duration::from_secs(30),
        )
        .unwrap()
    };
    assert!(!watch(&"b".repeat(40), "0").status.success());
    assert!(!dir.path().join("watched").exists());
    assert!(watch(&"a".repeat(40), "0").status.success());
    assert!(dir.path().join("watched").exists());
    assert!(!watch(&"a".repeat(40), "1").status.success());
}
