use lazypencil_xtask::ci::{self, verification};
use serde_json::json;

#[test]
fn pr_contracts_distinguish_prose_tools_behavior_and_shared_compiler_inputs() {
    let cases = [
        (vec!["README.md", "docs/tui.md"], "docs"),
        (
            vec!["xtask/src/dev.rs", "xtask/tests/development.rs"],
            "tools",
        ),
        (vec!["xtask/src/ci/watch.rs"], "tools"),
        (
            vec![
                "src/tui/footer.rs",
                "src/tui/footer_tests.rs",
                "docs/tui.md",
            ],
            "focused",
        ),
        (
            vec![
                "src/tui/mod.rs",
                "src/config/keys/navigation.rs",
                "tests/tui.rs",
            ],
            "focused",
        ),
        (vec!["resources/guidance/default.md"], "focused"),
        (
            vec!["src/agent.rs", "src/agent/steering.rs", "tests/agent.rs"],
            "focused",
        ),
        (
            vec!["src/config/agents.rs", "tests/user_agent_config.rs"],
            "focused",
        ),
        (
            vec!["src/runtime/download.rs", "tests/runtime.rs", "README.md"],
            "focused",
        ),
        (
            vec!["src/tui/input.rs", "src/tui/tests/navigation.rs"],
            "focused",
        ),
        (
            vec!["src/tui/saved.rs", "src/tui/documents/view.rs"],
            "focused",
        ),
        (vec!["src/agent/process/windows.rs"], "full"),
        (vec!["src/agent/process/unix.rs"], "full"),
        (vec!["src/agent/process.rs"], "full"),
        (vec!["src/agent.rs", "src/runtime.rs"], "full"),
        (vec!["src/agent.rs", "src/process_owner.rs"], "full"),
        (vec!["src/runtime/profiles.rs"], "full"),
        (vec!["src/runtime/diagnostics.rs"], "full"),
        (vec!["src/tui/input.rs", "src/tui/preview.rs"], "full"),
        (vec!["src/tui/footer.rs", "src/render.rs"], "full"),
        (vec!["src/tui/camera.rs"], "full"),
        (vec!["xtask/src/ci/verification.rs"], "full"),
        (vec!["xtask/src/test_support/terminal.rs"], "full"),
        (vec![".github/workflows/ci.yml"], "full"),
        (vec!["Cargo.toml"], "full"),
        (vec!["src/agent.rs", "Cargo.lock"], "full"),
        (vec!["src/runtime.rs", "build.rs"], "full"),
        (vec!["src/tui/input.rs", ".cargo/config.toml"], "full"),
        (vec!["xtask/Cargo.toml"], "full"),
        (vec!["xtask/src/process.rs"], "full"),
        (vec!["xtask/src/util.rs"], "full"),
        (vec!["xtask/src/terminal/hosted.rs"], "full"),
        (vec!["src/new_shared_module.rs"], "full"),
    ];
    for (paths, expected) in cases {
        let paths = paths.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            verification::select("pull_request", &paths).0,
            expected,
            "{paths:?}"
        );
        for event in ["push", "workflow_dispatch", "workflow_call", ""] {
            assert_eq!(verification::select(event, &paths).0, "full");
        }
    }
}

#[test]
fn verification_gate_rejects_missing_failed_cancelled_or_wrongly_skipped_required_jobs() {
    for mode in ["docs", "tools", "focused", "full"] {
        let mut jobs = json!({"plan":{"result":"success"},"audit":{"result":"skipped"},"native":{"result":"skipped"},"source":{"result":"skipped"},"targeted":{"result":"skipped"},"shader-preprocessor":{"result":"skipped"}});
        let required: &[&str] = match mode {
            "docs" => &["plan"],
            "tools" => &["plan", "audit"],
            "focused" => &["plan", "targeted"],
            _ => &["plan", "audit", "native", "source"],
        };
        for name in required {
            jobs[name]["result"] = json!("success");
        }
        assert!(verification::gate(mode, &jobs).is_ok());
        for name in required {
            for failure in ["failure", "cancelled", "skipped", ""] {
                let mut failed = jobs.clone();
                failed[name]["result"] = json!(failure);
                assert!(
                    verification::gate(mode, &failed).is_err(),
                    "{mode}/{name}/{failure}"
                );
            }
        }
        let mut missing = jobs.clone();
        missing.as_object_mut().unwrap().remove("source");
        assert!(verification::gate(mode, &missing).is_err());
        if mode != "full" {
            let mut forged = jobs.clone();
            forged["source"]["result"] = json!("success");
            assert!(verification::gate(mode, &forged).is_err());
        }
    }
    assert!(verification::gate("unknown", &json!({})).is_err());
    assert!(ci::require_success(&json!({"native":{"result":"success"},"audit":{"result":"success"},"shader-preprocessor":{"result":"skipped"}})).is_ok());
    assert!(ci::require_success(&json!({"native":{"result":"skipped"},"audit":{"result":"success"},"shader-preprocessor":{"result":"skipped"}})).is_err());
}

#[test]
fn pr_planner_uses_the_actual_base_diff_and_rejects_bad_documentation() {
    use lazypencil_xtask::process;
    use std::{fs, process::Command, time::Duration};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        String::from_utf8(
            process::checked(
                Command::new("git").current_dir(root).args(args),
                Duration::from_secs(15),
            )
            .unwrap(),
        )
        .unwrap()
    };
    git(&["init", "--quiet"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='lazypencil'\nversion='0.0.0'\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "Before\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "Base"]);
    let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
    let event = dir.path().join("event.json");
    fs::write(
        &event,
        serde_json::to_vec(&json!({"pull_request":{"base":{"sha":base}}})).unwrap(),
    )
    .unwrap();
    let plan = || {
        process::output(
            Command::new(env!("CARGO_BIN_EXE_xtask"))
                .current_dir(root)
                .env("GITHUB_EVENT_NAME", "pull_request")
                .env("GITHUB_EVENT_PATH", &event)
                .env_remove("GITHUB_OUTPUT")
                .args(["ci", "plan"]),
            Duration::from_secs(30),
        )
        .unwrap()
    };
    fs::write(root.join("README.md"), "After\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "--quiet", "-m", "Prose"]);
    let result = plan();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output["verification"], "docs");
    fs::write(root.join("README.md"), "Trailing whitespace  \n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "--quiet", "-m", "Bad prose"]);
    assert!(!plan().status.success());
    fs::write(&event, "{}").unwrap();
    // An unknown baseline must never be interpreted as an empty, prose-only diff.
    fs::create_dir(root.join("tests")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let repository = lazypencil_xtask::util::root();
    fs::copy(repository.join("src/lib.rs"), root.join("src/lib.rs")).unwrap();
    for entry in fs::read_dir(repository.join("tests")).unwrap() {
        let entry = entry.unwrap();
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            fs::copy(entry.path(), root.join("tests").join(entry.file_name())).unwrap();
        }
    }
    let result = plan();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output["verification"], "full");

    for (path, suite, targets) in [
        (
            "src/agent/steering.rs",
            "conversation",
            vec![
                "agent",
                "config",
                "conversation_history",
                "mcp",
                "user_agent_config",
            ],
        ),
        ("src/runtime/download.rs", "core", vec!["runtime"]),
        (
            "src/tui/input.rs",
            "graphics",
            vec!["tui", "agent", "user_config"],
        ),
        (
            "src/tui/saved.rs",
            "graphics",
            vec!["tui", "agent", "user_config", "conversation_history"],
        ),
        (
            "src/tui/documents/view.rs",
            "graphics",
            vec!["tui", "agent", "user_config", "document_launch"],
        ),
    ] {
        let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
        fs::write(
            &event,
            serde_json::to_vec(&json!({"pull_request":{"base":{"sha":base}}})).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        fs::write(root.join(path), "// Changed application input.\n").unwrap();
        git(&["add", path]);
        git(&["commit", "--quiet", "-m", "Scoped application change"]);
        let result = plan();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(output["verification"], "focused", "{path}");
        assert_eq!(output["suite"], suite);
        let selected: Vec<String> =
            serde_json::from_str(output["targets"].as_str().unwrap()).unwrap();
        assert_eq!(selected, targets);
        // Every selected contract must produce executable checks with the
        // existing planner, including its ownership and ignored reproductions.
        let commands = ci::commands::plan(&repository, suite, &selected).unwrap();
        assert!(!commands.build.is_empty() && !commands.checks.is_empty());
    }
}
