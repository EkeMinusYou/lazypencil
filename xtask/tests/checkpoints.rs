use lazypencil_xtask::ci::cache;
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[test]
fn cache_headroom_retires_only_replaced_downloads_before_tool_publication() {
    let prefix = "cargo-downloads-Linux-X64";
    let old = format!("{prefix}-{}", "a".repeat(64));
    let new = format!("{prefix}-{}", "b".repeat(64));
    let entry = |id, key: &str, version: &str, reference: &str| {
        json!({"id":id,"key":key,"version":version,"ref":reference,
            "created_at":format!("2026-10-{id:02}T00:00:00Z")})
    };
    let entries = vec![
        entry(1, &old, "wire", "refs/heads/main"),
        entry(2, &new, "wire", "refs/heads/main"),
        entry(3, &old, "other", "refs/heads/main"),
        entry(4, &old.replace("X64", "ARM64"), "wire", "refs/heads/main"),
        entry(5, &old.replace("Linux", "macOS"), "wire", "refs/heads/main"),
        entry(6, &old, "wire", "refs/pull/1/merge"),
        entry(
            7,
            &old.replace("downloads-", "downloads-v2-"),
            "wire",
            "refs/heads/main",
        ),
        entry(
            8,
            "native-v2-ubuntu-24.04-fixture",
            "wire",
            "refs/heads/main",
        ),
    ];
    assert_eq!(
        cache::obsolete(&entries, None)
            .unwrap()
            .iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1]
    );
    assert!(
        cache::obsolete(
            &entries
                .iter()
                .filter(|e| e["id"] != 2)
                .cloned()
                .collect::<Vec<_>>(),
            None
        )
        .unwrap()
        .is_empty(),
        "The last download set survives until a matching newer set exists"
    );
    let action = std::fs::read_to_string(
        lazypencil_xtask::util::root().join(".github/actions/xtask/action.yml"),
    )
    .unwrap();
    let position = |text| action.find(text).unwrap();
    assert!(
        position("Verify Rust tools before application builds")
            < position("Verify source inputs before reserving publication headroom")
    );
    assert!(
        position("Verify source inputs before reserving publication headroom")
            < position("Retire obsolete sources and duplicate downloads before publishing tools")
    );
    assert!(
        position("Retire obsolete sources and duplicate downloads before publishing tools")
            < position("Save verified source-bound tools")
    );
    let workflow =
        std::fs::read_to_string(lazypencil_xtask::util::root().join(".github/workflows/ci.yml"))
            .unwrap();
    let plan = workflow.split("  audit:").next().unwrap();
    assert!(plan.contains("reserve-source-headroom: 'true'"));
    assert!(!workflow.contains("cache-maintenance:"));
}

#[test]
fn manifest_retention_expires_old_release_sources_but_preserves_test_only_changes() {
    use lazypencil_xtask::dist::{Entry, Manifest};
    let mut manifest = Manifest {
        format_version: 1,
        package: json!({}),
        source_date_epoch: 1,
        git: json!({"revision":"a".repeat(40),"dirty":false}),
        files: ["src/lib.rs", "tests/one.rs"]
            .into_iter()
            .enumerate()
            .map(|(index, path)| Entry {
                path: path.into(),
                sha256: format!("{}", index + 1).repeat(64),
                bytes: 1,
                mode: 0o644,
            })
            .collect(),
    };
    let production = cache::release_source_fingerprint(&manifest).unwrap();
    let source = cache::source_fingerprint(&manifest).unwrap();
    let old = "f".repeat(64);
    let context = "b".repeat(64);
    let entry = |id, key: String, reference: &str| json!({"id":id,"key":key,"ref":reference,"version":"layout","created_at":"2026-10-01"});
    let entries = vec![
        entry(
            1,
            format!(
                "release-v1-ubuntu-24.04-{old}-{}-focus-401-1",
                "c".repeat(64)
            ),
            "refs/heads/main",
        ),
        entry(
            2,
            format!("release-v1-ubuntu-24.04-{production}-{context}-full"),
            "refs/heads/main",
        ),
        entry(
            3,
            format!("release-v1-ubuntu-24.04-{old}-{context}-full"),
            "refs/heads/main",
        ),
        entry(
            4,
            format!("tests-v1-ubuntu-24.04-{context}-full-{source}"),
            "refs/heads/main",
        ),
        entry(
            5,
            format!("native-v2-ubuntu-24.04-{context}"),
            "refs/heads/main",
        ),
        entry(
            6,
            format!("release-v1-ubuntu-24.04-{old}-{context}-full"),
            "refs/pull/1/merge",
        ),
        entry(
            7,
            format!("release-v2-ubuntu-24.04-{old}-{context}-full"),
            "refs/heads/main",
        ),
    ];
    assert!(cache::obsolete(&entries, None).unwrap().is_empty());
    let removed = |manifest: &Manifest| {
        cache::obsolete_for_manifest(&entries, manifest)
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(removed(&manifest), [1, 3]);
    manifest.files[1].sha256 = "3".repeat(64);
    assert_ne!(cache::source_fingerprint(&manifest).unwrap(), source);
    assert_eq!(
        cache::release_source_fingerprint(&manifest).unwrap(),
        production
    );
    assert_eq!(removed(&manifest), [1, 3]);
    manifest.files[0].sha256 = "4".repeat(64);
    assert_ne!(
        cache::release_source_fingerprint(&manifest).unwrap(),
        production
    );
    assert_eq!(removed(&manifest), [1, 2, 3]);
}

#[test]
fn prepublication_retirement_is_rejected_before_any_remote_operation() {
    let output = lazypencil_xtask::process::output(
        std::process::Command::new(env!("CARGO_BIN_EXE_xtask")).args([
            "ci",
            "cache",
            "retain",
            "--before-replacement",
            "--repository",
            "owner/repository",
        ]),
        std::time::Duration::from_secs(15),
    )
    .unwrap();
    assert!(!output.timed_out);
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("unexpected argument") && error.contains("--before-replacement"),
        "{error}"
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn publication_retires_only_the_replaced_configuration_compiler_and_wire_format() {
    let current = "a".repeat(64);
    let old = "b".repeat(64);
    let context = "c".repeat(64);
    let other_context = "d".repeat(64);
    let entry = |id, key: String, wire: &str| json!({"id":id,"key":key,"ref":"refs/heads/main","version":wire,"created_at":format!("2026-01-01T00:00:{id:02}Z")});
    let key = |role: &str, platform: &str, context: &str, source: &str| {
        format!("{role}-v1-{platform}-{context}-full-{source}")
    };
    let mut entries = vec![
        entry(1, key("tests", "macos-15", &context, &old), "wire"),
        entry(2, key("tests-core", "macos-15", &context, &old), "wire"),
        entry(3, key("tests", "macos-15", &other_context, &old), "wire"),
        entry(4, key("tests", "macos-15", &context, &old), "other-wire"),
        entry(5, key("tests", "ubuntu-24.04", &context, &old), "wire"),
        entry(6, format!("native-v2-macos-15-{context}"), "wire"),
    ];
    let removed = |entries: &[Value]| {
        cache::obsolete(entries, Some(&current))
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert!(removed(&entries).is_empty(), "Staging is not publication");
    entries.push(entry(
        7,
        key("tests", "macos-15", &context, &current),
        "wire",
    ));
    assert_eq!(
        removed(&entries),
        [1],
        "Publishing default outputs cannot erase the core or another compiler's fallback"
    );
    entries.push(entry(
        8,
        key("tests-core", "macos-15", &context, &current),
        "wire",
    ));
    assert_eq!(removed(&entries), [1, 2]);
}

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
    let release = base.replace("application-v2", "release-v1");
    assert_eq!(
        cache::application_checkpoint(&release, "402", "1").unwrap(),
        format!("{release}-402-1")
    );
    for namespace in ["tests-v1", "tests-core-v1"] {
        let base = format!(
            "{namespace}-macos-15-{}-focus-{}",
            "a".repeat(64),
            "b".repeat(64)
        );
        assert_eq!(
            cache::application_checkpoint(&base, "402", "2").unwrap(),
            format!("{base}-402-2")
        );
        assert!(
            cache::application_checkpoint(&base.replace("-focus-", "-full-"), "402", "2").is_err()
        );
    }
}

#[test]
fn focused_retention_requires_the_same_compiler_source_features_and_cache_layout() {
    let source = "a".repeat(64);
    let context = "b".repeat(64);
    let other = "c".repeat(64);
    let entry = |id, role, platform, context, phase, source, wire| {
        json!({
            "id":id,"key":format!("{role}-v1-{platform}-{context}-{phase}-{source}"),
            "version":wire,"ref":"refs/heads/main","created_at":format!("2026-10-{id:02}")
        })
    };
    let mut entries = vec![
        entry(
            1,
            "tests",
            "macos-15",
            &context,
            "focus",
            format!("{source}-401-1"),
            "wire",
        ),
        entry(
            2,
            "tests",
            "macos-15",
            &context,
            "focus",
            format!("{source}-402-1"),
            "wire",
        ),
        entry(
            3,
            "tests",
            "macos-15",
            &other,
            "focus",
            format!("{source}-402-1"),
            "wire",
        ),
        entry(
            4,
            "tests",
            "macos-15",
            &context,
            "focus",
            format!("{source}-402-1"),
            "other-wire",
        ),
        entry(
            5,
            "tests-core",
            "macos-15",
            &context,
            "focus",
            format!("{source}-402-1"),
            "wire",
        ),
        entry(
            6,
            "tests",
            "ubuntu-24.04",
            &context,
            "focus",
            format!("{source}-402-1"),
            "wire",
        ),
        entry(
            7,
            "tests",
            "macos-15",
            &context,
            "full",
            other.clone(),
            "wire",
        ),
    ];
    let removed = |entries: &[Value]| {
        cache::obsolete(entries, Some(&other))
            .unwrap()
            .iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        removed(&entries),
        [1],
        "A source change alone cannot retire another image's focused build"
    );
    entries.push(entry(
        8,
        "tests",
        "macos-15",
        &context,
        "full",
        source.clone(),
        "wire",
    ));
    assert_eq!(
        removed(&entries),
        [1, 2, 7],
        "Only the matching context and layout are covered"
    );
    entries.push(entry(
        9, "tests", "macos-15", &other, "full", source, "wire",
    ));
    assert_eq!(removed(&entries), [1, 2, 3, 7]);
}

#[test]
fn checkpoint_exchange_preserves_payloads_and_refuses_ambiguous_or_linked_paths() {
    use std::fs;
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("partial");
    let second = temp.path().join("complete");
    assert!(cache::artifact::exchange(&first, &second).is_err());
    fs::create_dir(&first).unwrap();
    fs::write(first.join("payload"), b"partial").unwrap();
    cache::artifact::exchange(&first, &second).unwrap();
    assert!(!first.exists());
    assert_eq!(fs::read(second.join("payload")).unwrap(), b"partial");
    cache::artifact::exchange(&first, &second).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(second.join("payload"), b"complete").unwrap();
    cache::artifact::exchange(&first, &second).unwrap();
    for invalid in [
        first.clone(),
        first.join("nested"),
        first.join("../complete"),
    ] {
        assert!(cache::artifact::exchange(&first, &invalid).is_err());
    }
    let file = temp.path().join("file");
    fs::write(&file, b"file").unwrap();
    assert!(cache::artifact::exchange(&first, &file).is_err());
    #[cfg(unix)]
    {
        let linked = temp.path().join("linked");
        std::os::unix::fs::symlink(&second, &linked).unwrap();
        assert!(cache::artifact::exchange(&first, &linked).is_err());
    }
    assert_eq!(fs::read(first.join("payload")).unwrap(), b"complete");
    assert_eq!(fs::read(second.join("payload")).unwrap(), b"partial");
    assert!(!fs::read_dir(temp.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".checkpoint-exchange-")
    }));
}

#[test]
fn release_retention_preserves_other_compiler_contexts_layouts_and_test_sources() {
    let stem = format!(
        "release-v1-ubuntu-24.04-{}-{}",
        "a".repeat(64),
        "b".repeat(64)
    );
    let entry = |id, key: String| json!({"id":id,"key":key,"version":"same-layout","ref":"refs/heads/main","created_at":format!("2026-10-{id:02}T00:00:00Z")});
    let focus = format!("{stem}-focus");
    let mut entries = vec![
        entry(1, format!("{focus}-402-1")),
        entry(2, format!("{focus}-403-1")),
        entry(
            3,
            format!(
                "application-v2-ubuntu-24.04-{}-{}-full",
                "c".repeat(64),
                "d".repeat(64)
            ),
        ),
    ];
    let removed = |entries: &[serde_json::Value]| {
        cache::obsolete(entries, Some(&"f".repeat(64)))
            .unwrap()
            .into_iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(removed(&entries), [1]);
    entries.push(entry(4, format!("{stem}-full")));
    assert_eq!(removed(&entries), [1, 2]);
    entries.push(entry(
        5,
        format!(
            "release-v1-ubuntu-24.04-{}-{}-full",
            "e".repeat(64),
            "f".repeat(64)
        ),
    ));
    assert_eq!(removed(&entries), [1, 2]);
    // A newer source replaces the preceding full output only within the same
    // compiler context, without losing the other SDK's independently usable map.
    entries.push(entry(
        6,
        format!(
            "release-v1-ubuntu-24.04-{}-{}-full",
            "e".repeat(64),
            "b".repeat(64)
        ),
    ));
    assert_eq!(removed(&entries), [1, 2, 4]);
    let mut other_layout = entry(7, format!("{stem}-full"));
    other_layout["version"] = json!("other-layout");
    entries.push(other_layout);
    assert_eq!(removed(&entries), [1, 2, 4]);
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
        [1, 2, 3]
    );
    let replacement = format!(
        "application-v2-ubuntu-24.04-{}-{}-full",
        "c".repeat(64),
        "d".repeat(64)
    );
    entries.push(entry(6, replacement, "refs/heads/main"));
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
        // Check the actual restore lists, independent of workflow outputs and
        // artifact names that may mention the same key outside cache restore.
        let lists = yaml
            .split("restore-keys: |\n")
            .skip(1)
            .map(|block| {
                block
                    .lines()
                    .take_while(|line| line.starts_with("            "))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|list| list.contains("outputs.checkpoint_prefix"))
            .collect::<Vec<_>>();
        assert_eq!(lists.len(), if workflow == "native.yml" { 2 } else { 1 });
        for list in lists {
            assert!(
                list.find("outputs.checkpoint_prefix").unwrap()
                    < list.find("outputs.checkpoint_legacy").unwrap()
            );
        }
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
    entries.push(entry(7, primary.clone(), "layout-a", "refs/heads/main"));
    assert_eq!(removed(&entries), [1, 2]);
    entries.push(entry(
        8,
        format!("complete-{primary}"),
        "layout-a",
        "refs/heads/main",
    ));
    assert_eq!(removed(&entries), [1, 2, 3, 7]);
}
#[test]
fn tool_dependencies_retire_only_after_a_newer_complete_matching_sdk_and_format() {
    let entry = |id, key: String, date, version, reference| serde_json::json!({"id":id,"key":key,"created_at":date,"version":version,"ref":reference});
    let prefix = "Linux-ARM64-image_one-rust1.94.0";
    let dependency = |prefix: &str| format!("xtask-deps-v1-{prefix}-{}", "a".repeat(64));
    let source = format!("xtask-v1-{prefix}-{}", "b".repeat(64));
    let entries = vec![
        entry(
            1,
            dependency(prefix),
            "2026-10-01",
            "one",
            "refs/heads/main",
        ),
        entry(2, source, "2026-10-02", "one", "refs/heads/main"),
        entry(
            3,
            dependency(prefix),
            "2026-10-03",
            "one",
            "refs/heads/main",
        ),
        entry(
            4,
            dependency(prefix),
            "2026-10-01",
            "two",
            "refs/heads/main",
        ),
        entry(
            5,
            dependency("Linux-ARM64-image_two-rust1.94.0"),
            "2026-10-01",
            "one",
            "refs/heads/main",
        ),
        entry(
            6,
            dependency(prefix),
            "2026-10-01",
            "one",
            "refs/pull/1/merge",
        ),
        entry(
            7,
            format!("native-v2-ubuntu-24.04-arm-image_one-{}", "a".repeat(64)),
            "2026-10-01",
            "one",
            "refs/heads/main",
        ),
    ];
    let removed = cache::obsolete(&entries, None).unwrap();
    assert_eq!(
        removed
            .iter()
            .map(|r| r["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1]
    );
    let without_source = entries
        .into_iter()
        .filter(|r| r["id"] != 2)
        .collect::<Vec<_>>();
    assert!(cache::obsolete(&without_source, None).unwrap().is_empty());
}
