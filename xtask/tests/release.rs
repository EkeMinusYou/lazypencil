#![cfg(unix)]

use lazypencil_xtask::util;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn command(script: &str, temporary: &Path) -> Command {
    let source = temporary.join("source");
    fs::create_dir_all(source.join("resources")).unwrap();
    fs::write(
        source.join("resources/release.json"),
        r#"{"version":"0.1.0","channel":"stable","target":"aarch64-apple-darwin"}"#,
    )
    .unwrap();
    let mut command = Command::new("bash");
    command
        .arg(util::root().join("scripts/release").join(script))
        .current_dir(source)
        .env("RUNNER_TEMP", temporary)
        .env("GITHUB_ACTIONS", "true")
        .env("RUNNER_OS", "macOS")
        .env("RUNNER_ARCH", "ARM64")
        .env("RELEASE_PLATFORM", "macos-15")
        .env("GITHUB_REF", "refs/tags/v0.1.0")
        .env("DEVELOPER_ID_P12_BASE64", "fixture")
        .env("DEVELOPER_ID_P12_PASSWORD", "fixture")
        .env("APPLE_TEAM_ID", "ABCDEFGHIJ")
        .env("NOTARY_PRIVATE_KEY", "fixture")
        .env("NOTARY_KEY_ID", "fixture")
        .env("NOTARY_ISSUER_ID", "fixture")
        .env("GH_TOKEN", "fixture");
    command
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn publication_requires_a_matching_tag_supported_runner_and_complete_credentials() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        command("macos.sh", temp.path())
            .arg("preflight")
            .status()
            .unwrap()
            .success()
    );
    for (name, value) in [
        ("GITHUB_REF", "refs/heads/main"),
        ("GITHUB_REF", "refs/tags/v9.9.9"),
        ("GITHUB_ACTIONS", "false"),
        ("RUNNER_OS", "Linux"),
        ("RUNNER_ARCH", "X64"),
        ("RELEASE_PLATFORM", "ubuntu-24.04"),
        ("DEVELOPER_ID_P12_BASE64", ""),
        ("DEVELOPER_ID_P12_PASSWORD", ""),
        ("NOTARY_PRIVATE_KEY", ""),
        ("NOTARY_KEY_ID", ""),
        ("NOTARY_ISSUER_ID", ""),
        ("APPLE_TEAM_ID", "wrong team"),
    ] {
        let result = command("macos.sh", temp.path())
            .arg("preflight")
            .env(name, value)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{name}={value}");
    }
}

#[test]
fn publication_reuses_only_successful_main_push_ci_for_the_exact_commit() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    executable(&bin.join("gh"), "#!/bin/bash\ncat \"$TEST_CI_RUNS\"\n");
    let fixture = temp.path().join("runs.json");
    let revision = "a".repeat(40);
    let valid = json!({"head_sha":revision,"head_branch":"main","event":"push","status":"completed","conclusion":"success"});
    let run = || {
        command("macos.sh", temp.path())
            .arg("check-ci")
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("GITHUB_REPOSITORY", "owner/repo")
            .env("GITHUB_SHA", &revision)
            .env("TEST_CI_RUNS", &fixture)
            .output()
            .unwrap()
    };
    fs::write(
        &fixture,
        serde_json::to_vec(&json!({"workflow_runs":[valid]})).unwrap(),
    )
    .unwrap();
    assert!(run().status.success());
    for (key, value) in [
        ("head_sha", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ("head_branch", "feature"),
        ("event", "pull_request"),
        ("status", "in_progress"),
        ("conclusion", "failure"),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = json!(value);
        fs::write(
            &fixture,
            serde_json::to_vec(&json!({"workflow_runs":[invalid]})).unwrap(),
        )
        .unwrap();
        assert!(!run().status.success(), "{key}");
    }
    fs::write(&fixture, br#"{"workflow_runs":[]}"#).unwrap();
    assert!(!run().status.success());
}

#[test]
fn tap_updates_are_idempotent_and_reject_downgrades_or_same_version_replacements() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let assets = temp.path().join("release-assets");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&assets).unwrap();
    let cask = "cask \"lazypencil\" do\n  version \"0.1.0\"\nend\n";
    fs::write(assets.join("lazypencil.rb"), cask).unwrap();
    executable(
        &bin.join("gh"),
        r#"#!/bin/bash
set -eu
case "$2" in
  */git/trees/*) if [[ -s $TEST_CURRENT_CASK ]]; then printf 'previous-blob-sha'; fi ;;
  */git/blobs/*) base64 < "$TEST_CURRENT_CASK" ;;
  --method) [[ $3 == PUT ]]; cp "$6" "$TEST_UPDATE_REQUEST" ;;
  *) exit 2 ;;
esac
"#,
    );
    let previous = temp.path().join("current.rb");
    let request = temp.path().join("request.json");
    let run = || {
        command("publish.sh", temp.path())
            .arg("update-tap")
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("TEST_CURRENT_CASK", &previous)
            .env("TEST_UPDATE_REQUEST", &request)
            .output()
            .unwrap()
    };
    let initial = run();
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let value: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    assert_eq!(value["message"], "Update lazypencil to 0.1.0");
    assert_eq!(value["branch"], "main");
    assert!(value.get("sha").is_none());
    fs::remove_file(&request).unwrap();
    for (body, success) in [
        (cask.to_owned(), true),
        (cask.replace("0.1.0", "0.2.0"), false),
        (cask.replace("end", "# different\nend"), false),
        ("invalid Cask".to_owned(), false),
    ] {
        fs::write(&previous, body).unwrap();
        assert_eq!(run().status.success(), success);
        assert!(!request.exists());
    }
    fs::write(&previous, cask.replace("0.1.0", "0.0.9")).unwrap();
    assert!(run().status.success());
    let value: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    assert_eq!(value["sha"], "previous-blob-sha");
}

#[cfg(target_os = "macos")]
#[test]
fn notarization_requires_accepted_success_and_cleans_up_private_key_material() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    executable(
        &bin.join("xcrun"),
        r#"#!/bin/bash
set -eu
case "$2" in
  submit)
    printf '{"id":"fixture-id","status":"%s"}\n' "$TEST_NOTARY_STATUS"
    exit "$TEST_NOTARY_EXIT" ;;
  log) printf '{}' > "${!#}" ;;
  *) exit 2 ;;
esac
"#,
    );
    for (status, exit, success) in [
        ("Accepted", "0", true),
        ("Invalid", "0", false),
        ("Accepted", "1", false),
    ] {
        let receipt = temp.path().join("receipt.json");
        let result = command("macos.sh", temp.path())
            .args(["notarize", "/usr/bin/true"])
            .arg(&receipt)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("TEST_NOTARY_STATUS", status)
            .env("TEST_NOTARY_EXIT", exit)
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(receipt.exists(), success);
        if success {
            let value: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
            assert_eq!(
                value["binary_sha256"],
                util::file_digest(Path::new("/usr/bin/true")).unwrap()
            );
            fs::remove_file(&receipt).unwrap();
        }
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("lazypencil-notary.")
        }));
    }
}

#[test]
fn signed_release_order_preserves_byte_bound_evidence_and_publication_dependencies() {
    let workflow =
        fs::read_to_string(util::root().join(".github/workflows/release-evidence.yml")).unwrap();
    let stages = [
        "macos.sh preflight",
        "ci native evidence-build",
        "macos.sh sign",
        "xtask/target/debug/xtask evidence",
        "ci native candidate",
        "macos.sh notarize",
        "publish.sh stage",
        "Attest the final release archive",
        "publish.sh finalize",
        "publish.sh publish",
        "publish.sh update-tap",
    ];
    let positions = stages.map(|stage| workflow.find(stage).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(workflow.contains("!inputs.publish_release"));
    assert!(workflow.contains("needs: evidence"));
    assert!(workflow.contains("needs: publish"));
}
