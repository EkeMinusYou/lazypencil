//! Conservative test scheduling; early reproductions are excluded only from later identical checks.
use super::focus;
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub type Invocation = Vec<String>;

pub const TEST_PROFILE_ENV: &str = "LAZYPENCIL_CI_TEST_PROFILE";
pub const TEST_PROFILE_CONFIG: [&str; 3] = [
    "profile.test.package.lazypencil.opt-level=1",
    "profile.test.package.lazypencil.debug-assertions=true",
    "profile.test.package.lazypencil.overflow-checks=true",
];

pub fn optimized_tests() -> Result<bool> {
    let value = std::env::var(TEST_PROFILE_ENV).unwrap_or_default();
    ensure!(
        value.is_empty() || value == "optimized-debug-v1",
        "Unknown CI test profile"
    );
    Ok(!value.is_empty())
}

/// Tune only our test package, retaining both development checks. Registry,
/// native, normal-development and release profiles keep their existing flags.
pub fn test_profile(invocation: &[String], enabled: bool) -> Invocation {
    let end = invocation
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(invocation.len());
    let cargo = &invocation[..end];
    let mut configured = invocation.to_vec();
    if enabled
        && cargo.first().is_some_and(|arg| arg == "cargo")
        && cargo.get(1).is_some_and(|arg| arg == "test")
        && !cargo
            .iter()
            .any(|arg| arg == "--release" || arg == "--profile" || arg.starts_with("--profile="))
    {
        configured.splice(
            2..2,
            TEST_PROFILE_CONFIG
                .into_iter()
                .flat_map(|option| ["--config".into(), option.into()]),
        );
    }
    configured
}

/// Change Cargo's scheduling without changing compiler-visible environment or flags.
pub fn build_jobs(invocation: &[String], linux: bool) -> Invocation {
    build_jobs_for(invocation, if linux { 2 } else { 1 })
}

pub fn build_jobs_for(invocation: &[String], jobs: usize) -> Invocation {
    let mut scheduled = invocation.to_vec();
    let cargo_end = invocation
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(invocation.len());
    let cargo = &invocation[..cargo_end];
    let compiles = cargo.first().is_some_and(|argument| argument == "cargo")
        && match cargo.get(1).map(String::as_str) {
            Some("build" | "rustc" | "clippy") => true,
            Some("test") => cargo.iter().any(|argument| argument == "--no-run"),
            _ => false,
        };
    let explicit = cargo.iter().skip(2).any(|argument| {
        argument == "--jobs" || argument.starts_with("--jobs=") || argument.starts_with("-j")
    });
    if jobs > 1 && compiles && !explicit {
        scheduled.splice(2..2, ["--jobs".into(), jobs.to_string()]);
    }
    scheduled
}

/// Keep both normal and test build configurations, ending in the test graph
/// that the following reproductions consume. Do not switch back between them.
pub fn catalog_builds(targets: &[String]) -> [Invocation; 2] {
    let normal = words(&["cargo", "build", "--locked"]);
    let mut tests = words(&[
        "cargo",
        "test",
        "--locked",
        "--no-run",
        "--message-format=json-render-diagnostics",
    ]);
    for target in targets {
        tests.extend(["--test".into(), target.clone()]);
    }
    [normal, tests]
}

pub const COLOR_FONT_REPRODUCTION: &str = "color_fonts::";
pub const MANAGEMENT_REPRODUCTION: &str = "conversation::management::";
pub const PREVIEW_REPRODUCTIONS: [&str; 3] = [
    "render::tests::",
    "tui::camera::tests::",
    "tui::tests::cached_camera_feedback_keeps_exact_exports_and_generation_ownership_separate",
];
pub const NATIVE_CLIPBOARD_TEST: &str =
    "real_native_clipboard_ctrl_v_preserves_two_images_and_delivers_their_pixels";
const DEFAULT_REPRODUCTIONS: [&str; 2] = [
    "tui::tests::",
    "cancelling_a_running_script_unblocks_the_mcp_document_worker",
];

fn default_flags() -> Invocation {
    let mut flags = words(&["--test-threads=2"]);
    flags.extend(skips(DEFAULT_REPRODUCTIONS));
    flags
}

pub fn default_doctests() -> Invocation {
    let mut command = words(&["cargo", "test", "--locked", "--doc", "--no-fail-fast", "--"]);
    command.extend(default_flags());
    command
}

pub fn default_cargo_tests() -> Invocation {
    let mut command = words(&["cargo", "test", "--locked", "--no-fail-fast", "--"]);
    command.extend(default_flags());
    command.extend(skips([MANAGEMENT_REPRODUCTION]));
    command
}

/// These tests share one bounded process-global cleanup worker. Keep the
/// resource in its own harness process while unrelated tests remain parallel.
pub fn management_tests(features: bool) -> Invocation {
    let mut command = words(&["cargo", "test", "--locked"]);
    if !features {
        command.push("--no-default-features".into());
    }
    command.extend(words(&[
        "--lib",
        MANAGEMENT_REPRODUCTION,
        "--",
        "--test-threads=1",
    ]));
    command
}

pub fn core_tests() -> Invocation {
    let mut command = words(&[
        "cargo",
        "test",
        "--locked",
        "--no-default-features",
        "--no-fail-fast",
        "--",
        "--test-threads=2",
    ]);
    command.extend(skips([MANAGEMENT_REPRODUCTION]));
    command
}

/// Require every Cargo test target before running immutable binaries without its build lock.
pub fn default_tests(root: &Path, binaries: &BTreeMap<String, PathBuf>) -> Result<Vec<Invocation>> {
    let (integration, _) = focus::integration_targets(root)?;
    let mut expected = integration.clone();
    expected.extend(["lib".into(), "lazypencil".into()]);
    ensure!(
        binaries.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "Default verification requires the complete library, binary and integration test catalog"
    );
    ["lib".to_owned(), "lazypencil".to_owned()]
        .into_iter()
        .chain(integration)
        .map(|name| default_test(&name, &binaries[&name]))
        .collect()
}

pub fn default_test(name: &str, path: &Path) -> Result<Invocation> {
    ensure!(path.is_absolute(), "Test executable must be absolute");
    let mut command = vec![path.to_string_lossy().into_owned()];
    command.extend(default_flags());
    if name == "lib" {
        command.extend(skips([MANAGEMENT_REPRODUCTION]));
    }
    Ok(command)
}

/// Clipboard ownership belongs to its dedicated stage, outside the runtime lifecycle suite.
pub fn lifecycle(binary: &Path, target: &str) -> Result<Invocation> {
    ensure!(["mcp", "tui"].contains(&target), "Unknown lifecycle target");
    let mut command = vec![binary.to_string_lossy().into_owned()];
    if target == "mcp" {
        command
            .push("idle_mcp_reaps_script_page_and_shader_processes_without_another_request".into());
    }
    command.extend(words(&["--ignored", "--test-threads=1"]));
    if target == "tui" {
        command.extend(skips([NATIVE_CLIPBOARD_TEST]));
    }
    Ok(command)
}

pub fn color_font_reproduction(suite: &str, targets: &[String]) -> Option<Invocation> {
    (["graphics", "dynamic", "copy"].contains(&suite)
        && targets.iter().any(|target| target == "font_sources"))
    .then(|| {
        words(&[
            "cargo",
            "test",
            "--locked",
            "--test",
            "font_sources",
            COLOR_FONT_REPRODUCTION,
            "--",
            "--test-threads=1",
        ])
    })
}

pub fn terminal_reproduction(suite: &str, targets: &[String]) -> Option<Invocation> {
    (!["core", "conversation"].contains(&suite) && targets.iter().any(|target| target == "tui"))
        .then(|| {
            words(&[
                "cargo",
                "test",
                "--locked",
                "--test",
                "tui",
                focus::TERMINAL_MODES,
                "--",
                "--exact",
                "--test-threads=1",
            ])
        })
}
fn words(values: &[&str]) -> Invocation {
    values.iter().map(|s| (*s).into()).collect()
}
fn append(base: &[String], values: &[&str]) -> Invocation {
    let mut out = base.to_vec();
    out.extend(words(values));
    out
}
fn selected(targets: &[String], exclude: &[&str]) -> Invocation {
    targets
        .iter()
        .filter(|t| !exclude.contains(&t.as_str()))
        .flat_map(|t| ["--test".into(), t.clone()])
        .collect()
}
fn skips(names: impl IntoIterator<Item = impl AsRef<str>>) -> Invocation {
    names
        .into_iter()
        .flat_map(|s| ["--skip".into(), s.as_ref().to_owned()])
        .collect()
}
pub struct Plan {
    pub build: Vec<Invocation>,
    pub checks: Vec<Invocation>,
}

/// Integration binaries used by FocusReproduce, rather than every target
/// selected for the later complete verification. The library is always built
/// separately through --lib. This changes build order, never the final suite.
pub fn reproduction_targets(suite: &str, targets: &[String]) -> Vec<String> {
    let mut required: Vec<_> = targets
        .iter()
        .filter(|name| {
            [
                "agent",
                "user_agent_config",
                "graphics",
                "tui",
                "user_config",
                "font_sources",
                "native_layout",
                "conversation_history",
                "mcp",
                "script_scope",
                "attachments",
            ]
            .contains(&name.as_str())
        })
        .cloned()
        .collect();
    for target in if suite == "dynamic" {
        &["dynamic_render", "shader_uniforms"][..]
    } else if suite == "shader" {
        &["shader_uniforms"][..]
    } else {
        &[][..]
    } {
        if !required.iter().any(|name| name == target) {
            required.push((*target).to_owned());
        }
    }
    required
}

pub fn reproduction_build(command: &Invocation, required: &[String]) -> Invocation {
    // Keep optional/core feature builds intact. Only the ordinary --no-run
    // compilation can leave unrelated integration binaries to the full build.
    if command.get(1).is_none_or(|verb| verb != "test")
        || !command.iter().any(|arg| arg == "--no-run")
        || command
            .iter()
            .any(|arg| ["--no-default-features", "--features"].contains(&arg.as_str()))
    {
        return command.clone();
    }
    let mut result = Vec::new();
    let mut arguments = command.iter();
    while let Some(argument) = arguments.next() {
        if argument == "--test" {
            let target = arguments.next().expect("Validated test target");
            if required.contains(target) {
                result.extend([argument.clone(), target.clone()]);
            }
        } else {
            result.push(argument.clone());
        }
    }
    result
}

pub fn plan(root: &Path, suite: &str, targets: &[String]) -> Result<Plan> {
    plan_with_core(root, suite, targets, false)
}

/// A complete core lane can own namespace checks without compiling that graph
/// again in the default builder. Rendering-only attachment checks stay here.
pub fn plan_with_core(
    root: &Path,
    suite: &str,
    targets: &[String],
    separate_core: bool,
) -> Result<Plan> {
    let (allowed, _) = focus::integration_targets(root)?;
    ensure!(
        !targets.is_empty()
            && targets
                .iter()
                .all(|t| allowed.contains(t) && t != "agent_workflow"),
        "Expected existing integration test targets"
    );
    ensure!(
        [
            "core",
            "conversation",
            "graphics",
            "copy",
            "dynamic",
            "browser",
            "shader",
            "release",
            "candidate",
            "render"
        ]
        .contains(&suite),
        "Invalid focused suite"
    );
    let has = |name| targets.iter().any(|t| t == name);
    let core = ["core", "conversation"].contains(&suite);
    let mut common = words(&["cargo", "test", "--locked"]);
    if core {
        common.push("--no-default-features".into());
    }
    let mut input_skip =
        if ["graphics", "dynamic"].contains(&suite) && has("tui") && has("user_config") {
            skips(["config::keys::", focus::INPUT_PTY, focus::INPUT_CONFIG])
        } else {
            vec![]
        };
    if color_font_reproduction(suite, targets).is_some() {
        input_skip.extend(skips([COLOR_FONT_REPRODUCTION]));
    }
    if terminal_reproduction(suite, targets).is_some() {
        input_skip.extend(skips([focus::TERMINAL_MODES]));
    }
    if !core && has("graphics") {
        input_skip.extend(skips([focus::TEXT_LAYER_REPRODUCTION]));
    }
    if ["graphics", "dynamic", "copy"].contains(&suite) && has("script_scope") {
        input_skip.extend(skips(["script_bindings::", "generated_browser_"]));
        input_skip.extend(skips(focus::COPY_CPU));
    }
    let shader_first: Vec<_> = if ["shader", "dynamic"].contains(&suite) && has("shader_uniforms") {
        focus::SHADER_REGRESSIONS
            .iter()
            .map(|name| {
                append(
                    &common,
                    &[
                        "--test",
                        "shader_uniforms",
                        name,
                        "--",
                        "--ignored",
                        "--exact",
                        "--test-threads=1",
                    ],
                )
            })
            .collect()
    } else {
        vec![]
    };
    let shader_skip = if shader_first.is_empty() {
        vec![]
    } else {
        skips(focus::SHADER_REGRESSIONS)
    };
    let all_selected = selected(targets, &[]);
    if suite == "core" && targets == ["guidance"] {
        // Guidance has integration contracts and no library unit tests of its
        // own. Do not repeat unrelated lifecycle/library tests in this focus.
        let mut build = common.clone();
        build.extend(all_selected.clone());
        build.push("--no-run".into());
        let mut check = common.clone();
        check.extend(all_selected);
        check.extend(words(&["--", "--test-threads=2"]));
        return Ok(Plan {
            build: vec![
                build,
                words(&["cargo", "build", "--locked", "--no-default-features"]),
            ],
            checks: vec![check],
        });
    }
    if suite == "shader" {
        ensure!(
            targets == ["shader_uniforms"],
            "Shader focus requires only shader_uniforms"
        );
        let mut build = common.clone();
        build.extend(all_selected.clone());
        build.push("--no-run".into());
        let mut checks = shader_first;
        let mut rest = common;
        rest.extend(all_selected);
        rest.extend(words(&["--", "--ignored", "--test-threads=1"]));
        rest.extend(shader_skip);
        checks.push(rest);
        return Ok(Plan {
            build: if separate_core {
                vec![build]
            } else {
                vec![build, words(&["cargo", "build", "--locked"])]
            },
            checks,
        });
    }
    let mut build = append(&common, &["--lib"]);
    build.extend(all_selected);
    build.push("--no-run".into());
    let mut app = words(&["cargo", "build", "--locked"]);
    if core {
        app.push("--no-default-features".into());
    }
    let mut builds = if separate_core && !core {
        vec![build]
    } else {
        vec![build, app]
    };
    if suite == "graphics" && has("attachments") {
        // Exercise the optional-TUI boundary before ending in the default
        // build graph used by reproductions and the shared focused artifact.
        // The same image contracts run in both distinct feature configurations.
        builds.insert(
            0,
            words(&[
                "cargo",
                "test",
                "--locked",
                "--no-default-features",
                "--features",
                "rendering",
                "--test",
                "attachments",
                "--",
                "--test-threads=2",
            ]),
        );
    }
    if !core && has("script_scope") && !separate_core {
        builds.push(words(&[
            "cargo",
            "test",
            "--locked",
            "--no-default-features",
            "--lib",
            "--no-run",
        ]));
    }
    if suite == "render" {
        ensure!(
            targets.iter().map(String::as_str).eq(focus::RENDER_TARGETS
                .iter()
                .copied()
                .chain(has("tui").then_some("tui"))),
            "Renderer focus requires its complete native and dynamic target set"
        );
        let mut normal = append(&common, &["--lib"]);
        normal.extend(selected(targets, &[]));
        normal.extend(words(&["--", "--test-threads=2"]));
        normal.extend(input_skip);
        let mut checks = vec![normal];
        for (target, name) in [
            ("dynamic_render", focus::RENDER_BROWSER),
            ("shader_uniforms", focus::SHADER_REGRESSIONS[1]),
        ] {
            checks.push(append(
                &common,
                &[
                    "--test",
                    target,
                    name,
                    "--",
                    "--ignored",
                    "--exact",
                    "--test-threads=1",
                ],
            ));
        }
        return Ok(Plan {
            build: builds,
            checks,
        });
    }
    if suite == "browser" {
        let required: BTreeSet<_> = ["browser_readiness", "project", "script_scope"].into();
        let selected = targets.iter().map(String::as_str).collect::<BTreeSet<_>>();
        ensure!(
            targets.len() == selected.len()
                && required.is_subset(&selected)
                && selected
                    .iter()
                    .all(|name| required.contains(name) || *name == "guidance"),
            "Browser focus requires the complete document/script/browser target set"
        );
        let reproductions = [
            focus::HEADED_LOGIN,
            focus::GENERATED_BROWSER,
            focus::LOCAL_FILES,
            focus::BROWSER_COPY,
            focus::LOCAL_PAGES,
        ];
        let mut checks = reproductions
            .iter()
            .map(|name| {
                append(
                    &common,
                    &[
                        "--test",
                        "browser_readiness",
                        name,
                        "--",
                        "--exact",
                        "--ignored",
                        "--test-threads=1",
                    ],
                )
            })
            .collect::<Vec<_>>();
        if has("guidance") {
            checks.insert(
                0,
                append(&common, &["--test", "guidance", "--", "--test-threads=2"]),
            );
        }
        let mut browsers = append(
            &common,
            &[
                "--test",
                "browser_readiness",
                "--",
                "--include-ignored",
                "--test-threads=1",
            ],
        );
        browsers.extend(skips(reproductions));
        checks.push(browsers);
        let mut scripts = append(
            &common,
            &[
                "--test",
                "script_scope",
                "--",
                "--include-ignored",
                "--test-threads=1",
            ],
        );
        scripts.extend(skips(["script_bindings::", "generated_browser_"]));
        scripts.extend(skips(focus::COPY_CPU));
        checks.push(scripts);
        checks.push(append(
            &common,
            &["--test", "project", "--", "--test-threads=2"],
        ));
        return Ok(Plan {
            build: builds,
            checks,
        });
    }
    let mut checks = shader_first;
    if suite == "dynamic" && has("browser_readiness") {
        checks.insert(
            0,
            append(
                &common,
                &[
                    "--test",
                    "browser_readiness",
                    focus::HEADED_LOGIN,
                    "--",
                    "--exact",
                    "--ignored",
                    "--test-threads=1",
                ],
            ),
        );
    }
    // Dynamic focus otherwise selects only CDP/pages/TUI library namespaces.
    // Exercise the native pixel transfer and camera math before browser work.
    if suite == "dynamic" && has("tui") {
        let preview = PREVIEW_REPRODUCTIONS
            .map(|filter| append(&common, &["--lib", filter, "--", "--test-threads=1"]));
        checks.splice(0..0, preview);
    }
    let history = has("conversation_history");
    if history && core {
        checks.push(append(
            &common,
            &[
                "--lib",
                "conversation::management::",
                "--",
                "--test-threads=2",
            ],
        ));
        checks.push(append(
            &common,
            &["--lib", "conversation::history::", "--", "--test-threads=2"],
        ));
        checks.push(append(
            &common,
            &["--test", "conversation_history", "--", "--test-threads=1"],
        ));
        if has("mcp") {
            checks.push(append(
                &common,
                &[
                    "--test",
                    "mcp",
                    focus::OWNED_MCP,
                    "--",
                    "--exact",
                    "--test-threads=1",
                ],
            ));
        }
    }
    let mut history_skip = if history && has("tui") {
        let mut s = skips(["tui::tests::saved_log_"]);
        s.extend(skips(focus::HISTORY_PTY));
        s
    } else {
        vec![]
    };
    if history && has("agent") && !core {
        history_skip.extend(skips([focus::RECLAIM]));
    }
    if history && has("mcp") {
        history_skip.extend(skips([focus::OWNED_MCP]));
    }
    let native = has("native_layout") && ["graphics", "dynamic", "copy"].contains(&suite);
    if native && ["graphics", "copy"].contains(&suite) {
        checks.push(append(
            &common,
            &["--lib", "native_cancellation", "--", "--test-threads=2"],
        ));
        checks.push(append(
            &common,
            &["--test", "native_layout", "--", "--test-threads=2"],
        ));
    }
    if suite == "conversation" {
        checks.push(append(
            &common,
            &[
                "--test",
                "agent",
                "agent_ownership::killed_cli_",
                "--",
                "--test-threads=1",
                "--nocapture",
            ],
        ));
        checks.push(append(
            &common,
            &[
                "--test",
                "agent",
                "embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits",
                "--",
                "--exact",
                "--nocapture",
            ],
        ));
        checks.push(append(
            &common,
            &[
                "--test",
                "agent",
                "session_restart_",
                "--",
                "--test-threads=1",
                "--nocapture",
            ],
        ));
    }
    if suite == "dynamic" {
        checks.push(append(
            &common,
            &["--lib", "cdp::", "--", "--test-threads=2"],
        ));
        checks.push(append(
            &common,
            &["--lib", "dynamic::pages::tests::", "--", "--test-threads=2"],
        ));
        if has("browser_readiness") {
            for reproduction in [
                focus::PAGE_DIAGNOSTICS,
                focus::NATIVE_ANIMATION,
                focus::OFFLINE_NETWORK,
                focus::OFFLINE_TRANSPORT,
                focus::GENERATED_BROWSER,
                focus::LOCAL_FILES,
                focus::BROWSER_COPY,
                focus::LOCAL_PAGES,
            ] {
                checks.push(append(
                    &common,
                    &[
                        "--test",
                        "browser_readiness",
                        reproduction,
                        "--",
                        "--exact",
                        "--ignored",
                        "--test-threads=1",
                    ],
                ));
            }
        }
        let mut tui = append(
            &common,
            &["--lib", "tui::tests::", "--", "--test-threads=2"],
        );
        tui.extend(history_skip.clone());
        if has("tui") {
            tui.extend(skips([PREVIEW_REPRODUCTIONS[2]]));
        }
        checks.push(tui);
        if has("dynamic_render") {
            checks.push(append(
                &common,
                &[
                    "--test",
                    "dynamic_render",
                    "retained_",
                    "--",
                    "--ignored",
                    "--test-threads=1",
                ],
            ));
        }
        for (only, names) in [
            (
                true,
                targets
                    .iter()
                    .filter(|t| ["dynamic_render", "shader_uniforms"].contains(&t.as_str()))
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            (
                false,
                targets
                    .iter()
                    .filter(|t| {
                        ![
                            "dynamic_render",
                            "shader_uniforms",
                            "conversation_history",
                            "native_layout",
                            "font_sources",
                        ]
                        .contains(&t.as_str())
                    })
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        ] {
            if names.is_empty() {
                continue;
            }
            let mut c = common.clone();
            c.extend(selected(&names, &[]));
            c.extend(words(&[
                "--",
                if only {
                    "--ignored"
                } else {
                    "--include-ignored"
                },
                "--test-threads=1",
                "--skip",
                "retained_",
            ]));
            if names.iter().any(|name| name == "browser_readiness") {
                if suite == "dynamic" {
                    c.extend(skips([
                        focus::HEADED_LOGIN,
                        focus::PAGE_DIAGNOSTICS,
                        focus::NATIVE_ANIMATION,
                        focus::OFFLINE_NETWORK,
                        focus::OFFLINE_TRANSPORT,
                    ]));
                }
                c.extend(skips([
                    focus::LOCAL_FILES,
                    focus::BROWSER_COPY,
                    focus::GENERATED_BROWSER,
                    focus::LOCAL_PAGES,
                ]));
            }
            c.extend(shader_skip.clone());
            c.extend(input_skip.clone());
            c.extend(history_skip.clone());
            if names.iter().any(|name| name == "tui") {
                c.extend(skips([NATIVE_CLIPBOARD_TEST]));
            }
            checks.push(c);
        }
        if has("font_sources") {
            let mut fonts = append(
                &common,
                &["--test", "font_sources", "--", "--test-threads=2"],
            );
            fonts.extend(input_skip.clone());
            checks.push(fonts);
        }
    } else {
        let mut excluded = BTreeSet::new();
        if history {
            excluded.insert("conversation_history");
        }
        if native {
            excluded.insert("native_layout");
        }
        let mut c = append(&common, &["--lib"]);
        let isolated_management = suite == "candidate" && has("tui") && !history;
        if isolated_management {
            checks.push(management_tests(true));
        }
        c.extend(selected(targets, &excluded.into_iter().collect::<Vec<_>>()));
        c.extend(words(&["--", "--test-threads=2"]));
        if isolated_management {
            c.extend(skips([MANAGEMENT_REPRODUCTION]));
        }
        c.extend(input_skip);
        c.extend(history_skip);
        if history {
            c.extend(skips([
                "conversation::history::",
                "conversation::management::",
            ]));
        }
        if native {
            c.extend(skips(["native_cancellation"]));
        }
        if suite == "conversation" {
            c.extend(skips([
                "agent_ownership::killed_cli_",
                "session_restart_",
                "embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits",
            ]));
        }
        checks.push(c);
    }
    Ok(Plan {
        build: builds,
        checks,
    })
}
