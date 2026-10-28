use lazypencil_xtask::{
    ci::{cache, commands, focus, setup},
    download, performance, process, util,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Cursor, Read},
    process::Command,
    time::{Duration, Instant},
};
#[path = "../src/test_support/install.rs"]
mod install;
fn helper() -> &'static str {
    env!("CARGO_BIN_EXE_lazypencil-test-helper")
}

#[test]
fn normal_build_before_test_catalog_avoids_rebuilds_without_merging_feature_conditions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    for path in ["src", "tests", "shared/src"] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    for (path, content) in [
        (
            "Cargo.toml",
            "[package]\nname='lazypencil'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nshared={path='shared',default-features=false}\n[dev-dependencies]\nshared={path='shared',default-features=false,features=['enhanced']}\n",
        ),
        (
            "shared/Cargo.toml",
            "[package]\nname='shared'\nversion='0.0.0'\nedition='2024'\n[features]\nenhanced=[]\n",
        ),
        (
            "shared/src/lib.rs",
            "pub const VALUE:u8=if cfg!(feature=\"enhanced\"){2}else{1};\n",
        ),
        ("src/lib.rs", "pub fn value()->u8{shared::VALUE}\n"),
        (
            "src/main.rs",
            "fn main(){println!(\"{}\",lazypencil::value());}\n",
        ),
        (
            "tests/identity.rs",
            "#[test] fn test_graph_and_binary_match(){assert_eq!(lazypencil::value(),2);let output=std::process::Command::new(env!(\"CARGO_BIN_EXE_lazypencil\")).output().unwrap();assert!(output.status.success());assert_eq!(String::from_utf8(output.stdout).unwrap().trim(),\"2\");}\n",
        ),
    ] {
        fs::write(root.join(path), content).unwrap();
    }
    let target = root.join("target");
    let command = |arguments: &[String]| {
        let mut command = Command::new(&arguments[0]);
        command
            .args(&arguments[1..])
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_INCREMENTAL", "0");
        command
    };
    process::checked(
        &mut command(&[
            "cargo".into(),
            "generate-lockfile".into(),
            "--offline".into(),
        ]),
        Duration::from_secs(30),
    )
    .unwrap();
    let build = |arguments: &[String]| {
        let mut command = command(arguments);
        command.arg("--offline");
        if !arguments
            .iter()
            .any(|arg| arg.starts_with("--message-format="))
        {
            command.arg("--message-format=json-render-diagnostics");
        }
        let output = process::checked(&mut command, Duration::from_secs(45)).unwrap();
        output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<Value>(line).unwrap())
            .filter(|event| {
                event["reason"] == "compiler-artifact"
                    && event["package_id"]
                        .as_str()
                        .is_some_and(|id| id.contains("#lazypencil@"))
            })
            .collect::<Vec<_>>()
    };
    let executable = target.join("debug").join(if cfg!(windows) {
        "lazypencil.exe"
    } else {
        "lazypencil"
    });
    let [normal, tests] = commands::catalog_builds(&["identity".into()]);
    assert!(!build(&normal).is_empty());
    assert_eq!(
        process::text(&mut Command::new(&executable))
            .unwrap()
            .trim(),
        "1"
    );
    assert!(!build(&tests).is_empty());
    assert_eq!(
        process::text(&mut Command::new(&executable))
            .unwrap()
            .trim(),
        "2"
    );
    let repeated = build(&tests);
    assert!(!repeated.is_empty());
    assert!(repeated.iter().all(|event| event["fresh"] == true));
    let test = repeated
        .iter()
        .find(|event| event["target"]["name"] == "identity")
        .unwrap()["executable"]
        .as_str()
        .unwrap();
    process::checked(
        Command::new(test).arg("--test-threads=2"),
        Duration::from_secs(15),
    )
    .unwrap();
}

#[test]
fn normal_test_lane_requires_all_compiled_targets_and_preserves_the_existing_skips() {
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };
    let temporary = tempfile::tempdir().unwrap();
    let (integration, _) = focus::integration_targets(&util::root()).unwrap();
    let names = ["lib".to_owned(), "lazypencil".to_owned()]
        .into_iter()
        .chain(integration)
        .collect::<Vec<_>>();
    let binaries: BTreeMap<_, _> = names
        .iter()
        .map(|name| (name.clone(), temporary.path().join(name)))
        .collect();
    let checks = commands::default_tests(&util::root(), &binaries).unwrap();
    assert_eq!(
        checks
            .iter()
            .map(|args| Path::new(&args[0]).file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        names
    );
    let flags = [
        "--test-threads=2",
        "--skip",
        "tui::tests::",
        "--skip",
        "cancelling_a_running_script_unblocks_the_mcp_document_worker",
    ];
    for args in checks {
        let mut expected = flags.to_vec();
        if Path::new(&args[0]) == binaries["lib"].as_path() {
            expected.extend(["--skip", commands::MANAGEMENT_REPRODUCTION]);
        }
        assert_eq!(args[1..], expected);
    }
    let docs = commands::default_doctests();
    assert_eq!(
        docs[..6],
        ["cargo", "test", "--locked", "--doc", "--no-fail-fast", "--"]
    );
    assert_eq!(docs[6..], flags);
    let cargo = commands::default_cargo_tests();
    assert_eq!(
        cargo[..5],
        ["cargo", "test", "--locked", "--no-fail-fast", "--"]
    );
    let mut expected = flags.to_vec();
    expected.extend(["--skip", commands::MANAGEMENT_REPRODUCTION]);
    assert_eq!(cargo[5..], expected);
    for missing in ["lib", "lazypencil", "mcp"] {
        let mut incomplete = binaries.clone();
        incomplete.remove(missing);
        assert!(commands::default_tests(&util::root(), &incomplete).is_err());
    }
    let mut foreign = binaries.clone();
    foreign.insert("undeclared".into(), temporary.path().join("undeclared"));
    assert!(commands::default_tests(&util::root(), &foreign).is_err());
    let mut relative = binaries;
    relative.insert("lib".into(), PathBuf::from("relative-library-test"));
    assert!(commands::default_tests(&util::root(), &relative).is_err());
}

#[test]
fn shared_cleanup_cases_run_once_in_each_feature_configuration_without_serializing_unrelated_tests()
{
    let names = [
        "conversation::management::retention::tests::retention_counts_pending_cleanup_and_recovers_it_after_the_original_document_was_removed",
        "conversation::management::deletion::tests::deletion_cleanup_recovers_partial_payloads_and_protects_busy_or_foreign_tombstones",
        "conversation::management::cleanup::tests::cleanup_wait_observes_cancellation_and_deadline_without_waiting_for_filesystem_work",
        "conversation::history::tests::record",
        "conversation::management_extra::unrelated",
    ];
    for features in [false, true] {
        let isolated = commands::management_tests(features);
        assert_eq!(
            isolated.contains(&"--no-default-features".into()),
            !features
        );
        assert!(
            isolated
                .windows(2)
                .any(|pair| pair == ["--lib", commands::MANAGEMENT_REPRODUCTION])
        );
        assert!(isolated.contains(&"--test-threads=1".into()));
        assert!(!isolated.contains(&"--ignored".into()));
        let rest = if features {
            commands::default_cargo_tests()
        } else {
            commands::core_tests()
        };
        assert_eq!(rest.contains(&"--no-default-features".into()), !features);
        assert!(rest.contains(&"--test-threads=2".into()));
        assert!(rest.contains(&"--no-fail-fast".into()));
        let skips = rest
            .windows(2)
            .filter(|pair| pair[0] == "--skip")
            .map(|pair| pair[1].as_str())
            .collect::<Vec<_>>();
        for name in names {
            let in_isolated = name.contains(commands::MANAGEMENT_REPRODUCTION);
            let in_remaining = !skips.iter().any(|filter| name.contains(*filter));
            assert_eq!(u8::from(in_isolated) + u8::from(in_remaining), 1, "{name}");
        }
    }
}

#[test]
fn focused_scheduling_preserves_reproduction_order_and_excludes_only_repeated_conditions() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/focus-commands.json")).unwrap();
    for case in cases {
        let targets: Vec<String> = serde_json::from_value(case["targets"].clone()).unwrap();
        let plan =
            commands::plan(&util::root(), case["suite"].as_str().unwrap(), &targets).unwrap();
        assert_eq!(json!(plan.build), case["build"]);
        assert_eq!(json!(plan.checks), case["checks"], "{}", case["suite"]);
    }
    for (suite, targets) in [
        ("ci", vec!["mcp".into()]),
        ("shader", vec!["mcp".into()]),
        ("core", vec!["agent_workflow".into()]),
        ("core", vec!["not-a-target".into()]),
    ] {
        assert!(
            commands::plan(&util::root(), suite, &targets).is_err(),
            "{suite}/{targets:?}"
        );
    }
}

#[test]
fn tui_candidate_tests_isolate_the_process_global_cleanup_worker_without_omitting_the_library() {
    let plan =
        commands::plan(&util::root(), "candidate", &["render".into(), "tui".into()]).unwrap();
    assert_eq!(plan.checks.len(), 2);
    assert_eq!(plan.checks[0], commands::management_tests(true));
    let remaining = &plan.checks[1];
    assert!(remaining.iter().any(|arg| arg == "--lib"));
    assert!(remaining.iter().any(|arg| arg == "--test-threads=2"));
    assert!(
        remaining.iter().position(|arg| arg == "--").unwrap()
            < remaining.iter().position(|arg| arg == "--skip").unwrap()
    );
    assert!(
        remaining
            .windows(2)
            .any(|args| args == ["--skip", commands::MANAGEMENT_REPRODUCTION])
    );
    for target in ["render", "tui"] {
        assert!(remaining.windows(2).any(|args| args == ["--test", target]));
    }
}

#[test]
fn focused_dynamic_preview_regressions_run_before_browser_work_and_are_not_repeated() {
    let targets = ["tui", "user_config", "dynamic_render", "shader_uniforms"].map(str::to_owned);
    let plan = commands::plan(&util::root(), "dynamic", &targets).unwrap();
    for (command, filter) in plan.checks.iter().zip(commands::PREVIEW_REPRODUCTIONS) {
        assert_eq!(
            command,
            &[
                "cargo",
                "test",
                "--locked",
                "--lib",
                filter,
                "--",
                "--test-threads=1"
            ]
        );
    }
    let later = plan
        .checks
        .iter()
        .skip(3)
        .find(|command| command.iter().any(|arg| arg == "tui::tests::"))
        .unwrap();
    assert!(
        later
            .windows(2)
            .any(|args| args == ["--skip", commands::PREVIEW_REPRODUCTIONS[2]])
    );
    for filter in commands::PREVIEW_REPRODUCTIONS {
        assert_eq!(
            plan.checks
                .iter()
                .filter(|command| command
                    .iter()
                    .position(|arg| arg == "--lib")
                    .is_some_and(|index| command.get(index + 1).is_some_and(|arg| arg == filter)))
                .count(),
            1
        );
    }
    let mut browser_targets = targets.to_vec();
    browser_targets.push("browser_readiness".into());
    let browser = commands::plan(&util::root(), "dynamic", &browser_targets).unwrap();
    for reproduction in [
        focus::PAGE_DIAGNOSTICS,
        focus::NATIVE_ANIMATION,
        focus::OFFLINE_NETWORK,
        focus::OFFLINE_TRANSPORT,
    ] {
        let native = browser
            .checks
            .iter()
            .position(|command| {
                command
                    .windows(2)
                    .any(|args| args == ["browser_readiness", reproduction])
            })
            .unwrap();
        let longer = browser
            .checks
            .iter()
            .position(|command| command.iter().any(|arg| arg == focus::GENERATED_BROWSER))
            .unwrap();
        assert!(
            native < longer,
            "The native policy reproduction must precede namespace/browser fixtures"
        );
        for flag in ["--exact", "--ignored", "--test-threads=1"] {
            assert!(browser.checks[native].iter().any(|arg| arg == flag));
        }
        let remaining = browser
            .checks
            .iter()
            .find(|command| {
                command.iter().any(|arg| arg == "browser_readiness")
                    && command.iter().any(|arg| arg == "--include-ignored")
            })
            .unwrap();
        assert!(
            remaining
                .windows(2)
                .any(|args| args == ["--skip", reproduction])
        );
        assert_eq!(
            browser
                .checks
                .iter()
                .filter(|command| command
                    .windows(2)
                    .any(|args| args == ["browser_readiness", reproduction]))
                .count(),
            1
        );
    }
}

#[test]
fn renderer_focus_runs_native_cases_and_exact_dynamic_pixels_before_the_full_scope() {
    let targets = focus::RENDER_TARGETS
        .iter()
        .map(|s| (*s).into())
        .collect::<Vec<_>>();
    let plan = commands::plan(&util::root(), "render", &targets).unwrap();
    assert_eq!(plan.checks.len(), 3);
    let normal = &plan.checks[0];
    assert!(normal.iter().any(|arg| arg == "--lib"));
    let selected = normal
        .windows(2)
        .filter(|pair| pair[0] == "--test")
        .map(|pair| pair[1].as_str())
        .collect::<Vec<_>>();
    assert_eq!(selected, focus::RENDER_TARGETS);
    assert!(
        normal
            .windows(2)
            .any(|pair| pair == ["--skip", focus::TEXT_LAYER_REPRODUCTION])
    );
    assert!(
        !normal
            .iter()
            .any(|arg| ["--ignored", "--include-ignored"].contains(&arg.as_str()))
    );
    for (command, (target, filter)) in plan.checks[1..].iter().zip([
        ("dynamic_render", focus::RENDER_BROWSER),
        ("shader_uniforms", focus::SHADER_REGRESSIONS[1]),
    ]) {
        assert!(command.windows(2).any(|pair| pair == ["--test", target]));
        assert!(command.iter().any(|arg| arg == filter));
        for arg in ["--ignored", "--exact", "--test-threads=1"] {
            assert!(command.iter().any(|actual| actual == arg));
        }
    }
    assert!(commands::plan(&util::root(), "render", &["render".into()]).is_err());
    let mixed = focus::select(
        &util::root(),
        &["src/render.rs".into(), "src/script.rs".into()],
        "auto",
    )
    .unwrap();
    assert_eq!(mixed.suite, "dynamic");
    assert!(mixed.targets.iter().any(|target| target == "script_scope"));
    assert_eq!(
        focus::PLATFORMS,
        ["ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"]
    );
}

#[test]
fn color_font_reproductions_run_before_chromium_and_are_excluded_only_afterwards() {
    for suite in ["graphics", "dynamic", "copy"] {
        let targets = vec!["font_sources".into(), "render".into()];
        let early = commands::color_font_reproduction(suite, &targets).unwrap();
        assert_eq!(
            early,
            [
                "cargo",
                "test",
                "--locked",
                "--test",
                "font_sources",
                "color_fonts::",
                "--",
                "--test-threads=1"
            ]
        );
        let plan = commands::plan(&util::root(), suite, &targets).unwrap();
        let remaining = plan
            .checks
            .iter()
            .filter(|command| {
                command
                    .windows(2)
                    .any(|args| args == ["--test", "font_sources"])
            })
            .collect::<Vec<_>>();
        assert_eq!(remaining.len(), 1);
        assert!(
            remaining[0]
                .windows(2)
                .any(|args| args == ["--skip", commands::COLOR_FONT_REPRODUCTION])
        );
        assert!(!remaining[0].contains(&"--ignored".into()));
        assert!(!remaining[0].contains(&"--include-ignored".into()));
        assert!(!plan.build.iter().flatten().any(|arg| arg == "--skip"));
        assert!(commands::color_font_reproduction(suite, &["render".into()]).is_none());
    }
    for suite in ["core", "conversation", "release", "candidate"] {
        let targets = vec!["font_sources".into()];
        assert!(commands::color_font_reproduction(suite, &targets).is_none());
        assert!(
            !commands::plan(&util::root(), suite, &targets)
                .unwrap()
                .checks
                .iter()
                .flatten()
                .any(|arg| arg == commands::COLOR_FONT_REPRODUCTION)
        );
    }
}

#[test]
fn browser_copy_focus_keeps_each_browser_condition_and_excludes_only_early_reproductions() {
    let targets = ["browser_readiness", "project", "script_scope"].map(str::to_owned);
    let plan = commands::plan(&util::root(), "browser", &targets).unwrap();
    let reproductions = [
        focus::GENERATED_BROWSER,
        focus::LOCAL_FILES,
        focus::BROWSER_COPY,
        focus::LOCAL_PAGES,
    ];
    assert!(
        plan.checks[0]
            .iter()
            .any(|arg| arg == focus::GENERATED_BROWSER)
    );
    for (command, expected) in plan.checks.iter().zip(reproductions) {
        assert!(command.iter().any(|arg| arg == expected));
        for flag in ["--exact", "--ignored"] {
            assert!(command.iter().any(|arg| arg == flag));
        }
    }
    let rest = plan
        .checks
        .iter()
        .find(|command| {
            command.iter().any(|arg| arg == "browser_readiness")
                && command.iter().any(|arg| arg == "--include-ignored")
        })
        .unwrap();
    for name in reproductions {
        assert_eq!(
            rest.windows(2)
                .filter(|pair| pair[0] == "--skip" && pair[1] == name)
                .count(),
            1
        );
    }
    let scripts = plan
        .checks
        .iter()
        .find(|command| command.iter().any(|arg| arg == "script_scope"))
        .unwrap();
    assert!(scripts.iter().any(|arg| arg == "--include-ignored"));
    for name in ["script_bindings::", "generated_browser_"]
        .into_iter()
        .chain(focus::COPY_CPU.iter().copied())
    {
        assert!(
            scripts
                .windows(2)
                .any(|pair| pair[0] == "--skip" && pair[1] == name)
        );
    }
    assert_eq!(
        plan.checks
            .iter()
            .filter(|command| command.iter().any(|arg| arg == "project"))
            .count(),
        1
    );
    assert!(
        !plan.build.iter().chain(&plan.checks).flatten().any(|arg| [
            "shader_uniforms",
            "dynamic_render",
            "tui"
        ]
        .contains(&arg.as_str()))
    );
    assert!(commands::plan(&util::root(), "browser", &targets[..2]).is_err());
    assert!(
        commands::plan(
            &util::root(),
            "browser",
            &[targets.to_vec(), vec!["project".into()]].concat()
        )
        .is_err()
    );
}
#[test]
fn bounded_commands_preserve_exit_codes_output_and_failure_logs() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("responses.json");
    fs::write(&config,br#"{"[\"ok\"]":{"stdout":"out","stderr":"err"},"[\"bad\"]":{"stdout":"partial","stderr":"failed","status":7}}"#).unwrap();
    let make = |mode: &str| {
        let mut c = Command::new(helper());
        c.arg("fixture-command").arg(&config).arg(mode);
        c
    };
    let output = process::output(&mut make("ok"), Duration::from_secs(5)).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"out");
    assert_eq!(output.stderr, b"err");
    let output = process::output(&mut make("bad"), Duration::from_secs(5)).unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"partial");
    let log = temp.path().join("failure.log");
    assert!(process::logged(&mut make("bad"), Duration::from_secs(5), &log, false).is_err());
    assert_eq!(fs::read(log).unwrap(), b"partialfailed");
}
#[test]
fn native_process_records_preserve_markers_exit_codes_streams_and_deadline_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let table = temp.path().join("responses.json");
    let pid = temp.path().join("pid");
    let marker = temp.path().join("launched");
    for sleeping in [false, true] {
        fs::write(&table, serde_json::to_vec(&json!({"*":{
            "pid_file":pid,"marker_file":marker,"stdout":"partial output","stderr":"private-value\n",
            "sleep_ms":if sleeping {30000}else{0},"status":91
        }})).unwrap()).unwrap();
        let result = process::output(
            Command::new(helper())
                .arg("fixture-command")
                .arg(&table)
                .arg("--browser-or-worker-argument"),
            if sleeping {
                Duration::from_millis(800)
            } else {
                Duration::from_secs(5)
            },
        )
        .unwrap();
        assert_eq!(result.timed_out, sleeping);
        if !sleeping {
            assert_eq!(result.status.code(), Some(91));
        }
        assert_eq!(result.stdout, b"partial output");
        assert_eq!(result.stderr, b"private-value\n");
        let child: u32 = fs::read_to_string(&pid).unwrap().parse().unwrap();
        assert_ne!(child, std::process::id());
        assert_eq!(fs::read(&marker).unwrap(), b"");
        #[cfg(unix)]
        assert_eq!(unsafe { libc::kill(child as i32, 0) }, -1);
    }
}

#[test]
fn acp_fixture_preserves_unexpected_error_and_assertion_diagnostics() {
    for input in [
        "invalid-json\n",
        "{\"method\":\"initialize\",\"params\":{\"protocolVersion\":2}}\n",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("input.jsonl");
        fs::write(&path, input).unwrap();
        let output = process::output(
            Command::new(helper())
                .args(["acp", "edit"])
                .current_dir(temp.path())
                .stdin(fs::File::open(&path).unwrap()),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(!output.timed_out && !output.status.success());
        let log = fs::read_to_string(temp.path().join("fixture-errors.log")).unwrap();
        assert!(!log.trim().is_empty());
        assert!(!output.stderr.is_empty());
    }
}
#[cfg(unix)]
#[test]
fn deadlines_kill_a_term_ignoring_descendant_without_waiting_on_inherited_logs() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("pids.json");
    let started = Instant::now();
    let result = process::output(
        Command::new(helper()).arg("hold-tree").arg(&marker),
        Duration::from_millis(800),
    )
    .unwrap();
    assert!(result.timed_out);
    assert!(started.elapsed() < Duration::from_secs(6));
    let pids: Value = util::json(&marker).unwrap();
    for name in ["parent", "child"] {
        let pid = pids[name].as_i64().unwrap() as i32;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if unsafe { libc::kill(pid, 0) } == -1 {
                break;
            }
            #[cfg(target_os = "linux")]
            if fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
                s.split(')')
                    .nth(1)
                    .is_some_and(|s| s.trim_start().starts_with('Z'))
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Owned descendant {pid} survived deadline"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
#[test]
fn resource_measurements_use_fresh_native_children_and_enforce_failure_and_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("pixels.png");
    let (usage, raw) = performance::measure(
        Command::new(helper())
            .arg("render-fixture")
            .args(["--json", "render", "shadow.pen", "--output"])
            .arg(&out),
        5.0,
    )
    .unwrap();
    assert!(usage["peak_rss_bytes"].as_u64().unwrap() > 0);
    assert!(usage["user_cpu_ms"].as_f64().unwrap() >= 0.0);
    assert!(usage["system_cpu_ms"].as_f64().unwrap() >= 0.0);
    performance::validate_render(&raw, &out, 0).unwrap();
    assert!(performance::measure(Command::new(helper()).arg("unknown"), 5.0).is_err());
    let started = Instant::now();
    assert!(performance::measure(Command::new(helper()).arg("sleep"), 0.1).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}
#[test]
fn repeated_benchmark_observes_separate_first_invocations_and_exact_repeated_samples() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("renderer");
    install::install(&util::root(), &binary, &["render-fixture".into()]).unwrap();
    let report = performance::benchmark(&binary, 2, 5.0).unwrap();
    assert_eq!(report["binary_sha256"], util::file_digest(&binary).unwrap());
    for scene in report["scenes"].as_array().unwrap() {
        assert_eq!(scene["repeated"].as_array().unwrap().len(), 2);
        assert!(
            scene["first_invocation"]["peak_rss_bytes"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(scene["expanded_nodes"], 1000);
    }
}
#[test]
fn streamed_originals_enforce_exact_limit_hash_no_overwrite_and_transactional_cleanup() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("broken source"))
        }
    }
    for mutation in ["hash", "size", "read", "exact", "overwrite"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("original");
        let bytes = b"original pinned font";
        let sha = util::digest(bytes);
        let result = match mutation {
            "read" => download::publish(Broken, &sha, &path, bytes.len() as u64),
            "size" => download::publish(Cursor::new(bytes), &sha, &path, bytes.len() as u64 - 1),
            "hash" => download::publish(
                Cursor::new(bytes),
                &"0".repeat(64),
                &path,
                bytes.len() as u64,
            ),
            "overwrite" => {
                fs::write(&path, b"keep").unwrap();
                download::publish(Cursor::new(bytes), &sha, &path, bytes.len() as u64)
            }
            _ => download::publish(Cursor::new(bytes), &sha, &path, bytes.len() as u64),
        };
        assert_eq!(result.is_ok(), mutation == "exact", "{mutation}");
        if mutation == "overwrite" {
            assert_eq!(fs::read(path).unwrap(), b"keep");
        } else if mutation == "exact" {
            assert_eq!(
                download::verify_file(&path, &sha, bytes.len() as u64).unwrap(),
                bytes.len() as u64
            );
        } else {
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        }
    }
    let original = "https://raw.githubusercontent.com/google/pinned"
        .parse()
        .unwrap();
    for url in [
        "https://example.invalid/pinned",
        "http://raw.githubusercontent.com/google/pinned",
    ] {
        assert!(download::validate_redirect(&original, &url.parse().unwrap()).is_err());
    }
    download::validate_redirect(
        &original,
        &"https://raw.githubusercontent.com/google/same-host"
            .parse()
            .unwrap(),
    )
    .unwrap();
}
#[test]
fn cached_material_sources_revalidate_immutable_pins_without_network() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("fonts");
    fs::create_dir(&output).unwrap();
    let catalogue = temp.path().join("catalogue.json");
    let mut records = Vec::new();
    for style in ["Outlined", "Rounded", "Sharp"] {
        let name = format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf");
        let bytes = format!("pinned-{style}");
        fs::write(output.join(&name), &bytes).unwrap();
        records.push(json!({"id":format!("Material Symbols {style}"),"version":"a".repeat(40),"source_url":format!("https://raw.githubusercontent.com/google/material-design-icons/{}/variablefont/MaterialSymbols{style}%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf","a".repeat(40)),"source_format":"ttf","source_sha256":util::digest(bytes)}));
    }
    fs::write(&catalogue, util::encode(&records).unwrap()).unwrap();
    let result = setup::material_fonts(&catalogue, &output).unwrap();
    assert!(
        result["fonts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["reused"] == true)
    );
    for field in ["version", "source_url", "source_sha256", "source_format"] {
        let mut changed = records.clone();
        changed[0][field] = "invalid".into();
        fs::write(&catalogue, util::encode(&changed).unwrap()).unwrap();
        assert!(
            setup::material_fonts(&catalogue, &output).is_err(),
            "{field}"
        );
    }
    fs::write(&catalogue, util::encode(&records).unwrap()).unwrap();
    fs::write(
        output.join("MaterialSymbolsOutlined[FILL,GRAD,opsz,wght].ttf"),
        b"tampered",
    )
    .unwrap();
    assert!(setup::material_fonts(&catalogue, &output).is_err());
}
#[test]
fn retention_preserves_unrelated_download_dependency_tool_and_pull_request_caches() {
    let mut entries = vec![
        json!({"id":1,"key":"cargo-downloads-Linux","ref":"refs/heads/main"}),
        json!({"id":2,"key":"native-v2-ubuntu-24.04-image-hash","ref":"refs/heads/main"}),
        json!({"id":3,"key":"xtask-v1-tools","ref":"refs/heads/main"}),
        json!({"id":4,"key":"application-v2-ubuntu-24.04-hash","ref":"refs/pull/1/merge"}),
    ];
    for entry in &mut entries {
        entry["version"] = "layout".into();
    }
    assert!(cache::obsolete(&entries, None).unwrap().is_empty());
}

#[test]
fn native_package_setup_keeps_signed_sources_scoped_and_stops_after_update_failure() {
    for arch in ["X64", "ARM64", "unknown"] {
        let temporary = tempfile::tempdir().unwrap();
        let mut invocations = Vec::new();
        let mut source_path = None;
        let result = setup::install_linux(arch, temporary.path(), |command| {
            assert_eq!(command.get_program(), "sudo");
            let args = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if arch == "X64" {
                let path = args
                    .iter()
                    .find_map(|s| s.strip_prefix("Dir::Etc::sourcelist="))
                    .unwrap();
                let text = fs::read_to_string(path).unwrap();
                assert!(
                    text.contains("https://archive.ubuntu.com/ubuntu")
                        && text.contains("https://security.ubuntu.com/ubuntu")
                        && text
                            .contains("Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg")
                );
                assert!(!text.contains("trusted=yes") && !text.contains("azure.archive"));
                let parts = args
                    .iter()
                    .find_map(|s| s.strip_prefix("Dir::Etc::sourceparts="))
                    .unwrap();
                assert_eq!(fs::read_dir(parts).unwrap().count(), 0);
                if let Some(previous) = &source_path {
                    assert_eq!(previous, path);
                }
                source_path = Some(path.to_owned());
            }
            invocations.push(args);
            Ok(())
        });
        assert_eq!(result.is_ok(), arch != "unknown");
        if arch == "unknown" {
            assert!(invocations.is_empty());
        } else {
            assert_eq!(invocations.len(), 2);
            assert_eq!(invocations[0].last().unwrap(), "update");
            assert!(invocations[1].contains(&"install".to_owned()));
            if arch == "ARM64" {
                assert_eq!(invocations[0], ["apt-get", "update"]);
            }
        }
        if let Some(path) = source_path {
            assert!(!std::path::Path::new(&path).exists());
        }
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
    }
    let temporary = tempfile::tempdir().unwrap();
    let mut count = 0;
    assert!(
        setup::install_linux("X64", temporary.path(), |_| {
            count += 1;
            Err(anyhow::anyhow!("update failed"))
        })
        .is_err()
    );
    assert_eq!(count, 1);
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
}
#[test]
fn tool_retention_preserves_dependencies_and_latest_generation_per_exact_environment() {
    let mut entries = Vec::new();
    let mut obsolete = Vec::new();
    let mut id = 0;
    for environment in [
        "Linux-X64-image1-rust1.94.0",
        "Linux-ARM64-image1-rust1.94.0",
        "macOS-ARM64-image1-rust1.94.0",
        "Linux-X64-image2-rust1.94.0",
        "Linux-X64-image1-rust1.95.0",
    ] {
        for layout in ["layout1", "layout2"] {
            for generation in 1..=4 {
                id += 1;
                entries.push(json!({"id":id,"ref":"refs/heads/main","version":layout,
                    "key":format!("xtask-v1-{environment}-{}", generation.to_string().repeat(64)),
                    "created_at":format!("2026-10-{generation:02}T00:00:00Z")}));
                if generation < 4 {
                    obsolete.push(id);
                }
            }
        }
    }
    for (key, reference) in [
        (
            format!("xtask-v1-Linux-X64-image1-rust1.94.0-{}", "0".repeat(64)),
            "refs/pull/1/merge",
        ),
        (
            "xtask-deps-v1-Linux-X64-image1-rust1.94.0-lock".into(),
            "refs/heads/main",
        ),
        ("native-v2-Linux-current".into(), "refs/heads/main"),
        ("cargo-downloads-current".into(), "refs/heads/main"),
    ] {
        id += 1;
        entries.push(json!({"id":id,"key":key,"ref":reference,"version":"layout1","created_at":"2026-01-01T00:00:00Z"}));
    }
    entries.reverse();
    assert_eq!(
        cache::obsolete(&entries, None)
            .unwrap()
            .into_iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        obsolete
    );
}

#[test]
fn dependency_checkpoint_and_application_retention_require_matching_main_layouts() {
    let entry = |id, key: String, reference: &str| json!({"id":id,"key":key,"ref":reference,"version":"layout","created_at":format!("2026-10-{id:02}T00:00:00Z")});
    let mut entries = vec![
        entry(1, "checkpoint-native-v2-current".into(), "refs/heads/main"),
        entry(2, "native-v2-current".into(), "refs/heads/main"),
        entry(3, "checkpoint-native-v2-missing".into(), "refs/heads/main"),
        entry(4, "native-v2-missing".into(), "refs/pull/1/merge"),
    ];
    assert_eq!(
        cache::obsolete(&entries, None)
            .unwrap()
            .iter()
            .map(|v| v["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1]
    );
    entries[1]["version"] = "different-layout".into();
    assert!(cache::obsolete(&entries, None).unwrap().is_empty());
    let current = "a".repeat(64);
    let prefix = format!("application-v2-ubuntu-24.04-{current}-");
    let mut entries = vec![
        entry(
            1,
            format!("application-v1-ubuntu-24.04-{}-full", "1".repeat(64)),
            "refs/heads/main",
        ),
        entry(
            2,
            format!(
                "application-v2-macos-15-{}-{}-full",
                "b".repeat(64),
                "2".repeat(64)
            ),
            "refs/heads/main",
        ),
        entry(
            3,
            format!("{prefix}{}-full", "3".repeat(64)),
            "refs/heads/main",
        ),
        entry(
            4,
            format!("{prefix}{}-full", "4".repeat(64)),
            "refs/heads/main",
        ),
        entry(
            5,
            format!("{prefix}{}-focus", "4".repeat(64)),
            "refs/heads/main",
        ),
        entry(
            6,
            format!("{prefix}{}-focus", "6".repeat(64)),
            "refs/heads/main",
        ),
    ];
    assert_eq!(
        cache::obsolete(&entries, Some(&current))
            .unwrap()
            .iter()
            .map(|v| v["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 3, 5]
    );
    entries[4]["version"] = "another-layout".into();
    assert_eq!(
        cache::obsolete(&entries[..5], Some(&current))
            .unwrap()
            .iter()
            .map(|v| v["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 3]
    );
    let mut entries = Vec::new();
    let mut id = 0;
    for platform in ["ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"] {
        for index in 1..=3 {
            for phase in ["full", "focus"] {
                id += 1;
                let mut e = entry(
                    id,
                    format!(
                        "application-v1-{platform}-{}-{phase}",
                        index.to_string().repeat(64)
                    ),
                    "refs/heads/main",
                );
                e["created_at"] = format!("2026-10-{index:02}T00:00:00Z").into();
                entries.push(e);
            }
        }
        for index in [4, 5] {
            id += 1;
            let mut e = entry(
                id,
                format!(
                    "application-v1-{platform}-{}-focus",
                    index.to_string().repeat(64)
                ),
                "refs/heads/main",
            );
            e["created_at"] = format!("2026-10-{index:02}T00:00:00Z").into();
            entries.push(e);
        }
    }
    let removed = cache::obsolete(&entries, None).unwrap();
    let kept = entries
        .iter()
        .filter(|e| !removed.iter().any(|r| r["id"] == e["id"]))
        .collect::<Vec<_>>();
    assert_eq!(kept.len(), 9);
    assert!(
        kept.iter()
            .all(|e| e["key"].as_str().unwrap().ends_with("-full")
                || e["key"].as_str().unwrap().contains(&"5".repeat(64)))
    );
}

#[test]
fn audit_cache_identity_and_observed_version_bind_the_actual_binary() {
    let original = cache::audit_identity("compiler-one", "image-one", "X64").unwrap();
    for (compiler, image, arch) in [
        ("compiler-two", "image-one", "X64"),
        ("compiler-one", "image-two", "X64"),
        ("compiler-one", "image-one", "ARM64"),
    ] {
        assert_ne!(
            cache::audit_identity(compiler, image, arch).unwrap(),
            original
        );
    }
    assert!(cache::audit_identity("compiler", "unknown", "X64").is_err());
    for version in ["cargo-deny 0.20.2\n", "cargo-deny wrong\n"] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let table = temp.path().join("version.json");
        fs::write(
            &table,
            util::encode(&json!({"[\"--version\"]":{"stdout":version}})).unwrap(),
        )
        .unwrap();
        let path = bin.join("cargo-deny");
        install::install(
            &util::root(),
            &path,
            &[
                "fixture-command".into(),
                table.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        let record = cache::audit_record(temp.path(), &original);
        if version.contains("wrong") {
            assert!(record.is_err());
        } else {
            let record = record.unwrap();
            assert_eq!(record["sha256"], util::file_digest(&path).unwrap());
            assert_eq!(record["fingerprint"], original);
        }
    }
}

#[test]
fn nested_cargo_preserves_fresh_build_scripts_when_the_calling_package_metadata_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let target = temp.path().join("target");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='nested-cargo-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "#[cfg(not(nested_fixture))]\ncompile_error!(\"Configured RUSTFLAGS were lost\");\npub const VALUE: usize = 1;\n").unwrap();
    fs::write(
        root.join("build.rs"),
        r#"fn main() {
 for name in ["CARGO_MANIFEST_DIR","CARGO_MANIFEST_PATH","CARGO_MANIFEST_LINKS","CARGO_PKG_NAME","CARGO_PKG_VERSION","CARGO_PRIMARY_PACKAGE","CARGO_CRATE_NAME","CARGO_BIN_NAME"] {
  println!("cargo:rerun-if-env-changed={name}");
 }
 let path=std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("counter");
 let count=std::fs::read_to_string(&path).ok().and_then(|s|s.parse::<u32>().ok()).unwrap_or(0);
 std::fs::write(path,(count+1).to_string()).unwrap();
}"#,
    )
    .unwrap();
    for outer in [None, Some("/different/outer/cargo/package")] {
        let mut command = Command::new("cargo");
        command
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .env("RUSTFLAGS", "--cfg nested_fixture")
            .env("CARGO_BUILD_JOBS", "1")
            .args(["check", "--offline", "-vv"]);
        if let Some(outer) = outer {
            for name in [
                "CARGO_MANIFEST_DIR",
                "CARGO_MANIFEST_PATH",
                "CARGO_MANIFEST_LINKS",
                "CARGO_PKG_NAME",
                "CARGO_PKG_VERSION",
                "CARGO_PRIMARY_PACKAGE",
                "CARGO_CRATE_NAME",
                "CARGO_BIN_NAME",
            ] {
                command.env(name, outer);
            }
        }
        let output = process::output(&mut command, Duration::from_secs(60)).unwrap();
        assert!(
            !output.timed_out && output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let counters = util::walk(&target)
            .unwrap()
            .into_iter()
            .filter(|p| p.file_name().unwrap() == "counter")
            .collect::<Vec<_>>();
        assert_eq!(counters.len(), 1);
        assert_eq!(
            fs::read_to_string(&counters[0]).unwrap(),
            "1",
            "Caller Cargo context must not rerun dependency build scripts"
        );
    }
}
