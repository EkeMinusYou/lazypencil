use lazypencil_xtask::ci::cache;
use serde_json::json;
use std::collections::BTreeSet;

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
