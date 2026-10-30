use lazypencil_xtask::ci::cache::{
    self,
    native_snapshots::{self, Publication},
};
use serde_json::json;

#[test]
fn release_baselines_are_extended_once_without_replacing_native_or_application_identity() {
    let primary = format!("native-v2-macos-15-image-{}", "a".repeat(64));
    let complete = native_snapshots::complete_key(&primary).unwrap();
    let checkpoint = cache::checkpoint_key(&primary, "full", "402", "1").unwrap();
    let core = format!("core-{primary}");
    assert_eq!(
        native_snapshots::preferred_key(&primary, false).unwrap(),
        complete
    );
    assert_eq!(
        native_snapshots::preferred_key(&primary, true).unwrap(),
        core
    );
    for (matched, core_only, full_catalog, release_verified, expected) in [
        // Extending the exact release baseline retains both dependency graphs.
        (
            primary.as_str(),
            false,
            true,
            false,
            Some(complete.as_str()),
        ),
        (primary.as_str(), false, false, true, None),
        (primary.as_str(), true, false, false, Some(core.as_str())),
        // Warm default/release lanes keep the complete graph unchanged.
        // Core may still need its distinct feature graph.
        (complete.as_str(), false, true, false, None),
        (complete.as_str(), false, false, true, None),
        (complete.as_str(), true, false, false, Some(core.as_str())),
        (core.as_str(), true, false, false, None),
        // Cold/default and legacy builds do not claim a current release graph.
        ("", false, true, false, Some(checkpoint.as_str())),
        ("legacy", false, true, false, Some(checkpoint.as_str())),
        ("legacy", false, false, true, Some(primary.as_str())),
        ("", true, false, false, Some(core.as_str())),
        // An early reproduction is not a complete default catalog.
        (primary.as_str(), false, false, false, None),
    ] {
        assert_eq!(
            native_snapshots::publication(Publication {
                primary: &primary,
                matched,
                checkpoint: &checkpoint,
                core_only,
                full_catalog,
                release_verified,
            })
            .unwrap()
            .as_deref(),
            expected,
            "{matched}: core={core_only}, default={full_catalog}, release={release_verified}"
        );
    }
    for bad in ["../native-v2-foreign", "native-v2-image-missing-digest"] {
        assert!(native_snapshots::preferred_key(bad, false).is_err());
    }
    for bad in [
        checkpoint.replace("402-1", "0-1"),
        checkpoint.replace("402-1", "402-01"),
        checkpoint.replace("402-1", "402-18446744073709551616"),
        checkpoint.replace("-full-", "-foreign-"),
        checkpoint.replace(&"a".repeat(64), &"b".repeat(64)),
    ] {
        assert!(
            native_snapshots::publication(Publication {
                primary: &primary,
                matched: &primary,
                checkpoint: &bad,
                core_only: false,
                full_catalog: true,
                release_verified: false,
            })
            .is_err()
        );
    }
}

#[test]
fn extended_dependencies_retire_only_their_baseline_and_partial_snapshots_after_publication() {
    let primary = format!("native-v2-macos-15-image-{}", "a".repeat(64));
    let checkpoint = cache::checkpoint_key(&primary, "full", "402", "1").unwrap();
    let complete = native_snapshots::complete_key(&primary).unwrap();
    let entry = |id, key: String, version: &str, reference: &str| {
        json!({"id":id,"key":key,"version":version,"ref":reference,
            "created_at":format!("2026-10-{id:02}T00:00:00Z")})
    };
    let mut entries = vec![
        entry(1, primary.clone(), "wire", "refs/heads/main"),
        entry(2, checkpoint, "wire", "refs/heads/main"),
        entry(3, primary.clone(), "other", "refs/heads/main"),
        entry(4, primary.clone(), "wire", "refs/pull/1/merge"),
        entry(
            5,
            primary.replace("-image-", "-other-image-"),
            "wire",
            "refs/heads/main",
        ),
        entry(6, format!("core-{primary}"), "wire", "refs/heads/main"),
    ];
    let removed = |entries: &[serde_json::Value]| {
        cache::obsolete(entries, None)
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert!(
        removed(&entries).is_empty(),
        "The release baseline must not retire the complementary default graph"
    );
    entries.push(entry(7, complete, "wire", "refs/heads/main"));
    assert_eq!(removed(&entries), [1, 2]);
}

#[test]
fn stale_dependency_catalog_cannot_prune_outputs_or_publish_a_checkpoint() {
    use lazypencil_xtask::process;
    use std::{fs, process::Command, time::Duration};
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("target");
    fs::create_dir(&target).unwrap();
    let marker = target.join("owned-output");
    fs::write(&marker, b"unchanged").unwrap();
    let output = temporary.path().join("github-output");
    let primary = format!("native-v2-macos-15-image-{}", "a".repeat(64));
    let checkpoint = cache::checkpoint_key(&primary, "full", "402", "1").unwrap();
    let result = process::output(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args([
                "ci",
                "cache",
                "native-checkpoint",
                "--primary",
                &primary,
                "--checkpoint",
                &checkpoint,
                "--matched-key",
                &primary,
                "--complete-catalog",
                &"b".repeat(40),
                "--source",
            ])
            .arg(temporary.path())
            .arg("--target")
            .arg(&target)
            .env("GITHUB_SHA", "a".repeat(40))
            .env("GITHUB_OUTPUT", &output),
        Duration::from_secs(15),
    )
    .unwrap();
    assert!(!result.timed_out && !result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("another source")
    );
    assert_eq!(fs::read(marker).unwrap(), b"unchanged");
    assert!(!output.exists());
}
