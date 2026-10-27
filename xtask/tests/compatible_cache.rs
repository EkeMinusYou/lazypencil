use lazypencil_xtask::{
    ci::cache,
    dist::{self, Entry, Manifest},
    process, util,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime},
};

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
    full: PathBuf,
    exact: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let f = Self {
            source: root.join("source"),
            target: root.join("target"),
            full: root.join("full"),
            exact: root.join("exact"),
            _temp: temp,
        };
        fs::create_dir_all(f.source.join("src")).unwrap();
        fs::create_dir_all(f.source.join("tests")).unwrap();
        fs::create_dir_all(f.source.join("examples")).unwrap();
        fs::write(
            f.source.join("examples/preview.rs"),
            "fn main() { assert_eq!(lazypencil::fixture(), 1); }\n",
        )
        .unwrap();
        fs::write(
            f.source.join("Cargo.toml"),
            "[package]\nname='lazypencil'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            f.source.join("Cargo.lock"),
            "version = 4\n[[package]]\nname = 'lazypencil'\nversion = '0.1.0'\n",
        )
        .unwrap();
        fs::write(
            f.source.join("src/lib.rs"),
            "pub fn fixture() -> u8 { 1 }\n",
        )
        .unwrap();
        for name in ["one", "two"] {
            fs::write(
                f.source.join(format!("tests/{name}.rs")),
                "#[test] fn value() { assert_eq!(lazypencil::fixture(), 1); }\n",
            )
            .unwrap();
        }
        f.manifest();
        f
    }
    fn manifest(&self) {
        let mut names = vec![
            "Cargo.toml",
            "Cargo.lock",
            "src/lib.rs",
            "tests/one.rs",
            "tests/two.rs",
            "examples/preview.rs",
        ];
        for extra in [
            "tests/three.rs",
            "tests/support/common.rs",
            "xtask/src/test_support/terminal.rs",
        ] {
            if self.source.join(extra).is_file() {
                names.push(extra);
            }
        }
        let files = names
            .into_iter()
            .map(|name| {
                let path = self.source.join(name);
                // CI normalizes mtimes. The regression must be detected by
                // content, even when the edited file retains this old timestamp.
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(
                        fs::FileTimes::new()
                            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
                    )
                    .unwrap();
                Entry {
                    path: name.into(),
                    sha256: util::file_digest(&path).unwrap(),
                    bytes: fs::metadata(&path).unwrap().len(),
                    mode: util::raw_mode(&path).unwrap(),
                }
            })
            .collect();
        let manifest = Manifest {
            format_version: 1,
            package: json!({}),
            source_date_epoch: 1,
            git: json!({"revision":"1111111111111111111111111111111111111111","dirty":false}),
            files,
        };
        fs::write(
            self.source.join(dist::MANIFEST),
            util::encode(&manifest).unwrap(),
        )
        .unwrap();
    }
    fn identity(&self) -> cache::ApplicationIdentity {
        cache::application_identity(&self.source, &self.target, "native-v2-ubuntu-24.04-fixture")
            .unwrap()
    }
    fn build(&self, args: &[&str]) -> BTreeMap<String, (bool, PathBuf)> {
        let bytes = process::checked(
            Command::new("cargo")
                .current_dir(&self.source)
                .env("CARGO_TARGET_DIR", &self.target)
                .args([
                    "test",
                    "--locked",
                    "--offline",
                    "--no-run",
                    "--message-format=json",
                ])
                .args(args),
            Duration::from_secs(120),
        )
        .unwrap();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|v| {
                v["reason"] == "compiler-artifact"
                    && ([json!(["test"]), json!(["example"])].contains(&v["target"]["kind"])
                        || (v["target"]["kind"] == json!(["lib"]) && v["profile"]["test"] == false))
            })
            .map(|v| {
                (
                    v["target"]["name"].as_str().unwrap().into(),
                    (
                        v["fresh"].as_bool().unwrap(),
                        PathBuf::from(
                            v["executable"]
                                .as_str()
                                .or_else(|| v["filenames"][0].as_str())
                                .unwrap(),
                        ),
                    ),
                )
            })
            .collect()
    }
    fn clear(&self) {
        fs::remove_dir_all(&self.target).unwrap();
    }
    fn edit(&self) {
        fs::write(
            self.source.join("tests/one.rs"),
            "#[test] fn value() { assert_eq!(lazypencil::fixture(), 2); }\n",
        )
        .unwrap();
        self.manifest();
    }
}

#[test]
fn real_cargo_rebuilds_only_the_edited_test_and_accepts_same_source_focused_outputs() {
    for focused in [false, true] {
        let f = Fixture::new();
        assert_eq!(f.build(&[]).len(), 4);
        let old = f.identity();
        cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
        let empty = cache::stage_focused(
            &f.source,
            &f.target,
            &f.exact,
            &f.full,
            &old.compatible_primary,
            &old,
        )
        .unwrap();
        assert_eq!(
            empty["files"], 0,
            "An unchanged full payload must not be saved twice"
        );
        assert!(!f.exact.exists());
        f.edit();
        f.clear();
        let current = f.identity();
        assert_ne!(old.primary, current.primary);
        assert_eq!(old.compatible_prefix, current.compatible_prefix);
        if focused {
            assert_eq!(f.build(&["--test", "one"]).len(), 2);
            cache::stage_focused(
                &f.source,
                &f.target,
                &f.exact,
                &f.full,
                &old.compatible_primary,
                &current,
            )
            .unwrap();
            let delta: Value = util::json(&f.exact.join("manifest.json")).unwrap();
            assert!(
                delta["files"].as_array().unwrap().iter().all(|entry| {
                    let path = entry["path"].as_str().unwrap();
                    !path.contains("/deps/liblazypencil-") && !path.contains("/deps/two-")
                }),
                "Focused cache copied unchanged full outputs"
            );
            f.clear();
            let report = cache::restore_combined(
                &f.source,
                &f.target,
                &f.exact,
                &f.full,
                &old.compatible_primary,
                &current,
            )
            .unwrap();
            assert!(report["compatible"]["invalidated_files"].as_u64().unwrap() > 0);
        } else {
            let report = cache::restore_combined(
                &f.source,
                &f.target,
                &f.exact,
                &f.full,
                &old.compatible_primary,
                &current,
            )
            .unwrap();
            assert!(report["invalidated_files"].as_u64().unwrap() > 0);
        }
        let built = f.build(&[]);
        assert_eq!(built.len(), 4);
        assert!(built["lazypencil"].0, "The production library was rebuilt");
        assert!(built["two"].0, "The untouched integration test was rebuilt");
        assert!(built["preview"].0, "The untouched example was rebuilt");
        assert_eq!(
            built["one"].0, focused,
            "The edited test reused stale code or lost the focused build"
        );
        let output =
            process::output(&mut Command::new(&built["one"].1), Duration::from_secs(15)).unwrap();
        assert!(
            !output.timed_out && !output.status.success(),
            "The changed assertion must actually execute"
        );
    }
}

#[test]
fn shared_helpers_invalidate_every_consumer_without_rebuilding_unrelated_outputs() {
    for helper in [
        "tests/support/common.rs",
        "xtask/src/test_support/terminal.rs",
    ] {
        for focused in [false, true] {
            let f = Fixture::new();
            let path = f.source.join(helper);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "pub const VALUE: u8 = 1;\n").unwrap();
            for name in ["one", "three"] {
                fs::write(f.source.join(format!("tests/{name}.rs")), format!(
                    "#[path=\"../{helper}\"] mod helper;\n#[test] fn value() {{ assert_eq!(lazypencil::fixture(), 1); assert_eq!(helper::VALUE, 1); }}\n"
                )).unwrap();
            }
            f.manifest();
            assert_eq!(f.build(&[]).len(), 5);
            let old = f.identity();
            cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
            fs::write(&path, "pub const VALUE: u8 = 2;\n").unwrap();
            f.manifest();
            f.clear();
            let current = f.identity();
            assert_eq!(old.compatible_prefix, current.compatible_prefix);
            assert_ne!(old.primary, current.primary);
            if focused {
                assert_eq!(f.build(&["--test", "one", "--test", "three"]).len(), 3);
                cache::stage_focused(
                    &f.source,
                    &f.target,
                    &f.exact,
                    &f.full,
                    &old.compatible_primary,
                    &current,
                )
                .unwrap();
                let delta: Value = util::json(&f.exact.join("manifest.json")).unwrap();
                assert!(delta["files"].as_array().unwrap().iter().all(|entry| {
                    let path = entry["path"].as_str().unwrap();
                    !path.contains("lazypencil-") || path.contains("/.fingerprint/")
                }));
                assert!(
                    delta["files"].as_array().unwrap().iter().all(|entry| {
                        let path = entry["path"].as_str().unwrap();
                        !path.contains("/deps/two-") && !path.contains("/examples/")
                    }),
                    "Focused delta copied unrelated outputs"
                );
                f.clear();
            }
            cache::restore_combined(
                &f.source,
                &f.target,
                &f.exact,
                &f.full,
                &old.compatible_primary,
                &current,
            )
            .unwrap();
            let built = f.build(&[]);
            assert_eq!(built.len(), 5);
            for name in ["lazypencil", "two", "preview"] {
                assert!(built[name].0, "{helper}: unrelated {name} rebuilt");
            }
            for name in ["one", "three"] {
                assert_eq!(built[name].0, focused, "{helper}: {name}");
                let output =
                    process::output(&mut Command::new(&built[name].1), Duration::from_secs(15))
                        .unwrap();
                assert!(
                    !output.timed_out && !output.status.success(),
                    "Stale shared-helper code executed"
                );
            }
        }
    }
}

#[test]
fn helper_reuse_rejects_production_consumers_and_incomplete_unit_dep_info() {
    for production in [false, true] {
        let f = Fixture::new();
        let path = f.source.join("tests/support/common.rs");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "pub const VALUE: u8 = 1;\n").unwrap();
        for name in ["one", "two"] {
            fs::write(f.source.join(format!("tests/{name}.rs")),
                "#[path=\"support/common.rs\"] mod helper;\n#[test] fn value() { assert_eq!(helper::VALUE, lazypencil::fixture()); }\n").unwrap();
        }
        if production {
            fs::write(f.source.join("src/lib.rs"),
                "#[path=\"../tests/support/common.rs\"] mod helper;\npub fn fixture() -> u8 { helper::VALUE }\n").unwrap();
        }
        f.manifest();
        f.build(&[]);
        let old = f.identity();
        cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
        if !production {
            let manifest = f.full.join("manifest.json");
            let mut report: Value = util::json(&manifest).unwrap();
            let entries = report["files"].as_array_mut().unwrap();
            let index = entries
                .iter()
                .position(|entry| {
                    let name = entry["path"].as_str().unwrap();
                    name.starts_with("debug/deps/two-") && name.ends_with(".d")
                })
                .unwrap();
            let omitted = entries.remove(index);
            fs::remove_file(f.full.join("files").join(omitted["path"].as_str().unwrap())).unwrap();
            fs::write(manifest, util::encode(&report).unwrap()).unwrap();
        }
        fs::write(&path, "pub const VALUE: u8 = 2;\n").unwrap();
        f.manifest();
        f.clear();
        let current = f.identity();
        assert_eq!(old.compatible_prefix, current.compatible_prefix);
        assert!(
            cache::restore_compatible(
                &f.source,
                &f.target,
                &f.full,
                &old.compatible_primary,
                &current
            )
            .is_err()
        );
        assert!(
            !f.target.exists(),
            "Rejected helper payload overlaid outputs"
        );
        if production {
            f.build(&[]);
            cache::stage_focused(
                &f.source,
                &f.target,
                &f.exact,
                &f.full,
                &old.compatible_primary,
                &current,
            )
            .unwrap();
            let staged: Value = util::json(&f.exact.join("manifest.json")).unwrap();
            assert_eq!(
                staged["files"].as_array().unwrap().len(),
                cache::outputs(&f.target).unwrap().len()
            );
            f.clear();
            cache::restore_combined(
                &f.source,
                &f.target,
                &f.exact,
                &f.full,
                &old.compatible_primary,
                &current,
            )
            .unwrap();
            assert!(
                f.build(&[]).values().all(|(fresh, _)| *fresh),
                "The verified same-source fallback was lost"
            );
        }
    }
}

#[test]
fn compatible_reuse_rejects_unbound_inputs_recipes_and_non_test_units_before_overlay() {
    for case in [
        "source",
        "mode",
        "set",
        "recipe",
        "key",
        "metadata",
        "import",
        "corruption",
    ] {
        let f = Fixture::new();
        f.build(&[]);
        let old = f.identity();
        cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
        f.edit();
        let manifest_path = f.source.join(dist::MANIFEST);
        let mut manifest: Manifest = util::json(&manifest_path).unwrap();
        match case {
            "source" => {
                fs::write(
                    f.source.join("src/lib.rs"),
                    "pub fn fixture() -> u8 { 3 }\n",
                )
                .unwrap();
                f.manifest();
            }
            "mode" => {
                manifest
                    .files
                    .iter_mut()
                    .find(|e| e.path == "tests/one.rs")
                    .unwrap()
                    .mode ^= 0o111;
                fs::write(&manifest_path, util::encode(&manifest).unwrap()).unwrap();
            }
            "set" => {
                manifest.files.retain(|e| e.path != "tests/two.rs");
                fs::write(&manifest_path, util::encode(&manifest).unwrap()).unwrap();
            }
            "metadata" => {
                let path = f.full.join("manifest.json");
                let mut v: Value = util::json(&path).unwrap();
                v["inputs"][0]["sha256"] = json!("tampered");
                fs::write(path, util::encode(&v).unwrap()).unwrap();
            }
            "import" => {
                let path = fs::read_dir(f.full.join("files/debug/deps"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| {
                        p.extension().is_some_and(|e| e == "d")
                            && p.file_name()
                                .unwrap()
                                .to_string_lossy()
                                .starts_with("lazypencil-")
                    })
                    .unwrap();
                fs::write(
                    path,
                    format!("library: {}\n", f.source.join("tests/one.rs").display()),
                )
                .unwrap();
            }
            "corruption" => {
                let path = f.full.join("manifest.json");
                let mut v: Value = util::json(&path).unwrap();
                v["files"][0]["sha256"] = json!("tampered");
                fs::write(path, util::encode(&v).unwrap()).unwrap();
            }
            _ => {}
        }
        f.clear();
        let current = if case == "recipe" {
            cache::application_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-other")
                .unwrap()
        } else {
            f.identity()
        };
        let key = if case == "key" {
            "incorrect-key"
        } else {
            &old.compatible_primary
        };
        assert!(
            cache::restore_compatible(&f.source, &f.target, &f.full, key, &current).is_err(),
            "{case}"
        );
        assert!(
            !f.target.exists(),
            "Rejected {case} payload overlaid outputs"
        );
    }
}

#[test]
fn compatible_cache_retention_is_bounded_and_preserves_other_platforms_and_versions() {
    let entries = (1..=4).map(|id| json!({"id":id,"key":format!("tests-v1-ubuntu-24.04-{}-full-{:064x}", "a".repeat(64), id),
        "ref":"refs/heads/main","version":"one","created_at":format!("2026-10-{id:02}")})).collect::<Vec<_>>();
    assert_eq!(cache::obsolete(&entries, None).unwrap().len(), 2);
    assert_eq!(
        cache::obsolete(&entries, Some(&"a".repeat(64)))
            .unwrap()
            .len(),
        3
    );
    let mut others = entries.clone();
    for (id, platform, version, reference) in [
        (5, "macos-15", "one", "refs/heads/main"),
        (6, "ubuntu-24.04", "two", "refs/heads/main"),
        (7, "ubuntu-24.04", "one", "refs/pull/1/merge"),
    ] {
        others.push(
            json!({"id":id,"key":format!("tests-v1-{platform}-{}-full-{:064x}", "a".repeat(64), id),
            "ref":reference,"version":version,"created_at":"2026-10-20"}),
        );
    }
    assert!(
        cache::obsolete(&others, None)
            .unwrap()
            .iter()
            .all(|v| v["id"].as_u64().unwrap() <= 2)
    );
}

#[test]
fn full_compatible_checkpoint_replaces_duplicate_legacy_payloads_only_after_it_exists() {
    let entry = |id, key: String, reference: &str| {
        json!({"id":id,"key":key,
        "ref":reference,"version":"format","created_at":"2026-10-26"})
    };
    let source = "b".repeat(64);
    let old = |platform: &str, phase: &str| {
        format!(
            "application-v2-{platform}-{source}-{}-{phase}",
            "c".repeat(64)
        )
    };
    let mut entries = vec![
        entry(1, old("ubuntu-24.04", "full"), "refs/heads/main"),
        entry(2, old("ubuntu-24.04", "focus-401-1"), "refs/heads/main"),
        entry(3, old("macos-15", "full"), "refs/heads/main"),
        entry(4, old("ubuntu-24.04", "full"), "refs/pull/1/merge"),
    ];
    // The matching old full build already makes its focus checkpoint redundant.
    assert_eq!(
        cache::obsolete(&entries, Some(&source))
            .unwrap()
            .iter()
            .map(|v| v["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [2]
    );
    entries.push(entry(
        5,
        format!("tests-v1-ubuntu-24.04-{}-full-{source}", "a".repeat(64)),
        "refs/heads/main",
    ));
    assert_eq!(
        cache::obsolete(&entries, Some(&source))
            .unwrap()
            .iter()
            .map(|v| v["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2]
    );
}
