"""Bound a hosted command and reap its entire process group on timeout."""

import argparse
import os
import signal
import subprocess


def run(command, timeout):
    process = subprocess.Popen(command, start_new_session=True)
    try:
        result = subprocess.CompletedProcess(command, process.wait(timeout=timeout))
        result.check_returncode()
        return result
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        # Cargo's native build children inherit log pipes. Killing Cargo alone
        # leaves Ninja running and prevents tee from finishing after a timeout.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=2)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.timeout <= 0 or not args.command:
        parser.error("A positive timeout and command are required")
    run(args.command, args.timeout)


if __name__ == "__main__":
    main()
