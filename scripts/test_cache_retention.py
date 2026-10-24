import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location("cache_retention", Path(__file__).parent / "ci/cache_retention.py")
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class CacheRetentionTests(unittest.TestCase):
    def entry(self, identifier, key, ref="refs/heads/main"):
        return {"id": identifier, "key": key, "ref": ref, "version": "fixture-cache-layout",
                "created_at": f"2026-10-{identifier:02d}T00:00:00Z"}

    def test_checkpoint_is_removed_only_with_an_exact_main_branch_full_cache(self):
        entries = [self.entry(1, "checkpoint-native-v2-current"), self.entry(2, "native-v2-current"),
                   self.entry(3, "checkpoint-native-v2-missing"), self.entry(4, "checkpoint-native-v2-pr-only"),
                   self.entry(5, "native-v2-pr-only", "refs/pull/1/merge"),
                   self.entry(6, "checkpoint-native-v2-current", "refs/pull/1/merge")]
        self.assertEqual([entry["id"] for entry in cache.obsolete(entries)], [1])
        entries[1]["version"] = "different-cached-paths"
        self.assertEqual(cache.obsolete(entries), [])

    def test_application_history_keeps_two_full_and_one_unmatched_focus_per_platform(self):
        entries = []
        for platform in ("ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"):
            for index in range(1, 4):
                for phase in ("full", "focus"):
                    entries.append(self.entry(index, f"application-v1-{platform}-{str(index) * 64}-{phase}"))
            entries.append(self.entry(4, f"application-v1-{platform}-{'4' * 64}-focus"))
            entries.append(self.entry(5, f"application-v1-{platform}-{'5' * 64}-focus"))
        # Cache IDs are unique in the API even when fixture creation dates match.
        for identifier, entry in enumerate(entries):
            entry["id"] = identifier
        removed = {entry["id"] for entry in cache.obsolete(entries)}
        kept = [entry for entry in entries if entry["id"] not in removed]
        self.assertEqual(len(kept), 9)
        self.assertTrue(all(entry["key"].endswith("-full") or "5" * 64 in entry["key"] for entry in kept))

    def test_unrelated_dependency_download_tool_and_pull_request_caches_are_preserved(self):
        entries = [self.entry(index, key) for index, key in enumerate(("native-v2-current", "cargo-downloads-current", "shader-wasm-current", "cargo-deny-v1-current"))]
        entries.append(self.entry(5, "application-v1-ubuntu-24.04-" + "1" * 64 + "-full", "refs/pull/1/merge"))
        self.assertEqual(cache.obsolete(entries), [])


if __name__ == "__main__":
    unittest.main()
