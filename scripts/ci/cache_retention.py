"""Remove redundant main-branch checkpoints and bound application cache history."""

import argparse
import json
import os
import re
import subprocess


def obsolete(caches):
    caches = [entry for entry in caches if entry["ref"] == "refs/heads/main"]
    keys = {(entry["key"], entry["version"]) for entry in caches}
    selected = {}
    groups = {}
    for entry in caches:
        key = entry["key"]
        if key.startswith("checkpoint-native-v2-") and (key.removeprefix("checkpoint-"), entry["version"]) in keys:
            selected[entry["id"]] = entry
        match = re.fullmatch(r"application-v1-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-([0-9a-f]{64})-(full|focus)", key)
        if match:
            platform, _, phase = match.groups()
            if phase == "focus" and (key.removesuffix("focus") + "full", entry["version"]) in keys:
                selected[entry["id"]] = entry
            else:
                groups.setdefault((platform, phase), []).append(entry)
    for (_, phase), entries in groups.items():
        ordered = sorted(entries, key=lambda entry: (entry["created_at"], entry["id"]), reverse=True)
        for entry in ordered[2 if phase == "full" else 1:]:
            selected[entry["id"]] = entry
    return sorted(selected.values(), key=lambda entry: entry["id"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--delete", action="store_true")
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    args = parser.parse_args()
    if not args.repository or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository):
        raise SystemExit("An explicit GitHub repository is required")
    endpoint = f"repos/{args.repository}/actions/caches"
    pages = subprocess.run(["gh", "api", "--paginate", "--slurp", endpoint + "?per_page=100"], check=True, capture_output=True, text=True, timeout=45)
    entries = obsolete([entry for page in json.loads(pages.stdout) for entry in page["actions_caches"]])
    print(json.dumps({"delete": args.delete, "bytes": sum(entry["size_in_bytes"] for entry in entries),
                      "caches": [{"id": entry["id"], "key": entry["key"]} for entry in entries]}, sort_keys=True), flush=True)
    for entry in entries:
        if args.delete:
            subprocess.run(["gh", "api", "--method", "DELETE", f"{endpoint}/{entry['id']}"], check=True, timeout=15)


if __name__ == "__main__":
    main()
