use lazypencil_xtask::{
    ci::{self, cache, focus},
    dist::{self, Entry, Manifest},
    util,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

#[test]
fn headed_browser_fixture_changes_reproduce_their_real_consumer_before_the_matrix() {
    for path in [
        "tests/support/browser_profiles.rs",
        "tests/support/page_wait_reference.rs",
        "tests/fixtures/browser-ui/chromium.json",
        "xtask/src/fixtures/browser_ui.rs",
    ] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.suite, "dynamic", "{path}");
        assert!(plan.targets.iter().any(|name| name == "browser_readiness"));
        let commands = ci::commands::plan(&util::root(), &plan.suite, &plan.targets).unwrap();
        let first = commands
            .checks
            .iter()
            .position(|args| {
                args.iter().any(|arg| arg == focus::HEADED_LOGIN)
                    && !args.iter().any(|arg| arg == "--skip")
            })
            .unwrap();
        let other = commands
            .checks
            .iter()
            .position(|args| {
                args.iter().any(|arg| arg == focus::GENERATED_BROWSER)
                    && !args.iter().any(|arg| arg == "--skip")
            })
            .unwrap();
        assert!(
            first < other,
            "The visible login must fail before unrelated long browser studies"
        );
        assert!(commands.checks[first].iter().any(|arg| arg == "--ignored"));
        assert!(commands.checks[first].iter().any(|arg| arg == "--exact"));
        assert_eq!(
            commands
                .checks
                .iter()
                .filter(|args| args
                    .windows(2)
                    .any(|pair| pair == ["--skip", focus::HEADED_LOGIN]))
                .count(),
            1
        );
    }
}

#[test]
fn installer_only_changes_reproduce_without_native_features_and_mixed_changes_stay_broad() {
    let root = util::root();
    let paths: Vec<String> = [
        "src/runtime.rs",
        "tests/runtime.rs",
        "xtask/src/ci/focus.rs",
        "xtask/tests/ci.rs",
        ".github/actions/xtask/action.yml",
        "xtask/src/ci/cache.rs",
        "xtask/tests/checkpoints.rs",
        "xtask/tests/compatible_cache.rs",
        "xtask/tests/fixtures/focus.json",
        "docs/fonts.md",
        "docs/runtimes.md",
        "docs/implementation-status.md",
        "docs/performance.md",
    ]
    .map(str::to_owned)
    .to_vec();
    let plan = focus::select(&root, &paths, "auto").unwrap();
    assert_eq!(plan.platform, "ubuntu-24.04");
    assert_eq!(plan.suite, "core");
    assert_eq!(plan.targets, ["runtime"]);
    let commands = ci::commands::plan(&root, &plan.suite, &plan.targets).unwrap();
    assert!(
        commands
            .build
            .iter()
            .chain(&commands.checks)
            .all(|command| command.iter().any(|arg| arg == "--no-default-features"))
    );
    let mut shared_lock = paths.clone();
    shared_lock.push("src/store.rs".into());
    let shared_lock = focus::select(&root, &shared_lock, "auto").unwrap();
    assert_eq!(shared_lock.suite, "core");
    assert_eq!(
        shared_lock.targets,
        ["document_store", "operations_cli", "project", "runtime"]
    );
    for extra in [
        "src/cdp.rs",
        "src/runtime/diagnostics.rs",
        "tests/dynamic_render.rs",
    ] {
        let mut mixed = paths.clone();
        mixed.push(extra.into());
        assert_ne!(focus::select(&root, &mixed, "auto").unwrap().suite, "core");
    }
}

#[test]
fn hosted_terminal_changes_select_linux_before_shared_bsd_bridge_checks() {
    for path in [
        "src/tui/graphics.rs",
        "src/tui/terminal.rs",
        "src/tui/preview.rs",
    ] {
        let paths = vec![
            "xtask/src/terminal/hosted.rs".into(),
            path.into(),
            "src/tui/mod.rs".into(),
            "src/tui/tests.rs".into(),
            "xtask/src/test_support/terminal.rs".into(),
        ];
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        assert_eq!(plan.platform, "ubuntu-24.04");
        assert_eq!(plan.suite, "candidate");
        assert_eq!(plan.targets, ["render", "tui", "dynamic_render"]);
        let override_plan = focus::select(&util::root(), &paths, "macos-15").unwrap();
        assert_eq!(override_plan.platform, "macos-15");
    }
}

#[test]
fn changed_measurements_use_one_source_bound_candidate_before_the_full_matrix() {
    for path in [
        "xtask/src/performance.rs",
        "xtask/src/performance/tui.rs",
        "xtask/src/performance/resources.rs",
        "xtask/src/evidence/release_build.rs",
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
    let helper = focus::select(
        &util::root(),
        &["xtask/src/test_support/terminal.rs".into()],
        "auto",
    )
    .unwrap();
    assert_eq!(helper.platform, "macos-15");
    assert_eq!(helper.suite, "candidate");
    assert_eq!(helper.targets, ["render", "tui", "dynamic_render"]);
    assert!(!focus::tui_measurement_changed(&[
        "xtask/tests/tui_measurement.rs".into(),
        "src/tui/tests.rs".into(),
        "docs/performance.md".into(),
        "xtask/src/notices/rust.rs".into()
    ]));
    let cpu_profile_only = vec!["xtask/src/performance/profile.rs".into()];
    assert!(!focus::tui_measurement_changed(&cpu_profile_only));
    assert_eq!(
        focus::select(&util::root(), &cpu_profile_only, "auto")
            .unwrap()
            .suite,
        "ci"
    );
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
    for path in ["src/render.rs", "src/render/backdrop.rs", "src/layout.rs"] {
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
    let preview = focus::select(
        &util::root(),
        &["src/render.rs".into(), "src/tui/preview.rs".into()],
        "auto",
    )
    .unwrap();
    assert_eq!(preview.suite, "render");
    let mut expected = text_layers.targets.clone();
    expected.push("tui".into());
    assert_eq!(preview.targets, expected);
    assert!(ci::commands::terminal_reproduction(&preview.suite, &preview.targets).is_some());
    let combined = ci::commands::plan(&util::root(), &preview.suite, &preview.targets).unwrap();
    assert_eq!(combined.checks.len(), 3);
    assert!(
        combined.checks[0]
            .windows(2)
            .any(|pair| pair == ["--test", "tui"])
    );
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
fn release_evidence_restores_only_mapped_release_outputs_before_cargo_verification() {
    let workflow =
        fs::read_to_string(util::root().join(".github/workflows/release-evidence.yml")).unwrap();
    let identify = workflow
        .find("application identify --release-only")
        .unwrap();
    let restore = workflow
        .find("application restore-or-prune --release-only")
        .unwrap();
    let compile = workflow
        .find("cargo xtask ci native evidence-build")
        .unwrap();
    assert!(identify < restore && restore < compile);
    assert!(workflow.contains("steps.release-key.outputs.primary"));
    assert!(workflow.contains("lazypencil-release-cache"));
    assert!(!workflow.contains("lazypencil-application-cache"));
    for required in [
        "cargo xtask evidence",
        "cargo xtask ci native candidate",
        "cargo xtask terminal --candidate",
        "cargo xtask ci provenance",
        "cargo xtask measure-tui",
    ] {
        assert!(workflow.contains(required), "{required}");
    }
}

#[test]
fn static_camera_and_pixel_transfer_changes_use_the_complete_tui_candidate_gate() {
    let paths = [
        "src/tui/camera.rs",
        "src/tui/mod.rs",
        "src/tui/terminal.rs",
        "src/render/preview.rs",
        "src/tui/tests.rs",
        "tests/support/terminal/cases.rs",
        "tests/support/terminal/mod.rs",
        "xtask/src/performance/tui.rs",
        "xtask/src/test_support/terminal.rs",
        "xtask/src/ci/commands.rs",
        "xtask/src/ci/focus.rs",
        "xtask/tests/terminal.rs",
        "xtask/tests/process.rs",
        "xtask/tests/ci.rs",
        "xtask/tests/fixtures/focus-commands.json",
        "docs/performance.md",
        "docs/tui.md",
        "docs/implementation-status.md",
    ]
    .map(str::to_owned)
    .to_vec();
    let plan = focus::select(&util::root(), &paths, "auto").unwrap();
    assert_eq!(plan.platform, "macos-15");
    assert_eq!(plan.suite, "candidate");
    assert_eq!(plan.targets, ["render", "tui", "dynamic_render"]);
    assert!(focus::tui_measurement_changed(&paths));
    assert!(focus::native_clipboard_changed(&paths));
    assert!(!focus::runtime_scheduler_changed(&paths));
    for shared in [
        "src/render.rs",
        "src/render/backdrop.rs",
        "src/dynamic/pages.rs",
        "src/agent.rs",
    ] {
        let mut mixed = paths.clone();
        mixed.push(shared.into());
        assert_ne!(
            focus::select(&util::root(), &mixed, "auto").unwrap().suite,
            "candidate",
            "{shared}"
        );
    }
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
    let artifact_only = ["xtask/src/ci/native/browser/evidence.rs".into()];
    assert!(!focus::runtime_scheduler_changed(&artifact_only));
    let artifact_plan = focus::select(&util::root(), &artifact_only, "auto").unwrap();
    assert_eq!(artifact_plan.suite, "ci");
    assert!(artifact_plan.targets.is_empty());
    assert!(focus::runtime_scheduler_changed(&[
        "xtask/src/ci/native.rs".into()
    ]));
    assert!(focus::runtime_scheduler_changed(&[
        "xtask/src/ci/native/browser.rs".into()
    ]));
    assert!(!focus::runtime_scheduler_changed(&[
        "tests/graphics.rs".into(),
        "xtask/src/ci/cache.rs".into()
    ]));
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
fn normal_scheduler_changes_reproduce_the_complete_catalog_before_the_matrix() {
    let paths = [
        "xtask/src/ci/native/normal.rs",
        "xtask/src/ci/native.rs",
        "xtask/src/ci/focus.rs",
        "xtask/src/ci/mod.rs",
        ".github/workflows/native.yml",
        "README.md",
    ]
    .map(str::to_owned);
    let plan = focus::select(&util::root(), &paths, "auto").unwrap();
    assert_eq!(plan.platform, "ubuntu-24.04");
    assert_eq!(plan.suite, "normal-ci");
    assert!(
        plan.targets.is_empty(),
        "The catalog builder must include every default target"
    );
    let mut mixed = paths.to_vec();
    mixed.push("src/render.rs".into());
    assert_eq!(
        focus::select(&util::root(), &mixed, "auto").unwrap().suite,
        "render"
    );
    for scheduler in [
        "xtask/src/ci/native/browser.rs",
        "xtask/src/ci/native/management.rs",
        "xtask/src/ci/native/parallel.rs",
    ] {
        let mut mixed = paths.to_vec();
        mixed.extend([scheduler.into(), "xtask/src/test_support/acp.rs".into()]);
        let plan = focus::select(&util::root(), &mixed, "auto").unwrap();
        assert_eq!(plan.suite, "normal-ci");
        assert!(plan.targets.is_empty());
        assert!(focus::runtime_scheduler_changed(&mixed));
    }
    let mut helper = paths.to_vec();
    helper.push("xtask/src/test_support/acp.rs".into());
    let plan = focus::select(&util::root(), &helper, "auto").unwrap();
    assert_eq!(plan.suite, "normal-ci");
    assert!(plan.targets.is_empty());
}

#[test]
fn macos_kitty_fixtures_reproduce_on_macos_before_preserving_linux_terminal_coverage() {
    for path in [
        "xtask/src/fixtures/kitty_ui.rs",
        "xtask/src/terminal/macos.rs",
    ] {
        let paths = [
            path,
            "xtask/src/terminal/hosted.rs",
            "xtask/src/ci/cache.rs",
        ]
        .map(str::to_owned);
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        assert_eq!(plan.platform, "macos-15");
        assert_eq!(plan.suite, "candidate");
        assert!(plan.targets.iter().any(|target| target == "tui"));
        assert!(ci::commands::terminal_reproduction(&plan.suite, &plan.targets).is_some());
        assert_eq!(
            focus::terminal_reproduction_stages(&paths, &plan),
            (false, true)
        );
    }
}

#[test]
fn terminal_harness_changes_keep_their_reproduction_in_mixed_application_changes() {
    let paths = ["xtask/src/terminal/hosted.rs", "src/cdp/tabs.rs"].map(str::to_owned);
    let plan = focus::select(&util::root(), &paths, "auto").unwrap();
    assert_ne!(plan.suite, "candidate");
    assert!(focus::terminal_harness_changed(&paths));
    assert_eq!(
        focus::terminal_reproduction_stages(&paths, &plan),
        (true, false)
    );
    for (platform, suite, expected) in [
        ("macos-15", "dynamic", (false, true)),
        ("ubuntu-24.04", "core", (false, true)),
        ("ubuntu-24.04", "ci", (false, true)),
        ("ubuntu-24.04-arm", "conversation", (false, true)),
    ] {
        let mut selected = plan.clone();
        selected.platform = platform.into();
        selected.suite = suite.into();
        assert_eq!(
            focus::terminal_reproduction_stages(&paths, &selected),
            expected
        );
    }
    assert!(focus::terminal_harness_changed(&[
        "xtask/tests/terminal_raster.rs".into()
    ]));
    assert!(!focus::terminal_harness_changed(&[
        "docs/tui.md".into(),
        "src/cdp/tabs.rs".into()
    ]));
    assert_eq!(
        focus::terminal_reproduction_stages(&["docs/tui.md".into()], &plan),
        (false, false)
    );
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
        let paths = [
            path,
            "tests/tui.rs",
            ".github/workflows/native.yml",
            "xtask/src/ci/cache.rs",
            "xtask/src/ci/cache/compatible.rs",
            "xtask/tests/checkpoints.rs",
            "xtask/tests/compatible_cache.rs",
        ]
        .map(str::to_owned);
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
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
        let explicit = focus::select(&util::root(), &paths, "ubuntu-24.04").unwrap();
        assert_eq!(explicit.platform, "ubuntu-24.04");
        assert_eq!(explicit.targets, plan.targets);
    }
    for companion in [
        "tests/user_agent_config.rs",
        "tests/config.rs",
        "tests/graphics.rs",
    ] {
        let plan = focus::select(
            &util::root(),
            &[
                "src/conversation/mod.rs".into(),
                "tests/support/terminal/cases.rs".into(),
                companion.into(),
            ],
            "auto",
        )
        .unwrap();
        let name = std::path::Path::new(companion)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(plan.suite, "graphics");
        assert!(plan.targets.iter().any(|target| target == name));
        assert!(plan.targets.iter().any(|target| target == "tui"));
        assert!(plan.targets.iter().any(|target| target == "agent"));
        assert!(!plan.targets.iter().any(|target| target == "agent_workflow"));
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
    let mut with_guides = paths.to_vec();
    with_guides.extend(["src/guidance.rs", "resources/guidance/components.md"].map(str::to_owned));
    let guided = focus::select(&util::root(), &with_guides, "auto").unwrap();
    assert_eq!(guided.suite, "browser");
    assert_eq!(
        guided.targets,
        ["browser_readiness", "project", "script_scope", "guidance"]
    );
    let guided_commands =
        ci::commands::plan(&util::root(), &guided.suite, &guided.targets).unwrap();
    assert!(
        guided_commands
            .build
            .iter()
            .flatten()
            .any(|arg| arg == "guidance")
    );
    assert!(
        guided_commands.checks[0]
            .windows(2)
            .any(|pair| pair == ["--test", "guidance"])
    );
    with_guides.push("src/guidance/project.rs".into());
    assert_ne!(
        focus::select(&util::root(), &with_guides, "auto")
            .unwrap()
            .suite,
        "browser"
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
        "xtask/src/terminal/mod.rs",
        "xtask/src/terminal/hosted.rs",
        "xtask/src/terminal/raster.rs",
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
fn terminal_protocol_and_observer_changes_reproduce_the_bsd_browser_consumer_first() {
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
            ],
            "auto",
        )
        .unwrap();
        assert_eq!(plan.platform, "macos-15");
        assert_eq!(plan.suite, "candidate");
        assert_eq!(plan.targets, ["render", "tui", "dynamic_render"]);
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
fn native_build_scheduling_preserves_compiler_arguments_and_test_execution() {
    let words = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    for args in [
        words(&["cargo", "build", "--locked"]),
        words(&["cargo", "test", "--locked", "--no-run"]),
        words(&[
            "cargo",
            "rustc",
            "--release",
            "--",
            "-C",
            "link-arg=-Wl,-Map=/tmp/output.map",
        ]),
    ] {
        let scheduled = ci::commands::build_jobs(&args, true);
        assert_eq!(&scheduled[..2], &args[..2]);
        assert_eq!(&scheduled[2..4], &["--jobs", "2"]);
        assert_eq!(&scheduled[4..], &args[2..]);
        assert_eq!(ci::commands::build_jobs(&args, false), args);
        assert_eq!(ci::commands::build_jobs(&scheduled, true), scheduled);
    }
    for args in [
        &["cargo", "test", "--locked", "--", "--no-run"][..],
        &["cargo", "test", "--locked", "--", "--test-threads=2"],
        &["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
        &["cargo", "build", "--jobs=1"],
        &["cargo", "build", "-j1"],
        &["cargo", "build", "-j", "1"],
        &["cargo", "build", "--jobs", "1"],
        &["cargo"],
        &[],
        &["/tmp/cargo", "build"],
    ] {
        let args = words(args);
        assert_eq!(ci::commands::build_jobs(&args, true), args);
    }
}

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
    fn write_source(&self, name: &str, bytes: &[u8]) {
        let path = self.source.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let mut manifest: Manifest = util::json(&self.source.join(dist::MANIFEST)).unwrap();
        manifest.files.retain(|entry| entry.path != name);
        manifest.files.push(Entry {
            path: name.into(),
            bytes: bytes.len() as u64,
            sha256: util::file_digest(&path).unwrap(),
            mode: 0o644,
        });
        fs::write(
            self.source.join(dist::MANIFEST),
            util::encode(&manifest).unwrap(),
        )
        .unwrap();
    }
    fn release(&self) -> cache::ApplicationIdentity {
        let directory = self
            .target
            .join("release/.fingerprint/lazypencil-1234567890abcdef");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("bin-lazypencil.json"), b"{}").unwrap();
        let directory = self.target.join("release/deps");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("lazypencil-1234567890abcdef"), b"executable").unwrap();
        fs::write(
            directory.join("lazypencil-1234567890abcdef.d"),
            format!("binary: {}/src/lib.rs\n", self.source.display()),
        )
        .unwrap();
        fs::write(self.target.join("release/lazypencil"), b"executable").unwrap();
        fs::write(
            lazypencil_xtask::evidence::link_map::path(&self.target),
            b"mapped executable",
        )
        .unwrap();
        cache::release_identity(&self.source, &self.target, "native-v2-ubuntu-24.04-fixture")
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
fn release_reuse_survives_test_changes_without_restoring_test_outputs() {
    let f = ApplicationFixture::new();
    f.write_source("tests/render.rs", b"original test");
    let all = f.identity();
    let release = f.release();
    assert_ne!(all.fingerprint, release.fingerprint);
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    f.write_source("tests/render.rs", b"expanded regression");
    f.write_source("xtask/src/test_support/fixture.rs", b"new fixture");
    let current =
        cache::release_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-fixture").unwrap();
    assert_eq!(current.primary, release.primary);
    assert_ne!(f.identity().primary, all.primary);
    fs::remove_dir_all(f.target.join("release")).unwrap();
    fs::write(&f.library, b"current debug output").unwrap();
    let value = cache::restore(&f.source, &f.target, &f.directory, &current).unwrap();
    assert_eq!(value["reused"], true);
    assert_eq!(fs::read(&f.library).unwrap(), b"current debug output");
    let manifest: Value = util::json(&f.directory.join("manifest.json")).unwrap();
    assert!(
        manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["path"].as_str().unwrap().starts_with("release/"))
    );
    assert_eq!(
        fs::read(f.target.join("release/lazypencil")).unwrap(),
        b"executable"
    );
}

#[test]
fn release_reuse_rejects_source_resource_recipe_and_scope_changes() {
    for name in ["src/lib.rs", "resources/guidance/design.md", "Cargo.toml"] {
        let f = ApplicationFixture::new();
        if name != "Cargo.toml" {
            f.write_source(name, b"original input");
        }
        let release = f.release();
        cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
        if name == "Cargo.toml" {
            let original = fs::read_to_string(f.source.join(name)).unwrap();
            f.write_source(
                name,
                format!("{original}\n[profile.release]\nlto='thin'\n").as_bytes(),
            );
        } else {
            f.write_source(name, b"changed input");
        }
        let current =
            cache::release_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-fixture")
                .unwrap();
        assert!(
            cache::restore(&f.source, &f.target, &f.directory, &current).is_err(),
            "{name}"
        );
    }
    let f = ApplicationFixture::new();
    let release = f.release();
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    let current =
        cache::release_identity(&f.source, &f.target, "native-v2-ubuntu-24.04-other-build")
            .unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &current).is_err());
    assert!(cache::restore(&f.source, &f.target, &f.directory, &f.identity()).is_err());
}

#[test]
fn release_cache_rejects_compiled_test_inputs_and_debug_entries_before_overlay() {
    let f = ApplicationFixture::new();
    f.write_source("tests/embedded.txt", b"embedded fixture");
    let release = f.release();
    let dep = f.target.join("release/deps/lazypencil-1234567890abcdef.d");
    fs::write(
        &dep,
        format!("binary: {}/tests/embedded.txt\n", f.source.display()),
    )
    .unwrap();
    assert!(cache::stage(&f.source, &f.target, &f.directory, &release).is_err());
    f.release();
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    let path = f.directory.join("manifest.json");
    let mut report: Value = util::json(&path).unwrap();
    report["files"].as_array_mut().unwrap().push(json!({"path":"debug/deps/liblazypencil-1234567890abcdef.rlib","sha256":"invalid","bytes":1,"mode":420}));
    fs::write(&path, util::encode(&report).unwrap()).unwrap();
    fs::remove_dir_all(f.target.join("release")).unwrap();
    assert!(cache::restore(&f.source, &f.target, &f.directory, &release).is_err());
    assert!(!f.target.join("release").exists());
}

#[test]
fn rejected_release_cli_cache_does_not_prune_verified_debug_outputs() {
    let f = ApplicationFixture::new();
    f.write_source(
        "Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"lazypencil\"\nversion = \"0.1.0\"\n",
    );
    let release = f.release();
    cache::stage(&f.source, &f.target, &f.directory, &release).unwrap();
    fs::write(
        f.directory.join("files/release/lazypencil.map"),
        b"tampered",
    )
    .unwrap();
    let output = f.directory.with_extension("outputs");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "ci",
            "cache",
            "application",
            "restore-or-prune",
            "--release-only",
        ])
        .arg("--source")
        .arg(&f.source)
        .arg("--target")
        .arg(&f.target)
        .arg("--directory")
        .arg(&f.directory)
        .args(["--native-key", "native-v2-ubuntu-24.04-fixture"])
        .arg("--github-output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["reused"],
        false
    );
    assert_eq!(fs::read(&f.library).unwrap(), b"compiled fixture library");
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "release_reused=false\n"
    );
    assert!(!lazypencil_xtask::evidence::link_map::path(&f.target).exists());
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
    #[cfg(unix)]
    {
        util::chmod(&f.library, 0o755).unwrap();
        fs::File::open(&f.library)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_789),
            ))
            .unwrap();
        fs::hard_link(&f.library, f.target.join("debug/liblazypencil.rlib")).unwrap();
    }
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
    #[cfg(unix)]
    {
        let archive = f._temp.path().join("checkpoint.tar.gz");
        let before = fs::metadata(&f.library).unwrap().modified().unwrap();
        cache::artifact::pack(&f.directory, &archive).unwrap();
        fs::remove_dir_all(&f.directory).unwrap();
        cache::artifact::unpack(&archive, &f.directory).unwrap();
        let file = f
            .directory
            .join("files/debug/deps/liblazypencil-1234567890abcdef.rlib");
        assert_eq!(util::raw_mode(&file).unwrap(), 0o755);
        assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), before);
    }
    fs::remove_dir_all(&f.target).unwrap();
    let report = cache::restore(&f.source, &f.target, &f.directory, &f.identity()).unwrap();
    assert_eq!(report["files"], if cfg!(unix) { 4 } else { 3 });
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

#[cfg(unix)]
#[test]
fn checkpoint_artifacts_reject_links_traversal_duplicates_and_corrupt_trailers_before_extraction() {
    let f = ApplicationFixture::new();
    f.stage();
    fs::write(f._temp.path().join("escaped"), b"sentinel").unwrap();
    for kind in ["symlink", "hardlink", "parent", "duplicate", "unexpected"] {
        let archive = f._temp.path().join(format!("{kind}.tar.gz"));
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let manifest = fs::read(f.directory.join("manifest.json")).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "manifest.json", manifest.as_slice())
            .unwrap();
        let report: Value = serde_json::from_slice(&manifest).unwrap();
        for (index, file) in report["files"].as_array().unwrap().iter().enumerate() {
            let name = format!("files/{}", file["path"].as_str().unwrap());
            for _ in 0..if kind == "duplicate" && index == 0 {
                2
            } else {
                1
            } {
                let mut data = fs::read(f.directory.join(&name)).unwrap();
                let mut header = tar::Header::new_gnu();
                header.set_mode(file["mode"].as_u64().unwrap() as u32);
                if index == 0 && matches!(kind, "symlink" | "hardlink") {
                    data.clear();
                    header.set_entry_type(if kind == "symlink" {
                        tar::EntryType::Symlink
                    } else {
                        tar::EntryType::Link
                    });
                    let target = if kind == "symlink" {
                        format!("{}escaped", "../".repeat(name.split('/').count() + 1))
                    } else {
                        "../../escaped".into()
                    };
                    header.set_link_name(target).unwrap();
                }
                header.set_size(data.len() as u64);
                header.set_cksum();
                if index == 0 && kind == "parent" {
                    header.as_mut_bytes()[..10].copy_from_slice(b"../escaped");
                    header.set_cksum();
                    builder.append(&header, data.as_slice()).unwrap();
                } else {
                    builder
                        .append_data(
                            &mut header,
                            if index == 0 && kind == "unexpected" {
                                "files/foreign"
                            } else {
                                &name
                            },
                            data.as_slice(),
                        )
                        .unwrap();
                }
            }
        }
        fs::write(&archive, builder.into_inner().unwrap().finish().unwrap()).unwrap();
        let parent = f._temp.path().join(format!("sandbox-{kind}"));
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("escaped"), b"sentinel").unwrap();
        let destination = parent.join("output");
        assert!(
            cache::artifact::unpack(&archive, &destination).is_err(),
            "{kind}"
        );
        assert!(!destination.exists());
        assert_eq!(
            fs::read(f._temp.path().join("escaped")).unwrap(),
            b"sentinel"
        );
        assert_eq!(fs::read(parent.join("escaped")).unwrap(), b"sentinel");
    }
    let archive = f._temp.path().join("valid.tar.gz");
    cache::artifact::pack(&f.directory, &archive).unwrap();
    let mut bytes = fs::read(&archive).unwrap();
    *bytes.last_mut().unwrap() ^= 0x80;
    fs::write(&archive, bytes).unwrap();
    let destination = f._temp.path().join("corrupt-output");
    assert!(cache::artifact::unpack(&archive, &destination).is_err());
    assert!(!destination.exists());
}

#[test]
fn verified_tool_replacement_retires_only_the_restored_sdk_and_cache_format() {
    let prefix = "xtask-v1-Linux-ARM64-20261023.149.3-rust1.94.0";
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
    ];
    let obsolete = cache::obsolete_tools_before_replacement(&entries, &primary, &matched).unwrap();
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
    assert!(cache::obsolete_tools_before_replacement(&entries, "unreviewed", &matched).is_err());
    assert!(
        cache::obsolete_tools_before_replacement(
            &entries,
            &primary,
            &matched.replace("ARM64", "X64")
        )
        .is_err()
    );
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
        "debug/examples/report.png",
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
    for runtime in [
        "xtask/src/fixtures/browser_ui.rs",
        "tests/fixtures/browser-ui/chromium.json",
    ] {
        f.write_source(runtime, b"runtime-only fixture\n");
    }
    for path in [
        "SOURCE-MANIFEST.json",
        review,
        "xtask/src/fixtures/browser_ui.rs",
        "tests/fixtures/browser-ui/chromium.json",
    ] {
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
            .env_remove("GITHUB_OUTPUT")
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
