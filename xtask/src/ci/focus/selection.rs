//! Ordered source-consumer rules for the focused verification plan.
use super::*;

pub fn select(root: &Path, paths: &[String], platform: &str) -> Result<Plan> {
    // Repository prose has no compiler consumers. Remove it before specialized
    // subset checks as well as stem matching; embedded resources remain inputs.
    let inputs: Vec<_> = paths
        .iter()
        .filter(|path| !documentation_only(path))
        .map(|path| source_owner(path).to_owned())
        .collect();
    let paths = inputs.as_slice();
    ensure!(
        platform == "auto" || PLATFORMS.contains(&platform),
        "Unsupported focus platform"
    );
    let platform = if platform == "auto" {
        RELEASE_PLATFORMS[0]
    } else {
        platform
    }
    .to_owned();
    let preprocessor = preprocessor_changed(paths);
    if agent_only(paths) {
        return Ok(Plan {
            platform,
            suite: "conversation".into(),
            targets: [
                "agent",
                "config",
                "conversation_history",
                "mcp",
                "user_agent_config",
            ]
            .map(str::to_owned)
            .to_vec(),
            preprocessor,
        });
    }
    if runtime_only(paths) {
        return Ok(Plan {
            platform,
            suite: "core".into(),
            targets: vec!["runtime".into()],
            preprocessor,
        });
    }
    // The clipboard worker and attachment persistence share the encoded image
    // boundary. Their real consumer needs rendering, even when conversation
    // metadata changes would otherwise select the core-only ownership suite.
    if image_input_only(paths) {
        let mut targets = vec!["attachments".into()];
        if paths.iter().any(|path| path.starts_with("src/tui/")) {
            targets.push("tui".into());
        }
        if paths
            .iter()
            .any(|path| path.starts_with("src/config") || path == "tests/user_config.rs")
        {
            targets.push("user_config".into());
        }
        return Ok(Plan {
            platform,
            suite: "graphics".into(),
            targets,
            preprocessor,
        });
    }
    // Installation and file verification have no rendering feature dependency.
    // Reproduce these failures before native compilation and browser setup;
    // mixed rendering/CDP changes retain the broader existing plan.
    if any(paths, &["src/runtime.rs", "tests/runtime.rs"], &[])
        && paths.iter().all(|path| {
            [
                "src/runtime.rs",
                "src/runtime/download.rs",
                "src/cli.rs",
                "src/main.rs",
                "src/store.rs",
                "tests/runtime.rs",
                "tests/attachments.rs",
                "xtask/src/bin/test-helper.rs",
                "xtask/src/ci/commands.rs",
                "xtask/src/ci/focus.rs",
                "xtask/tests/ci.rs",
                ".github/actions/xtask/action.yml",
                "xtask/src/ci/cache.rs",
                "xtask/tests/checkpoints.rs",
                "xtask/tests/compatible_cache.rs",
                "xtask/tests/fixtures/focus.json",
            ]
            .contains(&path.as_str())
        })
    {
        let attachments = paths
            .iter()
            .any(|path| path == "tests/attachments.rs" || path == "xtask/src/bin/test-helper.rs");
        let mut targets = if paths.iter().any(|path| path == "src/store.rs") {
            ["document_store", "operations_cli", "project", "runtime"]
                .map(str::to_owned)
                .to_vec()
        } else {
            vec!["runtime".into()]
        };
        if attachments {
            targets.insert(0, "attachments".into());
        }
        return Ok(Plan {
            platform,
            suite: if attachments { "graphics" } else { "core" }.into(),
            targets,
            preprocessor,
        });
    }
    // These bodies are real include_str! inputs, unlike repository prose. Their
    // catalog/CLI contract has no Skia or Chromium dependency; the full job still
    // exercises the rendering consumers and both feature configurations.
    if paths.iter().any(|path| bundled_guidance(path))
        && paths
            .iter()
            .all(|path| ci_only(path) || bundled_guidance(path))
    {
        return Ok(Plan {
            platform,
            suite: "core".into(),
            targets: vec!["guidance".into()],
            preprocessor,
        });
    }
    let font_inputs = paths.iter().any(|path| font_input(path));
    let color_input = any(paths, &["src/color.rs"], &[]);
    let renderer = |path: &str| path == "src/render.rs" || path.starts_with("src/render/");
    if paths.iter().any(|path| renderer(path))
        && paths.iter().all(|path| {
            ci_only(path)
                || path.starts_with("resources/licenses/native/")
                || renderer(path)
                || path == "src/tui/preview.rs"
                || [
                    "tests/graphics.rs",
                    "tests/render.rs",
                    "tests/blends.rs",
                    "tests/mesh.rs",
                ]
                .contains(&path.as_str())
        })
    {
        let mut targets = RENDER_TARGETS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        if paths.iter().any(|path| path == "src/tui/preview.rs") {
            targets.push("tui".into());
        }
        return Ok(Plan {
            platform,
            suite: "render".into(),
            targets,
            preprocessor,
        });
    }
    // The complete normal catalog also covers the runtime-only ACP helper's
    // normal consumers. Mixed scheduler changes must not fall back to a small
    // agent-only suite; changed runtime schedulers additionally enable their
    // existing complete runtime reproduction before the final matrix.
    if paths
        .iter()
        .all(|path| ci_only(path) || path == "xtask/src/test_support/acp.rs")
        && paths
            .iter()
            .any(|path| path == "xtask/src/ci/native/normal.rs")
    {
        return Ok(Plan {
            platform,
            suite: "normal-ci".into(),
            targets: Vec::new(),
            preprocessor,
        });
    }
    if paths.iter().all(|path| ci_only(path))
        && paths
            .iter()
            .any(|path| path == "xtask/src/ci/native/browser.rs")
    {
        return Ok(Plan {
            platform,
            suite: "browser-ci".into(),
            targets: BROWSER_CI_TARGETS
                .iter()
                .map(|name| (*name).into())
                .collect(),
            preprocessor,
        });
    }
    if paths.iter().any(|path| {
        [
            "src/tui/graphics.rs",
            "src/tui/terminal.rs",
            "src/tui/camera.rs",
            "src/render/preview.rs",
            "xtask/src/test_support/terminal.rs",
        ]
        .contains(&path.as_str())
            || path == "src/tui/preview.rs" && tui_measurement_changed(paths)
    }) && paths.iter().all(|path| {
        ci_only(path)
            || [
                "src/tui/graphics.rs",
                "src/tui/terminal.rs",
                "src/tui/camera.rs",
                "src/render/preview.rs",
                "src/tui/mod.rs",
                "src/tui/preview.rs",
                "src/tui/tests.rs",
                "tests/support/terminal/cases.rs",
                "tests/support/terminal/mod.rs",
                "tests/tui.rs",
                "xtask/src/test_support/terminal.rs",
            ]
            .contains(&path.as_str())
    }) {
        return Ok(Plan {
            platform,
            suite: "candidate".into(),
            targets: if paths.iter().any(|path| {
                path == "xtask/src/test_support/terminal.rs"
                    || path.starts_with("tests/support/terminal/")
            }) {
                vec!["render".into(), "tui".into(), "dynamic_render".into()]
            } else {
                vec!["render".into(), "tui".into()]
            },
            preprocessor,
        });
    }
    // TUI fixture/measurement changes need the actual TUI tests and candidate,
    // rather than unrelated browser suites selected by the shared file stem.
    if tui_measurement_changed(paths)
        && native_clipboard_changed(paths)
        && paths
            .iter()
            .all(|path| ci_only(path) || path == "tests/tui.rs")
    {
        return Ok(Plan {
            platform,
            suite: "candidate".into(),
            targets: vec!["render".into(), "tui".into()],
            preprocessor,
        });
    }
    if paths
        .iter()
        .all(|path| ci_only(path) || path.starts_with("resources/licenses/native/"))
        && (tui_measurement_changed(paths)
            || native_clipboard_changed(paths)
            || paths.iter().any(|path| {
                [
                    "xtask/src/dist.rs",
                    "xtask/src/dist/binary.rs",
                    "xtask/src/evidence.rs",
                    "xtask/src/evidence/release_build.rs",
                    "xtask/src/notices/skia.rs",
                    "xtask/src/notices/skia_wrapper.rs",
                    "resources/licenses/native/skia-wrapper-review.json",
                    "xtask/src/ci/native/candidate.rs",
                    "xtask/tests/binary_dist.rs",
                    "xtask/tests/evidence.rs",
                    ".github/workflows/release-evidence.yml",
                ]
                .contains(&path.as_str())
                    || path.starts_with("xtask/src/terminal/")
                    || path == "xtask/src/fixtures/kitty_ui.rs"
            }))
    {
        let targets = if platform == "macos-26" && terminal_harness_changed(paths) {
            vec!["render".into(), "tui".into()]
        } else {
            vec!["render".into()]
        };
        return Ok(Plan {
            platform,
            suite: "candidate".into(),
            targets,
            preprocessor,
        });
    }
    if paths
        .iter()
        .all(|p| ci_only(p) || p.starts_with("resources/licenses/native/"))
        && paths.iter().any(|p| {
            [
                "xtask/src/ci/native.rs",
                "xtask/src/ci/native/release.rs",
                "xtask/src/ci/cache.rs",
                "xtask/src/evidence/link_map.rs",
            ]
            .contains(&p.as_str())
        })
    {
        return Ok(Plan {
            platform,
            suite: "release".into(),
            targets: vec!["render".into()],
            preprocessor,
        });
    }
    // Font-only generators change native runtime fixtures without becoming
    // application compiler inputs. Keep their native focus after higher-priority gates.
    if !font_inputs && paths.iter().all(|p| ci_only(p)) {
        return Ok(Plan {
            platform,
            suite: "ci".into(),
            targets: vec![],
            preprocessor,
        });
    }
    let (all, rendering) = integration_targets(root)?;
    let browser_copy = any(
        paths,
        &[
            "src/project/browser_context.rs",
            "src/project/import/browsers.rs",
            "tests/support/script_browser_copy.rs",
        ],
        &[],
    );
    if browser_copy
        && paths.iter().all(|path| {
            ci_only(path)
                || path == "src/guidance.rs"
                || path.starts_with("resources/guidance/")
                || [
                    "src/project/bindings.rs",
                    "src/project/browser_context.rs",
                    "src/project/generated_refs.rs",
                    "src/project/import/browsers.rs",
                    "src/project/import/scripts.rs",
                    "src/project/import/scripts/composition.rs",
                    "tests/support/script_browser_copy.rs",
                    "tests/browser_readiness.rs",
                    "tests/script_scope.rs",
                ]
                .contains(&path.as_str())
        })
    {
        let mut targets = ["browser_readiness", "project", "script_scope"]
            .map(str::to_owned)
            .to_vec();
        if paths
            .iter()
            .any(|path| path == "src/guidance.rs" || path.starts_with("resources/guidance/"))
        {
            targets.push("guidance".into());
        }
        ensure!(
            targets.iter().all(|target| all.contains(target)),
            "Browser focus targets are missing"
        );
        return Ok(Plan {
            platform,
            suite: "browser".into(),
            targets,
            preprocessor,
        });
    }
    let modules =
        regex::Regex::new(r#"#\[cfg\(feature = "(?:rendering|tui)"\)\]\s+(?:pub )?mod (\w+);"#)?
            .captures_iter(&fs::read_to_string(root.join("src/lib.rs"))?)
            .map(|m| m[1].to_owned())
            .collect::<BTreeSet<_>>();
    let mut stems = BTreeSet::new();
    // These fixtures are consumed by the actual browser-readiness target.
    // Runtime acquisition helpers need their consumer even without app edits.
    if paths.iter().any(|path| {
        [
            "tests/support/browser_profiles.rs",
            "tests/support/page_wait_reference.rs",
            "xtask/src/fixtures/browser_ui.rs",
        ]
        .contains(&path.as_str())
            || path.starts_with("tests/fixtures/browser-ui/")
    }) {
        stems.insert("browser_readiness".into());
    }
    for p in paths {
        if let Some(stem) = Path::new(p).file_stem().and_then(|s| s.to_str()) {
            stems.insert(stem.into());
        }
        if p.starts_with("src/")
            && let Some(name) = p.split('/').nth(1)
        {
            stems.insert(name.trim_end_matches(".rs").into());
        }
    }
    let selected = || {
        stems
            .intersection(&all)
            .filter(|s| s.as_str() != "agent_workflow")
            .cloned()
            .collect::<BTreeSet<_>>()
    };
    let project = any(paths, &["src/project.rs"], &["src/project/"]);
    let operations = any(paths, &["src/operations.rs"], &["src/operations/"]);
    let grants = any(
        paths,
        &[
            "src/attachments.rs",
            "src/conversation/lease.rs",
            "src/conversation/storage.rs",
            "src/guidance/usage.rs",
            "tests/support/unix_agent_ownership.rs",
        ],
        &[],
    );
    let ownership = any(
        paths,
        &[
            "src/cdp/process.rs",
            "src/process_owner.rs",
            "tests/support/unix_browser_ownership.rs",
            "tests/support/unix_agent_ownership.rs",
        ],
        &["src/cdp/process/", "xtask/src/test_support/process_tree"],
    );
    let terminal = any(
        paths,
        &[],
        &[
            "xtask/src/test_support/terminal",
            "tests/support/terminal/",
            "tests/fixtures/terminal/",
        ],
    );
    let input = terminal
        || any(
            paths,
            &[
                "src/tui/mod.rs",
                "src/tui/saved.rs",
                "src/tui/checkpoints.rs",
            ],
            &["src/config/keys"],
        );
    let input_only = input
        && paths.iter().all(|p| {
            ci_only(p)
                || any(
                    std::slice::from_ref(p),
                    &[
                        "src/config.rs",
                        "src/tui/mod.rs",
                        "src/tui/checkpoints.rs",
                        "src/tui/saved.rs",
                        "src/tui/tests.rs",
                        "tests/tui.rs",
                        "tests/user_config.rs",
                    ],
                    &[
                        "src/config/keys",
                        ".github/",
                        "tests/fixtures/terminal/",
                        "tests/support/terminal/",
                        "xtask/src/test_support/terminal",
                        "xtask/src/test_support/pty",
                    ],
                )
        });
    let conversation = ownership
        || grants
        || has(&stems, &["agent", "conversation", "windows_agent"])
        || any(paths, &[], &["xtask/src/test_support/acp"]);
    let render = any(
        paths,
        &["src/render.rs"],
        &["src/render/", "src/cdp/", "src/tui/"],
    );
    let oracle_input = |p: &str| {
        p.starts_with("xtask/src/fixtures/oracles/")
            || p.starts_with("tests/fixtures/render-corpus/")
    };
    let oracle_only = paths.iter().any(|p| oracle_input(p))
        && paths.iter().all(|p| {
            ci_only(p)
                || oracle_input(p)
                || [
                    "tests/graphics.rs",
                    "tests/scripts.rs",
                    "tests/script_render.rs",
                    "tests/shader_uniforms.rs",
                    "tests/support/shader_preprocessor.mjs",
                ]
                .contains(&p.as_str())
        });
    let (mut suite, mut targets): (String, BTreeSet<String>) = if oracle_only {
        (
            "dynamic".into(),
            ["graphics", "scripts", "script_render", "shader_uniforms"]
                .map(str::to_owned)
                .into(),
        )
    } else if input_only {
        (
            "graphics".into(),
            ["tui", "user_config"].map(str::to_owned).into(),
        )
    } else if conversation {
        (
            "conversation".into(),
            if grants {
                ["agent", "attachments", "mcp"].map(str::to_owned).into()
            } else {
                ["agent".into()].into()
            },
        )
    } else if has(&stems, &["shader", "shader_uniforms", "webgl"])
        && !has(
            &stems,
            &DYNAMIC
                .iter()
                .copied()
                .filter(|s| !["shader", "shader_uniforms", "webgl"].contains(s))
                .collect::<Vec<_>>(),
        )
        && !has(
            &stems,
            &["script_render", "script_diagnostics", "script_scope"],
        )
        && !project
        && !operations
        && !render
    {
        ("shader".into(), ["shader_uniforms".into()].into())
    } else if has(
        &stems,
        &[
            "script",
            "scripts",
            "script_render",
            "script_diagnostics",
            "script_scope",
        ],
    ) && !has(
        &stems,
        &DYNAMIC
            .iter()
            .copied()
            .filter(|s| !["script", "scripts"].contains(s))
            .collect::<Vec<_>>(),
    ) && !render
    {
        let mut targets = selected();
        if any(paths, &["src/script.rs"], &["src/script/"]) {
            targets.extend(
                [
                    "scripts",
                    "script_render",
                    "script_diagnostics",
                    "script_scope",
                ]
                .map(str::to_owned),
            );
        }
        ("graphics".into(), targets)
    } else if has(&stems, DYNAMIC) || render || color_input {
        let mut targets = stems
            .intersection(&rendering)
            .filter(|s| s.as_str() != "agent_workflow")
            .cloned()
            .collect::<BTreeSet<_>>();
        targets.extend(
            [
                "dynamic_render",
                "browser_readiness",
                "shader_uniforms",
                "sampler_sources",
                "imports",
            ]
            .map(str::to_owned),
        );
        if any(paths, &["src/script.rs"], &["src/script/"]) {
            targets.extend(
                [
                    "scripts",
                    "script_render",
                    "script_diagnostics",
                    "script_scope",
                ]
                .map(str::to_owned),
            );
        }
        ("dynamic".into(), targets)
    } else if any(
        paths,
        &[],
        &[
            "src/graphics",
            "src/layout",
            "src/text",
            "src/icons",
            "resources/",
            "vendor/",
        ],
    ) || font_inputs
        || !stems.is_disjoint(&rendering)
        || !stems.is_disjoint(&modules)
        || has(
            &stems,
            &[
                "graphics",
                "rendering",
                "text",
                "font_sources",
                "fonts",
                "optional_icons",
                "clipboard",
                "tui",
            ],
        )
    {
        let mut targets = selected();
        if font_inputs {
            targets.insert("font_sources".into());
        }
        if any(paths, &["src/layout.rs"], &["src/layout/"]) {
            targets.extend(["layout", "graphics", "render"].map(str::to_owned));
        }
        if targets.is_empty() {
            targets.extend(
                [
                    "graphics",
                    "render",
                    "text",
                    "font_sources",
                    "optional_icons",
                ]
                .map(str::to_owned),
            );
        }
        ("graphics".into(), targets)
    } else {
        let mut targets = BTreeSet::new();
        for stem in &stems {
            let alias = match stem.as_str() {
                "store" | "document" => "document_store",
                "query" => "queries",
                "operations" => "operations_cli",
                s => s,
            };
            if all.contains(alias) {
                targets.insert(alias.to_owned());
            }
        }
        if targets.is_empty() {
            targets.extend(["document_store", "mcp"].map(str::to_owned));
        }
        ("core".into(), targets)
    };
    let store_changed = any(paths, &["src/store.rs"], &["src/store/"]);
    let main_changed = any(paths, &["src/main.rs"], &[]);
    let assets_changed = any(paths, &["src/assets.rs"], &["src/assets/"]);
    let remote_images_changed = any(
        paths,
        &[
            "src/remote_images.rs",
            "src/inline_images.rs",
            "src/project/remote_images.rs",
        ],
        &["src/remote_images/", "src/inline_images/"],
    );
    let renderer_changed = any(paths, &["src/render.rs"], &["src/render/"]);
    let text_changed = font_inputs && any(paths, &["src/text.rs"], &["src/text/"]);
    let font_lock_changed = any(
        paths,
        &["src/font_lock.rs", "tests/support/font_lock.rs"],
        &[],
    );
    let bindings_changed = any(
        paths,
        &[
            "src/project/bindings.rs",
            "src/project/generated_refs.rs",
            "tests/support/script_bindings.rs",
            "tests/support/script_generated_overrides.rs",
        ],
        &["src/project/bindings/"],
    );
    let script_copy_changed = any(
        paths,
        &[
            "src/project/bindings.rs",
            "src/project/scope.rs",
            "src/project/detach.rs",
            "src/project/import/scripts.rs",
        ],
        &[
            "src/project/import/scripts/",
            "src/project/bindings/",
            "tests/support/script_copy",
        ],
    );
    let history_changed = stems.contains("conversation_history")
        || terminal
        || any(paths, &[], &["src/conversation/"]);
    let native_layout_changed = any(
        paths,
        &[
            "src/layout.rs",
            "src/text.rs",
            "src/render.rs",
            "src/service.rs",
            "src/script/layout.rs",
        ],
        &["src/layout/", "src/text/", "src/render/"],
    );
    let command_fixture_changed = any(paths, &["xtask/src/test_support/command.rs"], &[]);

    // Mixed changes contribute the union of their independent source consumers.
    // Feature/scheduler promotion remains separate from this target inventory.
    let consumers: &[(bool, &[&str])] = &[
        (store_changed, &["assets", "document_store"]),
        (main_changed, &["assets", "runtime"]),
        (assets_changed, &["assets"]),
        (
            remote_images_changed,
            &["sampler_sources", "runtime", "assets", "mcp"],
        ),
        (renderer_changed, &["graphics", "render"]),
        (
            color_input,
            &[
                "graphics",
                "render",
                "scripts",
                "shader_uniforms",
                "dynamic_render",
                "browser_readiness",
                "sampler_sources",
                "imports",
            ],
        ),
        (font_inputs, &["font_sources"]),
        (text_changed, &["text"]),
        (project, &["project", "queries"]),
        (font_lock_changed, &["repair"]),
        (bindings_changed, &["script_scope"]),
        (script_copy_changed, &["project", "scripts", "script_scope"]),
        (operations, &["operations_cli"]),
        (browser_copy, &["script_scope", "browser_readiness"]),
        (history_changed, &["conversation_history"]),
        (native_layout_changed, &["native_layout"]),
        (input, &["tui", "user_config"]),
        (
            command_fixture_changed,
            &["runtime", "runtime_config", "scripts", "dynamic_render"],
        ),
    ];
    for (changed, consumers) in consumers {
        if *changed {
            targets.extend(consumers.iter().map(|name| (*name).to_owned()));
        }
    }

    let needs_graphics = font_inputs
        || font_lock_changed
        || bindings_changed
        || native_layout_changed
        || input
        || command_fixture_changed;
    if needs_graphics && ["core", "conversation"].contains(&suite.as_str()) {
        suite = "graphics".into();
    }
    if font_inputs && suite == "shader" {
        suite = "dynamic".into();
    }
    if script_copy_changed && ["core", "conversation", "graphics"].contains(&suite.as_str()) {
        suite = "copy".into();
    }
    if browser_copy || color_input {
        suite = "dynamic".into();
    }
    if suite == "shader" && (history_changed || input || command_fixture_changed) {
        suite = "dynamic".into();
        targets.extend(
            [
                "dynamic_render",
                "browser_readiness",
                "sampler_sources",
                "imports",
            ]
            .map(str::to_owned),
        );
    }
    // A specialized source/helper branch must not discard independently changed
    // integration targets. New targets use default features when the narrower
    // source suite cannot establish their required feature configuration.
    let changed_tests: BTreeSet<String> = paths
        .iter()
        .filter_map(|path| {
            let path = Path::new(path);
            if path.parent() != Some(Path::new("tests"))
                || path.extension().is_none_or(|extension| extension != "rs")
            {
                return None;
            }
            path.file_stem()?.to_str().map(str::to_owned)
        })
        .filter(|name| {
            !["agent_workflow", "windows_agent"].contains(&name.as_str()) && all.contains(name)
        })
        .collect();
    if !changed_tests.is_subset(&targets) && ["core", "conversation"].contains(&suite.as_str()) {
        suite = "graphics".into();
    }
    targets.extend(changed_tests);
    if paths.iter().any(|path| bundled_guidance(path)) {
        targets.insert("guidance".into());
    }
    ensure!(
        !targets.contains("agent_workflow") && targets.iter().all(|s| all.contains(s)),
        "Focus may run only deterministic integration targets"
    );
    Ok(Plan {
        platform,
        suite,
        targets: targets.into_iter().collect(),
        preprocessor,
    })
}
