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

#[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn release_codegen_override_rebuilds_only_application_units_and_keeps_test_edits_fresh() {
    use lazypencil_xtask::evidence::link_map;
    let f = Fixture::new();
    f.add_dependency();
    fs::write(
        f.source.join("Cargo.toml"),
        "[package]\nname='lazypencil'\nversion='0.1.0'\nedition='2024'\n[workspace]\n[dependencies]\nunchanged_dependency={path='dependency'}\n[profile.release]\ncodegen-units=1\nlto='thin'\n",
    )
    .unwrap();
    fs::write(
        f.source.join("src/lib.rs"),
        "pub fn fixture() -> u8 { unchanged_dependency::value() }\n",
    )
    .unwrap();
    fs::write(
        f.source.join("src/main.rs"),
        "fn main() { println!(\"{}\", lazypencil::fixture()); }\n",
    )
    .unwrap();
    process::checked(
        Command::new("cargo")
            .current_dir(&f.source)
            .args(["generate-lockfile", "--offline"]),
        Duration::from_secs(120),
    )
    .unwrap();
    let compiler = process::text(Command::new("rustc").arg("-vV")).unwrap();
    let host = compiler
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    let current = link_map::cargo_arguments(host, &f.target, true).unwrap();
    let mut baseline = current.clone();
    let config = baseline.iter().position(|arg| arg == "--config").unwrap();
    drop(baseline.drain(config..config + 2));
    let (before, _) = f.cargo_units(&baseline);
    assert_eq!(before.len(), 3);
    assert!(before.values().all(|fresh| !fresh));
    let (changed, compiler_commands) = f.cargo_units(&current);
    assert!(changed["unchanged_dependency:lib"]);
    assert!(!changed["lazypencil:lib"] && !changed["lazypencil:bin"]);
    assert_eq!(compiler_commands.matches("codegen-units=16").count(), 2);
    assert!(!compiler_commands.contains("--crate-name unchanged_dependency"));
    assert!(link_map::path(&f.target).is_file());
    fs::write(
        f.source.join("tests/one.rs"),
        "#[test] fn changed_test_only() { assert_eq!(lazypencil::fixture(), 1); }\n",
    )
    .unwrap();
    let (unchanged, _) = f.cargo_units(&current);
    assert!(unchanged.values().all(|fresh| *fresh));
    assert_eq!(
        process::checked(
            &mut Command::new(f.target.join("release/lazypencil")),
            Duration::from_secs(15),
        )
        .unwrap(),
        b"1\n"
    );
}

#[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn release_recipe_transition_reuses_debug_only_and_overlays_the_new_release_delta() {
    use lazypencil_xtask::evidence::link_map;
    let f = Fixture::new();
    fs::write(
        f.source.join("Cargo.toml"),
        "[package]\nname='lazypencil'\nversion='0.1.0'\nedition='2024'\n[workspace]\n[profile.release]\ncodegen-units=1\nlto='thin'\n",
    )
    .unwrap();
    fs::write(
        f.source.join("src/main.rs"),
        "fn main() { println!(\"{}\", lazypencil::fixture()); }\n",
    )
    .unwrap();
    f.manifest();
    let compiler = process::text(Command::new("rustc").arg("-vV")).unwrap();
    let host = compiler
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap();
    let current_command = link_map::cargo_arguments(host, &f.target, true).unwrap();
    let mut old_command = current_command.clone();
    let config = old_command
        .iter()
        .position(|arg| arg == "--config")
        .unwrap();
    drop(old_command.drain(config..config + 2));
    f.build(&[]);
    f.cargo_units(&old_command);
    let current = f.identity();
    let old = current.previous_debug_identity().unwrap().unwrap();
    cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
    let legacy: Value = util::json(&f.full.join("manifest.json")).unwrap();
    assert!(
        legacy["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["path"].as_str().unwrap().starts_with("release/") })
    );
    f.clear();
    let incompatible =
        cache::application_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-other-sdk")
            .unwrap();
    assert!(
        cache::restore_compatible(
            &f.source,
            &f.target,
            &f.full,
            &old.compatible_primary,
            &incompatible,
        )
        .is_err()
    );
    assert!(!f.target.exists());
    let retained = cache::restore_compatible(
        &f.source,
        &f.target,
        &f.full,
        &old.compatible_primary,
        &current,
    )
    .unwrap();
    assert_eq!(retained["release_recipe_changed"], true);
    assert!(!f.target.join("release/lazypencil").exists());
    assert!(!link_map::path(&f.target).exists());
    assert!(f.build(&[]).values().all(|(fresh, _)| *fresh));
    assert!(
        f.cargo_units(&current_command)
            .0
            .values()
            .all(|fresh| !fresh)
    );
    let release_cache = f._temp.path().join("release-cache");
    let release =
        cache::release_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-fixture").unwrap();
    cache::stage(&f.source, &f.target, &release_cache, &release).unwrap();
    let delta = cache::stage_focused(
        &f.source,
        &f.target,
        &f.exact,
        &f.full,
        &old.compatible_primary,
        &current,
    )
    .unwrap();
    assert_eq!(
        delta["files"], 0,
        "Release already has a separate checkpoint"
    );
    assert!(!f.exact.exists());
    f.clear();
    cache::restore_combined(
        &f.source,
        &f.target,
        &f.exact,
        &f.full,
        cache::CacheMatches {
            full: &old.compatible_primary,
            ..Default::default()
        },
        &current,
    )
    .unwrap();
    assert!(!f.target.join("release/lazypencil").exists());
    cache::restore(&f.source, &f.target, &release_cache, &release).unwrap();
    assert!(f.build(&[]).values().all(|(fresh, _)| *fresh));
    let (release, _) = f.cargo_units(&current_command);
    assert!(release.values().all(|fresh| *fresh));

    // New complete and fallback test checkpoints never duplicate the release.
    cache::stage_tests(&f.source, &f.target, &f.full, &current).unwrap();
    cache::stage_focused(
        &f.source,
        &f.target,
        &f.exact,
        &f._temp.path().join("missing-cache"),
        "",
        &current,
    )
    .unwrap();
    for directory in [&f.full, &f.exact] {
        let report: Value = util::json(&directory.join("manifest.json")).unwrap();
        let files = report["files"].as_array().unwrap();
        assert!(!files.is_empty());
        assert!(
            files
                .iter()
                .all(|entry| { entry["path"].as_str().unwrap().starts_with("debug/") })
        );
    }
    f.edit();
    let edited = f.identity();
    let release =
        cache::release_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-fixture").unwrap();
    f.clear();
    cache::restore_compatible(
        &f.source,
        &f.target,
        &f.full,
        &current.compatible_primary,
        &edited,
    )
    .unwrap();
    cache::restore(&f.source, &f.target, &release_cache, &release).unwrap();
    let debug = f.build(&[]);
    assert!(!debug["one"].0 && debug["two"].0);
    assert!(
        f.cargo_units(&current_command)
            .0
            .values()
            .all(|fresh| *fresh)
    );
}

#[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn source_fallback_uses_cargo_ownership_to_discard_stale_tests_and_keep_unchanged_dependencies() {
    let f = Fixture::new();
    f.add_dependency();
    fs::write(f.source.join("Cargo.toml"), "[package]\nname='lazypencil'\nversion='0.1.0'\nedition='2024'\n[workspace]\n[dependencies]\nunchanged_dependency={path='dependency'}\n").unwrap();
    fs::write(
        f.source.join("src/lib.rs"),
        "pub fn fixture() -> u8 { unchanged_dependency::value() }\n",
    )
    .unwrap();
    fs::write(
        f.source.join("src/main.rs"),
        "fn main() { println!(\"{}\", lazypencil::fixture()); }\n",
    )
    .unwrap();
    process::checked(
        Command::new("cargo")
            .current_dir(&f.source)
            .args(["generate-lockfile", "--offline"]),
        Duration::from_secs(120),
    )
    .unwrap();
    f.manifest();
    f.build(&[]);
    f.edit();
    // A complete older source checkpoint contains test binaries too. Without
    // cleaning this package, normalized source mtimes conceal the edited test.
    let stale = f.build(&[]);
    assert!(stale["one"].0);
    let run = |path: &std::path::Path| {
        process::output(&mut Command::new(path), Duration::from_secs(15)).unwrap()
    };
    let stale_result = run(&stale["one"].1);
    assert!(!stale_result.timed_out && stale_result.status.success());
    process::checked(
        Command::new("cargo")
            .current_dir(&f.source)
            .env("CARGO_TARGET_DIR", &f.target)
            .args(["clean", "--locked", "--package", "lazypencil"]),
        Duration::from_secs(120),
    )
    .unwrap();
    let arguments = [
        "cargo",
        "build",
        "--locked",
        "--offline",
        "--bins",
        "--message-format=json",
    ]
    .map(str::to_owned);
    let (units, _) = f.cargo_units(&arguments);
    assert!(units["unchanged_dependency:lib"]);
    assert!(!units["lazypencil:lib"] && !units["lazypencil:bin"]);
    let rebuilt = f.build(&[]);
    assert!(rebuilt["unchanged_dependency"].0);
    assert!(!rebuilt["one"].0);
    let updated = run(&rebuilt["one"].1);
    assert!(
        !updated.timed_out && !updated.status.success(),
        "The edited assertion was not executed"
    );
}

impl Fixture {
    #[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
    fn add_dependency(&self) {
        fs::create_dir_all(self.source.join("dependency/src")).unwrap();
        fs::write(
            self.source.join("dependency/Cargo.toml"),
            "[package]\nname='unchanged_dependency'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        fs::write(
            self.source.join("dependency/src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .unwrap();
    }
    #[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
    fn cargo_units(&self, arguments: &[String]) -> (BTreeMap<String, bool>, String) {
        let output = process::output(
            Command::new(&arguments[0])
                .current_dir(&self.source)
                .env("CARGO_TARGET_DIR", &self.target)
                .args(&arguments[1..]),
            Duration::from_secs(120),
        )
        .unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!output.timed_out && output.status.success(), "{stderr}");
        let mut units = BTreeMap::new();
        for line in output.stdout.split(|&byte| byte == b'\n') {
            let Ok(event) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            if event["reason"] == "compiler-artifact" {
                let key = format!(
                    "{}:{}",
                    event["target"]["name"].as_str().unwrap(),
                    event["target"]["kind"][0].as_str().unwrap()
                );
                units.insert(key, event["fresh"].as_bool().unwrap());
            }
        }
        assert!(!units.is_empty(), "Cargo reported no compiler units");
        (units, stderr)
    }

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
            "src/main.rs",
            "src/verification.rs",
            "tests/three.rs",
            "tests/support/common.rs",
            "xtask/src/test_support/terminal.rs",
            "xtask/src/test_support/acp.rs",
            "xtask/src/fixtures/browser_ui.rs",
            "tests/fixtures/browser-ui/chromium.json",
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
                        || v["target"]["kind"] == json!(["lib"]))
            })
            .map(|v| {
                (
                    if v["target"]["kind"] == json!(["lib"]) && v["profile"]["test"] == true {
                        "library-tests".into()
                    } else {
                        v["target"]["name"].as_str().unwrap().into()
                    },
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
        assert_eq!(f.build(&[]).len(), 5);
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
                cache::CacheMatches {
                    full: &old.compatible_primary,
                    ..Default::default()
                },
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
                cache::CacheMatches {
                    full: &old.compatible_primary,
                    ..Default::default()
                },
                &current,
            )
            .unwrap();
            assert!(report["invalidated_files"].as_u64().unwrap() > 0);
        }
        let built = f.build(&[]);
        assert_eq!(built.len(), 5);
        assert!(built["lazypencil"].0, "The production library was rebuilt");
        assert!(
            built["library-tests"].0,
            "The unrelated library tests were rebuilt"
        );
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
fn older_partial_checkpoint_reuses_only_unchanged_compiler_units_after_helper_edits() {
    let f = Fixture::new();
    let helper = f.source.join("tests/support/common.rs");
    fs::create_dir_all(helper.parent().unwrap()).unwrap();
    fs::write(&helper, "pub const VALUE: u8 = 1;\n").unwrap();
    fs::write(f.source.join("tests/one.rs"),
        "#[path=\"support/common.rs\"] mod helper;\n#[test] fn value() { assert_eq!(helper::VALUE, 1); }\n").unwrap();
    f.manifest();
    assert_eq!(f.build(&["--test", "one", "--test", "two"]).len(), 3);
    let old = f.identity();
    let key = cache::application_checkpoint(&old.checkpoint, "401", "1").unwrap();
    cache::stage_tests(&f.source, &f.target, &f.exact, &old).unwrap();
    fs::write(&helper, "pub const VALUE: u8 = 2;\n").unwrap();
    f.manifest();
    f.clear();
    let current = f.identity();
    assert_ne!(old.primary, current.primary);
    assert_eq!(old.application_prefix, current.application_prefix);
    for bad_key in [
        format!("{key}-0"),
        format!("{}-0-1", old.checkpoint),
        current.checkpoint.clone(),
    ] {
        assert!(
            cache::restore_compatible(&f.source, &f.target, &f.exact, &bad_key, &current).is_err()
        );
        assert!(!f.target.exists());
    }
    let incompatible =
        cache::application_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-other-sdk")
            .unwrap();
    assert!(
        cache::restore_compatible(&f.source, &f.target, &f.exact, &key, &incompatible).is_err()
    );
    assert!(!f.target.exists());
    let original = fs::read(f.source.join("src/lib.rs")).unwrap();
    fs::write(
        f.source.join("src/lib.rs"),
        "pub fn fixture() -> u8 { 2 }\n",
    )
    .unwrap();
    f.manifest();
    assert!(
        cache::restore_compatible(&f.source, &f.target, &f.exact, &key, &f.identity()).is_err()
    );
    assert!(
        !f.target.exists(),
        "A production edit reused an old partial build"
    );
    fs::write(f.source.join("src/lib.rs"), original).unwrap();
    f.manifest();
    let report = cache::restore_combined(
        &f.source,
        &f.target,
        &f.exact,
        &f.full,
        cache::CacheMatches {
            focused: &key,
            ..Default::default()
        },
        &current,
    )
    .unwrap();
    assert_eq!(report["reused"], true);
    assert_eq!(report["changed_tests"], json!(["tests/support/common.rs"]));
    let built = f.build(&[]);
    assert!(
        built["lazypencil"].0 && built["two"].0,
        "Unchanged units rebuilt"
    );
    assert!(!built["one"].0, "Changed helper reused stale code");
    assert!(
        !built["library-tests"].0 && !built["preview"].0,
        "Missing units were not built"
    );
    let output =
        process::output(&mut Command::new(&built["one"].1), Duration::from_secs(15)).unwrap();
    assert!(
        !output.timed_out && !output.status.success(),
        "The changed assertion did not execute"
    );
}

#[test]
fn shared_helpers_invalidate_every_consumer_without_rebuilding_unrelated_outputs() {
    for helper in [
        "tests/support/common.rs",
        "xtask/src/test_support/terminal.rs",
        "src/verification.rs",
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
            fs::write(f.source.join("src/lib.rs"), format!(
                "pub fn fixture() -> u8 {{ 1 }}\n#[cfg(test)] #[path=\"../{helper}\"] mod helper;\n#[cfg(test)] #[test] fn shared_value() {{ assert_eq!(helper::VALUE, 1); }}\n"
            )).unwrap();
            f.manifest();
            assert_eq!(f.build(&[]).len(), 6);
            let old = f.identity();
            let old_key = if focused {
                old.compatible_keys(true).unwrap().0
            } else {
                old.compatible_primary.clone()
            };
            cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
            fs::write(&path, "pub const VALUE: u8 = 2;\n").unwrap();
            f.manifest();
            f.clear();
            let current = f.identity();
            assert_eq!(old.compatible_prefix, current.compatible_prefix);
            assert_ne!(old.primary, current.primary);
            if focused {
                assert_eq!(f.build(&["--test", "one", "--test", "three"]).len(), 3);
                cache::stage_focused(&f.source, &f.target, &f.exact, &f.full, &old_key, &current)
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
                cache::CacheMatches {
                    full: &old_key,
                    ..Default::default()
                },
                &current,
            )
            .unwrap();
            let built = f.build(&[]);
            assert_eq!(built.len(), 6);
            for name in ["lazypencil", "two", "preview"] {
                assert!(built[name].0, "{helper}: unrelated {name} rebuilt");
            }
            for name in ["one", "three", "library-tests"] {
                assert_eq!(
                    built[name].0,
                    focused && name != "library-tests",
                    "{helper}: {name}"
                );
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
fn headed_browser_runtime_fixtures_keep_real_cargo_units_fresh_and_reject_compiled_consumers() {
    let paths = [
        "xtask/src/fixtures/browser_ui.rs",
        "tests/fixtures/browser-ui/chromium.json",
    ];
    for consumed in [None, Some(paths[0]), Some(paths[1])] {
        let f = Fixture::new();
        for path in paths {
            let path = f.source.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"first runtime fixture\n").unwrap();
        }
        if let Some(path) = consumed {
            fs::write(
                f.source.join("src/lib.rs"),
                format!(
                    "pub fn fixture() -> u8 {{ assert!(!include_bytes!({}).is_empty()); 1 }}\n",
                    json!(format!("../{path}"))
                ),
            )
            .unwrap();
        }
        f.manifest();
        assert_eq!(f.build(&[]).len(), 5);
        let old = f.identity();
        let staged = cache::stage(&f.source, &f.target, &f.full, &old);
        if let Some(path) = consumed {
            assert!(
                staged.is_err(),
                "Actual compiler input was excluded: {path}"
            );
            assert!(!f.full.exists(), "Rejected compiler inputs were cached");
            continue;
        }
        staged.unwrap();
        for path in paths {
            fs::write(f.source.join(path), b"changed runtime fixture\n").unwrap();
        }
        f.manifest();
        let current = f.identity();
        assert_eq!(current.primary, old.primary);
        f.clear();
        cache::restore(&f.source, &f.target, &f.full, &current).unwrap();
        let built = f.build(&[]);
        assert_eq!(built.len(), 5);
        assert!(built.values().all(|(fresh, _)| *fresh));
        for name in ["library-tests", "one", "two"] {
            process::checked(&mut Command::new(&built[name].1), Duration::from_secs(15)).unwrap();
        }
        let delta = cache::stage_focused(
            &f.source,
            &f.target,
            &f.exact,
            &f.full,
            &old.compatible_primary,
            &current,
        )
        .unwrap();
        assert_eq!(delta["files"], 0);
        assert!(!f.exact.exists(), "Unchanged full outputs were saved again");
    }
}

#[test]
fn runtime_only_helper_edits_keep_real_cargo_outputs_fresh_without_a_duplicate_checkpoint() {
    for incomplete in [false, true] {
        let f = Fixture::new();
        let helper = "xtask/src/test_support/acp.rs";
        let path = f.source.join(helper);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "pub const VERSION: u8 = 1;\n").unwrap();
        f.manifest();
        assert_eq!(f.build(&[]).len(), 5);
        let old = f.identity();
        cache::stage(&f.source, &f.target, &f.full, &old).unwrap();
        if incomplete {
            let manifest = f.full.join("manifest.json");
            let mut report: Value = util::json(&manifest).unwrap();
            let entries = report["files"].as_array_mut().unwrap();
            let index = entries
                .iter()
                .position(|entry| {
                    let name = entry["path"].as_str().unwrap();
                    name.starts_with("debug/deps/lazypencil-") && name.ends_with(".d")
                })
                .unwrap();
            let omitted = entries.remove(index);
            fs::remove_file(f.full.join("files").join(omitted["path"].as_str().unwrap())).unwrap();
            fs::write(manifest, util::encode(&report).unwrap()).unwrap();
        }
        fs::write(&path, "pub const VERSION: u8 = 2;\n").unwrap();
        f.manifest();
        f.clear();
        let current = f.identity();
        assert_eq!(old.compatible_prefix, current.compatible_prefix);
        assert_ne!(old.primary, current.primary);
        let restored = cache::restore_compatible(
            &f.source,
            &f.target,
            &f.full,
            &old.compatible_primary,
            &current,
        );
        if incomplete {
            assert!(
                restored.is_err(),
                "Missing dep-info cannot establish zero compiler consumers"
            );
            assert!(!f.target.exists());
            continue;
        }
        let restored = restored.unwrap();
        assert_eq!(restored["invalidated_files"], 0);
        assert_eq!(restored["uncompiled_helpers"], json!([helper]));
        let built = f.build(&[]);
        assert_eq!(built.len(), 5);
        assert!(
            built.values().all(|(fresh, _)| *fresh),
            "Runtime-only source changes rebuilt application outputs"
        );
        for name in ["library-tests", "one", "two"] {
            process::checked(&mut Command::new(&built[name].1), Duration::from_secs(15)).unwrap();
        }
        fs::create_dir(&f.exact).unwrap();
        fs::write(f.exact.join("obsolete"), b"must not be saved").unwrap();
        let staged = cache::stage_focused(
            &f.source,
            &f.target,
            &f.exact,
            &f.full,
            &old.compatible_primary,
            &current,
        )
        .unwrap();
        assert_eq!(staged["files"], 0);
        assert!(!f.exact.exists(), "Unchanged full outputs were saved again");
    }
}

#[test]
fn helper_reuse_rejects_production_consumers_and_incomplete_unit_dep_info() {
    for helper in ["tests/support/common.rs", "src/verification.rs"] {
        for production in [false, true] {
            let f = Fixture::new();
            let path = f.source.join(helper);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "pub const VALUE: u8 = 1;\n").unwrap();
            for name in ["one", "two"] {
                fs::write(f.source.join(format!("tests/{name}.rs")),
                format!("#[path=\"../{helper}\"] mod helper;\n#[test] fn value() {{ assert_eq!(helper::VALUE, lazypencil::fixture()); }}\n")).unwrap();
            }
            if production {
                fs::write(f.source.join("src/lib.rs"),
                format!("#[path=\"../{helper}\"] mod helper;\npub fn fixture() -> u8 {{ helper::VALUE }}\n")).unwrap();
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
                fs::remove_file(f.full.join("files").join(omitted["path"].as_str().unwrap()))
                    .unwrap();
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
                    cache::CacheMatches {
                        full: &old.compatible_primary,
                        ..Default::default()
                    },
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
