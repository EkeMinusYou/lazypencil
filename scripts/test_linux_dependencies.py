import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location("linux_dependencies", Path(__file__).parent / "ci/linux_dependencies.py")
dependencies = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dependencies)


class LinuxDependencyTests(unittest.TestCase):
    def test_x64_uses_one_temporary_signed_ubuntu_source_for_both_commands(self):
        observed = []
        def run(arguments, **kwargs):
            source = Path(next(a.removeprefix("Dir::Etc::sourcelist=") for a in arguments if a.startswith("Dir::Etc::sourcelist=")))
            self.assertTrue(source.is_file())
            self.assertEqual(source.read_text(encoding="utf-8"), dependencies.SOURCES)
            parts = Path(next(a.removeprefix("Dir::Etc::sourceparts=") for a in arguments if a.startswith("Dir::Etc::sourceparts=")))
            self.assertTrue(parts.is_dir())
            self.assertEqual(list(parts.iterdir()), [])
            self.assertIn("Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg", source.read_text())
            self.assertNotIn("azure.archive.ubuntu.com", source.read_text())
            self.assertNotIn("trusted=yes", source.read_text())
            self.assertTrue(kwargs["check"])
            observed.append((arguments, source))
        with tempfile.TemporaryDirectory(prefix="apt fixture ") as temporary, mock.patch.object(dependencies.subprocess, "run", side_effect=run):
            dependencies.install("X64", temporary)
            self.assertEqual(observed[0][0][-1], "update")
            self.assertEqual(observed[1][0][-len(dependencies.PACKAGES)-2:], ["install", "--yes", *dependencies.PACKAGES])
            self.assertEqual(observed[0][0][:-1], observed[1][0][:-len(dependencies.PACKAGES)-2])
            self.assertEqual(observed[0][1], observed[1][1])
            self.assertFalse(observed[0][1].exists())

    def test_arm64_keeps_the_previously_verified_runner_sources(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(dependencies.subprocess, "run") as run:
            dependencies.install("ARM64", temporary)
            self.assertEqual(run.call_args_list, [
                mock.call(["sudo", "apt-get", "update"], check=True),
                mock.call(["sudo", "apt-get", "install", "--yes", *dependencies.PACKAGES], check=True)])

    def test_update_failure_propagates_without_attempting_an_install(self):
        failure = subprocess.CalledProcessError(100, "apt-get")
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(dependencies.subprocess, "run", side_effect=failure) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                dependencies.install("X64", temporary)
            self.assertEqual(run.call_count, 1)
            self.assertEqual(list(Path(temporary).iterdir()), [])

    def test_unknown_architecture_and_local_invocation_cannot_launch_apt(self):
        with mock.patch.object(dependencies.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                dependencies.install("unknown", None)
            with mock.patch.dict(dependencies.os.environ, {"GITHUB_ACTIONS": "false"}):
                with self.assertRaises(ValueError):
                    dependencies.main()
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
