import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


spec = importlib.util.spec_from_file_location("ci_command", Path(__file__).parent / "ci/command.py")
command = importlib.util.module_from_spec(spec)
spec.loader.exec_module(command)


@unittest.skipUnless(os.name == "posix", "Initial release CI runs on POSIX hosts")
class CommandTests(unittest.TestCase):
    def test_success_and_nonzero_exit_are_preserved(self):
        self.assertEqual(command.run([sys.executable, "-c", "pass"], 5).returncode, 0)
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            command.run([sys.executable, "-c", "raise SystemExit(7)"], 5)
        self.assertEqual(raised.exception.returncode, 7)

    def test_timeout_kills_a_descendant_that_ignores_term_and_holds_the_log_pipe(self):
        with tempfile.TemporaryDirectory() as temporary:
            child = Path(temporary) / "child.pid"
            parent = """import pathlib, signal, subprocess, sys, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
subprocess.Popen([sys.executable, '-c', "import pathlib, signal, sys, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); pathlib.Path(sys.argv[1]).write_text(str(__import__('os').getpid())); time.sleep(60)", sys.argv[1]])
time.sleep(60)
"""
            wrapper = Path(__file__).parent / "ci/command.py"
            started = time.monotonic()
            process = subprocess.Popen([sys.executable, str(wrapper), "--timeout", "1", sys.executable, "-c", parent, str(child)],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                _, error = process.communicate(timeout=8)
                self.assertNotEqual(process.returncode, 0)
                self.assertIn(b"TimeoutExpired", error)
                self.assertLess(time.monotonic() - started, 8)
                self.assertTrue(child.exists(), "The timed command must actually start its child")
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
                if child.exists():
                    # Reap a surviving fixture even if the regression returns.
                    try:
                        os.kill(int(child.read_text()), signal.SIGKILL)
                    except ProcessLookupError:
                        pass

