//! Conservative test scheduling; early reproductions are excluded only from later identical checks.
use super::focus;
use anyhow::{Result, ensure};
use std::{collections::BTreeSet, path::Path};

pub type Invocation = Vec<String>;
pub const COLOR_FONT_REPRODUCTION: &str = "color_fonts::";
pub const NATIVE_CLIPBOARD_TEST: &str =
    "real_native_clipboard_ctrl_v_preserves_two_images_and_delivers_their_pixels";

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
pub fn plan(root: &Path, suite: &str, targets: &[String]) -> Result<Plan> {
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
            build: vec![build, words(&["cargo", "build", "--locked"])],
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
    let mut builds = vec![build, app];
    if !core && has("script_scope") {
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
            targets
                .iter()
                .map(String::as_str)
                .eq(focus::RENDER_TARGETS.iter().copied()),
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
        ensure!(
            targets.len() == 3
                && targets.iter().map(String::as_str).collect::<BTreeSet<_>>()
                    == ["browser_readiness", "project", "script_scope"].into(),
            "Browser focus requires the complete document/script/browser target set"
        );
        let reproductions = [
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
        c.extend(selected(targets, &excluded.into_iter().collect::<Vec<_>>()));
        c.extend(words(&["--", "--test-threads=2"]));
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
