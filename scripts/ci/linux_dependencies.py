"""Install native CI packages on the disposable Ubuntu 24.04 hosted runner."""

import os
from pathlib import Path
import platform
import subprocess
import tempfile


PACKAGES = ["clang", "libclang-dev", "cmake", "ninja-build", "pkg-config",
            "libfontconfig1-dev", "libfreetype6-dev", "libwayland-dev", "libx11-dev"]
SOURCES = """Types: deb
URIs: https://archive.ubuntu.com/ubuntu
Suites: noble noble-updates noble-backports
Components: main restricted universe multiverse
Architectures: amd64
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: https://security.ubuntu.com/ubuntu
Suites: noble-security
Components: main restricted universe multiverse
Architectures: amd64
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
"""


def install(architecture, temporary):
    if architecture not in {"X64", "ARM64"}:
        raise ValueError("Unsupported hosted Linux architecture")
    with tempfile.TemporaryDirectory(prefix="lazypencil-apt-", dir=temporary) as directory:
        options = []
        if architecture == "X64":
            # The Azure mirror stalled repeatedly on package bodies. Select
            # the same Ubuntu suites with the official archive signature key.
            # Per-command sources leave the runner's global source files intact.
            source = Path(directory) / "ubuntu.sources"
            source.write_text(SOURCES, encoding="utf-8")
            parts = Path(directory) / "empty-parts"
            parts.mkdir()
            options = ["-o", f"Dir::Etc::sourcelist={source}",
                       "-o", f"Dir::Etc::sourceparts={parts}",
                       "-o", "Acquire::Retries=2",
                       "-o", "Acquire::https::Timeout=30"]
            print("Using official Ubuntu HTTPS archives for native CI packages", flush=True)
        base = ["sudo", "apt-get", *options]
        subprocess.run([*base, "update"], check=True)
        subprocess.run([*base, "install", "--yes", *PACKAGES], check=True)


def main():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_OS") != "Linux" or platform.system() != "Linux":
        raise ValueError("This installer requires a disposable GitHub Actions Linux runner")
    release = platform.freedesktop_os_release()
    if release.get("ID") != "ubuntu" or release.get("VERSION_ID") != "24.04":
        raise ValueError("This installer requires Ubuntu 24.04")
    temporary = Path(os.environ["RUNNER_TEMP"])
    if not temporary.is_absolute() or not temporary.is_dir():
        raise ValueError("RUNNER_TEMP must name the hosted temporary directory")
    install(os.environ["RUNNER_ARCH"], temporary)


if __name__ == "__main__":
    main()
