use crate::{process, util};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path, process::Command};

pub const PLATFORMS: &[&str] = &["ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"];
const DYNAMIC: &[&str] = &[
    "cdp",
    "dynamic",
    "runtime",
    "runtime_config",
    "scripts",
    "shader",
    "shader_uniforms",
    "sampler_sources",
    "imports",
    "script",
    "webgl",
    "browser_readiness",
    "dynamic_render",
    "tui",
];
pub const LOCAL_FILES: &str =
    "captured_local_files_render_after_source_removal_and_deny_unlisted_host_files";
pub const NATIVE_ANIMATION: &str =
    "native_animation_policy_freezes_css_transitions_and_web_animations_without_stopping_frames";
pub const PAGE_DIAGNOSTICS: &str =
    "page_evaluation_failures_omit_private_content_and_recover_in_fresh_and_retained_modes";
pub const OFFLINE_NETWORK: &str = "local_pages::offline_network_policy_blocks_http_websocket_and_tls_routes_for_pages_and_workers";
pub const OFFLINE_TRANSPORT: &str = "local_pages::offline_transport::offline_page_process_blocks_stun_turn_and_webtransport_and_online_resumes";
pub const BROWSER_COPY: &str =
    "copied_browser_namespaces_keep_pixels_across_removal_recopy_themes_and_detach";
pub const GENERATED_BROWSER: &str =
    "generated_browser_namespaces_keep_pixels_after_source_removal_and_recopy";
pub const LOCAL_PAGES: &str =
    "local_pages::project_local_pages_update_resources_retire_offscreen_state_and_recover";
pub const COPY_CPU: &[&str] = &[
    "script_copy::independent_copy_keeps_namespace_nested_workers_and_live_choices_after_source_removal",
    "script_copy::copy_rejects_unmanifested_generated_paths_without_using_consumer_files",
    "script_copy::repeated_copy_keeps_original_and_saved_input_namespaces_after_each_source_is_removed",
    "script_copy::detach::imported_bound_detach_preserves_saved_default_and_live_variable_inputs",
    "script_copy::detach::detached_bound_scopes_survive_another_import_and_independent_copy",
];
pub const SHADER_REGRESSIONS: &[&str] = &[
    "linked_constant_dimensions_validate_inputs_and_recover_after_an_error",
    "annotation_binding_matches_independent_contract_pixels",
];
pub const HISTORY_PTY: &[&str] = &[
    "real_pty_saved_logs_page_format_refresh_and_keep_the_unsent_draft_without_starting_an_agent",
    "real_pty_selects_saved_conversations_and_keeps_both_drafts_without_starting_an_agent",
    "real_pty_deletes_only_reviewed_inactive_local_conversations_and_preserves_current_state",
    "real_pty_reviewed_deletion_remains_readable_at_minimum_terminal_size",
];
pub const INPUT_PTY: &str = "real_pty_custom_navigation_preserves_draft_and_changes_preview_pixels";
pub const TERMINAL_MODES: &str =
    "real_pty_handles_japanese_input_live_turns_resize_and_image_protocols";
pub const RETAINED_PAGE: &str =
    "retained_page_frame_capture_preserves_layout_dom_and_static_shader_acquisitions";
pub const TEXT_LAYER_REPRODUCTION: &str =
    "text_layer_bounds_preserve_native_alpha_at_fractional_scales_and_rotations";
pub const BROWSER_CI_TARGETS: &[&str] = &[
    "agent",
    "browser_readiness",
    "dynamic_render",
    "imports",
    "sampler_sources",
    "script_scope",
    "shader_uniforms",
    "windows_agent",
];
pub const RENDER_TARGETS: &[&str] = &[
    "blends",
    "dynamic_render",
    "graphics",
    "mesh",
    "render",
    "shader_uniforms",
];
pub const RENDER_BROWSER: &str = "browser_pixels_are_captured_composited_and_reused_until_refresh";
pub const INPUT_CONFIG: &str =
    "navigation_shortcuts_are_user_owned_replace_defaults_and_validate_scope_conflicts";
pub const RECLAIM: &str =
    "agent_ownership::killed_cli_revokes_grants_and_resume_preserves_images_and_transcripts";
pub const OWNED_MCP: &str =
    "owned_mcp_rejects_grants_after_the_host_is_killed_without_reviving_or_writing_journals";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub platform: String,
    pub suite: String,
    pub targets: Vec<String>,
    pub preprocessor: bool,
}

fn font_input(path: &str) -> bool {
    [
        "src/text.rs",
        "tests/font_sources.rs",
        "tests/support/color_fonts.rs",
        "tests/support/real_emoji.rs",
        "xtask/src/fixtures/fonts.rs",
        "xtask/src/fixtures/emoji.rs",
    ]
    .contains(&path)
        || path.starts_with("src/text/")
        || path.starts_with("xtask/src/fixtures/fonts/")
        || (path.starts_with("tests/fixtures/fonts/") && !path.ends_with(".md"))
}

pub fn ci_only(path: &str) -> bool {
    path.starts_with(".github/")
        || path.starts_with("scripts/ci/")
        || (path.starts_with("xtask/")
            && !path.starts_with("xtask/src/test_support/")
            && !path.starts_with("xtask/src/fixtures/oracles/")
            && path != "xtask/src/bin/test-helper.rs")
        || (path.ends_with(".md") && (!path.contains('/') || path.starts_with("docs/")))
        || (path.starts_with("tests/fixtures/") && path.ends_with("/README.md"))
}

pub fn preprocessor_changed(paths: &[String]) -> bool {
    paths.iter().any(|p| {
        [
            "src/shader/preprocessor/",
            "resources/shader-preprocessor/",
            "resources/licenses/shader-preprocessor/",
            "resources/licenses/rust/",
            "resources/licenses/toolchain/",
        ]
        .iter()
        .any(|prefix| p.starts_with(prefix))
            || [
                "xtask/src/shader.rs",
                "xtask/src/notices/rust.rs",
                "xtask/src/notices/toolchain.rs",
                "xtask/src/ci/toolchain.rs",
                "xtask/src/ci/component.rs",
                "tests/support/shader_preprocessor.mjs",
                ".github/workflows/shader-preprocessor.yml",
            ]
            .contains(&p.as_str())
    })
}

pub fn tui_measurement_changed(paths: &[String]) -> bool {
    paths.iter().any(|path| {
        path.starts_with("xtask/src/performance/") && path != "xtask/src/performance/profile.rs"
            || path.starts_with("src/tui/") && path != "src/tui/tests.rs"
            || path == "src/render.rs"
            || path == "src/layout.rs"
            || path.starts_with("src/render/")
            || [
                "xtask/src/performance.rs",
                "xtask/src/test_support/terminal.rs",
                "xtask/src/evidence/release_build.rs",
            ]
            .contains(&path.as_str())
    })
}

pub fn runtime_scheduler_changed(paths: &[String]) -> bool {
    paths.iter().any(|path| {
        [
            "xtask/src/ci/native.rs",
            "xtask/src/ci/native/browser.rs",
            "xtask/src/ci/native/parallel.rs",
            "xtask/src/ci/native/management.rs",
            ".github/workflows/native.yml",
        ]
        .contains(&path.as_str())
    })
}

pub fn native_clipboard_changed(paths: &[String]) -> bool {
    paths.iter().any(|path| {
        path.starts_with("tests/support/terminal/")
            || [
                "src/clipboard.rs",
                "tests/tui.rs",
                "xtask/src/ci/native/clipboard.rs",
                "xtask/tests/native_clipboard.rs",
            ]
            .contains(&path.as_str())
    })
}

fn has(stems: &BTreeSet<String>, names: &[&str]) -> bool {
    names.iter().any(|s| stems.contains(*s))
}
fn any(paths: &[String], names: &[&str], prefixes: &[&str]) -> bool {
    paths
        .iter()
        .any(|p| names.contains(&p.as_str()) || prefixes.iter().any(|prefix| p.starts_with(prefix)))
}

pub fn integration_targets(root: &Path) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let mut all = BTreeSet::new();
    let mut rendering = BTreeSet::new();
    for entry in fs::read_dir(root.join("tests"))? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .unwrap()
            .to_str()
            .context("Invalid test target")?
            .to_owned();
        if fs::read_to_string(&path)?.contains("#![cfg(feature = \"rendering\")]") {
            rendering.insert(stem.clone());
        }
        all.insert(stem);
    }
    Ok((all, rendering))
}

pub fn select(root: &Path, paths: &[String], platform: &str) -> Result<Plan> {
    ensure!(
        platform == "auto" || PLATFORMS.contains(&platform),
        "Unsupported focus platform"
    );
    let mac = paths.iter().any(|p| {
        let p = p.to_lowercase();
        // Exercise the shared PTY bridge against BSD libc and terminal restoration first.
        p.contains("macos")
            || p.contains("darwin")
            || p.starts_with("tests/support/terminal/")
            || p == "xtask/src/test_support/terminal.rs"
            || ["src/paths.rs", "tests/paths.rs"].contains(&p.as_str())
    });
    let platform = if platform == "auto" {
        // The X11 emulator runner exists only on Linux. Exercise it before
        // expanding mixed keyboard/PTY changes to the complete platform set.
        if paths
            .iter()
            .any(|path| path.starts_with("xtask/src/terminal/"))
        {
            "ubuntu-24.04"
        } else if mac {
            "macos-15"
        } else {
            "ubuntu-24.04"
        }
    } else {
        platform
    }
    .to_owned();
    let preprocessor = preprocessor_changed(paths);
    // Installation and file verification have no rendering feature dependency.
    // Reproduce these failures before native compilation and browser setup;
    // mixed rendering/CDP changes retain the broader existing plan.
    if any(paths, &["src/runtime.rs", "tests/runtime.rs"], &[])
        && paths.iter().all(|path| {
            [
                "src/runtime.rs",
                "src/store.rs",
                "tests/runtime.rs",
                "xtask/src/ci/focus.rs",
                "xtask/tests/ci.rs",
                ".github/actions/xtask/action.yml",
                "xtask/src/ci/cache.rs",
                "xtask/tests/checkpoints.rs",
                "xtask/tests/compatible_cache.rs",
                "xtask/tests/fixtures/focus.json",
                "README.md",
            ]
            .contains(&path.as_str())
                || path.starts_with("docs/") && path.ends_with(".md")
        })
    {
        return Ok(Plan {
            platform,
            suite: "core".into(),
            targets: if paths.iter().any(|path| path == "src/store.rs") {
                ["document_store", "operations_cli", "project", "runtime"]
                    .map(str::to_owned)
                    .to_vec()
            } else {
                vec!["runtime".into()]
            },
            preprocessor,
        });
    }
    let font_inputs = paths.iter().any(|path| font_input(path));
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
            }))
    {
        return Ok(Plan {
            platform,
            suite: "candidate".into(),
            targets: vec!["render".into()],
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
    if !paths.is_empty() && !font_inputs && paths.iter().all(|p| ci_only(p)) {
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
            // Deletions of the retired corpus must select the same consumers.
            || p.starts_with("tests/fixtures/pencil-0.3.9/")
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
    } else if has(&stems, DYNAMIC) || render {
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
    if any(paths, &["src/render.rs"], &["src/render/"]) {
        targets.extend(["graphics", "render"].map(str::to_owned));
    }
    if font_inputs {
        if suite == "shader" {
            suite = "dynamic".into();
        } else if ["core", "conversation"].contains(&suite.as_str()) {
            suite = "graphics".into();
        }
        targets.insert("font_sources".into());
        if any(paths, &["src/text.rs"], &["src/text/"]) {
            targets.insert("text".into());
        }
    }
    if project {
        targets.extend(["project", "queries"].map(str::to_owned));
    }
    if any(
        paths,
        &[
            "src/project/bindings.rs",
            "src/project/generated_refs.rs",
            "tests/support/script_bindings.rs",
            "tests/support/script_generated_overrides.rs",
        ],
        &[],
    ) {
        if ["core", "conversation"].contains(&suite.as_str()) {
            suite = "graphics".into();
        }
        targets.insert("script_scope".into());
    }
    if any(
        paths,
        &[
            "src/project/bindings.rs",
            "src/project/scope.rs",
            "src/project/detach.rs",
            "src/project/import/scripts.rs",
        ],
        &["src/project/import/scripts/", "tests/support/script_copy"],
    ) {
        if ["core", "conversation", "graphics"].contains(&suite.as_str()) {
            suite = "copy".into();
        }
        targets.extend(["project", "scripts", "script_scope"].map(str::to_owned));
    }
    if operations {
        targets.insert("operations_cli".into());
    }
    if any(
        paths,
        &[
            "src/project/browser_context.rs",
            "src/project/import/browsers.rs",
            "tests/support/script_browser_copy.rs",
        ],
        &[],
    ) {
        suite = "dynamic".into();
        targets.extend(["script_scope", "browser_readiness"].map(str::to_owned));
    }
    if stems.contains("conversation_history") || terminal || any(paths, &[], &["src/conversation/"])
    {
        targets.insert("conversation_history".into());
        if suite == "shader" {
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
    }
    if any(
        paths,
        &[
            "src/layout.rs",
            "src/text.rs",
            "src/render.rs",
            "src/service.rs",
            "src/script/layout.rs",
        ],
        &["src/layout/", "src/text/", "src/render/"],
    ) {
        if ["core", "conversation"].contains(&suite.as_str()) {
            suite = "graphics".into();
        }
        targets.insert("native_layout".into());
    }
    if input {
        if ["core", "conversation"].contains(&suite.as_str()) {
            suite = "graphics".into();
        } else if suite == "shader" {
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
        targets.extend(["tui", "user_config"].map(str::to_owned));
    }
    if paths
        .iter()
        .any(|p| p == "xtask/src/test_support/command.rs")
    {
        if ["core", "conversation"].contains(&suite.as_str()) {
            suite = "graphics".into();
        } else if suite == "shader" {
            suite = "dynamic".into();
        }
        targets
            .extend(["runtime", "runtime_config", "scripts", "dynamic_render"].map(str::to_owned));
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

pub fn changed_paths(root: &Path) -> Result<Vec<String>> {
    fn api(root: &Path, endpoint: &str) -> Result<Value> {
        let output = process::text(Command::new("gh").current_dir(root).args(["api", endpoint]))?;
        Ok(serde_json::from_str(&output)?)
    }
    fn baseline(root: &Path) -> Result<String> {
        let repo = std::env::var("GITHUB_REPOSITORY")?;
        let runs = api(
            root,
            &format!(
                "repos/{repo}/actions/workflows/ci.yml/runs?status=success&branch=main&per_page=3"
            ),
        )?;
        for run in runs["workflow_runs"]
            .as_array()
            .context("Missing workflow history")?
        {
            let jobs = api(
                root,
                &format!("repos/{repo}/actions/runs/{}/jobs?per_page=100", run["id"]),
            )?;
            if jobs["jobs"]
                .as_array()
                .context("Missing jobs")?
                .iter()
                .any(|j| j["name"] == "Verified source artifact" && j["conclusion"] == "success")
            {
                let mut sha = run["head_sha"]
                    .as_str()
                    .context("Missing source identity")?
                    .to_owned();
                let event: Value = util::json(Path::new(&std::env::var("GITHUB_EVENT_PATH")?))?;
                let base = event["pull_request"]["base"]["sha"]
                    .as_str()
                    .or_else(|| event["before"].as_str());
                if let Some(base) = base.filter(|s| !s.chars().all(|c| c == '0')) {
                    sha = process::text(Command::new("git").current_dir(root).args([
                        "merge-base",
                        base,
                        &sha,
                    ]))?
                    .trim()
                    .into();
                }
                return Ok(sha);
            }
        }
        anyhow::bail!("No verified all-platform baseline")
    }
    let bytes = match baseline(root) {
        Ok(base) => process::text(Command::new("git").current_dir(root).args([
            "diff",
            "--name-only",
            "-z",
            &base,
            "HEAD",
        ]))?,
        Err(error) => {
            eprintln!("Expanding focus: {error:#}");
            process::text(
                Command::new("git")
                    .current_dir(root)
                    .args(["ls-files", "-z"]),
            )?
        }
    };
    Ok(bytes
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Prefer the platform with a concrete native failure on the direct parent.
/// This also handles shared fixtures whose failure is specific to a host's
/// canonical path representation, without growing path-name heuristics.
pub fn retry_platform(root: &Path, requested: &str) -> String {
    if requested != "auto" {
        return requested.into();
    }
    fn previous(root: &Path) -> Result<Option<String>> {
        let parent = process::text(
            Command::new("git")
                .current_dir(root)
                .args(["rev-parse", "HEAD^"]),
        )?;
        let parent = parent.trim();
        let repo = std::env::var("GITHUB_REPOSITORY")?;
        let output = process::text(Command::new("gh").current_dir(root).args([
            "api",
            &format!("repos/{repo}/actions/workflows/ci.yml/runs?head_sha={parent}&per_page=10"),
        ]))?;
        let runs: Value = serde_json::from_str(&output)?;
        for run in runs["workflow_runs"]
            .as_array()
            .context("Missing parent workflow runs")?
        {
            if run["head_sha"] != parent {
                continue;
            }
            let id = run["id"].as_u64().context("Missing parent run identity")?;
            let output = process::text(Command::new("gh").current_dir(root).args([
                "api",
                &format!("repos/{repo}/actions/runs/{id}/jobs?per_page=100"),
            ]))?;
            let jobs: Value = serde_json::from_str(&output)?;
            return failed_platform(&jobs);
        }
        Ok(None)
    }
    match previous(root) {
        Ok(Some(platform)) => {
            println!("Prioritizing parent native failure on {platform}");
            platform
        }
        Ok(None) => "auto".into(),
        Err(error) => {
            eprintln!("Keeping path-based focus: {error:#}");
            "auto".into()
        }
    }
}

pub fn failed_platform(jobs: &Value) -> Result<Option<String>> {
    let mut failed = BTreeSet::new();
    for job in jobs["jobs"]
        .as_array()
        .context("Missing native job history")?
    {
        if !matches!(job["conclusion"].as_str(), Some("failure" | "timed_out")) {
            continue;
        }
        for platform in PLATFORMS {
            if ["Focused reproduction", "Full verification"]
                .into_iter()
                .any(|phase| job["name"] == format!("{phase} / {platform} / verify"))
            {
                failed.insert((*platform).to_owned());
            }
        }
    }
    Ok((failed.len() == 1).then(|| failed.into_iter().next().expect("one failed platform")))
}

pub fn build_before_lint(suite: &str, matched_key: &str) -> bool {
    matched_key.is_empty()
        || (matched_key.starts_with("core-") && !["core", "conversation"].contains(&suite))
}
