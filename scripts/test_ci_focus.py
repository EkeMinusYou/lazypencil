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
    def test_windows_fix_reproduces_on_windows_before_the_matrix(self):
        for path in ("src/agent/process/windows.rs", "src/conversation/mod.rs", "tests/windows_agent.rs"):
            with self.subTest(path=path):
                result = focus.select([path])
                self.assertEqual(result["platform"], "windows-2025")
                build, checks = focus.commands(result["suite"], result["targets"])
                self.assertIn("--no-default-features", build[0])
                self.assertIn("embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits", checks[0])
                self.assertIn("--test", checks[1])

    def test_shared_dynamic_changes_use_one_linux_runner(self):
        for path in ("src/cdp/tabs.rs", "src/script/context.rs", "src/render.rs", "src/tui/preview.rs", "tests/shader_uniforms.rs"):
            result = focus.select([path])
            self.assertEqual(result["platform"], "ubuntu-24.04")
            self.assertEqual(result["suite"], "dynamic")
            build, checks = focus.commands(result["suite"], result["targets"])
            self.assertNotIn("--no-default-features", build[0])
            self.assertIn("retained_", checks[1])
            self.assertIn("--skip", checks[2])
            self.assertIn("--test-threads=1", checks[2])

    def test_core_edit_selects_its_existing_integration_target(self):
        result = focus.select(["src/queries.rs", "tests/queries.rs"])
        self.assertEqual(result["suite"], "core")
        self.assertEqual(result["targets"], ["queries"])
        self.assertEqual(result["platform"], "ubuntu-24.04")

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
                self.assertIn(result["suite"], ["dynamic", "graphics"])
                self.assertIn(target, result["targets"])
                build, _ = focus.commands(result["suite"], result["targets"])
                self.assertNotIn("--no-default-features", build[0])
        self.assertNotIn("agent_workflow", focus.select(["tests/agent_workflow.rs"])["targets"])

    def test_platform_specific_rendering_fixes_do_not_become_core_agent_checks(self):
        result = focus.select(["src/text/windows.rs"])
        self.assertEqual(result["platform"], "windows-2025")
        self.assertEqual(result["suite"], "graphics")
        self.assertIn("text", result["targets"])
        for path in ["src/color.rs", "src/image_geometry.rs"]:
            result = focus.select([path])
            self.assertEqual(result["suite"], "graphics")
            build, _ = focus.commands(result["suite"], result["targets"])
            self.assertNotIn("--no-default-features", build[0])

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
        for suite in ("core", "conversation", "graphics", "dynamic"):
            result = focus.select(["src/cdp.rs"] if suite == "dynamic" else ["tests/queries.rs"])
            _, checks = focus.commands(suite, result["targets"])
            for command in checks:
                if "--include-ignored" in command:
                    self.assertNotIn("agent_workflow", command)
                    self.assertNotIn("agent", command)
                    self.assertIn("--test", command)


if __name__ == "__main__":
    unittest.main()
