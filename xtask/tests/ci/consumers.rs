use super::*;

#[test]
fn focus_keeps_the_existing_reproduction_scope_for_every_source_module() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("../fixtures/focus.json")).unwrap();
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
    assert_eq!(plan.platform, "macos-15");
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
    assert_eq!(plan.platform, "macos-15");
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
fn macos_terminal_changes_select_native_pty_without_requiring_deferred_linux_pixels() {
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
            (false, false)
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
        (false, false)
    );
    for (platform, suite, expected) in [
        ("ubuntu-24.04", "dynamic", (true, false)),
        ("macos-15", "dynamic", (false, false)),
        ("ubuntu-24.04", "core", (false, false)),
        ("ubuntu-24.04", "ci", (false, false)),
        ("ubuntu-24.04-arm", "conversation", (false, false)),
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
        assert_eq!(plan.platform, "macos-15");
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
        assert_eq!(plan.platform, "macos-15");
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
        assert_eq!(plan.platform, "macos-15");
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
        assert_eq!(plan.platform, "macos-15");
        assert_eq!(plan.suite, "candidate");
        let expected = if path.starts_with("xtask/src/terminal/") {
            vec!["render", "tui"]
        } else {
            vec!["render"]
        };
        assert_eq!(plan.targets, expected);
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
fn extracted_ci_modules_keep_their_original_consumer_and_runtime_gates() {
    for (extracted, owner) in [
        ("xtask/src/ci/focus/selection.rs", "xtask/src/ci/focus.rs"),
        ("xtask/src/ci/native/stages.rs", "xtask/src/ci/native.rs"),
        ("xtask/src/ci/cache/migrations.rs", "xtask/src/ci/cache.rs"),
    ] {
        for application in [None, Some("src/render.rs"), Some("src/runtime.rs")] {
            let mut extracted = vec![extracted.to_owned()];
            let mut owner = vec![owner.to_owned()];
            if let Some(application) = application {
                extracted.push(application.to_owned());
                owner.push(application.to_owned());
            }
            assert_eq!(
                focus::select(&util::root(), &extracted, "auto").unwrap(),
                focus::select(&util::root(), &owner, "auto").unwrap()
            );
            assert_eq!(
                focus::runtime_scheduler_changed(&extracted),
                focus::runtime_scheduler_changed(&owner)
            );
        }
    }
}
