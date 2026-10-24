import importlib.util
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
        for path in ("src/cdp/tabs.rs", "src/render.rs", "src/tui/preview.rs", "tests/shader_uniforms.rs"):
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

    def test_platform_override_preserves_the_affected_suite(self):
        result = focus.select(["src/cdp.rs"], "macos-15")
        self.assertEqual(result["platform"], "macos-15")
        self.assertEqual(result["suite"], "dynamic")

    def test_graphics_targets_exist_and_unknown_changes_have_a_core_gate(self):
        for paths in (["src/graphics/mod.rs"], ["some-new-module.rs"], ["Cargo.lock"]):
            result = focus.select(paths)
            self.assertTrue(result["targets"])
            self.assertTrue(set(result["targets"]) <= focus.CORE_TESTS)

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
