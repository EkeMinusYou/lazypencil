import importlib.util
import io
import json
import os
import tempfile
from unittest.mock import patch
import urllib.error
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location("focus", Path(__file__).parent / "ci/focus.py")
focus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(focus)


class FocusTests(unittest.TestCase):
    def test_preprocessor_inputs_are_validated_before_native_builds_only_when_affected(self):
        for path in ["src/shader/preprocessor/src/lib.rs", "src/shader/preprocessor/Cargo.lock",
                     "tests/support/shader_preprocessor.mjs", ".github/workflows/shader-preprocessor.yml",
                     "scripts/ci/toolchain_evidence.py", "resources/licenses/rust/upstream.json",
                     "tests/fixtures/pencil-0.3.9/shader-logical-coordinates/hardcoded.glsl",
                     "scripts/license_notices.py", "resources/licenses/toolchain/upstream.json",
                     "resources/shader-preprocessor/module.wasm", "resources/licenses/shader-preprocessor/packages/index.json",
                     "scripts/shader_preprocessor.py"]:
            with self.subTest(path=path):
                self.assertTrue(focus.preprocessor_changed([path]))
        for path in ["src/query.rs", "docs/design.md", "src/shader/defaults.rs"]:
            with self.subTest(path=path):
                self.assertFalse(focus.preprocessor_changed([path]))

    def test_deferred_windows_changes_keep_shared_checks_on_a_release_runner(self):
        for path in ("src/agent/process/windows.rs", "src/conversation/mod.rs", "tests/windows_agent.rs"):
            with self.subTest(path=path):
                result = focus.select([path])
                self.assertEqual(result["platform"], "ubuntu-24.04")
                build, checks = focus.commands(result["suite"], result["targets"])
                self.assertIn("--no-default-features", build[0])
                self.assertIn("agent_ownership::killed_cli_", checks[0])
                self.assertIn("embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits", checks[1])
                self.assertIn("--skip", checks[-1])
                self.assertIn("--test", checks[-1])

    def test_shared_dynamic_changes_use_one_linux_runner(self):
        for path in ("src/cdp/tabs.rs", "src/render.rs", "src/tui/preview.rs"):
            result = focus.select([path])
            self.assertEqual(result["platform"], "ubuntu-24.04")
            self.assertEqual(result["suite"], "dynamic")
            build, checks = focus.commands(result["suite"], result["targets"])
            self.assertNotIn("--no-default-features", build[0])
            self.assertIn("cdp::", checks[0])
            self.assertIn("retained_", checks[2])
            self.assertIn("--skip", checks[3])
            self.assertIn("--test-threads=1", checks[3])

    def test_shader_changes_select_cpu_validation_then_only_real_shader_cases(self):
        for paths in (["src/shader.rs"], ["src/shader/defaults.rs"], ["tests/shader_uniforms.rs"], ["src/shader.rs", ".github/workflows/native.yml"]):
            result=focus.select(paths)
            self.assertEqual(result,{"platform":"ubuntu-24.04","suite":"shader","targets":["shader_uniforms"]})
            build,checks=focus.commands(result["suite"],result["targets"])
            self.assertNotIn("--lib",build[0])
            self.assertNotIn("--no-default-features",build[0])
            self.assertEqual(len(checks),1)
            self.assertIn("--ignored",checks[0])
            self.assertNotIn("--include-ignored",checks[0])
            self.assertEqual(checks[0][checks[0].index("--test")+1],"shader_uniforms")
        native=(Path(__file__).resolve().parents[1]/".github/workflows/native.yml").read_text()
        self.assertLess(native.index("Validate shader inputs before acquiring Chromium"),native.index("Install and verify the pinned Chromium runtime"))
        self.assertLess(native.index("Check focused shader code before acquiring Chromium"),native.index("Install and verify the pinned Chromium runtime"))
        with self.assertRaises(ValueError):
            focus.commands("shader",["agent"])
        for other in ["tests/dynamic_render.rs", "src/script/sandbox.js", "src/render.rs"]:
            mixed = focus.select(["src/shader.rs", other])
            self.assertEqual(mixed["suite"], "dynamic")
            self.assertIn("shader_uniforms", mixed["targets"])
            self.assertIn("dynamic_render", mixed["targets"])

    def test_script_compatibility_runs_core_first_then_raster_consumers_without_chromium(self):
        for path in ("src/script/header.rs", "src/script/sandbox.js", "src/script/context.rs"):
            result = focus.select([path])
            self.assertEqual(result, {"platform": "ubuntu-24.04", "suite": "graphics", "targets": ["script_diagnostics", "script_render", "script_scope", "scripts"]})
            _, checks = focus.commands(result["suite"], result["targets"])
            self.assertEqual(len(checks), 1)
            self.assertNotIn("--include-ignored", checks[0])
        self.assertEqual(focus.select(["tests/script_render.rs"])["targets"], ["script_render"])
        self.assertEqual(focus.select(["tests/scripts.rs"])["targets"], ["scripts"])
        native = (Path(__file__).resolve().parents[1] / ".github/workflows/native.yml").read_text()
        self.assertLess(native.index("Reproduce script worker compatibility before native rendering"), native.index("Build focused test binaries"))
        mixed = focus.select(["src/script/sandbox.js", "tests/dynamic_render.rs"])
        self.assertEqual(mixed["suite"], "dynamic")
        self.assertTrue({"scripts", "script_render", "script_diagnostics", "script_scope", "dynamic_render"} <= set(mixed["targets"]))
        self.assertLess(native.index("Validate dynamic inputs and launch deadlines before acquiring Chromium"), native.index("Install and verify the pinned Chromium runtime"))

    def test_browser_ownership_and_nested_fixtures_run_actual_process_cases_before_rendering(self):
        for path in ("src/cdp/process.rs", "src/cdp/process/unix.rs", "src/process_owner.rs", "src/agent/process/unix.rs", "tests/support/unix_browser_ownership.rs", "tests/support/unix_agent_ownership.rs", "tests/support/agent_process_tree.py", "tests/support/browser_process_unix.py"):
            with self.subTest(path=path):
                result = focus.select([path])
                self.assertEqual(result, {"platform": "ubuntu-24.04", "suite": "conversation", "targets": ["agent", "attachments", "mcp"] if path == "tests/support/unix_agent_ownership.rs" else ["agent"]})
                build, checks = focus.commands(result["suite"], result["targets"])
                self.assertIn("--no-default-features", build[0])
                self.assertIn("--lib", checks[-1])
                self.assertEqual(checks[-1][checks[-1].index("--test") + 1], "agent")

    def test_initial_release_gate_and_manual_entries_exclude_deferred_targets(self):
        self.assertEqual(set(focus.PLATFORMS), {"ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"})
        self.assertEqual(focus.select([".github/workflows/ci.yml"])["platform"], "ubuntu-24.04")
        self.assertEqual(focus.select(["tests/windows_agent.rs"])["targets"], ["agent"])
        root = Path(__file__).resolve().parents[1]
        for name in ("ci", "chromium", "process", "release-evidence", "toolchain-evidence"):
            workflow = (root / ".github/workflows" / (name + ".yml")).read_text()
            with self.subTest(workflow=name):
                self.assertNotIn("- macos-15-intel", workflow)
                self.assertNotIn(", macos-15-intel", workflow)
                self.assertNotIn("- windows-2025", workflow)
                self.assertNotIn(", windows-2025", workflow)
        pipeline = (root / ".github/workflows/ci.yml").read_text()
        self.assertIn('platforms = runpy.run_path("scripts/ci/focus.py")["PLATFORMS"]', pipeline)

    def test_connection_grants_cover_actual_owner_and_mcp_consumers_before_rendering(self):
        for path in ("src/attachments.rs", "src/conversation/lease.rs", "src/conversation/storage.rs", "src/guidance/usage.rs"):
            with self.subTest(path=path):
                selected = focus.select([path])
                self.assertEqual(selected, {"platform": "ubuntu-24.04", "suite": "conversation", "targets": ["agent", "attachments", "mcp"]})
                build, checks = focus.commands(selected["suite"], selected["targets"])
                self.assertIn("--no-default-features", build[0])
                self.assertIn("agent_ownership::killed_cli_", checks[0])
                for target in selected["targets"]:
                    self.assertIn(target, checks[-1])
                self.assertIn("--skip", checks[-1])

    def test_session_restart_reproduction_runs_before_the_remaining_core_suite_without_repeating(self):
        for path in ("src/conversation/continuity.rs", "src/conversation/mod.rs", "src/agent.rs", "tests/support/acp_fixture.py"):
            selected = focus.select([path])
            self.assertEqual(selected["suite"], "conversation")
            build, checks = focus.commands(selected["suite"], selected["targets"])
            self.assertIn("--no-default-features", build[0])
            self.assertIn("session_restart_", checks[2])
            self.assertIn("--nocapture", checks[2])
            skip = checks[-1].index("session_restart_")
            self.assertEqual(checks[-1][skip - 1], "--skip")

    def test_core_edit_selects_its_existing_integration_target(self):
        result = focus.select(["src/queries.rs", "tests/queries.rs"])
        self.assertEqual(result["suite"], "core")
        self.assertEqual(result["targets"], ["queries"])
        self.assertEqual(result["platform"], "ubuntu-24.04")

    def test_project_changes_cover_queries_without_losing_script_or_shader_consumers(self):
        self.assertEqual(focus.select(["src/project/variables.rs"])["targets"], ["project", "queries"])
        for other, suite in [("src/script/header.rs", "graphics"), ("src/shader.rs", "dynamic")]:
            result = focus.select(["src/project.rs", other])
            self.assertEqual(result["suite"], suite)
            self.assertTrue({"project", "queries"} <= set(result["targets"]))
            self.assertIn("scripts" if suite == "graphics" else "shader_uniforms", result["targets"])
        self.assertEqual(focus.select(["tests/project.rs"])["targets"], ["project"])
        for other in ["src/script/header.rs", "src/shader.rs"]:
            result = focus.select(["src/operations.rs", other])
            self.assertIn("operations_cli", result["targets"])
        native = (Path(__file__).resolve().parents[1] / ".github/workflows/native.yml").read_text()
        self.assertIn("--no-default-features --test project --test scripts", native)

    def test_nested_core_and_adapter_fixtures_keep_their_affected_targets(self):
        self.assertEqual(focus.select(["src/store/content.rs"])["targets"], ["document_store"])
        result = focus.select(["tests/support/acp_fixture.py"])
        self.assertEqual(result["suite"], "conversation")
        self.assertEqual(result["platform"], "ubuntu-24.04")
        self.assertEqual(focus.select(["src/clipboard/macos.rs"])["platform"], "macos-15")

    def test_rendering_only_targets_are_never_checked_as_zero_test_core_binaries(self):
        for target in focus.RENDER_TESTS - focus.CREDENTIALED_TESTS:
            with self.subTest(target=target):
                result = focus.select([f"tests/{target}.rs"])
                self.assertIn(result["suite"], ["dynamic", "graphics", "shader"])
                self.assertIn(target, result["targets"])
                build, _ = focus.commands(result["suite"], result["targets"])
                self.assertNotIn("--no-default-features", build[0])
        self.assertNotIn("agent_workflow", focus.select(["tests/agent_workflow.rs"])["targets"])

    def test_platform_specific_rendering_fixes_do_not_become_core_agent_checks(self):
        result = focus.select(["src/text/windows.rs"])
        self.assertEqual(result["platform"], "ubuntu-24.04")
        self.assertEqual(result["suite"], "graphics")
        self.assertIn("text", result["targets"])
        for path in ["src/color.rs", "src/image_geometry.rs"]:
            result = focus.select([path])
            self.assertEqual(result["suite"], "graphics")
            build, _ = focus.commands(result["suite"], result["targets"])
            self.assertNotIn("--no-default-features", build[0])

    def test_graphics_with_workflow_changes_keeps_pixels_first_without_duplicate_tui_units(self):
        result = focus.select(["src/layout.rs", "tests/graphics.rs", ".github/workflows/native.yml"])
        self.assertEqual(result, {"platform": "ubuntu-24.04", "suite": "graphics", "targets": ["graphics", "layout", "render"]})
        _, checks = focus.commands(result["suite"], result["targets"])
        self.assertEqual(len(checks), 1)
        self.assertIn("--lib", checks[0])
        self.assertNotIn("tui::tests::", checks[0])
        self.assertIn("graphics", checks[0])
        self.assertIn("layout", checks[0])
        core = focus.select(["src/queries.rs", ".github/workflows/ci.yml"])
        self.assertEqual(core["suite"], "core")
        self.assertEqual(core["targets"], ["queries"])

    def test_layout_implementation_changes_include_raster_consumers_without_browser_optins(self):
        for paths in (["src/layout.rs"], ["src/layout/metrics.rs"], ["src/layout.rs", ".github/workflows/ci.yml"]):
            with self.subTest(paths=paths):
                result = focus.select(paths)
                self.assertEqual(result, {"platform": "ubuntu-24.04", "suite": "graphics", "targets": ["graphics", "layout", "render"]})
                build, checks = focus.commands(result["suite"], result["targets"])
                self.assertNotIn("--no-default-features", build[0])
                self.assertEqual(len(checks), 1)
                for target in result["targets"]:
                    self.assertIn(target, checks[0])
                self.assertNotIn("--ignored", checks[0])
                self.assertNotIn("--include-ignored", checks[0])
        self.assertEqual(focus.select(["tests/layout.rs"])["targets"], ["layout"])

    def test_static_render_regressions_stay_in_the_cpu_graphics_gate(self):
        for paths, targets in [
            (["tests/render.rs"], ["render"]),
            (["src/layout.rs", "tests/graphics.rs", "tests/render.rs"], ["graphics", "layout", "render"]),
        ]:
            result = focus.select(paths)
            self.assertEqual(result["platform"], "ubuntu-24.04")
            self.assertEqual(result["suite"], "graphics")
            self.assertEqual(result["targets"], targets)
            _, checks = focus.commands(result["suite"], targets)
            self.assertEqual(len(checks), 1)
            self.assertNotIn("--include-ignored", checks[0])
        self.assertEqual(focus.select(["src/render.rs"])["suite"], "dynamic")

    def test_platform_override_preserves_the_affected_suite(self):
        result = focus.select(["src/cdp.rs"], "macos-15")
        self.assertEqual(result["platform"], "macos-15")
        self.assertEqual(result["suite"], "dynamic")

    def test_graphics_targets_exist_and_unknown_changes_have_a_core_gate(self):
        for paths in (["src/graphics/mod.rs"], ["some-new-module.rs"], ["Cargo.lock"]):
            result = focus.select(paths)
            self.assertTrue(result["targets"])
            self.assertTrue(set(result["targets"]) <= focus.CORE_TESTS)

    def test_single_platform_success_cannot_replace_the_full_verified_baseline(self):
        replies = [
            {"workflow_runs": [{"id": 2, "head_sha": "diagnostic"}, {"id": 1, "head_sha": "verified"}]},
            {"jobs": [{"name": "Verified source artifact", "conclusion": "skipped"}]},
            {"jobs": [{"name": "Verified source artifact", "conclusion": "success"}]},
        ]
        with tempfile.TemporaryDirectory() as directory:
            event = Path(directory) / "event.json"
            event.write_text(json.dumps({"before": "previous"}), encoding="utf-8")
            environment = {"GITHUB_EVENT_PATH": str(event), "GITHUB_API_URL": "https://api.example.invalid", "GITHUB_REPOSITORY": "owner/repo", "GH_TOKEN": "fixture"}
            with patch.dict(os.environ, environment), patch.object(focus.urllib.request, "urlopen", side_effect=[io.BytesIO(json.dumps(value).encode()) for value in replies]), patch.object(focus.subprocess, "check_output", side_effect=["ancestor\n", "src/store/content.rs\n"]) as command:
                self.assertEqual(focus.changed_paths(), ["src/store/content.rs"])
                self.assertEqual(command.call_args_list[0].args[0], ["git", "merge-base", "previous", "verified"])
                self.assertEqual(command.call_args_list[1].args[0], ["git", "diff", "--name-only", "ancestor", "HEAD"])

    def test_failed_baseline_lookup_expands_instead_of_skipping_unverified_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            event = Path(directory) / "event.json"
            event.write_text("{}", encoding="utf-8")
            environment = {"GITHUB_EVENT_PATH": str(event), "GITHUB_API_URL": "https://api.example.invalid", "GITHUB_REPOSITORY": "owner/repo", "GH_TOKEN": "fixture"}
            with patch.dict(os.environ, environment), patch.object(focus.urllib.request, "urlopen", side_effect=urllib.error.URLError("offline")), patch.object(focus.subprocess, "check_output", return_value="src/agent/process/windows.rs\nsrc/render.rs\n") as command:
                self.assertEqual(focus.changed_paths(), ["src/agent/process/windows.rs", "src/render.rs"])
                self.assertEqual(command.call_args.args[0], ["git", "ls-files"])

    def test_runtime_optins_never_expand_to_credentialed_agent_requests(self):
        for suite in ("core", "conversation", "graphics", "dynamic", "shader"):
            result = focus.select(["src/shader.rs"] if suite=="shader" else ["src/cdp.rs"] if suite == "dynamic" else ["tests/queries.rs"])
            _, checks = focus.commands(suite, result["targets"])
            for command in checks:
                if "--include-ignored" in command:
                    self.assertNotIn("agent_workflow", command)
                    self.assertNotIn("agent", command)
                    self.assertIn("--test", command)


if __name__ == "__main__":
    unittest.main()
