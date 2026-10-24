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
    def test_saved_browser_only_changes_use_the_static_frontend_reproduction(self):
        for path in ["src/tui/saved.rs", "src/tui/checkpoints.rs"]:
            self.assertEqual(focus.select([path]), {"platform":"ubuntu-24.04","suite":"graphics","targets":["tui","user_config"]})

    def test_saved_log_ui_is_reproduced_before_chromium_and_omitted_from_later_checks(self):
        result = focus.select(["src/conversation/history/display.rs", "src/tui/mod.rs", "src/tui/saved.rs", "tests/conversation_history.rs", "tests/tui.rs"])
        self.assertEqual(result["suite"], "graphics")
        self.assertTrue({"agent", "conversation_history", "tui", "user_config"} <= set(result["targets"]))
        native = (Path(__file__).resolve().parents[1] / ".github/workflows/native.yml").read_text()
        self.assertLess(native.index("Reproduce saved log browsing"), native.index("Install and verify the pinned Chromium runtime"))
        self.assertIn(focus.SAVED_LOG_PTY, native)
        self.assertIn(focus.SESSION_SELECTION_PTY, native)
        self.assertIn(focus.SESSION_RECLAIM, native)
        self.assertIn(focus.OWNED_MCP_RECLAIM, native)
        self.assertIn("cargo test --locked --no-default-features --lib conversation::management::", native)
        self.assertIn(focus.SESSION_DELETION_PTY, native)
        self.assertIn(focus.SESSION_DELETION_SMALL_PTY, native)
        for suite in ["graphics", "dynamic"]:
            _, checks = focus.commands(suite, result["targets"] + ["mcp"] + (["shader_uniforms", "dynamic_render"] if suite == "dynamic" else []))
            self.assertNotIn("conversation_history", checks[-1])
            self.assertEqual(checks[-1][checks[-1].index(focus.SAVED_LOG_PTY) - 1], "--skip")
            self.assertEqual(checks[-1][checks[-1].index("tui::tests::saved_log_") - 1], "--skip")
            for name in (focus.SESSION_SELECTION_PTY, focus.SESSION_DELETION_PTY, focus.SESSION_DELETION_SMALL_PTY, focus.SESSION_RECLAIM, focus.OWNED_MCP_RECLAIM):
                self.assertEqual(checks[-1][checks[-1].index(name) - 1], "--skip")
        mixed = focus.select(["tests/conversation_history.rs", "src/shader.rs"])
        self.assertEqual(mixed["suite"], "dynamic")
        focus.commands(mixed["suite"], mixed["targets"])

    def test_saved_conversation_deletion_reproduces_core_default_and_actual_ui_before_all_platforms(self):
        result = focus.select(["src/conversation/management/deletion.rs", "src/conversation/storage.rs", "src/tui/saved.rs", "src/tui/checkpoints.rs", "tests/conversation_history.rs", "tests/tui.rs"])
        self.assertEqual(result["suite"], "graphics")
        self.assertEqual(result["platform"], "ubuntu-24.04")
        self.assertTrue({"agent", "attachments", "mcp", "conversation_history", "tui", "user_config"} <= set(result["targets"]))

    def test_saved_history_runs_its_units_and_cli_reproduction_once_before_agent_checks(self):
        result = focus.select(["src/conversation/history.rs", "src/cli.rs", "src/main.rs", "tests/conversation_history.rs"])
        self.assertEqual(result["suite"], "conversation")
        self.assertEqual(result["targets"], ["agent", "conversation_history"])
        build, checks = focus.commands(result["suite"], result["targets"])
        self.assertIn("conversation_history", build[0])
        self.assertIn("conversation::management::", checks[0])
        self.assertIn("conversation::history::", checks[1])
        self.assertIn("conversation_history", checks[2])
        self.assertNotIn("conversation_history", checks[-1])
        self.assertEqual(checks[-1][checks[-1].index("conversation::history::") - 1], "--skip")

    def test_input_dispatch_reproduces_real_static_tui_before_full_dynamic_checks(self):
        paths = ["src/config/keys.rs", "src/config/keys/navigation.rs", "src/config.rs",
                 "src/tui/mod.rs", "src/tui/tests.rs", "tests/tui.rs", "tests/user_config.rs",
                 "tests/support/tui_pty.py", "docs/tui.md", "scripts/ci/focus.py",
                 "scripts/test_ci_focus.py", ".github/workflows/native.yml"]
        selected = focus.select(paths)
        self.assertEqual(selected["platform"], "ubuntu-24.04")
        self.assertEqual(selected["suite"], "graphics")
        self.assertEqual(selected["targets"], ["tui", "user_config"])
        build, checks = focus.commands(selected["suite"], selected["targets"])
        self.assertNotIn("--no-default-features", build[0])
        self.assertIn("tui", checks[-1])
        for extra in ["src/tui/preview.rs", "src/config/runtime.rs", "src/render.rs"]:
            with self.subTest(extra=extra):
                self.assertEqual(focus.select([*paths, extra])["suite"], "dynamic")

    def test_macos_terminal_capture_reproduces_saved_logs_on_macos_first(self):
        result = focus.select(["tests/support/terminal_screen.py", "scripts/test_terminal_screen.py",
                               "tests/fixtures/terminal/macos-saved-log-refresh.json", "scripts/ci/focus.py"])
        self.assertEqual(result["platform"], "macos-15")
        self.assertEqual(result["suite"], "graphics")
        self.assertEqual(result["targets"], ["conversation_history", "tui", "user_config"])

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
                first = next(index for index, command in enumerate(checks) if "agent_ownership::killed_cli_" in command and "--skip" not in command)
                self.assertIn("embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits", checks[first + 1])
                self.assertIn("--skip", checks[-1])
                self.assertIn("--test", checks[-1])

    def test_shared_dynamic_changes_use_one_linux_runner(self):
        for path in ("src/cdp/tabs.rs", "src/render.rs", "src/tui/preview.rs"):
            result = focus.select([path])
            self.assertEqual(result["platform"], "ubuntu-24.04")
            self.assertEqual(result["suite"], "dynamic")
            build, checks = focus.commands(result["suite"], result["targets"])
            self.assertNotIn("--no-default-features", build[0])
            self.assertIn("cdp::", checks[2])
            self.assertIn("retained_", checks[4])
            self.assertIn("--skip", checks[5])
            self.assertIn("--test-threads=1", checks[5])

    def test_shader_changes_select_cpu_validation_then_only_real_shader_cases(self):
        for paths in (["src/shader.rs"], ["src/shader/defaults.rs"], ["tests/shader_uniforms.rs"], ["src/webgl.js"], ["src/shader.rs", ".github/workflows/native.yml"]):
            result=focus.select(paths)
            self.assertEqual(result,{"platform":"ubuntu-24.04","suite":"shader","targets":["shader_uniforms"]})
            build,checks=focus.commands(result["suite"],result["targets"])
            self.assertNotIn("--lib",build[0])
            self.assertNotIn("--no-default-features",build[0])
            self.assertEqual(len(checks),3)
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
                targets = ["agent", "attachments", "mcp"]
                if path.startswith("src/conversation/"):
                    targets.insert(2, "conversation_history")
                self.assertEqual(selected, {"platform": "ubuntu-24.04", "suite": "conversation", "targets": targets})
                build, checks = focus.commands(selected["suite"], selected["targets"])
                self.assertIn("--no-default-features", build[0])
                first = next(command for command in checks if "agent_ownership::killed_cli_" in command and "--skip" not in command)
                self.assertIn("--nocapture", first)
                for target in selected["targets"]:
                    self.assertTrue(any(target in command for command in checks))
                self.assertIn("--skip", checks[-1])

    def test_session_restart_reproduction_runs_before_the_remaining_core_suite_without_repeating(self):
        for path in ("src/conversation/continuity.rs", "src/conversation/mod.rs", "src/agent.rs", "tests/support/acp_fixture.py"):
            selected = focus.select([path])
            self.assertEqual(selected["suite"], "conversation")
            build, checks = focus.commands(selected["suite"], selected["targets"])
            self.assertIn("--no-default-features", build[0])
            reproduction = next(command for command in checks if "session_restart_" in command and "--skip" not in command)
            self.assertIn("--nocapture", reproduction)
            skip = checks[-1].index("session_restart_")
            self.assertEqual(checks[-1][skip - 1], "--skip")

    def test_preprocessed_shader_pipeline_covers_rendering_and_checks_cpu_before_browser(self):
        result = focus.select(["src/dynamic/shader.rs", "src/render.rs", "src/shader.rs"])
        self.assertEqual(result["suite"], "dynamic")
        self.assertEqual(result["platform"], "ubuntu-24.04")
        self.assertIn("shader_uniforms", result["targets"])
        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/native.yml").read_text()
        early = workflow[workflow.index("- name: Validate dynamic inputs"):workflow.index("- name: Validate shader inputs")]
        self.assertIn("cargo test --locked --lib dynamic::", early)
        self.assertIn("shader::tests::prepared_declarations_bind_values_without_replacing_captured_shader_bytes", early)
        self.assertIn("shader::tests::preprocessing_preserves_directive_annotation_barriers_and_source_positions", early)
        self.assertIn("cargo test --locked --test shader_uniforms", early)
        self.assertIn("cargo clippy --locked --lib --test dynamic_render --test shader_uniforms -- -D warnings", workflow)
        for step in ("Check focused native calculation code", "Check focused dynamic code", "Check focused shader code"):
            self.assertLess(workflow.index("- name: " + step), workflow.index("- name: Build focused test binaries"))

    def test_observed_shader_regressions_run_first_without_repeating_or_losing_other_cases(self):
        cases = ["linked_constant_dimensions_validate_inputs_and_recover_after_an_error",
                 "native_annotation_binding_matches_public_pencil_reference_pixels"]
        self.assertEqual(focus.SHADER_REGRESSIONS, cases)
        for suite in ["dynamic", "shader"]:
            _, checks = focus.commands(suite, ["shader_uniforms"])
            for case, check in zip(cases, checks[:2]):
                self.assertIn(case, check)
                self.assertIn("--exact", check)
                self.assertIn("--ignored", check)
                self.assertNotIn("--skip", check)
                self.assertEqual(check[check.index("--test") + 1], "shader_uniforms")
                self.assertEqual(checks[-1][checks[-1].index(case)-1], "--skip")
        _, checks = focus.commands("dynamic", ["dynamic_render"])
        self.assertTrue(all(case not in check for check in checks for case in cases))

    def test_native_calculation_changes_keep_cancellation_reproduction_and_feature_configuration(self):
        for path in ["src/layout.rs", "src/text.rs", "src/render.rs", "src/render/backdrop.rs", "src/service.rs", "src/script/layout.rs"]:
            result = focus.select([path])
            self.assertIn("native_layout", result["targets"])
            self.assertIn(result["suite"], ["graphics", "dynamic"])
            build, checks = focus.commands(result["suite"], result["targets"])
            self.assertNotIn("--no-default-features", build[0])
            self.assertIn("native_layout", build[0])
            if result["suite"] == "graphics":
                self.assertIn("native_cancellation", checks[0])
                self.assertIn("native_layout", checks[1])
                self.assertNotIn("native_layout", checks[-1])
                self.assertEqual(checks[-1][checks[-1].index("native_cancellation")-1], "--skip")
            else:
                self.assertNotIn("native_layout", checks[-1])
        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/native.yml").read_text()
        self.assertLess(workflow.index("- name: Validate native cancellation"), workflow.index("- name: Install and verify the pinned Chromium runtime"))
        self.assertIn("inputs.suite == 'dynamic' && contains(fromJSON(inputs.targets), 'native_layout')", workflow)
        self.assertIn("cargo test --locked --lib native_cancellation", workflow)
        self.assertIn("cargo test --locked --test native_layout", workflow)
        mixed = focus.select(["src/agent.rs", "src/layout.rs"])
        self.assertEqual(mixed["suite"], "graphics")
        self.assertTrue({"agent", "native_layout"} <= set(mixed["targets"]))

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
        self.assertEqual(result, {"platform": "ubuntu-24.04", "suite": "graphics", "targets": ["graphics", "layout", "native_layout", "render"]})
        _, checks = focus.commands(result["suite"], result["targets"])
        self.assertEqual(len(checks), 3 if "native_layout" in result["targets"] else 1)
        self.assertIn("--lib", checks[-1])
        self.assertNotIn("tui::tests::", checks[-1])
        self.assertIn("graphics", checks[-1])
        self.assertIn("layout", checks[-1])
        core = focus.select(["src/queries.rs", ".github/workflows/ci.yml"])
        self.assertEqual(core["suite"], "core")
        self.assertEqual(core["targets"], ["queries"])

    def test_layout_implementation_changes_include_raster_consumers_without_browser_optins(self):
        for paths in (["src/layout.rs"], ["src/layout/metrics.rs"], ["src/layout.rs", ".github/workflows/ci.yml"]):
            with self.subTest(paths=paths):
                result = focus.select(paths)
                self.assertEqual(result, {"platform": "ubuntu-24.04", "suite": "graphics", "targets": ["graphics", "layout", "native_layout", "render"]})
                build, checks = focus.commands(result["suite"], result["targets"])
                self.assertNotIn("--no-default-features", build[0])
                self.assertEqual(len(checks), 3 if "native_layout" in result["targets"] else 1)
                for target in result["targets"]:
                    self.assertTrue(any(target in check for check in checks))
                self.assertNotIn("--ignored", checks[-1])
                self.assertNotIn("--include-ignored", checks[-1])
        self.assertEqual(focus.select(["tests/layout.rs"])["targets"], ["layout"])

    def test_static_render_regressions_stay_in_the_cpu_graphics_gate(self):
        for paths, targets in [
            (["tests/render.rs"], ["render"]),
            (["src/layout.rs", "tests/graphics.rs", "tests/render.rs"], ["graphics", "layout", "native_layout", "render"]),
        ]:
            result = focus.select(paths)
            self.assertEqual(result["platform"], "ubuntu-24.04")
            self.assertEqual(result["suite"], "graphics")
            self.assertEqual(result["targets"], targets)
            _, checks = focus.commands(result["suite"], targets)
            self.assertEqual(len(checks), 3 if "native_layout" in result["targets"] else 1)
            self.assertNotIn("--include-ignored", checks[-1])
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

    def test_transient_baseline_api_failure_retries_once_and_never_trusts_partial_success(self):
        for failures, expected_calls in (([503], 3), ([503, 503], 2), ([403], 1)):
            with self.subTest(failures=failures), tempfile.TemporaryDirectory() as directory:
                event = Path(directory) / "event.json"
                event.write_text(json.dumps({"before": "previous"}))
                environment = {"GITHUB_EVENT_PATH": str(event), "GITHUB_API_URL": "https://api.example.invalid", "GITHUB_REPOSITORY": "owner/repo", "GH_TOKEN": "fixture"}
                replies = [urllib.error.HTTPError("https://api.example.invalid", code, "fixture failure", {}, None) for code in failures]
                if len(failures) == 1 and failures[0] == 503:
                    replies += [io.BytesIO(json.dumps(value).encode()) for value in [
                        {"workflow_runs": [{"id": 1, "head_sha": "verified"}]},
                        {"jobs": [{"name": "Verified source artifact", "conclusion": "success"}]},
                    ]]
                with patch.dict(os.environ, environment), patch.object(focus.urllib.request, "urlopen", side_effect=replies) as api, patch.object(focus.time, "sleep") as sleep, patch.object(focus.subprocess, "check_output", side_effect=["ancestor\n", "src/tui/mod.rs\n"] if expected_calls == 3 else ["src/tui/mod.rs\nsrc/render.rs\n"]) as command:
                    self.assertEqual(focus.changed_paths(), ["src/tui/mod.rs"] if expected_calls == 3 else ["src/tui/mod.rs", "src/render.rs"])
                    self.assertEqual(api.call_count, expected_calls)
                    self.assertEqual(sleep.call_count, int(failures[0] == 503))
                    if expected_calls != 3:
                        self.assertEqual(command.call_args.args[0], ["git", "ls-files"])

    def test_unreadable_recent_candidate_can_use_an_older_completed_full_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            event = Path(directory) / "event.json"
            event.write_text(json.dumps({"before": "previous"}))
            environment = {"GITHUB_EVENT_PATH": str(event), "GITHUB_API_URL": "https://api.example.invalid", "GITHUB_REPOSITORY": "owner/repo", "GH_TOKEN": "fixture"}
            replies = [io.BytesIO(json.dumps({"workflow_runs": [{"id": 2, "head_sha": "unreadable"}, {"id": 1, "head_sha": "verified"}]}).encode()),
                       urllib.error.HTTPError("https://api.example.invalid", 404, "fixture missing", {}, None),
                       io.BytesIO(json.dumps({"jobs": [{"name": "Verified source artifact", "conclusion": "success"}]}).encode())]
            with patch.dict(os.environ, environment), patch.object(focus.urllib.request, "urlopen", side_effect=replies), patch.object(focus.subprocess, "check_output", side_effect=["ancestor\n", "src/tui/mod.rs\n"]) as command:
                self.assertEqual(focus.changed_paths(), ["src/tui/mod.rs"])
                self.assertEqual(command.call_args_list[0].args[0], ["git", "merge-base", "previous", "verified"])

    def test_mixed_or_expanded_changes_cannot_omit_changed_input_reproductions(self):
        result = focus.select(["src/config/keys/navigation.rs", "src/agent.rs", "src/render.rs"])
        self.assertEqual(result["suite"], "graphics")
        self.assertTrue({"agent", "native_layout", "tui", "user_config"} <= set(result["targets"]))

    def test_input_reproductions_run_before_chromium_and_are_not_repeated(self):
        native = (Path(__file__).resolve().parents[1] / ".github/workflows/native.yml").read_text()
        self.assertLess(native.index("Reproduce configured input"), native.index("Install and verify the pinned Chromium runtime"))
        for suite in ("graphics", "dynamic"):
            targets = ["tui", "user_config"] + (["dynamic_render", "shader_uniforms"] if suite == "dynamic" else [])
            _, checks = focus.commands(suite, targets)
            for name in ("config::keys::", *focus.INPUT_REPRODUCTIONS):
                index = checks[-1].index(name)
                self.assertEqual(checks[-1][index - 1], "--skip")
            for name in focus.INPUT_REPRODUCTIONS:
                self.assertIn(name, native)

    def test_cold_build_precedes_short_lints_and_is_not_built_again_on_success(self):
        root = Path(__file__).resolve().parents[1]
        native = (root / ".github/workflows/native.yml").read_text()
        self.assertLess(native.index("id: cold-build"), native.index("Check focused core code"))
        self.assertIn("steps.native-cache.outputs.cache-matched-key == ''", native)
        self.assertIn("if: inputs.phase == 'focus' && steps.cold-build.outcome != 'success'", native)
        prune = native[native.index("id: cache-prune"):]
        self.assertIn("!cancelled()", prune)
        self.assertIn("steps.cold-build.outcome == 'success'", prune)
        ci = (root / ".github/workflows/ci.yml").read_text()
        self.assertIn("needs: [plan, shader-preprocessor, cache-maintenance]", ci)
        self.assertIn("needs.cache-maintenance.result == 'skipped'", ci)
        self.assertIn('cache_retention.py --manifest "$RUNNER_TEMP/cache-source/SOURCE-MANIFEST.json" --delete', ci)

    def test_infrastructure_only_changes_do_not_add_browser_tests_to_input_dispatch(self):
        paths = ["src/config/keys/navigation.rs", "src/tui/mod.rs", "tests/tui.rs", "tests/user_config.rs",
                 "scripts/ci/application_cache.py", "scripts/ci/audit_tool_cache.py", "scripts/ci/cache_retention.py",
                 "scripts/test_application_cache.py", ".github/workflows/ci.yml", ".github/workflows/native.yml"]
        result = focus.select(paths)
        self.assertEqual(result["suite"], "graphics")
        self.assertEqual(result["targets"], ["tui", "user_config"])
        mixed = focus.select(["src/config/keys/navigation.rs", "src/shader.rs"])
        self.assertEqual(mixed["suite"], "dynamic")
        self.assertTrue({"tui", "user_config", "shader_uniforms", "dynamic_render"} <= set(mixed["targets"]))
        focus.commands(mixed["suite"], mixed["targets"])

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
