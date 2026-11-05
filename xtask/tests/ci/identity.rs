use super::*;

#[test]
fn native_identity_binds_compiler_image_flags_and_dependency_tree_but_not_scheduling() {
    let original =
        cache::native_identity("ubuntu-24.04", "20261012.334.1", TREE, COMPILER, &flags()).unwrap();
    for (name, value) in [
        ("CARGO_BUILD_JOBS", "2"),
        ("RUSTFLAGS", "--cfg changed"),
        ("CARGO_PROFILE_RELEASE_LTO", "thin"),
        ("SKIA_SOURCE_DIR", "/changed"),
    ] {
        let mut env = flags();
        env.insert(name.into(), value.into());
        let changed =
            cache::native_identity("ubuntu-24.04", "20261012.334.1", TREE, COMPILER, &env).unwrap();
        assert_ne!(changed["primary"], original["primary"]);
        assert_eq!(changed["legacy"], "");
    }
    let mut env = flags();
    env.insert("CARGO_TARGET_DIR".into(), "/another/temp".into());
    env.insert("GITHUB_SHA".into(), "new source".into());
    env.insert("UNRELATED_SECRET".into(), "never-in-report".into());
    assert_eq!(
        cache::native_identity("ubuntu-24.04", "20261012.334.1", TREE, COMPILER, &env).unwrap(),
        original
    );
    let previous = cache::native_identity(
        "ubuntu-24.04",
        "20261012.334.1",
        "c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7",
        COMPILER,
        &flags(),
    )
    .unwrap();
    assert_eq!(original["legacy"], previous["primary"]);
    let compiler = COMPILER.replace("x86_64-unknown-linux-gnu", "aarch64-apple-darwin");
    let font_tree = "11d1597cbd47f826b985b5e514b42c0ff8aa9d4a3d378264e88ac38781b0e160";
    let baseline = cache::native_identity(
        "macos-26",
        "20260907.0337.1",
        "80b3b3fc770be2245a731ad66682615c3b4979af5c410c5bf196d10c61aba68b",
        &compiler,
        &flags(),
    )
    .unwrap();
    let migrated = cache::native_identity(
        "macos-26",
        "20260907.0337.1",
        font_tree,
        &compiler,
        &flags(),
    )
    .unwrap();
    assert_eq!(migrated["legacy"], baseline["primary"]);
    assert_ne!(migrated["primary"], baseline["primary"]);
    let release_tree = "9d9ec938606c1c29c9ac36d0868af4b0bf3d61be85d66449287258edbb680e1b";
    let release_baseline = cache::native_identity(
        "macos-26",
        "20261028.0489.1",
        "4cd30528eaee7f6025192ea093fe203e7c59be06b95b84c8cf0c00f66790f6fa",
        &compiler,
        &flags(),
    )
    .unwrap();
    let release_identity = cache::native_identity(
        "macos-26",
        "20261028.0489.1",
        release_tree,
        &compiler,
        &flags(),
    )
    .unwrap();
    assert_eq!(release_identity["legacy"], release_baseline["primary"]);
    assert_ne!(release_identity["primary"], release_baseline["primary"]);
    for (name, value) in [
        ("RUSTFLAGS", "--cfg changed"),
        ("SKIA_SOURCE_DIR", "/changed"),
    ] {
        let mut env = flags();
        env.insert(name.into(), value.into());
        assert_eq!(
            cache::native_identity("macos-26", "20261028.0489.1", release_tree, &compiler, &env)
                .unwrap()["legacy"],
            ""
        );
        assert_eq!(
            cache::native_identity("macos-26", "20260907.0337.1", font_tree, &compiler, &env)
                .unwrap()["legacy"],
            ""
        );
    }
    assert_eq!(
        cache::native_identity(
            "macos-26",
            "20260907.0337.1",
            font_tree,
            &compiler.replace("1.94.0", "1.95.0"),
            &flags()
        )
        .unwrap()["legacy"],
        ""
    );
    for (platform, image, tree) in [
        ("unknown", "image", TREE),
        ("ubuntu-24.04", "unknown", TREE),
        ("ubuntu-24.04", "../image", TREE),
        ("ubuntu-24.04", "image\nlegacy=injected", TREE),
        ("ubuntu-24.04", "image", "bad"),
    ] {
        assert!(cache::native_identity(platform, image, tree, COMPILER, &flags()).is_err());
    }
}

#[test]
fn test_profile_changes_only_application_cache_identity_and_preserves_release_reuse() {
    let f = ApplicationFixture::new();
    let identify = |optimized: bool, release: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
        command
            .args(["ci", "cache", "application", "identify", "--source"])
            .arg(&f.source)
            .arg("--target")
            .arg(&f.target)
            .arg("--directory")
            .arg(&f.directory)
            .args([
                "--native-key",
                "native-v2-ubuntu-24.04-fixture",
                "--github-output",
            ])
            .arg(f.source.join("identity-output"));
        if optimized {
            command.env(ci::commands::TEST_PROFILE_ENV, "optimized-debug-v1");
        } else {
            command.env_remove(ci::commands::TEST_PROFILE_ENV);
        }
        if release {
            command.arg("--release-only");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["primary"].clone()
    };
    assert_ne!(identify(false, false), identify(true, false));
    assert_eq!(identify(false, true), identify(true, true));
}
#[test]
fn application_identify_cli_emits_unique_generation_keys_and_ordered_compatible_fallbacks() {
    let fixture = ApplicationFixture::new();
    let identity = fixture.identity();
    let mut checkpoints = BTreeMap::new();
    for attempt in ["1", "2"] {
        let output = fixture.directory.with_extension(attempt);
        let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["ci", "cache", "application", "identify"])
            .arg("--source")
            .arg(&fixture.source)
            .arg("--target")
            .arg(&fixture.target)
            .arg("--directory")
            .arg(&fixture.directory)
            .args(["--native-key", "native-v2-ubuntu-24.04-fixture"])
            .arg("--github-output")
            .arg(&output)
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_RUN_ID", "402")
            .env("GITHUB_RUN_ATTEMPT", attempt)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["primary"], identity.primary);
        assert_eq!(report["checkpoint_legacy"], identity.checkpoint);
        assert_eq!(report["application_prefix"], "application-v2-ubuntu-24.04-");
        assert_eq!(
            report["checkpoint_prefix"],
            format!("{}-", identity.checkpoint)
        );
        assert_eq!(
            report["checkpoint"],
            format!("{}-402-{attempt}", identity.checkpoint)
        );
        let outputs = fs::read_to_string(output).unwrap();
        let previous_keys: Vec<String> = serde_json::from_str(
            outputs
                .lines()
                .find_map(|line| line.strip_prefix("previous_debug_keys="))
                .unwrap(),
        )
        .unwrap();
        let previous = identity.previous_debug_identities().unwrap();
        assert_eq!(previous.len(), 2);
        let workflow =
            fs::read_to_string(util::root().join(".github/workflows/native.yml")).unwrap();
        let current_keys = workflow
            .split("id: compatible-cache\n")
            .nth(1)
            .unwrap()
            .split("restore-keys: |\n")
            .nth(1)
            .unwrap()
            .split("          path:")
            .next()
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.contains("join("))
            .count();
        assert!(
            1 + current_keys + previous_keys.len() <= 10,
            "Actions rejects the entire cache lookup when its key budget is exceeded"
        );
        for old in &previous {
            let (legacy_primary, _) = old.compatible_keys(true).unwrap();
            for primary in [&old.compatible_primary, &legacy_primary] {
                assert!(
                    previous_keys
                        .iter()
                        .any(|prefix| primary.starts_with(prefix)),
                    "Lost a reviewed older compiler context: {primary}"
                );
            }
        }
        let (focused, focused_prefix) = identity.focused_keys().unwrap();
        for (key, expected) in [
            ("focused_checkpoint", format!("{focused}-402-{attempt}")),
            ("focused_checkpoint_prefix", format!("{focused}-")),
            ("focused_prefix", focused_prefix),
        ] {
            assert!(
                outputs
                    .lines()
                    .any(|line| line == format!("{key}={expected}")),
                "Missing {key}"
            );
        }
        for key in [
            "primary",
            "checkpoint",
            "checkpoint_prefix",
            "checkpoint_legacy",
            "application_prefix",
        ] {
            assert!(
                outputs
                    .lines()
                    .any(|line| line == format!("{key}={}", report[key].as_str().unwrap()))
            );
        }
        checkpoints.insert(attempt, report["checkpoint"].clone());
    }
    assert_ne!(checkpoints["1"], checkpoints["2"]);
}

#[test]
fn verified_tool_replacement_retires_only_the_restored_sdk_and_cache_format() {
    for prefix in [
        "xtask-v1-Linux-ARM64-20261023.149.3-rust1.94.0",
        "xtask-binaries-v1-Linux-ARM64-20261023.149.3-rust1.94.0",
    ] {
        let primary = format!("{prefix}-{}", "a".repeat(64));
        let matched = format!("{prefix}-{}", "b".repeat(64));
        let entry = |id, key: &str, version: &str, reference: &str| json!({"id":id,"key":key,"version":version,"ref":reference});
        let entries = vec![
            entry(1, &matched, "current", "refs/heads/main"),
            entry(2, &primary, "current", "refs/heads/main"),
            entry(
                3,
                &format!("{prefix}-{}", "c".repeat(64)),
                "other",
                "refs/heads/main",
            ),
            entry(4, &matched, "current", "refs/pull/1/merge"),
            entry(
                5,
                &matched.replace("20261023.149.3", "20261026.166.1"),
                "current",
                "refs/heads/main",
            ),
            entry(
                6,
                "native-v2-ubuntu-24.04-arm-existing",
                "current",
                "refs/heads/main",
            ),
            entry(
                7,
                &if prefix.starts_with("xtask-binaries-") {
                    matched.replace("xtask-binaries-v1", "xtask-v1")
                } else {
                    matched.replace("xtask-v1", "xtask-binaries-v1")
                },
                "current",
                "refs/heads/main",
            ),
        ];
        let obsolete =
            cache::obsolete_tools_before_replacement(&entries, &primary, &matched).unwrap();
        assert_eq!(
            cache::obsolete_tools_before_replacement(&entries, &primary, &primary).unwrap(),
            obsolete,
            "An exact verified hit must still retire older compatible sources"
        );
        assert_eq!(
            obsolete
                .iter()
                .map(|entry| entry["id"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(
            cache::obsolete_tools_before_replacement(&entries, &primary, "")
                .unwrap()
                .is_empty()
        );
        let mut ambiguous = entries.clone();
        ambiguous.push(entry(7, &matched, "second-format", "refs/heads/main"));
        assert!(
            cache::obsolete_tools_before_replacement(&ambiguous, &primary, &matched)
                .unwrap()
                .is_empty()
        );
        assert!(
            cache::obsolete_tools_before_replacement(&entries, "unreviewed", &matched).is_err()
        );
        assert!(
            cache::obsolete_tools_before_replacement(
                &entries,
                &primary,
                &matched.replace("ARM64", "X64")
            )
            .is_err()
        );
    }
}

#[test]
fn cache_wire_format_stays_compatible_including_unicode_and_separators() {
    assert_eq!(
        cache::stable_json(&json!({"b":[1,true,null],"a":"日本語"})),
        br#"{"a": "\u65e5\u672c\u8a9e", "b": [1, true, null]}"#
    );
}

#[test]
fn source_timestamp_normalization_rejects_tampering_before_touching_any_input() {
    let f = ApplicationFixture::new();
    let before = fs::metadata(f.source.join("src/lib.rs"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(f.source.join("Cargo.toml"), b"changed").unwrap();
    assert!(ci::normalize_source(&f.source).is_err());
    assert_eq!(
        fs::metadata(f.source.join("src/lib.rs"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[test]
fn native_dependency_tree_requires_clean_committed_inputs_and_ignores_docs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let hooks = root.join("empty-hooks");
    fs::create_dir(&hooks).unwrap();
    let git = |args: &[&str]| {
        let result = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
            ])
            .arg(format!("core.hooksPath={}", hooks.display()))
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    git(&["init", "--quiet"]);
    fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
    fs::write(root.join("Cargo.lock"), "version=4\n").unwrap();
    git(&["add", "Cargo.toml", "Cargo.lock"]);
    git(&["commit", "-m", "Create fixture"]);
    let original = cache::dependency_tree(root).unwrap();
    fs::write(root.join("README.md"), "docs").unwrap();
    assert_eq!(cache::dependency_tree(root).unwrap(), original);
    fs::write(root.join("Cargo.lock"), "version=3\n").unwrap();
    assert!(cache::dependency_tree(root).is_err());
    git(&["add", "Cargo.lock"]);
    assert!(cache::dependency_tree(root).is_err());
    git(&["commit", "-m", "Change fixture lock"]);
    assert_ne!(cache::dependency_tree(root).unwrap(), original);
    fs::create_dir_all(root.join("vendor/skia-bindings")).unwrap();
    fs::write(root.join("vendor/skia-bindings/new.cpp"), "changed").unwrap();
    assert!(cache::dependency_tree(root).is_err());
}

#[test]
fn source_fingerprints_exclude_ci_prose_and_native_review_data_and_ignore_manifest_order() {
    let f = ApplicationFixture::new();
    let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
    let original = cache::source_fingerprint(&manifest).unwrap();
    manifest.files.reverse();
    assert_eq!(cache::source_fingerprint(&manifest).unwrap(), original);
    for path in [
        "README.md",
        "AGENTS.md",
        "docs/design.md",
        "docs/nested/guide.md",
        "tests/fixtures/fonts/README.md",
        ".github/actions/xtask/action.yml",
        "xtask/src/ci/cache.rs",
        "resources/licenses/native/quickjs-review.json",
        "resources/licenses/native/upstream.json",
        "xtask/src/fixtures/browser_ui.rs",
        "tests/fixtures/browser-ui/chromium.json",
    ] {
        let mut changed = manifest.clone();
        changed.files.push(Entry {
            path: path.into(),
            bytes: 7,
            sha256: util::digest(b"changed"),
            mode: 0o644,
        });
        assert_eq!(
            cache::source_fingerprint(&changed).unwrap(),
            original,
            "{path}"
        );
    }
    for path in [
        "tests/support/fixture.rs",
        "xtask/src/fixtures/oracles/shapes.rs",
        "xtask/src/test_support/acp.rs",
        "xtask/src/bin/test-helper.rs",
        "docs/fixture.json",
        "resources/guidance/prompt.md",
        "tests/fixture.md",
        "tests/fixtures/fonts/input.md",
        "scripts/test_input.json",
    ] {
        let mut changed = manifest.clone();
        changed.files.push(Entry {
            path: path.into(),
            bytes: 7,
            sha256: util::digest(b"changed"),
            mode: 0o644,
        });
        assert_ne!(
            cache::source_fingerprint(&changed).unwrap(),
            original,
            "{path}"
        );
    }
}
