"""Select conservative, change-scoped hosted checks before the full matrix."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import urllib.error
import urllib.request


PLATFORMS = ["ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"]
DYNAMIC = {"cdp", "dynamic", "runtime", "runtime_config", "scripts", "shader", "shader_uniforms", "sampler_sources", "imports", "script", "browser_readiness", "dynamic_render", "tui"}
TEST_FILES = list((Path(__file__).resolve().parents[2] / "tests").glob("*.rs"))
CORE_TESTS = {path.stem for path in TEST_FILES}
RENDER_TESTS = {path.stem for path in TEST_FILES if '#![cfg(feature = "rendering")]' in path.read_text(encoding="utf-8")}
# These explicit opt-ins consume real model usage and stay outside hosted CI.
CREDENTIALED_TESTS = {"agent_workflow"}
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
    # Deferred Windows code remains in the tree; release CI uses supported runners.
    ci_change = any(path.startswith((".github/", "scripts/ci/")) for path in paths)
    macos = any("macos" in path.lower() or "darwin" in path.lower() for path in paths)
    os_name = ("macos-15" if macos else "ubuntu-24.04") if platform == "auto" else platform
    ownership = any(path.startswith("src/cdp/process/") or path in {
        "src/cdp/process.rs", "tests/support/unix_browser_ownership.rs",
        "tests/support/browser_process_unix.py",
    } for path in paths)
    conversation = ownership or bool(stems & {"agent", "conversation", "windows_agent"}) or any(path.startswith("tests/support/acp_") for path in paths)
    if conversation:
        suite, targets = "conversation", ["agent"]
    elif stems & {"shader", "shader_uniforms"} and not (
        stems & (DYNAMIC - {"shader", "shader_uniforms"})
        or stems & {"script_render", "script_diagnostics", "script_scope"}
        or project_change
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
            with urllib.request.urlopen(request, timeout=15) as response:
                return json.load(response)

        runs = api("actions/workflows/ci.yml/runs?status=success&branch=main&per_page=3")["workflow_runs"]
        verified = None
        for run in runs:
            jobs = api(f"actions/runs/{run['id']}/jobs?per_page=100")["jobs"]
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
        print(f"Expanding focus because no verified baseline is available: {type(error).__name__}")
        return subprocess.check_output(["git", "ls-files"], text=True, timeout=15).splitlines()


def commands(suite, targets):
    core = suite in ("core", "conversation")
    features = ["--no-default-features"] if core else []
    selected = [arg for target in targets for arg in ("--test", target)]
    common = ["cargo", "test", "--locked", *features]
    if suite == "shader":
        if targets != ["shader_uniforms"]:
            raise ValueError("Shader focus requires only the shader_uniforms target")
        # CPU validation runs before Chromium acquisition in the native workflow.
        return ([[*common, *selected, "--no-run"], ["cargo", "build", "--locked"]],
                [[*common, *selected, "--", "--ignored", "--test-threads=1"]])
    build = [[*common, "--lib", *selected, "--no-run"], ["cargo", "build", "--locked", *features]]
    checks = []
    if suite == "conversation":
        checks.append([*common, "--test", "agent", "embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits", "--", "--exact", "--nocapture"])
    if suite == "dynamic":
        checks.append([*common, "--lib", "cdp::", "--", "--test-threads=2"])
    if suite == "dynamic":
        # Put the previously hanging TUI cases ahead of unrelated checks.
        checks.append([*common, "--lib", "tui::tests::", "--", "--test-threads=2"])
    if suite == "dynamic":
        checks.append([*common, "--test", "dynamic_render", "retained_", "--", "--ignored", "--test-threads=1"])
        checks.append([*common, *selected, "--", "--include-ignored", "--test-threads=1", "--skip", "retained_"])
    else:
        checks.append([*common, "--lib", *selected, "--", "--test-threads=2"])
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
    for command in build if args.mode == "build" else checks:
        print("+ " + " ".join(command), flush=True)
        subprocess.run(command, check=True, timeout=2400 if args.mode == "build" else 300)


if __name__ == "__main__":
    main()
