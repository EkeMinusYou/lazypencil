use lazypencil_xtask::{
    ci::{self, cache, focus},
    dist::{self, Entry, Manifest},
    util,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

#[test]
fn changed_measurements_use_one_source_bound_candidate_before_the_full_matrix() {
    for path in [
        "xtask/src/performance.rs",
        "xtask/src/performance/tui.rs",
        "xtask/src/performance/resources.rs",
    ] {
        let paths = vec![path.into()];
        assert!(focus::tui_measurement_changed(&paths));
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "candidate");
        assert_eq!(plan.targets, ["render"]);
    }
    assert!(focus::tui_measurement_changed(&[
        "xtask/src/test_support/terminal.rs".into()
    ]));
    assert!(!focus::tui_measurement_changed(&[
        "xtask/tests/tui_measurement.rs".into(),
        "src/tui/tests.rs".into(),
        "docs/performance.md".into(),
        "xtask/src/notices/rust.rs".into()
    ]));
    for path in [
        "src/tui/mod.rs",
        "src/tui/preview.rs",
        "src/tui/graphics.rs",
    ] {
        assert!(focus::tui_measurement_changed(&[path.into()]));
    }
    let tests_only = vec!["xtask/tests/tui_measurement.rs".into()];
    assert_eq!(
        focus::select(&util::root(), &tests_only, "auto")
            .unwrap()
            .suite,
        "ci"
    );
    for path in ["src/render.rs", "src/render/backdrop.rs"] {
        assert!(focus::tui_measurement_changed(&[path.into()]));
    }
    let text_layers = focus::select(
        &util::root(),
        &["src/render.rs".into(), "tests/graphics.rs".into()],
        "auto",
    )
    .unwrap();
    assert_eq!(text_layers.suite, "render");
    for policy in [
        "resources/licenses/native/skia-wrapper-review.json",
        "resources/licenses/native/quickjs-review.json",
        "resources/licenses/native/upstream.json",
    ] {
        let mut paths = vec![
            "src/render.rs".into(),
            "src/render/backdrop.rs".into(),
            "xtask/src/notices/skia_wrapper.rs".into(),
            policy.into(),
        ];
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        assert_eq!(plan.suite, "render");
        assert_eq!(plan.targets, text_layers.targets);
        assert!(focus::tui_measurement_changed(&paths));
        for source in ["src/dynamic.rs", "src/mcp.rs", "src/tui/mod.rs"] {
            paths.push(source.into());
            assert_ne!(
                focus::select(&util::root(), &paths, "auto").unwrap().suite,
                "render"
            );
            paths.pop();
        }
    }
    assert!(
        text_layers
            .targets
            .iter()
            .any(|target| target == "graphics")
    );
    let commands = lazypencil_xtask::ci::commands::plan(
        &util::root(),
        &text_layers.suite,
        &text_layers.targets,
    )
    .unwrap();
    let graphics = commands
        .checks
        .iter()
        .find(|command| {
            command
                .windows(2)
                .any(|args| args == ["--test", "graphics"])
        })
        .unwrap();
    assert!(
        graphics
            .windows(2)
            .any(|args| args == ["--skip", focus::TEXT_LAYER_REPRODUCTION])
    );
    let mixed = vec!["src/store.rs".into(), "xtask/src/performance/tui.rs".into()];
    assert!(focus::tui_measurement_changed(&mixed));
    assert!(
        !["ci", "candidate"].contains(
            &focus::select(&util::root(), &mixed, "auto")
                .unwrap()
                .suite
                .as_str()
        )
    );
    let workflow =
        std::fs::read_to_string(util::root().join(".github/workflows/native.yml")).unwrap();
    assert!(
        workflow.find("native candidate\n").unwrap()
            < workflow.find("cargo xtask measure-tui").unwrap()
    );
    assert!(workflow.contains("if: inputs.phase == 'focus' && inputs.measure_tui"));
    let ci = std::fs::read_to_string(util::root().join(".github/workflows/ci.yml")).unwrap();
    assert!(ci.contains("measure_tui: ${{ needs.plan.outputs.measure_tui == 'true' }}"));
    assert!(ci.contains("needs: [plan, focus, shader-preprocessor]"));
}

#[test]
fn focus_keeps_the_existing_reproduction_scope_for_every_source_module() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/focus.json")).unwrap();
    for case in cases {
        let paths: Vec<String> = serde_json::from_value(case["paths"].clone()).unwrap();
        let actual = focus::select(&util::root(), &paths, "auto").unwrap();
        let mut expected = case["expected"].clone();
        expected["targets"]
            .as_array_mut()
            .unwrap()
            .sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        assert_eq!(serde_json::to_value(actual).unwrap(), expected, "{paths:?}");
    }
}

#[test]
fn browser_scheduler_changes_execute_the_actual_two_lanes_before_the_full_matrix() {
    let paths = [
        "xtask/src/ci/native/browser.rs",
        "xtask/src/ci/native.rs",
        "xtask/src/ci/focus.rs",
        "xtask/tests/parallel.rs",
        ".github/workflows/native.yml",
        "docs/performance.md",
    ]
    .map(str::to_owned);
    let plan = focus::select(&util::root(), &paths, "auto").unwrap();
    assert_eq!(plan.platform, "ubuntu-24.04");
    assert_eq!(plan.suite, "browser-ci");
    assert_eq!(plan.targets, focus::BROWSER_CI_TARGETS);
    let actual: std::collections::BTreeSet<_> = ci::native::browser::TEST_LANES
        .into_iter()
        .flatten()
        .copied()
        .chain(["agent"])
        .collect();
    assert_eq!(actual, focus::BROWSER_CI_TARGETS.iter().copied().collect());
    let mut mixed = paths.to_vec();
    mixed.push("src/render.rs".into());
    let mixed_plan = focus::select(&util::root(), &mixed, "auto").unwrap();
    assert_eq!(mixed_plan.suite, "render");
    assert!(mixed_plan.targets.iter().any(|target| target == "graphics"));
    assert!(focus::tui_measurement_changed(&mixed));
}

#[test]
fn font_sources_generators_and_pins_select_native_color_reproductions_before_the_full_matrix() {
    for path in [
        "tests/support/color_fonts.rs",
        "tests/support/real_emoji.rs",
        "tests/fixtures/fonts/emoji-upstream.json",
        "tests/fixtures/fonts/regular.ttf",
        "xtask/src/fixtures/fonts.rs",
        "xtask/src/fixtures/fonts/color.rs",
        "xtask/src/fixtures/emoji.rs",
    ] {
        assert_eq!(focus::ci_only(path), path.starts_with("xtask/"), "{path}");
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "graphics", "{path}");
        assert_eq!(plan.targets, ["font_sources"], "{path}");
        assert!(ci::commands::color_font_reproduction(&plan.suite, &plan.targets).is_some());
    }
    for paths in [
        vec!["src/text/fonts.rs"],
        vec!["src/text/container.rs", "src/shader.rs"],
        vec!["tests/support/real_emoji.rs", "src/shader.rs"],
    ] {
        let paths = paths.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        assert!(plan.targets.iter().any(|target| target == "font_sources"));
        assert!(ci::commands::color_font_reproduction(&plan.suite, &plan.targets).is_some());
        if paths.iter().any(|path| path.starts_with("src/text/")) {
            for target in ["text", "native_layout"] {
                assert!(plan.targets.iter().any(|actual| actual == target));
            }
        }
        if paths.iter().any(|path| path == "src/shader.rs") {
            assert_eq!(plan.suite, "dynamic");
            assert!(
                plan.targets
                    .iter()
                    .any(|target| target == "shader_uniforms")
            );
        }
    }
    // Preserve the existing priority of source-bound artifact reproductions
    // for mixed tool-only changes. The full native gate still checks fonts.
    for (path, expected) in [
        ("xtask/src/ci/cache.rs", "release"),
        ("xtask/src/dist/binary.rs", "candidate"),
    ] {
        let plan = focus::select(
            &util::root(),
            &["xtask/src/fixtures/emoji.rs".into(), path.into()],
            "auto",
        )
        .unwrap();
        assert_eq!(plan.suite, expected);
        assert_eq!(plan.targets, ["render"]);
    }
}

#[test]
fn path_identity_changes_reproduce_macos_alias_and_hardlink_behavior_first() {
    for path in ["src/paths.rs", "tests/paths.rs"] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.platform, "macos-15");
        assert_eq!(plan.suite, "core");
        assert_eq!(plan.targets, vec!["paths"]);
    }
}

#[test]
fn rust_pty_bridge_changes_select_the_stricter_supported_host_and_all_frontend_reproductions() {
    for path in [
        "tests/support/terminal/mod.rs",
        "tests/support/terminal/cases.rs",
        "tests/support/terminal/cases/conversations.rs",
    ] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.platform, "macos-15");
        assert_eq!(plan.suite, "graphics");
        let early = ci::commands::terminal_reproduction(&plan.suite, &plan.targets).unwrap();
        assert!(early.iter().any(|arg| arg == focus::TERMINAL_MODES));
        assert!(early.iter().any(|arg| arg == "--exact"));
        let commands = ci::commands::plan(&util::root(), &plan.suite, &plan.targets).unwrap();
        assert!(commands.checks.iter().any(|command| {
            command
                .windows(2)
                .any(|args| args == ["--skip", focus::TERMINAL_MODES])
        }));
        for target in ["tui", "user_config", "conversation_history"] {
            assert!(
                plan.targets.iter().any(|value| value == target),
                "{path}/{target}"
            );
        }
        let explicit = focus::select(&util::root(), &[path.into()], "ubuntu-24.04").unwrap();
        assert_eq!(explicit.platform, "ubuntu-24.04");
        assert_eq!(explicit.targets, plan.targets);
    }
}

#[test]
fn native_command_fixture_changes_reproduce_their_browser_and_script_consumers_first() {
    let plan = focus::select(
        &util::root(),
        &["xtask/src/test_support/command.rs".into()],
        "auto",
    )
    .unwrap();
    assert_eq!(plan.suite, "graphics");
    for target in ["runtime", "runtime_config", "scripts", "dynamic_render"] {
        assert!(plan.targets.iter().any(|value| value == target), "{target}");
    }
}

#[test]
fn generated_reference_changes_reproduce_the_real_script_binding_cases_first() {
    for path in [
        "src/project/generated_refs.rs",
        "tests/support/script_generated_overrides.rs",
    ] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "graphics");
        assert!(plan.targets.iter().any(|target| target == "script_scope"));
        assert!(!plan.preprocessor);
    }
}

#[test]
fn generated_browser_copy_changes_run_namespace_and_real_pixel_reproductions_first() {
    for path in [
        "src/project/browser_context.rs",
        "src/project/import/browsers.rs",
        "tests/support/script_browser_copy.rs",
    ] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.suite, "browser");
        for target in ["script_scope", "browser_readiness"] {
            assert!(
                plan.targets.iter().any(|value| value == target),
                "{path}/{target}"
            );
        }
        let commands = ci::commands::plan(&util::root(), &plan.suite, &plan.targets).unwrap();
        assert!(
            commands
                .checks
                .iter()
                .any(|command| command.iter().any(|arg| arg == focus::GENERATED_BROWSER))
        );
    }
    let paths = [
        "src/project/bindings.rs",
        "src/project/browser_context.rs",
        "src/project/generated_refs.rs",
        "src/project/import/browsers.rs",
        "src/project/import/scripts.rs",
        "src/project/import/scripts/composition.rs",
        "tests/support/script_browser_copy.rs",
        "tests/browser_readiness.rs",
        "tests/script_scope.rs",
        "xtask/src/ci/native.rs",
        "README.md",
    ]
    .map(str::to_owned);
    let plan = focus::select(&util::root(), &paths, "macos-15").unwrap();
    assert_eq!(plan.platform, "macos-15");
    assert_eq!(plan.suite, "browser");
    assert_eq!(
        plan.targets,
        ["browser_readiness", "project", "script_scope"]
    );
    let broad = focus::select(
        &util::root(),
        &[
            "src/project/browser_context.rs".into(),
            "src/dynamic.rs".into(),
        ],
        "auto",
    )
    .unwrap();
    assert_eq!(broad.suite, "dynamic");
    assert!(
        broad.targets.contains(&"shader_uniforms".into())
            && broad.targets.contains(&"dynamic_render".into())
    );
}

#[test]
fn tool_changes_only_bypass_focus_native_compilation_when_they_are_not_application_inputs() {
    for path in [
        "xtask/Cargo.lock",
        "docs/design.md",
        "tests/fixtures/fonts/README.md",
        ".github/workflows/native.yml",
    ] {
        assert!(focus::ci_only(path), "{path}");
        assert_eq!(
            focus::select(&util::root(), &[path.into()], "auto")
                .unwrap()
                .suite,
            "ci"
        );
    }
    for path in [
        "xtask/src/test_support/acp.rs",
        "xtask/src/fixtures/oracles/shapes.rs",
        "xtask/src/bin/test-helper.rs",
        "resources/guidance/components.md",
        "docs/input.json",
        "tests/fixtures/performance/shadow.pen",
        "tests/fixtures/fonts/input.md",
    ] {
        assert!(!focus::ci_only(path), "{path}");
    }
    assert!(
        focus::select(
            &util::root(),
            &[
                "src/project/bindings.rs".into(),
                "xtask/src/ci/cache.rs".into()
            ],
            "auto"
        )
        .unwrap()
        .targets
        .contains(&"script_scope".into())
    );
    assert!(focus::select(&util::root(), &[], "Windows").is_err());
}

#[test]
fn release_recipe_helpers_compile_on_one_platform_before_the_full_matrix() {
    for path in [
        "xtask/src/ci/cache.rs",
        "xtask/src/ci/native.rs",
        "xtask/src/ci/native/release.rs",
        "xtask/src/evidence/link_map.rs",
    ] {
        let plan = focus::select(
            &util::root(),
            &[
                path.into(),
                "resources/licenses/native/quickjs-review.json".into(),
            ],
            "auto",
        )
        .unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "release");
        assert_eq!(plan.targets, ["render"]);
        let commands = ci::commands::plan(&util::root(), &plan.suite, &plan.targets).unwrap();
        assert!(!commands.build.is_empty() && !commands.checks.is_empty());
    }
    let plan = focus::select(
        &util::root(),
        &[
            "src/project/generated_refs.rs".into(),
            "xtask/src/ci/native/release.rs".into(),
        ],
        "auto",
    )
    .unwrap();
    assert!(plan.targets.contains(&"script_scope".into()));
}

#[test]
fn candidate_packaging_changes_run_a_real_candidate_gate_before_the_matrix() {
    for path in [
        "xtask/src/dist.rs",
        "xtask/src/dist/binary.rs",
        "xtask/src/evidence.rs",
        "xtask/src/ci/native/candidate.rs",
        "xtask/src/notices/skia.rs",
        "xtask/src/notices/skia_wrapper.rs",
        "resources/licenses/native/skia-wrapper-review.json",
        "xtask/tests/binary_dist.rs",
        "xtask/tests/evidence.rs",
        ".github/workflows/release-evidence.yml",
    ] {
        let plan = focus::select(
            &util::root(),
            &[
                path.into(),
                "xtask/src/ci/native.rs".into(),
                "tests/fixtures/fonts/README.md".into(),
            ],
            "auto",
        )
        .unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "candidate");
        assert_eq!(plan.targets, ["render"]);
        let commands = ci::commands::plan(&util::root(), &plan.suite, &plan.targets).unwrap();
        assert!(!commands.build.is_empty() && !commands.checks.is_empty());
    }
}

#[test]
fn terminal_protocol_fixes_reproduce_in_real_emulators_before_the_full_matrix() {
    for path in [
        "src/tui/graphics.rs",
        "src/tui/terminal.rs",
        "src/tui/preview.rs",
    ] {
        let plan = focus::select(
            &util::root(),
            &[
                path.into(),
                "src/tui/mod.rs".into(),
                "src/tui/tests.rs".into(),
                "xtask/src/test_support/terminal.rs".into(),
                "xtask/src/terminal/hosted.rs".into(),
            ],
            "auto",
        )
        .unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "candidate");
        assert_eq!(plan.targets, ["render", "tui"]);
        let commands = ci::commands::plan(&util::root(), &plan.suite, &plan.targets).unwrap();
        assert!(!commands.build.is_empty() && !commands.checks.is_empty());
    }
}

#[test]
fn release_gate_rejects_every_incomplete_or_malformed_result() {
    let valid = json!({"native":{"result":"success"},"audit":{"result":"success"}});
    assert!(ci::require_success(&valid).is_ok());
    for job in ["native", "audit"] {
        for state in [
            json!("skipped"),
            json!("failure"),
            json!("cancelled"),
            json!("pending"),
            json!("SUCCESS"),
            Value::Null,
            json!(true),
        ] {
            let mut value = valid.clone();
            value[job]["result"] = state;
            assert!(ci::require_success(&value).is_err());
        }
    }
    for value in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"native":{"result":"success"}}),
        json!({"native":{"result":"success"},"audit":{"result":"success"},"extra":{}}),
    ] {
        assert!(ci::require_success(&value).is_err());
    }
    for value in [
        Value::Null,
        json!([]),
        json!("success"),
        json!({}),
        json!({"outputs":{}}),
    ] {
        let mut data = valid.clone();
        data["native"] = value;
        assert!(ci::require_success(&data).is_err());
    }
}

#[test]
fn parent_native_failures_prioritize_one_affected_platform_without_overriding_explicit_choices() {
    let job = |name: &str, state: &str| json!({"name":name,"conclusion":state});
    let mac = job("Full verification / macos-15 / verify", "failure");
    let unrelated = job("Dependency policy and RustSec", "failure");
    assert_eq!(focus::failed_platform(&json!({"jobs":[mac.clone(),unrelated.clone(),job("Full verification / ubuntu-24.04 / verify","success")]})).unwrap(),Some("macos-15".into()));
    assert_eq!(
        focus::failed_platform(
            &json!({"jobs":[mac.clone(),job("Focused reproduction / macos-15 / verify","failure")]})
        )
        .unwrap(),
        Some("macos-15".into())
    );
    assert_eq!(
        focus::failed_platform(
            &json!({"jobs":[mac,job("Full verification / ubuntu-24.04-arm / verify","timed_out")]})
        )
        .unwrap(),
        None
    );
    assert_eq!(
        focus::failed_platform(
            &json!({"jobs":[unrelated,job("Full verification / macos-15 / verify","cancelled")]})
        )
        .unwrap(),
        None
    );
    assert!(focus::failed_platform(&json!({"jobs":null})).is_err());
    assert_eq!(
        focus::retry_platform(std::path::Path::new("/nonexistent"), "ubuntu-24.04"),
        "ubuntu-24.04"
    );
}

fn flags() -> BTreeMap<String, String> {
    [
        ("CARGO_BUILD_JOBS", "1"),
        ("CARGO_INCREMENTAL", "0"),
        ("CARGO_PROFILE_DEV_DEBUG", "0"),
        ("CARGO_PROFILE_TEST_DEBUG", "0"),
    ]
    .map(|(k, v)| (k.into(), v.into()))
    .into()
}
const COMPILER: &str = "rustc 1.94.0\nrelease: 1.94.0\ncommit-hash: 4a4ef493e3a1488c6e321570238084b38948f6db\nhost: x86_64-unknown-linux-gnu\n";
const TREE: &str = "b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d";

#[test]
fn native_identity_binds_compiler_image_flags_and_dependency_tree_but_not_scheduling() {
    let original =
        cache::native_identity("ubuntu-24.04", "20261012.334.1", TREE, COMPILER, &flags()).unwrap();
    for (name, value) in [
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

struct ApplicationFixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
    directory: PathBuf,
    library: PathBuf,
    dep_info: PathBuf,
}
impl ApplicationFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let directory = temp.path().join("cache");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("Cargo.toml"),
            "[package]\nname='lazypencil'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(source.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        let entries = ["Cargo.toml", "src/lib.rs"]
            .map(|name| {
                let bytes = fs::read(source.join(name)).unwrap();
                Entry {
                    path: name.into(),
                    sha256: util::digest(&bytes),
                    bytes: bytes.len() as u64,
                    mode: 0o644,
                }
            })
            .into();
        let manifest = Manifest {
            format_version: 1,
            package: json!({}),
            source_date_epoch: 0,
            git: json!({"revision":"1111111111111111111111111111111111111111","dirty":false}),
            files: entries,
        };
        fs::write(
            source.join("SOURCE-MANIFEST.json"),
            util::encode(&manifest).unwrap(),
        )
        .unwrap();
        let fingerprint = target.join("debug/.fingerprint/lazypencil-1234567890abcdef");
        fs::create_dir_all(&fingerprint).unwrap();
        fs::write(fingerprint.join("lib-lazypencil.json"), "{}").unwrap();
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        let library = target.join("debug/deps/liblazypencil-1234567890abcdef.rlib");
        fs::write(&library, b"compiled fixture library").unwrap();
        let dep_info = target.join("debug/deps/lazypencil-1234567890abcdef.d");
        fs::write(
            &dep_info,
            format!("library: {}/src/lib.rs\n", source.display()),
        )
        .unwrap();
        Self {
            _temp: temp,
            source,
            target,
            directory,
            library,
            dep_info,
        }
    }
    fn identity(&self) -> cache::ApplicationIdentity {
        cache::application_identity(&self.source, &self.target, "native-v2-ubuntu-24.04-fixture")
            .unwrap()
    }
    fn stage(&self) {
        cache::stage(
            &self.source,
            &self.target,
            &self.directory,
            &self.identity(),
        )
        .unwrap();
    }
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
        assert_eq!(
            report["checkpoint_prefix"],
            format!("{}-", identity.checkpoint)
        );
        assert_eq!(
            report["checkpoint"],
            format!("{}-402-{attempt}", identity.checkpoint)
        );
        let outputs = fs::read_to_string(output).unwrap();
        for key in [
            "primary",
            "checkpoint",
            "checkpoint_prefix",
            "checkpoint_legacy",
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
fn application_reuse_checks_all_outputs_before_overlaying_and_never_reuses_images_or_dependencies()
{
    let f = ApplicationFixture::new();
    fs::write(f.target.join("release-smoke.png"), b"old evidence").unwrap();
    fs::write(
        f.library
            .parent()
            .unwrap()
            .join("libdependency-1234567890abcdef.rlib"),
        b"dependency",
    )
    .unwrap();
    f.stage();
    fs::remove_dir_all(&f.target).unwrap();
    let report = cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap();
    assert_eq!(report["files"], 3);
    assert_eq!(fs::read(&f.library).unwrap(), b"compiled fixture library");
    assert!(!f.target.join("release-smoke.png").exists());
    fs::remove_dir_all(&f.target).unwrap();
    fs::write(
        f.directory
            .join("files/debug/deps/liblazypencil-1234567890abcdef.rlib"),
        b"tampered",
    )
    .unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    assert!(!f.target.exists());
}

#[test]
fn release_cache_keeps_the_mapped_executable_and_rejects_partial_or_changed_pairs() {
    for (binary, map) in [(true, false), (false, true), (true, true)] {
        let f = ApplicationFixture::new();
        fs::create_dir_all(f.target.join("release")).unwrap();
        let executable = f.target.join("release/lazypencil");
        let map_path = lazypencil_xtask::evidence::link_map::path(&f.target);
        if binary {
            fs::write(&executable, b"release executable").unwrap();
        }
        if map {
            fs::write(&map_path, b"original map").unwrap();
        }
        let staged = cache::stage(&f.source, &f.target, &f.directory, &f.identity());
        if binary != map {
            assert!(staged.is_err());
            continue;
        }
        staged.unwrap();
        fs::remove_dir_all(&f.target).unwrap();
        cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap();
        assert_eq!(fs::read(&executable).unwrap(), b"release executable");
        assert_eq!(fs::read(&map_path).unwrap(), b"original map");
        fs::remove_dir_all(&f.target).unwrap();
        let manifest_path = f.directory.join("manifest.json");
        let original: Value = util::json(&manifest_path).unwrap();
        let mut partial = original.clone();
        partial["files"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["path"] != "release/lazypencil.map");
        fs::write(&manifest_path, util::encode(&partial).unwrap()).unwrap();
        assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
        assert!(!f.target.exists());
        fs::write(manifest_path, util::encode(&original).unwrap()).unwrap();
        fs::write(
            f.directory.join("files/release/lazypencil.map"),
            b"changed map",
        )
        .unwrap();
        assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
        assert!(!f.target.exists());
    }
}

#[test]
fn application_reuse_rejects_changed_sources_external_dep_info_and_compiled_excluded_files() {
    for dependency in [
        "outside.rs",
        ".github/workflows/ci.yml",
        "tests/fixtures/fonts/README.md",
    ] {
        let f = ApplicationFixture::new();
        let path = if dependency == "outside.rs" {
            f.source.parent().unwrap().join(dependency)
        } else {
            f.source.join(dependency)
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"input").unwrap();
        if dependency != "outside.rs" {
            let manifest_path = f.source.join(dist::MANIFEST);
            let mut manifest: Manifest = util::json(&manifest_path).unwrap();
            manifest.files.push(Entry {
                path: dependency.into(),
                bytes: 5,
                sha256: util::digest(b"input"),
                mode: 0o644,
            });
            fs::write(manifest_path, util::encode(&manifest).unwrap()).unwrap();
        }
        fs::write(&f.dep_info, format!("library: {}\n", path.display())).unwrap();
        assert!(cache::stage(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    }
    let f = ApplicationFixture::new();
    f.stage();
    fs::write(f.source.join("src/lib.rs"), b"changed").unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    for name in [
        "../outside",
        "/tmp/outside",
        "debug/deps/../../outside",
        "debug/build/anything",
        "release-smoke.png",
    ] {
        assert!(!cache::allowed(name));
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

#[test]
fn independent_oracle_changes_test_all_consumers_without_rebuilding_the_wasm_component() {
    for path in [
        "xtask/src/fixtures/oracles/shapes.rs",
        "xtask/src/fixtures/oracles/shaders.rs",
        "tests/fixtures/render-corpus/shader-logical-coordinates/logical-coordinates.pen",
        "tests/fixtures/pencil-0.3.9/triangle.png",
    ] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.suite, "dynamic", "{path}");
        assert!(
            !plan.preprocessor,
            "Native-only corpus is not a Wasm compile input"
        );
        assert_eq!(
            plan.targets,
            ["graphics", "script_render", "scripts", "shader_uniforms"]
        );
    }
    let relocated_abi_fixture = focus::select(
        &util::root(),
        &[
            "xtask/src/fixtures/oracles/shaders.rs".into(),
            "tests/shader_uniforms.rs".into(),
            "tests/support/shader_preprocessor.mjs".into(),
        ],
        "auto",
    )
    .unwrap();
    assert_eq!(relocated_abi_fixture.suite, "dynamic");
    assert_eq!(
        relocated_abi_fixture.targets,
        ["graphics", "script_render", "scripts", "shader_uniforms"]
    );
    assert!(relocated_abi_fixture.preprocessor);
}
#[test]
fn application_reuse_rejects_same_mtime_mutations_and_unknown_generated_or_custom_build_inputs() {
    for update_manifest in [false, true] {
        let f = ApplicationFixture::new();
        f.stage();
        let path = f.source.join("src/lib.rs");
        let metadata = fs::metadata(&path).unwrap();
        fs::write(&path, b"pub fn changed() {}\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(metadata.modified().unwrap())
                    .set_accessed(metadata.accessed().unwrap()),
            )
            .unwrap();
        if update_manifest {
            let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
            let entry = manifest
                .files
                .iter_mut()
                .find(|e| e.path == "src/lib.rs")
                .unwrap();
            entry.sha256 = util::file_digest(&path).unwrap();
            entry.bytes = fs::metadata(path).unwrap().len();
            fs::write(
                f.source.join(dist::MANIFEST),
                util::encode(&manifest).unwrap(),
            )
            .unwrap();
        }
        assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
    }
    let f = ApplicationFixture::new();
    let review = "resources/licenses/native/quickjs-review.json";
    let path = f.source.join(review);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"{}\n").unwrap();
    let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
    manifest.files.push(Entry {
        path: review.into(),
        bytes: 3,
        sha256: util::file_digest(&path).unwrap(),
        mode: 0o644,
    });
    fs::write(
        f.source.join(dist::MANIFEST),
        util::encode(&manifest).unwrap(),
    )
    .unwrap();
    for path in ["SOURCE-MANIFEST.json", review] {
        fs::write(
            &f.dep_info,
            format!("library: {}/{path}\n", f.source.display()),
        )
        .unwrap();
        assert!(
            cache::stage(&f.source, &f.target, &f.directory, &f.identity()).is_err(),
            "Excluded compiler input: {path}"
        );
    }
    fs::write(f.source.join("build.rs"), b"fn main() {}\n").unwrap();
    assert!(
        cache::application_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-fixture")
            .is_err()
    );
}
#[test]
fn escaped_space_inputs_are_bound_and_compile_time_environment_changes_cannot_reuse_outputs() {
    let f = ApplicationFixture::new();
    let path = f.source.join("resources/guidance/a guide.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"Embedded space\n").unwrap();
    let mut manifest: Manifest = util::json(&f.source.join(dist::MANIFEST)).unwrap();
    manifest.files.push(Entry {
        path: "resources/guidance/a guide.md".into(),
        bytes: 15,
        sha256: util::file_digest(&path).unwrap(),
        mode: 0o644,
    });
    fs::write(
        f.source.join(dist::MANIFEST),
        util::encode(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        &f.dep_info,
        format!(
            "library: {}/resources/guidance/a\\ guide.md\n",
            f.source.display()
        ),
    )
    .unwrap();
    f.stage();
    fs::remove_dir_all(&f.target).unwrap();
    assert!(
        cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap()["reused"]
            == true
    );
    // Test environment binding in separate processes rather than mutating global test state.
    fs::remove_dir_all(&f.directory).unwrap();
    fs::write(
        &f.dep_info,
        format!(
            "library: {}/src/lib.rs\n# env-dep:EXAMPLE_BUILD_SETTING=one\n",
            f.source.display()
        ),
    )
    .unwrap();
    let invoke = |operation: &str, value: &str| {
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .env("EXAMPLE_BUILD_SETTING", value)
            .args(["ci", "cache", "application", operation, "--source"])
            .arg(&f.source)
            .arg("--target")
            .arg(&f.target)
            .arg("--directory")
            .arg(&f.directory)
            .args(["--native-key", "native-v2-ubuntu-24.04-fixture"])
            .output()
            .unwrap()
    };
    let staged = invoke("stage", "one");
    assert!(
        staged.status.success(),
        "{}",
        String::from_utf8_lossy(&staged.stderr)
    );
    let restored = invoke("restore", "two");
    assert!(!restored.status.success());
    assert!(String::from_utf8_lossy(&restored.stderr).contains("environment"));
}
#[cfg(unix)]
#[test]
fn cache_symlinks_cannot_redirect_source_or_overlay_destinations() {
    use std::os::unix::fs::symlink;
    for location in ["source", "cache", "destination"] {
        let f = ApplicationFixture::new();
        f.stage();
        let external = f._temp.path().join("external");
        match location {
            "source" => {
                let original = f.source.join("src/lib.rs");
                fs::rename(&original, &external).unwrap();
                symlink(&external, original).unwrap();
            }
            "cache" => {
                let original = f
                    .directory
                    .join("files/debug/deps/liblazypencil-1234567890abcdef.rlib");
                fs::rename(&original, &external).unwrap();
                symlink(&external, original).unwrap();
            }
            _ => {
                fs::create_dir(&external).unwrap();
                fs::remove_dir_all(&f.target).unwrap();
                fs::create_dir(&f.target).unwrap();
                symlink(&external, f.target.join("debug")).unwrap();
            }
        }
        assert!(
            cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err(),
            "{location}"
        );
        if external.is_dir() {
            assert_eq!(fs::read_dir(external).unwrap().count(), 0);
        }
    }
}

#[test]
fn warm_dependencies_put_lints_before_codegen_while_cold_rendering_keeps_its_build_budget() {
    for suite in [
        "core",
        "conversation",
        "graphics",
        "copy",
        "dynamic",
        "shader",
    ] {
        assert!(focus::build_before_lint(suite, ""));
        assert!(!focus::build_before_lint(suite, "native-v2-hosted"));
        assert!(!focus::build_before_lint(
            suite,
            "checkpoint-native-v2-hosted"
        ));
        assert!(!focus::build_before_lint(
            suite,
            "checkpoint-native-v2-hosted-full-402-1"
        ));
        assert_eq!(
            focus::build_before_lint(suite, "core-native-v2-hosted"),
            !["core", "conversation"].contains(&suite)
        );
    }
}
