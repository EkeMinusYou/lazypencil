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
pub const BROWSER_COPY: &str =
    "copied_browser_namespaces_keep_pixels_across_removal_recopy_themes_and_detach";
pub const COPY_CPU: &[&str] = &[
    "script_copy::independent_copy_keeps_namespace_nested_workers_and_live_choices_after_source_removal",
    "script_copy::copy_rejects_unmanifested_generated_paths_without_using_consumer_files",
    "script_copy::repeated_copy_keeps_original_and_saved_input_namespaces_after_each_source_is_removed",
    "script_copy::detach::imported_bound_detach_preserves_saved_default_and_live_variable_inputs",
    "script_copy::detach::detached_bound_scopes_survive_another_import_and_independent_copy",
];
pub const SHADER_REGRESSIONS: &[&str] = &[
    "linked_constant_dimensions_validate_inputs_and_recover_after_an_error",
    "native_annotation_binding_matches_public_pencil_reference_pixels",
];
pub const HISTORY_PTY: &[&str] = &[
    "real_pty_saved_logs_page_format_refresh_and_keep_the_unsent_draft_without_starting_an_agent",
    "real_pty_selects_saved_conversations_and_keeps_both_drafts_without_starting_an_agent",
    "real_pty_deletes_only_reviewed_inactive_local_conversations_and_preserves_current_state",
    "real_pty_reviewed_deletion_remains_readable_at_minimum_terminal_size",
];
pub const INPUT_PTY: &str = "real_pty_custom_navigation_preserves_draft_and_changes_preview_pixels";
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

pub fn ci_only(path: &str) -> bool {
    path.starts_with(".github/")
        || path.starts_with("scripts/ci/")
        || (path.starts_with("xtask/")
            && !path.starts_with("xtask/src/test_support/")
            && path != "xtask/src/bin/test-helper.rs")
        || (path.ends_with(".md") && (!path.contains('/') || path.starts_with("docs/")))
}

pub fn preprocessor_changed(paths: &[String]) -> bool {
    paths.iter().any(|p| {
        [
            "src/shader/preprocessor/",
            "resources/shader-preprocessor/",
            "resources/licenses/shader-preprocessor/",
            "resources/licenses/rust/",
            "resources/licenses/toolchain/",
            "tests/fixtures/pencil-0.3.9/shader-logical-coordinates/",
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
            || ["src/paths.rs", "tests/paths.rs"].contains(&p.as_str())
    });
    let platform = if platform == "auto" {
        if mac { "macos-15" } else { "ubuntu-24.04" }
    } else {
        platform
    }
    .to_owned();
    let preprocessor = preprocessor_changed(paths);
    if !paths.is_empty() && paths.iter().all(|p| ci_only(p)) {
        return Ok(Plan {
            platform,
            suite: "ci".into(),
            targets: vec![],
            preprocessor,
        });
    }
    let (all, rendering) = integration_targets(root)?;
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
    let (mut suite, mut targets): (String, BTreeSet<String>) = if input_only {
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
    ) || !stems.is_disjoint(&rendering)
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
    if project {
        targets.extend(["project", "queries"].map(str::to_owned));
    }
    if any(
        paths,
        &[
            "src/project/bindings.rs",
            "tests/support/script_bindings.rs",
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

pub fn build_before_lint(suite: &str, matched_key: &str) -> bool {
    matched_key.is_empty()
        || (matched_key.starts_with("core-") && !["core", "conversation"].contains(&suite))
}
