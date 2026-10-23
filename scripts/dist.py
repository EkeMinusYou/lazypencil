"""Create and verify complete, deterministic source distributions (Python 3.11+)."""

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import tempfile
import tomllib
import unicodedata
import zlib


MAX_FILES = 10000
MAX_BYTES = 256 * 1024 * 1024
MAX_MANIFEST = 4 * 1024 * 1024
ROOT_FILES = ("Cargo.toml", "Cargo.lock", "LICENSE", "README.md", ".gitignore", ".gitattributes", "deny.toml")
ROOT_DIRS = ("src", "resources", "examples", "tests", "scripts", "docs", ".github", "vendor/skia-bindings")
MANIFEST = "SOURCE-MANIFEST.json"


class DistributionError(Exception):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def safe_path(value):
    if not isinstance(value, str) or "\\" in value or "\x00" in value:
        raise DistributionError("Archive paths must be relative POSIX paths")
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or any(p in ("", ".", "..") for p in value.split("/")):
        raise DistributionError("Archive paths must not escape the distribution")
    if len(value.encode("utf-8")) > 4096 or any(len(p.encode("utf-8")) > 255 for p in path.parts):
        raise DistributionError("Archive paths exceed portable filename limits")
    if any(":" in p for p in path.parts):
        raise DistributionError("Archive paths must not contain drive or stream names")
    reserved = {"con", "prn", "aux", "nul"} | {f"{base}{n}" for base in ("com", "lpt") for n in range(1, 10)}
    if any(p.endswith((".", " ")) or p.split(".")[0].casefold() in reserved or any(ord(c) < 32 for c in p) for p in path.parts):
        raise DistributionError("Archive paths must be usable as ordinary files on Windows")
    return value


def path_key(value):
    return unicodedata.normalize("NFC", value).casefold()


def excluded(path):
    return (
        path == PurePosixPath("vendor/skia-bindings/skia")
        or PurePosixPath("vendor/skia-bindings/skia") in path.parents
        or any(p in (".git", "target", "__pycache__", ".DS_Store") for p in path.parts)
        or path.suffix in (".pyc", ".pyo", ".tmp")
    )


def source_files(root, selected=None):
    files = {}
    folded = set()
    total = 0
    directories = None if selected is None else {
        parent.as_posix() for name in selected for parent in PurePosixPath(name).parents
    }

    def visit(path):
        nonlocal total
        relative = PurePosixPath(path.relative_to(root).as_posix())
        if excluded(relative):
            return
        if selected is not None and relative.as_posix() not in selected and relative.as_posix() not in directories:
            return
        info = path.lstat()
        if stat.S_ISLNK(info.st_mode):
            raise DistributionError(f"Source distributions do not follow symlinks: {relative}")
        if stat.S_ISDIR(info.st_mode):
            for child in sorted(path.iterdir()):
                visit(child)
            return
        if not stat.S_ISREG(info.st_mode):
            raise DistributionError(f"Unsupported source file: {relative}")
        key = safe_path(relative.as_posix())
        if path_key(key) in folded:
            raise DistributionError(f"Source paths collide on case-insensitive filesystems: {key}")
        folded.add(path_key(key))
        if len(files) >= MAX_FILES or info.st_size > MAX_BYTES - total:
            raise DistributionError("Source distribution exceeds its file/byte budget")
        flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_BINARY", 0)
        with os.fdopen(os.open(path, flags), "rb") as original:
            opened = os.fstat(original.fileno())
            if not stat.S_ISREG(opened.st_mode) or (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
                raise DistributionError(f"Source changed before reading: {key}")
            data = original.read(info.st_size + 1)
        after = path.lstat()
        if (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ) or len(data) != info.st_size:
            raise DistributionError(f"Source changed while reading: {key}")
        total += len(data)
        files[key] = (data, 0o755 if info.st_mode & 0o111 else 0o644)

    for name in ROOT_FILES:
        path = root / name
        if path.exists() or path.is_symlink():
            visit(path)
        elif name not in (".gitignore", ".gitattributes", "deny.toml"):
            raise DistributionError(f"Required source file is missing: {name}")
    for name in ROOT_DIRS:
        path = root / name
        if path.exists() or path.is_symlink():
            visit(path)
    return files


def package_identity(files):
    try:
        manifest = tomllib.loads(files["Cargo.toml"][0].decode("utf-8"))
        package = manifest["package"]
        name, version = package["name"], package["version"]
        if not re.fullmatch(r"[A-Za-z0-9_-]+", name) or not re.fullmatch(r"[A-Za-z0-9.+-]+", version):
            raise DistributionError("Package name/version cannot form a safe archive prefix")
        for table in manifest.get("patch", {}).values():
            for dependency in table.values():
                if isinstance(dependency, dict) and "path" in dependency:
                    path = safe_path(dependency["path"])
                    if path + "/Cargo.toml" not in files:
                        raise DistributionError(f"The source distribution omits a patched dependency: {path}")
        for required in ("Cargo.lock", "LICENSE", "README.md", "src/main.rs"):
            if required not in files:
                raise DistributionError(f"Required source file is missing: {required}")
        return {"name": name, "version": version, "rust_version": package.get("rust-version")}
    except (KeyError, UnicodeError, tomllib.TOMLDecodeError, TypeError) as error:
        raise DistributionError("Invalid source package manifest") from error


def vcs(root, allow_dirty):
    def git(*arguments):
        return subprocess.run(["git", "-C", str(root), *arguments], capture_output=True, text=True, check=True).stdout.strip()
    try:
        revision = git("rev-parse", "HEAD")
        dirty = bool(git("status", "--porcelain=v1", "--untracked-files=all"))
    except (OSError, subprocess.CalledProcessError):
        if not allow_dirty:
            raise DistributionError("Source provenance requires Git; use --allow-dirty for a local development archive")
        return None
    if dirty and not allow_dirty:
        raise DistributionError("Refusing to label a dirty checkout as release source; use --allow-dirty for a development archive")
    return {"revision": revision, "dirty": dirty}


def git_source_paths(root, allow_dirty, required):
    arguments = ["git", "-C", str(root), "ls-files", "--cached", "-z"]
    if allow_dirty:
        arguments.extend(["--others", "--exclude-standard"])
    arguments.extend(["--", "."])
    try:
        result = subprocess.run(arguments, capture_output=True, check=True)
    except (OSError, subprocess.CalledProcessError) as error:
        if required:
            raise DistributionError("Cannot enumerate the Git source files") from error
        return None
    try:
        return {name.decode("utf-8") for name in result.stdout.split(b"\0") if name}
    except UnicodeError as error:
        raise DistributionError("Source filenames must be valid UTF-8") from error


def write_member(archive, name, data, mode, epoch):
    member = tarfile.TarInfo(name)
    member.size = len(data)
    member.mode = mode
    member.mtime = epoch
    member.uid = member.gid = 0
    member.uname = member.gname = ""
    archive.addfile(member, io.BytesIO(data))


def source(root, output, epoch=0, allow_dirty=False):
    if type(epoch) is not int or epoch < 0 or epoch > 0xFFFFFFFF:
        raise DistributionError("Archive epoch must be within 0..4294967295")
    root = root.resolve(strict=True)
    provenance = vcs(root, allow_dirty)
    selected = git_source_paths(root, allow_dirty, required=provenance is not None)
    files = source_files(root, selected)
    package = package_identity(files)
    if vcs(root, allow_dirty) != provenance:
        raise DistributionError("Source provenance changed while collecting files")
    prefix = package["name"] + "-" + package["version"]
    manifest = {
        "format_version": 1,
        "package": package,
        "source_date_epoch": epoch,
        "git": provenance,
        "files": [
            {"path": name, "sha256": digest(data), "bytes": len(data), "mode": mode}
            for name, (data, mode) in sorted(files.items())
        ],
    }
    encoded = (json.dumps(manifest, ensure_ascii=False, indent=2) + "\n").encode()
    if len(encoded) > MAX_MANIFEST:
        raise DistributionError("Source manifest exceeds its byte budget")
    output = output.absolute()
    if output.exists() or output.is_symlink():
        raise DistributionError("Refusing to overwrite an existing distribution")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="lazypencil-dist-", dir=output.parent) as directory:
        temporary = Path(directory) / "source.tar.gz"
        with temporary.open("wb") as destination:
            with gzip.GzipFile(filename="", mode="wb", fileobj=destination, mtime=epoch) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    write_member(archive, prefix + "/" + MANIFEST, encoded, 0o644, epoch)
                    for name, (data, mode) in sorted(files.items()):
                        write_member(archive, prefix + "/" + name, data, mode, epoch)
        verify(temporary)
        # The staging file shares its filesystem with the output; a hard link
        # publishes the complete verified archive atomically without replacement.
        try:
            os.link(temporary, output)
        except FileExistsError as error:
            raise DistributionError("Distribution output appeared before publication") from error
    return {"archive": str(output), "sha256": digest(output.read_bytes()), "files": len(files), "package": package, "git": provenance}


def read_archive(path):
    if path.stat().st_size > MAX_BYTES:
        raise DistributionError("Compressed distribution exceeds its byte budget")
    limit = MAX_BYTES + MAX_MANIFEST + MAX_FILES * 4096
    with gzip.open(path, "rb") as original:
        data = original.read(limit + 1)
    if len(data) > limit:
        raise DistributionError("Expanded distribution exceeds its byte budget")
    result = {}
    folded = set()
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:") as archive:
        for member in archive:
            name = safe_path(member.name)
            if not member.isreg() or member.linkname or member.issparse():
                raise DistributionError("Source archives may contain only regular files")
            if path_key(name) in folded:
                raise DistributionError("Distribution contains duplicate or colliding paths")
            folded.add(path_key(name))
            if len(result) > MAX_FILES or member.size < 0 or member.size > MAX_BYTES + MAX_MANIFEST - total:
                raise DistributionError("Distribution exceeds its file/byte budget")
            total += member.size
            stream = archive.extractfile(member)
            result[name] = (stream.read(), member.mode)
    return result


def verify(path, extract=None):
    members = read_archive(path)
    manifests = [name for name in members if name.count("/") == 1 and name.endswith("/" + MANIFEST)]
    if len(manifests) != 1:
        raise DistributionError("Distribution requires one root source manifest")
    manifest_name = manifests[0]
    if len(members[manifest_name][0]) > MAX_MANIFEST:
        raise DistributionError("Source manifest exceeds its byte budget")
    manifest = json.loads(members[manifest_name][0])
    if not isinstance(manifest, dict) or manifest.get("format_version") != 1 or not isinstance(manifest.get("files"), list):
        raise DistributionError("Unsupported source manifest")
    prefix = manifest_name.split("/")[0]
    files = {}
    for entry in manifest["files"]:
        if not isinstance(entry, dict) or set(entry) != {"path", "sha256", "bytes", "mode"} or type(entry["bytes"]) is not int or type(entry["mode"]) is not int:
            raise DistributionError("Invalid source manifest file entry")
        name = safe_path(entry["path"])
        if name in files or name == MANIFEST:
            raise DistributionError("Source manifest contains duplicate or reserved paths")
        member = members.get(prefix + "/" + name)
        if member is None or entry["bytes"] != len(member[0]) or entry["sha256"] != digest(member[0]) or entry["mode"] != member[1] or member[1] not in (0o644, 0o755):
            raise DistributionError(f"Distribution integrity mismatch: {name}")
        files[name] = member
    if set(members) != {prefix + "/" + name for name in files} | {manifest_name}:
        raise DistributionError("Distribution contains unlisted files")
    package = package_identity(files)
    if package != manifest["package"] or prefix != package["name"] + "-" + package["version"]:
        raise DistributionError("Distribution package identity mismatch")
    if extract is not None:
        extract = extract.absolute()
        if extract.exists() or extract.is_symlink():
            raise DistributionError("Extraction destination must not exist")
        extract.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="lazypencil-extract-", dir=extract.parent) as temporary:
            staging = Path(temporary) / "source"
            staging.mkdir()
            for name, (data, mode) in files.items():
                destination = staging / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                destination.chmod(mode)
            (staging / MANIFEST).write_bytes(members[manifest_name][0])
            # Reserve the destination exclusively. A concurrent extractor cannot
            # replace an existing empty directory on POSIX through rename().
            extract.mkdir(mode=0o700)
            for child in staging.iterdir():
                child.rename(extract / child.name)
    return {"package": package, "files": len(files), "sha256": digest(path.read_bytes()), "git": manifest.get("git")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("source", help="Create complete source, retaining local patches")
    create.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    create.add_argument("--output", type=Path, required=True)
    create.add_argument("--epoch", type=int, default=os.environ.get("SOURCE_DATE_EPOCH", "0"))
    create.add_argument("--allow-dirty", action="store_true")
    check = commands.add_parser("verify-source", help="Verify members, hashes, and patched dependencies before optional extraction")
    check.add_argument("archive", type=Path)
    check.add_argument("--extract", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "source":
            if args.epoch < 0 or args.epoch > 0xFFFFFFFF:
                raise DistributionError("Archive epoch must be within 0..4294967295")
            result = source(args.root, args.output, args.epoch, args.allow_dirty)
        else:
            result = verify(args.archive, args.extract)
        print(json.dumps(result, ensure_ascii=False))
    except (DistributionError, OSError, ValueError, KeyError, TypeError, tarfile.TarError, EOFError, zlib.error, RecursionError) as error:
        parser.exit(1, f"Distribution error: {error}\n")


if __name__ == "__main__":
    main()
