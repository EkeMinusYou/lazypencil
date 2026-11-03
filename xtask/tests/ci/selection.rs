use super::*;

#[test]
fn asset_activity_and_frontend_changes_prepare_asset_and_history_reproductions() {
    for paths in [
        vec!["src/store/activity.rs".into()],
        vec!["src/store.rs".into(), "src/project.rs".into()],
        vec!["src/main.rs".into()],
    ] {
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        let early = ci::commands::reproduction_targets(&plan.suite, &plan.targets);
        for target in ["assets", "document_store"] {
            assert!(plan.targets.iter().any(|name| name == target));
            assert!(early.iter().any(|name| name == target));
        }
        if paths.iter().any(|path| path == "src/main.rs") {
            assert!(plan.targets.iter().any(|name| name == "runtime"));
            assert!(early.iter().any(|name| name == "runtime"));
        }
    }
}

#[test]
fn asset_maintenance_submodules_keep_their_reproductions_in_mixed_project_changes() {
    let paths = ["src/assets/prune/scan.rs", "src/project.rs"].map(str::to_owned);
    let plan = focus::select(&util::root(), &paths, "auto").unwrap();
    assert!(plan.targets.iter().any(|target| target == "assets"));
    assert!(
        ci::commands::reproduction_targets(&plan.suite, &plan.targets)
            .iter()
            .any(|target| target == "assets")
    );
}

#[test]
fn remote_image_changes_prepare_http_cancellation_and_retention_before_full_tests() {
    for path in [
        "src/remote_images.rs",
        "src/project/remote_images.rs",
        "src/remote_images/tests.rs",
        "src/inline_images.rs",
        "src/inline_images/tests.rs",
    ] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        let targets = ci::commands::reproduction_targets(&plan.suite, &plan.targets);
        for target in ["sampler_sources", "runtime", "assets", "mcp"] {
            assert!(
                plan.targets.iter().any(|name| name == target),
                "{path}: {plan:?}"
            );
            assert!(
                targets.iter().any(|name| name == target),
                "{path}: {targets:?}"
            );
        }
    }
}

#[test]
fn font_lock_changes_reproduce_revision_checked_repairs_before_the_complete_suite() {
    for path in ["src/font_lock.rs", "tests/support/font_lock.rs"] {
        let plan = focus::select(&util::root(), &[path.into()], "auto").unwrap();
        assert_eq!(plan.suite, "graphics");
        assert!(plan.targets.iter().any(|target| target == "repair"));
        assert!(
            ci::commands::reproduction_targets(&plan.suite, &plan.targets)
                .iter()
                .any(|target| target == "repair")
        );
    }
}

#[test]
fn binding_submodules_keep_their_script_copy_reproduction_before_complete_verification() {
    for path in [
        "src/project/bindings/custom.rs",
        "src/project/bindings/generated.rs",
        "src/project/bindings/asset_tests.rs",
    ] {
        let paths = [path.to_owned(), "xtask/src/ci/native/compilation.rs".into()];
        let plan = focus::select(&util::root(), &paths, "auto").unwrap();
        assert_eq!(plan.suite, "copy", "{path}");
        assert!(plan.targets.iter().any(|target| target == "script_scope"));
    }
}

#[test]
fn encoded_clipboard_inputs_reproduce_in_the_rendering_attachment_consumer() {
    let root = util::root();
    let paths = [
        "src/clipboard.rs",
        "src/attachments.rs",
        "src/conversation/mod.rs",
        "src/conversation/management.rs",
        "src/conversation/management/retention/tests.rs",
        "tests/attachments.rs",
        "docs/image-attachments.md",
        "xtask/src/ci/focus.rs",
    ]
    .map(str::to_owned);
    let plan = focus::select(&root, &paths, "auto").unwrap();
    assert_eq!(plan.suite, "graphics");
    assert_eq!(plan.targets, ["attachments"]);
    let mut tui_paths = paths.to_vec();
    tui_paths.extend(
        [
            "src/tui/mod.rs",
            "src/tui/images.rs",
            "src/tui/tests.rs",
            "src/prompt.rs",
        ]
        .map(str::to_owned),
    );
    assert_eq!(
        focus::select(&root, &tui_paths, "auto").unwrap().targets,
        ["attachments", "tui"]
    );
    assert!(!focus::tui_measurement_changed(&tui_paths));
    let mut configured = tui_paths.clone();
    configured.extend(
        [
            "src/config.rs",
            "src/config/attachments.rs",
            "tests/user_config.rs",
        ]
        .map(str::to_owned),
    );
    assert_eq!(
        focus::select(&root, &configured, "auto").unwrap().targets,
        ["attachments", "tui", "user_config"]
    );
    assert!(!focus::tui_measurement_changed(&configured));
    configured.push("src/config/runtime.rs".into());
    assert!(focus::tui_measurement_changed(&configured));
    for additional in [
        "src/render.rs",
        "src/tui/preview.rs",
        "xtask/src/performance.rs",
    ] {
        let mut changed = tui_paths.clone();
        changed.push(additional.into());
        assert!(focus::tui_measurement_changed(&changed), "{additional}");
    }
    let mut fonts = tui_paths.clone();
    fonts.push("xtask/src/fixtures/fonts.rs".into());
    assert!(
        focus::select(&root, &fonts, "auto")
            .unwrap()
            .targets
            .iter()
            .any(|target| target == "font_sources")
    );
    let commands = ci::commands::plan(&root, &plan.suite, &plan.targets).unwrap();
    assert_eq!(
        commands.build[0],
        [
            "cargo",
            "test",
            "--locked",
            "--no-default-features",
            "--features",
            "rendering",
            "--test",
            "attachments",
            "--",
            "--test-threads=2"
        ]
    );
    let combined = ci::commands::plan_with_core(&root, &plan.suite, &plan.targets, true).unwrap();
    assert_eq!(combined.build, commands.build[..commands.build.len() - 1]);
    assert_eq!(combined.checks, commands.checks);
    assert!(
        commands
            .build
            .iter()
            .skip(1)
            .chain(&commands.checks)
            .all(|args| !args.iter().any(|arg| arg == "--no-default-features"))
    );
    assert!(commands.checks.iter().any(|args| {
        args.windows(2)
            .any(|pair| pair == ["--test", "attachments"])
    }));
    // Revocation-only changes retain their existing core ownership reproduction.
    assert_eq!(
        focus::select(&root, &["src/attachments.rs".into()], "auto")
            .unwrap()
            .suite,
        "conversation"
    );
}

#[test]
fn prose_filenames_never_expand_the_actual_source_consumers() {
    for sources in [
        vec![],
        vec!["src/runtime.rs"],
        vec!["src/clipboard.rs"],
        vec!["src/render.rs"],
        vec!["src/project/bindings.rs", "src/shader.rs"],
        vec!["resources/guidance/browser.md"],
        vec!["xtask/src/ci/native.rs"],
    ] {
        let mut paths = sources.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let before = focus::select(&util::root(), &paths, "auto").unwrap();
        paths.extend(
            [
                "docs/tui.md",
                "docs/shader.md",
                "docs/scripts.md",
                "docs/graphics.md",
                "README.md",
                "tests/fixtures/fonts/README.md",
            ]
            .map(str::to_owned),
        );
        assert_eq!(
            focus::select(&util::root(), &paths, "auto").unwrap(),
            before,
            "{paths:?}"
        );
    }
}

#[test]
fn bundled_guidance_with_ci_and_prose_uses_its_real_core_contract() {
    let root = util::root();
    // Reproduce the source categories in 7413a94, including the misleading TUI
    // prose name and the embedded Markdown that must still invalidate Cargo.
    let paths = [
        "docs/tui.md",
        "docs/graphics.md",
        "docs/implementation-status.md",
        "README.md",
        "tests/fixtures/fonts/README.md",
        "resources/guidance/browser.md",
        "src/guidance.rs",
        "xtask/src/ci/focus.rs",
        "xtask/src/ci/mod.rs",
        "xtask/tests/ci.rs",
        "xtask/tests/fixtures/focus.json",
        ".github/actions/xtask/action.yml",
        ".github/workflows/release-evidence.yml",
    ]
    .map(str::to_owned);
    let plan = focus::select(&root, &paths, "auto").unwrap();
    assert_eq!(plan.suite, "core");
    assert_eq!(plan.targets, ["guidance"]);
    let commands = ci::commands::plan(&root, &plan.suite, &plan.targets).unwrap();
    assert_eq!(commands.checks.len(), 1);
    for command in commands.build.iter().chain(&commands.checks) {
        assert!(command.iter().any(|arg| arg == "--no-default-features"));
        assert!(!command.iter().any(|arg| arg == "--lib"));
    }
    assert!(
        commands.checks[0]
            .windows(2)
            .any(|pair| pair == ["--test", "guidance"])
    );
    for source in [
        "src/guidance/project.rs",
        "src/guidance/usage.rs",
        "src/cdp.rs",
    ] {
        let mut mixed = paths.to_vec();
        mixed.push(source.into());
        let mixed = focus::select(&root, &mixed, "auto").unwrap();
        assert_ne!(mixed.targets, ["guidance"], "{source}");
    }
}

#[test]
fn initial_release_selects_only_mac_and_preserves_explicit_deferred_diagnostics() {
    assert_eq!(focus::verification_platforms("all").unwrap(), ["macos-26"]);
    assert_eq!(
        focus::verification_platforms("macos-26").unwrap(),
        ["macos-26"]
    );
    for platform in ["ubuntu-24.04", "ubuntu-24.04-arm"] {
        assert_eq!(focus::retry_platform(platform), platform);
        assert_eq!(focus::verification_platforms(platform).unwrap(), [platform]);
    }
    for invalid in ["windows-2025", "macos-26-intel", "unknown"] {
        assert!(focus::verification_platforms(invalid).is_err());
    }
    assert_eq!(focus::retry_platform("auto"), "macos-26");
    for paths in [
        vec!["src/cdp.rs".to_owned()],
        vec!["src/paths.rs".to_owned()],
        vec!["xtask/src/terminal/hosted.rs".to_owned()],
    ] {
        assert_eq!(
            focus::select(&util::root(), &paths, "auto")
                .unwrap()
                .platform,
            "macos-26"
        );
    }
}

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
        "src/runtime/download.rs",
        "src/cli.rs",
        "src/main.rs",
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
    assert_eq!(plan.platform, "macos-26");
    assert_eq!(plan.suite, "core");
    assert_eq!(plan.targets, ["runtime"]);
    let mut combined = paths.clone();
    combined.extend(
        [
            "tests/attachments.rs",
            "xtask/src/bin/test-helper.rs",
            "xtask/src/ci/commands.rs",
        ]
        .map(str::to_owned),
    );
    let combined = focus::select(&root, &combined, "auto").unwrap();
    assert_eq!(combined.suite, "graphics");
    assert_eq!(combined.targets, ["attachments", "runtime"]);
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
fn hosted_terminal_changes_select_mac_and_retain_explicit_linux_diagnostics() {
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
        assert_eq!(plan.platform, "macos-26");
        assert_eq!(plan.suite, "candidate");
        assert_eq!(plan.targets, ["render", "tui", "dynamic_render"]);
        let override_plan = focus::select(&util::root(), &paths, "ubuntu-24.04").unwrap();
        assert_eq!(override_plan.platform, "ubuntu-24.04");
        assert_eq!(override_plan.targets, plan.targets);
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
    assert_eq!(plan.platform, "macos-26");
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
