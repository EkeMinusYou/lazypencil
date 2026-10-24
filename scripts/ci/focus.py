"""Select conservative, change-scoped hosted checks before the full matrix."""

import argparse
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import time
import urllib.error
import urllib.request


PLATFORMS = ["ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"]
DYNAMIC = {"cdp", "dynamic", "runtime", "runtime_config", "scripts", "shader", "shader_uniforms", "sampler_sources", "imports", "script", "webgl", "browser_readiness", "dynamic_render", "tui"}
TEST_FILES = list((Path(__file__).resolve().parents[2] / "tests").glob("*.rs"))
CORE_TESTS = {path.stem for path in TEST_FILES}
RENDER_TESTS = {path.stem for path in TEST_FILES if '#![cfg(feature = "rendering")]' in path.read_text(encoding="utf-8")}
# These explicit opt-ins consume real model usage and stay outside hosted CI.
CREDENTIALED_TESTS = {"agent_workflow"}
INPUT_REPRODUCTIONS = (
    "real_pty_custom_navigation_preserves_draft_and_changes_preview_pixels",
    "navigation_shortcuts_are_user_owned_replace_defaults_and_validate_scope_conflicts",
)
SAVED_LOG_PTY = "real_pty_saved_logs_page_format_refresh_and_keep_the_unsent_draft_without_starting_an_agent"
SESSION_SELECTION_PTY = "real_pty_selects_saved_conversations_and_keeps_both_drafts_without_starting_an_agent"
SESSION_DELETION_PTY = "real_pty_deletes_only_reviewed_inactive_local_conversations_and_preserves_current_state"
SESSION_RECLAIM = "agent_ownership::killed_cli_revokes_grants_and_resume_preserves_images_and_transcripts"
OWNED_MCP_RECLAIM = "owned_mcp_rejects_grants_after_the_host_is_killed_without_reviving_or_writing_journals"
SHADER_REGRESSIONS = [
    "linked_constant_dimensions_validate_inputs_and_recover_after_an_error",
    "native_annotation_binding_matches_public_pencil_reference_pixels",
]
RENDER_MODULES = set(re.findall(r'#\[cfg\(feature = "(?:rendering|tui)"\)\]\s+(?:pub )?mod (\w+);', (Path(__file__).resolve().parents[2] / "src/lib.rs").read_text(encoding="utf-8")))


def preprocessor_changed(paths):
    return any(path.startswith(("src/shader/preprocessor/", "resources/shader-preprocessor/", "resources/licenses/shader-preprocessor/", "resources/licenses/rust/", "resources/licenses/toolchain/",
                                "tests/fixtures/pencil-0.3.9/shader-logical-coordinates/"))
               or path in {"scripts/ci/shader_preprocessor_notices.py", "scripts/ci/toolchain_evidence.py",
                           "scripts/license_notices.py", "scripts/toolchain_notice_inventory.py", "scripts/shader_preprocessor.py",
                           "tests/support/shader_preprocessor.mjs", ".github/workflows/shader-preprocessor.yml"}
               for path in paths)


def select(paths, platform="auto"):
    stems = {Path(path).stem for path in paths}
    stems.update(Path(path).parts[1].removesuffix(".rs") for path in paths if path.startswith("src/") and len(Path(path).parts) > 1)
    project_change = any(path == "src/project.rs" or path.startswith("src/project/") for path in paths)
    operations_change = any(path == "src/operations.rs" or path.startswith("src/operations/") for path in paths)
    grants_change = any(path in {"src/attachments.rs", "src/conversation/lease.rs", "src/conversation/storage.rs", "src/guidance/usage.rs", "tests/support/unix_agent_ownership.rs"} for path in paths)
    # Deferred Windows code remains in the tree; release CI uses supported runners.
    ci_change = any(path.startswith((".github/", "scripts/ci/")) for path in paths)
    macos = any("macos" in path.lower() or "darwin" in path.lower() for path in paths)
    os_name = ("macos-15" if macos else "ubuntu-24.04") if platform == "auto" else platform
    ownership = any(path.startswith("src/cdp/process/") or path in {
        "src/cdp/process.rs", "src/process_owner.rs", "tests/support/unix_browser_ownership.rs",
        "tests/support/unix_agent_ownership.rs", "tests/support/agent_process_tree.py",
        "tests/support/browser_process_unix.py",
    } for path in paths)
    terminal_change = any(path == "tests/support/terminal_screen.py" or path.startswith("tests/fixtures/terminal/") for path in paths)
    input_source = terminal_change or any(path in {"src/tui/mod.rs", "src/tui/saved.rs", "src/tui/checkpoints.rs"} or path.startswith("src/config/keys") for path in paths)
    input_only = input_source and all(
        (path == "README.md" or path.startswith("docs/") and path.endswith(".md"))
        or path.startswith(("src/config/keys", ".github/", "scripts/ci/", "scripts/test_", "tests/fixtures/terminal/")) or path in {
            "src/config.rs", "src/tui/mod.rs", "src/tui/checkpoints.rs", "src/tui/tests.rs", "tests/tui.rs", "tests/user_config.rs",
            "tests/support/tui_pty.py", "tests/support/terminal_screen.py", "scripts/ci/focus.py", "scripts/test_ci_focus.py",
            ".github/workflows/native.yml",
        } for path in paths
    )
    conversation = ownership or grants_change or bool(stems & {"agent", "conversation", "windows_agent"}) or any(path.startswith("tests/support/acp_") for path in paths)
    if input_only:
        # Input dispatch has a real static PTY reproduction. Keep Chromium and
        # the complete dynamic/TUI suites in the unchanged final matrix.
        suite, targets = "graphics", ["tui", "user_config"]
    elif conversation:
        suite, targets = "conversation", ["agent"]
        if grants_change:
            targets = ["agent", "attachments", "mcp"]
    elif stems & {"shader", "shader_uniforms", "webgl"} and not (
        stems & (DYNAMIC - {"shader", "shader_uniforms", "webgl"})
        or stems & {"script_render", "script_diagnostics", "script_scope"}
        or project_change
        or operations_change
        or any(path == "src/render.rs" or path.startswith("src/render/") for path in paths)
    ):
        suite, targets = "shader", ["shader_uniforms"]
    elif ci_change and not any(path.startswith(("src/", "tests/")) for path in paths):
        suite, targets = "conversation", ["agent"]
    elif stems & {"script", "scripts", "script_render", "script_diagnostics", "script_scope"} and not (
        stems & (DYNAMIC - {"script", "scripts"})
        or any(path == "src/render.rs" or path.startswith(("src/cdp/", "src/tui/", "src/render/")) for path in paths)
    ):
        suite = "graphics"
        selected = stems & CORE_TESTS - CREDENTIALED_TESTS
        if any(path == "src/script.rs" or path.startswith("src/script/") for path in paths):
            selected.update({"scripts", "script_render", "script_diagnostics", "script_scope"})
        targets = sorted(selected)
    elif stems & DYNAMIC or any(path == "src/render.rs" or path.startswith(("src/cdp/", "src/tui/", "src/render/")) for path in paths):
        suite = "dynamic"
        targets = sorted({"dynamic_render", "browser_readiness", "shader_uniforms", "sampler_sources", "imports"} | (stems & RENDER_TESTS - CREDENTIALED_TESTS))
        if any(path == "src/script.rs" or path.startswith("src/script/") for path in paths):
            targets = sorted(set(targets) | {"scripts", "script_render", "script_diagnostics", "script_scope"})
    elif any(path.startswith(("src/graphics", "src/layout", "src/text", "src/icons", "resources/", "vendor/")) for path in paths) or stems & (RENDER_TESTS | RENDER_MODULES | {"graphics", "rendering", "text", "font_sources", "fonts", "optional_icons"}):
        suite = "graphics"
        selected = stems & CORE_TESTS - CREDENTIALED_TESTS
        # Layout metrics also determine raster bounds and stroke geometry. Run
        # those consumers even when their test files did not change.
        if any(path == "src/layout.rs" or path.startswith("src/layout/") for path in paths):
            selected.update({"layout", "graphics", "render"})
        targets = sorted(selected) or ["graphics", "render", "text", "font_sources", "optional_icons"]
    else:
        suite = "core"
        aliases = {"store": "document_store", "document": "document_store", "query": "queries", "operations": "operations_cli"}
        targets = sorted(({aliases.get(stem, stem) for stem in stems}) & CORE_TESTS)
        if not targets:
            targets = ["document_store", "mcp"]
    if project_change:
        targets = sorted(set(targets) | {"project", "queries"})
    if operations_change:
        targets = sorted(set(targets) | {"operations_cli"})
    if "conversation_history" in stems or terminal_change or any(path.startswith("src/conversation/") for path in paths):
        targets = sorted(set(targets) | {"conversation_history"})
        if suite == "shader":
            suite = "dynamic"
            targets = sorted(set(targets) | {"dynamic_render", "browser_readiness", "sampler_sources", "imports"})
    native = any(path in {"src/layout.rs", "src/text.rs", "src/render.rs", "src/service.rs", "src/script/layout.rs"}
                 or path.startswith(("src/layout/", "src/text/", "src/render/")) for path in paths)
    if native:
        if suite in ("core", "conversation"):
            suite = "graphics"
        targets = sorted(set(targets) | {"native_layout"})
    if input_source:
        if suite in ("core", "conversation"):
            suite = "graphics"
        elif suite == "shader":
            suite = "dynamic"
            targets = sorted(set(targets) | {"browser_readiness", "dynamic_render", "imports", "sampler_sources"})
        targets = sorted(set(targets) | {"tui", "user_config"})
    return {"platform": os_name, "suite": suite, "targets": targets}


def changed_paths():
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
    base = event.get("pull_request", {}).get("base", {}).get("sha") or event.get("before")
    # Include unverified earlier changes when newer commits supersede a run.
    # Failure to find a trustworthy baseline expands the focused check.
    try:
        def api(path):
            request = urllib.request.Request(
                f"{os.environ['GITHUB_API_URL']}/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
                headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json"},
            )
            for attempt in range(2):
                try:
                    with urllib.request.urlopen(request, timeout=15) as response:
                        return json.load(response)
                except urllib.error.HTTPError as error:
                    error.close()
                    print(f"Verified baseline API returned HTTP {error.code}")
                    if attempt or error.code not in {500, 502, 503, 504}:
                        raise
                    time.sleep(0.25)

        runs = api("actions/workflows/ci.yml/runs?status=success&branch=main&per_page=3")["workflow_runs"]
        verified = None
        for run in runs:
            try:
                jobs = api(f"actions/runs/{run['id']}/jobs?per_page=100")["jobs"]
            except urllib.error.HTTPError as error:
                if error.code in {401, 403}:
                    raise
                continue
            if any(job["name"] == "Verified source artifact" and job["conclusion"] == "success" for job in jobs):
                verified = run["head_sha"]
                break
        if verified is None:
            raise ValueError("No successful all-platform baseline")
        if base and set(base) != {"0"}:
            verified = subprocess.check_output(["git", "merge-base", base, verified], text=True, timeout=15).strip()
        base = verified
        return subprocess.check_output(["git", "diff", "--name-only", base, "HEAD"], text=True, timeout=15).splitlines()
    except (KeyError, ValueError, OSError, subprocess.SubprocessError, urllib.error.URLError) as error:
        cause = f"HTTP {error.code}" if isinstance(error, urllib.error.HTTPError) else type(error).__name__
        print(f"Expanding focus because no verified baseline is available: {cause}")
        return subprocess.check_output(["git", "ls-files"], text=True, timeout=15).splitlines()


def commands(suite, targets):
    core = suite in ("core", "conversation")
    features = ["--no-default-features"] if core else []
    selected = [arg for target in targets for arg in ("--test", target)]
    common = ["cargo", "test", "--locked", *features]
    input_skip = [argument for name in ("config::keys::", *INPUT_REPRODUCTIONS) for argument in ("--skip", name)] if suite in ("graphics", "dynamic") and {"tui", "user_config"} <= set(targets) else []
    shader_first = [
        [*common, "--test", "shader_uniforms", name, "--", "--ignored", "--exact", "--test-threads=1"]
        for name in SHADER_REGRESSIONS
    ] if suite in ("shader", "dynamic") and "shader_uniforms" in targets else []
    shader_skip = [arg for name in SHADER_REGRESSIONS for arg in ("--skip", name)] if shader_first else []
    if suite == "shader":
        if targets != ["shader_uniforms"]:
            raise ValueError("Shader focus requires only the shader_uniforms target")
        # CPU validation runs before Chromium acquisition in the native workflow.
        return ([[*common, *selected, "--no-run"], ["cargo", "build", "--locked"]],
                [*shader_first, [*common, *selected, "--", "--ignored", "--test-threads=1", *shader_skip]])
    build = [[*common, "--lib", *selected, "--no-run"], ["cargo", "build", "--locked", *features]]
    checks = list(shader_first)
    history = "conversation_history" in targets
    if history:
        # Rendering suites reproduce history in the native workflow before
        # other tests and Chromium; core suites need no rendering dependency.
        if core:
            checks.extend([
                [*common, "--lib", "conversation::management::", "--", "--test-threads=2"],
                [*common, "--lib", "conversation::history::", "--", "--test-threads=2"],
                [*common, "--test", "conversation_history", "--", "--test-threads=1"],
            ])
            if "mcp" in targets:
                checks.append([*common, "--test", "mcp", OWNED_MCP_RECLAIM, "--", "--exact", "--test-threads=1"])
        selected = [arg for target in targets if target != "conversation_history" for arg in ("--test", target)]
    history_ui_skip = ["--skip", "tui::tests::saved_log_", "--skip", SAVED_LOG_PTY, "--skip", SESSION_SELECTION_PTY, "--skip", SESSION_DELETION_PTY] if history and "tui" in targets else []
    history_ui_skip += ["--skip", SESSION_RECLAIM] if history and "agent" in targets and not core else []
    history_ui_skip += ["--skip", OWNED_MCP_RECLAIM] if history and "mcp" in targets else []
    native = "native_layout" in targets and suite in ("graphics", "dynamic")
    if native:
        # Dynamic focus runs these CPU cases in the workflow before Chromium.
        if suite == "graphics":
            checks.extend([
                [*common, "--lib", "native_cancellation", "--", "--test-threads=2"],
                [*common, "--test", "native_layout", "--", "--test-threads=2"],
            ])
        selected = [arg for target in targets if target != "native_layout" for arg in ("--test", target)]
    if suite == "conversation":
        checks.append([*common, "--test", "agent", "agent_ownership::killed_cli_", "--", "--test-threads=1", "--nocapture"])
        checks.append([*common, "--test", "agent", "embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits", "--", "--exact", "--nocapture"])
        checks.append([*common, "--test", "agent", "session_restart_", "--", "--test-threads=1", "--nocapture"])
    if suite == "dynamic":
        checks.append([*common, "--lib", "cdp::", "--", "--test-threads=2"])
    if suite == "dynamic":
        # Put the previously hanging TUI cases ahead of unrelated checks.
        checks.append([*common, "--lib", "tui::tests::", "--", "--test-threads=2", *history_ui_skip])
    if suite == "dynamic":
        checks.append([*common, "--test", "dynamic_render", "retained_", "--", "--ignored", "--test-threads=1"])
        checks.append([*common, *selected, "--", "--include-ignored", "--test-threads=1", "--skip", "retained_", *shader_skip, *input_skip, *history_ui_skip])
    else:
        checks.append([*common, "--lib", *selected, "--", "--test-threads=2", *input_skip, *history_ui_skip])
        if history:
            checks[-1].extend(["--skip", "conversation::history::", "--skip", "conversation::management::"])
        if native:
            checks[-1].extend(["--skip", "native_cancellation"])
        if suite == "conversation":
            checks[-1].extend(["--skip", "agent_ownership::killed_cli_",
                               "--skip", "session_restart_",
                               "--skip", "embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits"])
    return build, checks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("plan", "build", "test"))
    parser.add_argument("--platform", default="auto", choices=["auto", *PLATFORMS])
    parser.add_argument("--suite", choices=("core", "conversation", "graphics", "dynamic", "shader"))
    parser.add_argument("--targets", default="[]")
    args = parser.parse_args()
    if args.mode == "plan":
        paths = changed_paths()
        result = select(paths, args.platform)
        result["preprocessor"] = str(preprocessor_changed(paths)).lower()
        print(json.dumps(result))
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            for key, value in result.items():
                stream.write(f"{key}={json.dumps(value) if isinstance(value, list) else value}\n")
        return
    targets = json.loads(args.targets)
    if not isinstance(targets, list) or not targets or any(target not in CORE_TESTS for target in targets):
        parser.error("Expected existing integration test targets")
    build, checks = commands(args.suite, targets)
    run = runpy.run_path(str(Path(__file__).with_name("command.py")))["run"]
    for command in build if args.mode == "build" else checks:
        print("+ " + " ".join(command), flush=True)
        run(command, timeout=(720 if args.suite in ("core", "conversation") else 2400) if args.mode == "build" else 300)


if __name__ == "__main__":
    main()
