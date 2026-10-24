import copy
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).parent / "ci/release_gate.py"
spec = importlib.util.spec_from_file_location("release_gate", SCRIPT)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class ReleaseGateTests(unittest.TestCase):
    def results(self):
        return {"native": {"result": "success", "outputs": {}}, "audit": {"result": "success", "outputs": {}}}

    def test_both_required_jobs_must_succeed(self):
        self.assertEqual(gate.require_success(self.results()), {"native": "success", "audit": "success"})

    def test_skipped_failed_cancelled_and_unknown_results_never_authorize_an_archive(self):
        for name in ("native", "audit"):
            for state in ("skipped", "failure", "cancelled", "pending", "SUCCESS", None, True):
                with self.subTest(job=name, state=state):
                    results = self.results()
                    results[name]["result"] = state
                    with self.assertRaises(ValueError):
                        gate.require_success(results)

    def test_missing_extra_and_malformed_job_results_fail_closed(self):
        missing = self.results()
        del missing["native"]
        extra = self.results()
        extra["unexpected"] = {"result": "success"}
        for results in (None, [], {}, missing, extra):
            with self.subTest(results=results), self.assertRaises(ValueError):
                gate.require_success(results)
        for malformed in (None, [], "success", {}, {"outputs": {}}):
            results = copy.deepcopy(self.results())
            results["native"] = malformed
            with self.subTest(job=malformed), self.assertRaises(ValueError):
                gate.require_success(results)

    def test_cli_returns_failure_for_invalid_json_or_missing_results(self):
        for raw in ("{", "null", "{}"):
            with self.subTest(raw=raw):
                result = subprocess.run([sys.executable, str(SCRIPT)], env={**os.environ, "VERIFICATION_RESULTS": raw}, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 1)
                self.assertFalse(result.stdout)
                self.assertTrue(result.stderr)


if __name__ == "__main__":
    unittest.main()
