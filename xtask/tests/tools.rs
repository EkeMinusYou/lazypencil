use lazypencil_xtask::process;
use std::{
    fs,
    process::Command,
    time::{Duration, UNIX_EPOCH},
};

#[test]
fn hosted_preparation_changes_source_times_without_walking_build_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("Cargo.toml"), "[package]\nname='lazypencil'\n").unwrap();
    fs::create_dir_all(root.join("xtask/src")).unwrap();
    fs::create_dir_all(root.join("xtask/target/debug")).unwrap();
    let source = root.join("xtask/src/main.rs");
    fs::write(&source, "source").unwrap();
    let helper = root.join("xtask/target/debug").join(format!(
        "lazypencil-test-helper{}",
        std::env::consts::EXE_SUFFIX
    ));
    fs::write(&helper, "built helper").unwrap();
    let built_time = fs::metadata(&helper).unwrap().modified().unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.join("not-present"),
        root.join("xtask/target/build-link"),
    )
    .unwrap();
    for input in [
        "vendor/skia-bindings/build_support/binary_cache/source_archive.rs",
        "resources/release.json",
        "build.rs",
        "src/tui/gestures/macos.m",
    ] {
        let path = root.join(input);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "input").unwrap();
    }
    let output = process::output(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(root)
            .env("GITHUB_ACTIONS", "true")
            .env("RUNNER_TEMP", root.join("runner"))
            .env("GITHUB_ENV", root.join("github-env"))
            .args(["ci", "tools", "prepare"]),
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::metadata(source).unwrap().modified().unwrap(),
        UNIX_EPOCH + Duration::from_secs(946684800)
    );
    assert_eq!(
        fs::metadata(helper).unwrap().modified().unwrap(),
        built_time
    );
    assert!(
        fs::read_to_string(root.join("github-env"))
            .unwrap()
            .contains("LAZYPENCIL_TEST_HELPER=")
    );
}
