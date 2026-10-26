use lazypencil_xtask::ci::cache;
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn application_checkpoints_do_not_collide_across_later_focused_runs_or_retries() {
    let base = format!(
        "application-v2-ubuntu-24.04-{}-{}-focus",
        "a".repeat(64),
        "b".repeat(64)
    );
    let prefix = format!("{base}-");
    let mut keys = BTreeSet::new();
    for (run, attempt) in [("402", "1"), ("402", "2"), ("403", "1")] {
        let key = cache::application_checkpoint(&base, run, attempt).unwrap();
        assert!(key.starts_with(&prefix));
        assert!(keys.insert(key));
    }
    for bad in ["", "0", "01", "-1", "1\n", "18446744073709551616"] {
        assert!(cache::application_checkpoint(&base, bad, "1").is_err());
        assert!(cache::application_checkpoint(&base, "402", bad).is_err());
    }
    assert!(cache::application_checkpoint(&base.replace("-focus", "-full"), "402", "1").is_err());
    assert!(cache::application_checkpoint("../application-v2-foreign", "402", "1").is_err());
}

#[test]
fn application_retention_keeps_the_latest_generation_then_retires_it_after_full_success() {
    let source = "a".repeat(64);
    let stem = format!("application-v2-ubuntu-24.04-{source}-{}", "b".repeat(64));
    let base = format!("{stem}-focus");
    let entry = |id, key: String, reference: &str| json!({"id":id,"key":key,"version":"same-layout","ref":reference,"created_at":format!("2026-10-{id:02}T00:00:00Z")});
    let mut entries = vec![
        entry(1, base.clone(), "refs/heads/main"),
        entry(
            2,
            cache::application_checkpoint(&base, "402", "1").unwrap(),
            "refs/heads/main",
        ),
        entry(
            3,
            cache::application_checkpoint(&base, "402", "2").unwrap(),
            "refs/heads/main",
        ),
        entry(
            4,
            cache::application_checkpoint(&base, "403", "1").unwrap(),
            "refs/pull/1/merge",
        ),
    ];
    let removed = |entries: &[serde_json::Value]| {
        cache::obsolete(entries, Some(&source))
            .unwrap()
            .iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(removed(&entries), [1, 2]);
    entries.push(entry(5, format!("{stem}-full"), "refs/heads/main"));
    assert_eq!(removed(&entries), [1, 2, 3]);
    assert_eq!(
        cache::obsolete(&entries, Some(&"c".repeat(64)))
            .unwrap()
            .iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3, 5]
    );
    for workflow in ["native.yml", "release-evidence.yml"] {
        let yaml = std::fs::read_to_string(
            lazypencil_xtask::util::root()
                .join(".github/workflows")
                .join(workflow),
        )
        .unwrap();
        assert!(
            yaml.find("outputs.checkpoint_prefix").unwrap()
                < yaml.find("outputs.checkpoint_legacy").unwrap()
        );
    }
}

#[test]
fn immutable_checkpoints_distinguish_phase_run_and_retry_without_changing_native_identity() {
    let primary = format!("native-v2-macos-15-image-{}", "a".repeat(64));
    let prefix = format!("checkpoint-{primary}-");
    let mut unique = BTreeSet::new();
    for (phase, run, attempt) in [
        ("focus", "402", "1"),
        ("full", "402", "1"),
        ("full", "402", "2"),
        ("full", "403", "1"),
    ] {
        let key = cache::checkpoint_key(&primary, phase, run, attempt).unwrap();
        assert!(key.starts_with(&prefix));
        assert!(unique.insert(key));
    }
    for bad in ["", "0", "01", "-1", "1\n", "18446744073709551616"] {
        assert!(cache::checkpoint_key(&primary, "full", bad, "1").is_err());
        assert!(cache::checkpoint_key(&primary, "full", "402", bad).is_err());
    }
    assert!(cache::checkpoint_key(&primary, "unknown", "402", "1").is_err());
    assert!(cache::checkpoint_key("../native-v2-foreign", "full", "402", "1").is_err());
}

#[test]
fn newest_compatible_checkpoint_survives_until_a_complete_main_cache_replaces_it() {
    let primary = format!("native-v2-ubuntu-24.04-image-{}", "a".repeat(64));
    let other = format!("native-v2-ubuntu-24.04-image-{}", "b".repeat(64));
    let entry = |id, key: String, version: &str, reference: &str| {
        json!({
            "id":id,"key":key,"version":version,"ref":reference,
            "created_at":format!("2026-10-{id:02}T00:00:00Z")
        })
    };
    let mut entries = vec![
        entry(
            1,
            format!("checkpoint-{primary}"),
            "layout-a",
            "refs/heads/main",
        ),
        entry(
            2,
            cache::checkpoint_key(&primary, "focus", "402", "1").unwrap(),
            "layout-a",
            "refs/heads/main",
        ),
        entry(
            3,
            cache::checkpoint_key(&primary, "full", "402", "1").unwrap(),
            "layout-a",
            "refs/heads/main",
        ),
        entry(
            4,
            cache::checkpoint_key(&other, "full", "402", "1").unwrap(),
            "layout-a",
            "refs/heads/main",
        ),
        entry(
            5,
            cache::checkpoint_key(&primary, "full", "403", "1").unwrap(),
            "layout-a",
            "refs/pull/1/merge",
        ),
        entry(6, primary.clone(), "layout-b", "refs/heads/main"),
    ];
    let removed = |entries: &[serde_json::Value]| {
        cache::obsolete(entries, None)
            .unwrap()
            .iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(removed(&entries), [1, 2]);
    entries.push(entry(7, primary, "layout-a", "refs/heads/main"));
    assert_eq!(removed(&entries), [1, 2, 3]);
}
