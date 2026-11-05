use super::*;

#[test]
fn focused_prs_run_regressions_before_lint_and_keep_their_core_checks() {
    let root = util::root();
    let native = fs::read_to_string(root.join(".github/workflows/native.yml")).unwrap();
    let stages: Vec<_> = ["focus-build", "focus-reproduce", "focus-test", "focus-lint"]
        .map(|stage| native.find(&format!("ci native {stage} --suite")).unwrap())
        .into();
    assert!(stages.windows(2).all(|pair| pair[0] < pair[1]));
    // A standalone focus has no separate core consumer. It must retain the
    // optional-feature checks and CLI build owned by the existing planner.
    assert!(native.contains(
        "LAZYPENCIL_CI_SEPARATE_CORE: ${{ inputs.lane == 'build' && inputs.phase != 'focus' }}"
    ));
    assert!(native.contains(
        "(inputs.phase == 'focus' || inputs.phase == 'verify' && inputs.lane != 'build')"
    ));
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let targeted = ci
        .split("  targeted:")
        .nth(1)
        .unwrap()
        .split("  checks:")
        .next()
        .unwrap();
    assert!(targeted.contains("verification == 'focused'"));
    assert!(targeted.contains("&& 'ubuntu-24.04' || 'macos-26'"));
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
        assert_eq!(plan.platform, "macos-26");
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
    assert_eq!(helper.platform, "macos-26");
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
    assert_eq!(combined.checks.len(), 4);
    assert_eq!(combined.checks[0], ci::commands::management_tests(true));
    assert!(
        combined.checks[1]
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
            < workflow
                .find("xtask/target/debug/xtask measure-tui")
                .unwrap()
    );
    assert!(workflow.contains(
        "if: (inputs.phase == 'focus' || inputs.lane == 'release') && inputs.measure_tui"
    ));
    let ci = std::fs::read_to_string(util::root().join(".github/workflows/ci.yml")).unwrap();
    assert!(ci.contains(
        "measure_tui: ${{ github.event_name == 'workflow_dispatch' && inputs.measure_tui == true }}"
    ));
    assert!(!ci.contains("measure_tui: ${{ needs.plan.outputs.measure_tui == 'true' }}"));
    let dispatch = ci
        .split("\n  workflow_dispatch:\n")
        .nth(1)
        .unwrap()
        .split("\n  workflow_call:\n")
        .next()
        .unwrap();
    let measurement = dispatch
        .split("      measure_tui:\n")
        .nth(1)
        .unwrap()
        .split("      rebuild_application:\n")
        .next()
        .unwrap();
    assert!(measurement.contains("type: boolean\n"));
    assert!(measurement.contains("default: false\n"));
    assert!(workflow.contains("--chromium-resources"));
    assert!(workflow.contains("--enforce-targets --allocation-diagnostics=true"));
    let release_smoke = workflow
        .split("id: release-smoke\n")
        .nth(1)
        .unwrap()
        .split("      - name:")
        .next()
        .unwrap();
    assert!(release_smoke.contains("ci native release\n"));
    assert!(!release_smoke.contains("inputs.measure_tui"));
    let native = ci
        .split("\n  native:\n")
        .nth(1)
        .unwrap()
        .split("\n  source:\n")
        .next()
        .unwrap();
    assert!(native.contains("needs: plan\n"));
    assert!(!native.contains("needs.shader-preprocessor"));
    assert!(ci.contains("needs: [native, audit, shader-preprocessor]"));
    assert!(ci.contains("uses: ./.github/workflows/native-lanes.yml"));
    assert!(!ci.contains("  focus:\n"));
}

#[test]
fn portable_core_moves_to_linux_while_mac_keeps_native_contracts_and_normal_builds() {
    use ci::commands;
    let root = util::root();
    let lanes = fs::read_to_string(root.join(".github/workflows/native-lanes.yml")).unwrap();
    let core = lanes
        .split("\n  core:\n")
        .nth(1)
        .unwrap()
        .split("\n  release:\n")
        .next()
        .unwrap();
    assert!(core.contains("os: ubuntu-24.04\n"));
    assert!(core.contains("phase: full\n"));
    assert!(core.contains("lane: core\n"));
    let workflow = fs::read_to_string(root.join(".github/workflows/native.yml")).unwrap();
    assert!(workflow.contains("xtask/target/debug/xtask ci linux-core-dependencies"));
    let position = |text| workflow.find(text).unwrap();
    assert!(position("ci native core-platform\n") < position("id: default-build\n"));
    assert!(position("ci native default-lint\n") < position("id: default-build\n"));
    assert!(position("ci native default-development-build\n") < position("id: default-build\n"));
    let default_build = workflow
        .split("id: default-build\n")
        .nth(1)
        .unwrap()
        .split("      - name:")
        .next()
        .unwrap();
    assert!(default_build.contains("ci native default-build --suite"));
    assert!(!default_build.contains("build-and-reproduce"));
    let commands = commands::core_platform_tests();
    for command in &commands {
        assert!(command.iter().any(|s| s == "--no-default-features"));
        assert!(!command.iter().any(|s| s == "--features"));
    }
    assert!(
        commands[0]
            .iter()
            .any(|s| s == commands::MANAGEMENT_REPRODUCTION)
    );
    assert!(commands[0].iter().any(|s| s == "--test-threads=1"));
    assert!(commands[1].iter().any(|s| s == "--lib"));
    for target in [
        "agent",
        "assets",
        "completions",
        "config",
        "paths",
        "project",
        "runtime",
        "user_agent_config",
        "user_config",
    ] {
        assert!(
            commands[1]
                .windows(2)
                .any(|args| args == ["--test", target]),
            "{target}"
        );
    }
    assert!(
        commands[1]
            .windows(2)
            .any(|args| args == ["--skip", commands::MANAGEMENT_REPRODUCTION])
    );
    let build = commands::core_platform_build();
    assert_eq!(
        &build[..build.len() - 1],
        commands[1].split(|s| s == "--").next().unwrap()
    );
    assert_eq!(build.last().unwrap(), "--no-run");
    // The complete featureless suite is still enforced independently of Mac allocation.
    assert!(commands::core_tests().iter().any(|s| s == "--no-fail-fast"));
}

#[test]
fn release_caches_survive_later_test_failure_and_stay_scoped_to_the_pull_request() {
    let root = util::root();
    let workflow = fs::read_to_string(root.join(".github/workflows/native.yml")).unwrap();
    let release = workflow
        .split("id: release-save\n")
        .nth(1)
        .unwrap()
        .split("      - name:")
        .next()
        .unwrap();
    assert!(release.contains("!cancelled()"));
    assert!(release.contains("steps.release-build.outcome == 'success'"));
    assert!(release.contains("github.event_name == 'pull_request'"));
    assert!(release.contains("application stage --release-only"));
    assert!(workflow.contains("--release-verified '${{ steps.release-build.outcome == 'success'"));
    let dependencies = workflow
        .split("id: native-published\n")
        .nth(1)
        .unwrap()
        .split("      - name:")
        .next()
        .unwrap();
    assert!(dependencies.contains("github.event_name == 'pull_request'"));
    assert!(dependencies.contains("inputs.lane == 'core' || inputs.lane == 'release'"));
    let cleanup = fs::read_to_string(root.join(".github/workflows/pr-cache-cleanup.yml")).unwrap();
    assert!(cleanup.contains("types: [closed]"));
    assert!(cleanup.contains("CACHE_REF: refs/pull/${{ github.event.number }}/merge"));
    assert!(cleanup.contains("--ref \"$CACHE_REF\""));
    assert!(!cleanup.contains("actions/checkout"));
    assert!(!cleanup.contains("pull_request_target"));
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
        .find("xtask/target/debug/xtask ci native evidence-build")
        .unwrap();
    assert!(identify < restore && restore < compile);
    assert!(workflow.contains("steps.release-key.outputs.primary"));
    assert!(workflow.contains("lazypencil-release-cache"));
    assert!(!workflow.contains("lazypencil-application-cache"));
    for required in [
        "xtask/target/debug/xtask evidence",
        "xtask/target/debug/xtask ci native candidate",
        "xtask/target/debug/xtask terminal --candidate",
        "xtask/target/debug/xtask ci provenance",
        "xtask/target/debug/xtask measure-tui",
    ] {
        assert!(workflow.contains(required), "{required}");
    }
}

#[test]
fn release_gate_rejects_every_incomplete_or_malformed_result() {
    let valid = json!({"native":{"result":"success"},"audit":{"result":"success"},"shader-preprocessor":{"result":"success"}});
    assert!(ci::require_success(&valid).is_ok());
    let mut unchanged_shader = valid.clone();
    unchanged_shader["shader-preprocessor"]["result"] = json!("skipped");
    assert!(ci::require_success(&unchanged_shader).is_ok());
    let mut extra = valid.clone();
    extra["extra"] = json!({"result":"success"});
    assert!(ci::require_success(&extra).is_err());
    for job in ["native", "audit", "shader-preprocessor"] {
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
            if job == "shader-preprocessor" && state == "skipped" {
                continue;
            }
            value[job]["result"] = state;
            assert!(ci::require_success(&value).is_err());
        }
    }
    for value in [
        Value::Null,
        json!([]),
        json!({}),
        json!({"native":{"result":"success"}}),
        json!({"native":{"result":"success"},"audit":{"result":"success"}}),
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
        for job in ["native", "audit", "shader-preprocessor"] {
            let mut data = valid.clone();
            data[job] = value.clone();
            assert!(ci::require_success(&data).is_err());
        }
    }
}
