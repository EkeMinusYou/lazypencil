import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("native_cache", Path(__file__).parent / "ci/native_cache.py")
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class NativeCacheTests(unittest.TestCase):
    def report(self, **changes):
        arguments = dict(platform="ubuntu-24.04", image="20261012.334.1", tree=cache.AUDITED_TREE,
                         compiler=f"rustc 1.94.0\nrelease: 1.94.0\ncommit-hash: {cache.AUDITED_RUST_COMMIT}\nhost: x86_64-unknown-linux-gnu\n",
                         environment=dict(cache.BASELINE_FLAGS))
        arguments.update(changes)
        return cache.identify(**arguments)

    def test_migration_is_limited_to_the_audited_inputs_flags_and_toolchain(self):
        self.assertTrue(self.report()["legacy"].endswith(cache.LEGACY_HASHES["ubuntu-24.04"]))
        variants = [dict(tree="1" * 64), dict(compiler="release: 1.95.0\n"),
                    dict(compiler="release: 1.94.0\ncommit-hash: different\n"),
                    dict(environment={**cache.BASELINE_FLAGS, "RUSTFLAGS": "--cfg changed"}),
                    dict(environment={**cache.BASELINE_FLAGS, "CARGO_PROFILE_RELEASE_LTO": "thin"}),
                    dict(environment={**cache.BASELINE_FLAGS, "SKIA_SOURCE_DIR": "/changed"})]
        original = self.report()["primary"]
        for variant in variants:
            with self.subTest(variant=variant):
                report = self.report(**variant)
                self.assertEqual(report["legacy"], "")
                self.assertNotEqual(report["primary"], original)

    def test_platform_image_and_actual_compiler_identity_bind_the_primary_key(self):
        original = self.report()["primary"]
        for changes in [dict(platform="macos-15"), dict(image="different-image"), dict(compiler="release: 1.94.0\ncommit-hash: different\n")]:
            self.assertNotEqual(self.report(**changes)["primary"], original)
        self.assertEqual(self.report(platform="macos-15-intel")["legacy"], "")
        self.assertTrue(self.report(platform="windows-2025")["legacy"].endswith(cache.LEGACY_HASHES["windows-2025"]))

    def test_audited_windows_feature_addition_restores_only_its_exact_previous_dependency_key(self):
        tree = "c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7"
        for platform in cache.PLATFORMS:
            with self.subTest(platform=platform):
                old = self.report(platform=platform)
                migrated = self.report(platform=platform, tree=tree)
                self.assertEqual(migrated["legacy"], old["primary"])
                self.assertNotEqual(migrated["primary"], old["primary"])
        for variant in [dict(compiler="release: 1.95.0\n"),
                        dict(compiler="release: 1.94.0\ncommit-hash: different\n"),
                        dict(environment={**cache.BASELINE_FLAGS, "RUSTFLAGS": "--cfg changed"}),
                        dict(environment={**cache.BASELINE_FLAGS, "SKIA_SOURCE_DIR": "/changed"}),
                        dict(environment={**cache.BASELINE_FLAGS, "CARGO_PROFILE_RELEASE_LTO": "thin"})]:
            self.assertEqual(self.report(tree=tree, **variant)["legacy"], "")
        self.assertEqual(self.report(tree="0" * 64)["legacy"], "")

    def test_non_build_environment_and_temporary_paths_do_not_invalidate_dependencies(self):
        original = self.report()
        changed = self.report(environment={**cache.BASELINE_FLAGS, "CARGO_TARGET_DIR": "/another/temp",
                                           "GITHUB_SHA": "new application", "UNRELATED_SECRET": "not-in-report"})
        self.assertEqual(changed, original)
        self.assertNotIn("not-in-report", str(changed))

    def test_unknown_or_untrusted_output_fields_are_rejected(self):
        for changes in [dict(platform="unknown"), dict(image="unknown"), dict(image="image\nlegacy=injected"),
                        dict(image="../image"), dict(tree="bad")]:
            with self.subTest(changes=changes), self.assertRaises(cache.CacheError):
                self.report(**changes)

    def test_git_inputs_bind_committed_changes_and_reject_dirty_or_untracked_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            hooks = root / "empty-hooks"
            hooks.mkdir()
            command = ["git", "-C", str(root), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                       "-c", "commit.gpgsign=false", "-c", f"core.hooksPath={hooks.as_posix()}"]
            def git(*args):
                subprocess.run([*command, *args], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            git("init")
            (root / "Cargo.toml").write_text("[package]\nname='fixture'\n", encoding="utf-8")
            (root / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
            git("add", "Cargo.toml", "Cargo.lock")
            git("commit", "-m", "Create fixture")
            original = cache.dependency_tree(root)
            (root / "README.md").write_text("Application documentation\n", encoding="utf-8")
            self.assertEqual(cache.dependency_tree(root), original)
            (root / "Cargo.lock").write_text("version = 3\n", encoding="utf-8")
            with self.assertRaises(cache.CacheError): cache.dependency_tree(root)
            git("add", "Cargo.lock")
            with self.assertRaises(cache.CacheError): cache.dependency_tree(root)
            git("commit", "-m", "Change fixture lock")
            self.assertNotEqual(cache.dependency_tree(root), original)
            (root / "vendor/skia-bindings").mkdir(parents=True)
            (root / "vendor/skia-bindings/new.cpp").write_text("changed", encoding="utf-8")
            with self.assertRaises(cache.CacheError): cache.dependency_tree(root)


if __name__ == "__main__":
    unittest.main()
