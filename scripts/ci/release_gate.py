"""Reject incomplete final verification before producing a release source archive."""

import json
import os
import sys


def require_success(results):
    if not isinstance(results, dict) or set(results) != {"native", "audit"}:
        raise ValueError("Expected exactly the native and audit verification results")
    states = {}
    for name in ("native", "audit"):
        result = results[name]
        status = result.get("result") if isinstance(result, dict) else None
        if status != "success":
            raise ValueError(f"Required {name} verification did not succeed: {status!r}")
        states[name] = status
    return states


if __name__ == "__main__":
    try:
        print(json.dumps(require_success(json.loads(os.environ.get("VERIFICATION_RESULTS", "null")))))
    except ValueError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1) from None
