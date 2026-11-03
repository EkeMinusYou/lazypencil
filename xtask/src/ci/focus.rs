use crate::{process, util};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

mod selection;
pub use selection::select;

/// Retained native backends available for explicitly requested diagnostics.
pub const PLATFORMS: &[&str] = &["ubuntu-24.04", "ubuntu-24.04-arm", "macos-26"];
/// Required application targets for the first release, independent of helper hosts.
pub const RELEASE_PLATFORMS: &[&str] = &["macos-26"];

pub fn verification_platforms(requested: &str) -> Result<Vec<String>> {
    ensure!(
        requested == "all" || PLATFORMS.contains(&requested),
        "Unsupported verification platform"
    );
    Ok(if requested == "all" {
        RELEASE_PLATFORMS.iter().map(|s| (*s).to_owned()).collect()
    } else {
        vec![requested.to_owned()]
    })
}
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
pub const HEADED_LOGIN: &str = "browser_profiles::headed_login_closes_normally_and_preserves_storage_for_the_same_full_browser";
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

fn documentation_only(path: &str) -> bool {
    (path.ends_with(".md") && (!path.contains('/') || path.starts_with("docs/")))
        || (path.starts_with("tests/fixtures/") && path.ends_with("/README.md"))
}

fn bundled_guidance(path: &str) -> bool {
    [
        "src/guidance.rs",
        "src/service/guidance.rs",
        "tests/guidance.rs",
    ]
    .contains(&path)
        || path.starts_with("resources/guidance/")
}

/// ACP and user-owned adapter settings have deterministic core consumers.
pub fn agent_only(paths: &[String]) -> bool {
    paths.iter().any(|path| !documentation_only(path))
        && paths.iter().all(|path| {
            documentation_only(path)
                || path == "src/agent.rs"
                || (path.starts_with("src/agent/") && path != "src/agent/process/windows.rs")
                || [
                    "src/config/agents.rs",
                    "src/config/agent_execution.rs",
                    "tests/agent.rs",
                    "tests/user_agent_config.rs",
                ]
                .contains(&path.as_str())
        })
}

/// Browser profiles and shared runtime configuration retain full verification.
pub fn runtime_only(paths: &[String]) -> bool {
    paths.iter().any(|path| !documentation_only(path))
        && paths.iter().all(|path| {
            documentation_only(path)
                || [
                    "src/runtime.rs",
                    "src/runtime/download.rs",
                    "tests/runtime.rs",
                ]
                .contains(&path.as_str())
        })
}

pub fn ci_only(path: &str) -> bool {
    path.starts_with(".github/")
        || path.starts_with("scripts/ci/")
        || (path.starts_with("xtask/")
            && !path.starts_with("xtask/src/test_support/")
            && !path.starts_with("xtask/src/fixtures/oracles/")
            && path != "xtask/src/fixtures/browser_ui.rs"
            && path != "xtask/src/bin/test-helper.rs")
        || documentation_only(path)
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

fn image_input_only(paths: &[String]) -> bool {
    paths.iter().any(|path| path == "src/clipboard.rs")
        && paths.iter().all(|path| {
            (ci_only(path) && !font_input(path) && !tui_measurement_source(path))
                || [
                    "src/attachments.rs",
                    "src/image_input.rs",
                    "src/clipboard.rs",
                    "src/config.rs",
                    "src/config/attachments.rs",
                    "src/conversation/mod.rs",
                    "src/conversation/management.rs",
                    "src/conversation/management/retention/tests.rs",
                    "src/prompt.rs",
                    "src/tui/mod.rs",
                    "src/tui/images.rs",
                    "src/tui/tests.rs",
                    "tests/attachments.rs",
                    "tests/user_config.rs",
                ]
                .contains(&path.as_str())
        })
}

pub fn tui_measurement_changed(paths: &[String]) -> bool {
    // Input-only work has a small real consumer reproduction. Keep the same
    // packaged performance acceptance in the full job rather than duplicate it
    // in focus. Changes to preview/rendering or measurement tools keep that gate.
    if image_input_only(paths) {
        return false;
    }
    paths.iter().any(|path| tui_measurement_source(path))
}

fn tui_measurement_source(path: &str) -> bool {
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
        .contains(&path)
}

// Extracted modules retain the consumer set of their original owner. Keep
// this mapping shared by plan selection and workflow-level runtime gating.
fn source_owner(path: &str) -> &str {
    match path {
        "xtask/src/ci/focus/selection.rs" => "xtask/src/ci/focus.rs",
        "xtask/src/ci/native/stages.rs" => "xtask/src/ci/native.rs",
        "xtask/src/ci/cache/migrations.rs" => "xtask/src/ci/cache.rs",
        _ => path,
    }
}

pub fn runtime_scheduler_changed(paths: &[String]) -> bool {
    paths.iter().any(|path| {
        let path = source_owner(path);
        [
            "xtask/src/ci/native.rs",
            "xtask/src/ci/native/browser.rs",
            "xtask/src/ci/native/parallel.rs",
            "xtask/src/ci/native/management.rs",
            ".github/workflows/native.yml",
        ]
        .contains(&path)
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

pub fn terminal_harness_changed(paths: &[String]) -> bool {
    paths.iter().any(|path| {
        path.starts_with("xtask/src/terminal/")
            || path == "xtask/src/fixtures/kitty_ui.rs"
            || path.starts_with("xtask/tests/terminal") && path.ends_with(".rs")
    })
}

pub fn terminal_reproduction_stages(paths: &[String], plan: &Plan) -> (bool, bool) {
    let changed = terminal_harness_changed(paths);
    let available = plan.platform.starts_with("ubuntu-")
        && !["core", "conversation", "ci"].contains(&plan.suite.as_str());
    // A core-only build has no TUI. Keep that focused check small and use the
    // full jobs' already-required default binaries for the reproduction. Standard
    // macOS VMs lack Kitty's OpenGL context: verify native PTYs on the release
    // target. Linux pixel checks remain available for explicit diagnostics;
    // they no longer create release jobs. Native Mac pixels need a GPU runner.
    (
        changed && available,
        changed && !available && RELEASE_PLATFORMS.iter().any(|p| p.starts_with("ubuntu-")),
    )
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

fn history_json(mut request: impl FnMut(Duration) -> Result<process::Output>) -> Result<Value> {
    // Preserve the original GET budget across retries. A temporary history
    // outage must not immediately expand a small change into every build lane.
    let deadline = Instant::now() + Duration::from_secs(45);
    for attempt in 0..3 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero(),
            "CI history GET exceeded its 45s budget"
        );
        let output = request(remaining)?;
        ensure!(!output.timed_out, "CI history GET exceeded its 45s budget");
        if output.status.success() {
            return Ok(serde_json::from_slice(&output.stdout)?);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let transient = stderr.lines().any(|line| {
            line.starts_with("gh: ")
                && [500, 502, 503, 504]
                    .iter()
                    .any(|code| line.ends_with(&format!("(HTTP {code})")))
        });
        let delay = Duration::from_secs(attempt + 1);
        if !transient || attempt == 2 || deadline.saturating_duration_since(Instant::now()) <= delay
        {
            anyhow::bail!("CI history GET failed ({}): {stderr}", output.status);
        }
        eprintln!(
            "Retrying CI history GET after a temporary GitHub server error (attempt {}/3)",
            attempt + 2
        );
        std::thread::sleep(delay);
    }
    unreachable!("Each final attempt returns its result")
}

pub fn changed_paths(root: &Path) -> Result<Vec<String>> {
    fn api(root: &Path, endpoint: &str) -> Result<Value> {
        history_json(|timeout| {
            process::output(
                Command::new("gh")
                    .current_dir(root)
                    .args(["api", "--method", "GET", endpoint]),
                timeout,
            )
        })
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
                let original = run["head_sha"]
                    .as_str()
                    .context("Missing source identity")?;
                let mut sha = verified_baseline(root, original, || {
                    let commit = api(root, &format!("repos/{repo}/git/commits/{original}"))?;
                    ensure!(
                        commit["sha"] == original,
                        "Verified source identity changed"
                    );
                    Ok(commit["tree"]["sha"]
                        .as_str()
                        .context("Missing verified file tree")?
                        .to_owned())
                })?;
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
    paths_since(root, baseline(root))
}

fn verified_baseline(
    root: &Path,
    original: &str,
    verified_tree: impl FnOnce() -> Result<String>,
) -> Result<String> {
    let hash =
        |value: &str| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    ensure!(hash(original), "Invalid verified source identity");
    let available = process::output(
        Command::new("git").current_dir(root).args([
            "cat-file",
            "-e",
            &format!("{original}^{{commit}}"),
        ]),
        Duration::from_secs(10),
    )?;
    if available.status.success() && !available.timed_out {
        return Ok(original.to_owned());
    }
    let tree = verified_tree()?;
    ensure!(hash(&tree), "Invalid verified file tree");
    let ancestry = process::text(Command::new("git").current_dir(root).args([
        "log",
        "--no-color",
        "--no-show-signature",
        "--format=%H %T",
        "HEAD",
    ]))?;
    let matching = ancestry
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(commit, candidate)| hash(commit) && *candidate == tree)
        .context("Verified source tree is absent from the current ancestry")?
        .0;
    eprintln!("Using file-identical verified baseline {original} -> {matching} (tree {tree})");
    Ok(matching.to_owned())
}

fn paths_since(root: &Path, baseline: Result<String>) -> Result<Vec<String>> {
    let changes = baseline.and_then(|base| {
        process::text(Command::new("git").current_dir(root).args([
            "diff",
            "--name-only",
            "-z",
            &base,
            "HEAD",
        ]))
    });
    let bytes = match changes {
        Ok(bytes) => bytes,
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

/// A single release target needs no parent-run API query to choose its runner.
/// Explicit diagnostic choices keep their existing native backends.
pub fn retry_platform(requested: &str) -> String {
    if requested == "auto" {
        RELEASE_PLATFORMS[0].to_owned()
    } else {
        requested.to_owned()
    }
}

pub fn build_before_lint(suite: &str, matched_key: &str) -> bool {
    matched_key.is_empty()
        || (matched_key.starts_with("core-") && !["core", "conversation"].contains(&suite))
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn unavailable_baseline_objects_expand_to_all_tracked_files_after_a_history_rewrite() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            process::text(
                Command::new("git")
                    .current_dir(root)
                    .args([
                        "-c",
                        "commit.gpgsign=false",
                        "-c",
                        "user.name=Fixture",
                        "-c",
                        "user.email=fixture@example.invalid",
                    ])
                    .args(args),
            )
            .unwrap()
        };
        git(&["init", "--quiet"]);
        fs::write(root.join("changed.rs"), "before").unwrap();
        fs::write(root.join("unchanged.rs"), "keep").unwrap();
        git(&["add", "changed.rs", "unchanged.rs"]);
        git(&["commit", "--quiet", "-m", "Fixture baseline"]);
        let baseline = git(&["rev-parse", "HEAD"]).trim().to_owned();
        let tree = git(&["rev-parse", "HEAD^{tree}"]).trim().to_owned();
        fs::write(root.join("changed.rs"), "after").unwrap();
        fs::write(root.join("untracked.rs"), "not a compiler input").unwrap();
        assert!(paths_since(root, Ok(baseline.clone())).unwrap().is_empty());
        git(&["add", "changed.rs"]);
        git(&["commit", "--quiet", "-m", "Fixture change"]);
        assert_eq!(
            paths_since(root, Ok(baseline.clone())).unwrap(),
            ["changed.rs"]
        );
        assert_eq!(
            verified_baseline(root, &baseline, || panic!(
                "An available baseline needs no API lookup"
            ))
            .unwrap(),
            baseline
        );
        let missing = "f".repeat(40);
        let mapped = verified_baseline(root, &missing, || Ok(tree)).unwrap();
        assert_eq!(mapped, baseline);
        assert_eq!(paths_since(root, Ok(mapped)).unwrap(), ["changed.rs"]);
        let all = ["changed.rs", "unchanged.rs"];
        assert_eq!(paths_since(root, Ok("f".repeat(40))).unwrap(), all);
        assert_eq!(
            paths_since(
                root,
                verified_baseline(root, &missing, || Ok("0".repeat(40)))
            )
            .unwrap(),
            all
        );
        fs::write(root.join("changed.rs"), "unrelated branch").unwrap();
        git(&["add", "changed.rs"]);
        let unrelated_tree = git(&["write-tree"]).trim().to_owned();
        let unrelated = git(&["commit-tree", &unrelated_tree, "-m", "Unrelated fixture"])
            .trim()
            .to_owned();
        git(&["update-ref", "refs/heads/unrelated", &unrelated]);
        git(&["read-tree", "HEAD"]);
        fs::write(root.join("changed.rs"), "after").unwrap();
        assert_eq!(
            paths_since(
                root,
                verified_baseline(root, &missing, || Ok(unrelated_tree))
            )
            .unwrap(),
            all
        );
        assert_eq!(
            paths_since(root, Err(anyhow::anyhow!("History lookup failed"))).unwrap(),
            all
        );
    }

    fn response(success: bool, stdout: &[u8], stderr: &[u8]) -> process::Output {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        process::Output {
            status: std::process::ExitStatus::from_raw(if success { 0 } else { 256 }),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
            elapsed: Duration::ZERO,
            timed_out: false,
        }
    }

    #[test]
    fn temporary_history_outages_recover_under_one_deadline_without_using_error_bodies() {
        let mut budgets = Vec::new();
        let result = history_json(|budget| {
            budgets.push(budget);
            Ok(match budgets.len() {
                1 => response(
                    false,
                    br#"{"workflow_runs":[{"head_sha":"untrusted-error-body"}]}"#,
                    b"gh: Server Error (HTTP 502)\n",
                ),
                2 => response(false, b"", b"gh: Service Unavailable (HTTP 503)\n"),
                3 => response(true, br#"{"workflow_runs":[{"head_sha":"verified"}]}"#, b""),
                _ => panic!("History lookup exceeded its retry limit"),
            })
        })
        .unwrap();
        assert_eq!(result["workflow_runs"][0]["head_sha"], "verified");
        assert_eq!(budgets.len(), 3);
        assert!(budgets[0] <= Duration::from_secs(45));
        assert!(budgets.windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn persistent_server_errors_stop_after_three_attempts() {
        let mut attempts = 0;
        let error = history_json(|_| {
            attempts += 1;
            Ok(response(false, b"", b"gh: Gateway Timeout (HTTP 504)\n"))
        })
        .unwrap_err();
        assert_eq!(attempts, 3);
        assert!(error.to_string().contains("HTTP 504"));
    }

    #[test]
    fn permission_rate_limit_transport_timeout_and_malformed_history_never_retry() {
        for stderr in [
            "gh: Requires authentication (HTTP 401)\n",
            "gh: Forbidden (HTTP 403)\n",
            "gh: Not Found (HTTP 404)\n",
            "gh: Too Many Requests (HTTP 429)\n",
            "unrecognized client failure (HTTP 502)\n",
        ] {
            let mut attempts = 0;
            assert!(
                history_json(|_| {
                    attempts += 1;
                    Ok(response(false, b"{}", stderr.as_bytes()))
                })
                .is_err()
            );
            assert_eq!(attempts, 1, "{stderr}");
        }
        for kind in ["transport", "timeout", "json"] {
            let mut attempts = 0;
            assert!(
                history_json(|_| {
                    attempts += 1;
                    if kind == "transport" {
                        anyhow::bail!("Cannot launch gh");
                    }
                    let mut output = response(true, b"not JSON", b"");
                    output.timed_out = kind == "timeout";
                    Ok(output)
                })
                .is_err()
            );
            assert_eq!(attempts, 1, "{kind}");
        }
    }
}
