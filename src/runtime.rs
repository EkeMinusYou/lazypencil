//! Explicit installation and verification of version-pinned browser artifacts.

use crate::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

mod diagnostics;
pub(crate) mod download;
pub(crate) use diagnostics::host_status;
pub mod profiles;

#[derive(Deserialize)]
struct Manifest {
    version: String,
    artifacts: Vec<Artifact>,
}
#[derive(Deserialize)]
struct Artifact {
    platform: String,
    url: String,
    sha256: String,
    bytes: u64,
    executable: String,
    files: BTreeMap<String, PinnedFile>,
}
#[derive(Deserialize)]
struct PinnedFile {
    sha256: String,
    bytes: u64,
    #[cfg_attr(not(unix), allow(dead_code))]
    executable: bool,
}

pub struct InstalledBrowser {
    pub executable: PathBuf,
    pub version: String,
    pub platform: String,
}

fn manifest() -> Result<Manifest> {
    serde_json::from_str(include_str!("../resources/runtimes/chromium.json"))
        .map_err(|e| Error::new("INTERNAL_ERROR", format!("Invalid runtime manifest: {e}")))
}
pub fn platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("mac-arm64"),
        ("macos", "x86_64") => Ok("mac-x64"),
        ("linux", "x86_64") => Ok("linux64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("windows", "x86_64") => Ok("win64"),
        _ => Err(unavailable("This OS/CPU has no pinned Chromium artifact")),
    }
}
fn directory(data: &Path, version: &str, platform: &str) -> PathBuf {
    data.join("runtimes")
        .join("chromium")
        .join(version)
        .join(platform)
}
pub fn installed(data: &Path) -> Result<InstalledBrowser> {
    if let Some(path) = std::env::var_os("LAZYPENCIL_CHROMIUM") {
        let executable = crate::paths::canonical(Path::new(&path))?;
        if !executable.is_file() {
            return Err(unavailable("Configured Chromium executable is not a file"));
        }
        return Ok(InstalledBrowser {
            executable,
            version: "external-unverified".into(),
            platform: platform()?.into(),
        });
    }
    let manifest = manifest()?;
    let platform = platform()?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == platform)
        .ok_or_else(|| unavailable("No pinned artifact for this platform"))?;
    let directory = directory(data, &manifest.version, platform);
    if !directory.is_dir() {
        return Err(unavailable(
            "Chromium is not installed; run lazypencil runtime setup chromium",
        ));
    }
    verify(&directory, artifact, None, false)?;
    Ok(InstalledBrowser {
        executable: directory.join(&artifact.executable),
        version: manifest.version,
        platform: platform.into(),
    })
}

/// A local archive is accepted only when it matches the same pinned official hash.
pub fn setup(data: &Path, archive: Option<&Path>) -> Result<Value> {
    setup_with_cancellation(data, archive, None, None)
}

/// A cancellable network installation needs the real application executable,
/// so embedded hosts never launch their own binary as an internal worker.
pub fn setup_with_cancellation(
    data: &Path,
    archive: Option<&Path>,
    cancel: Option<&CancellationToken>,
    executable: Option<&Path>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    let manifest = manifest()?;
    let platform = platform()?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == platform)
        .ok_or_else(|| unavailable("No pinned artifact for this platform"))?;
    install(
        data,
        &manifest.version,
        artifact,
        archive,
        cancel,
        executable,
    )
}

fn install(
    data: &Path,
    version: &str,
    artifact: &Artifact,
    archive: Option<&Path>,
    cancel: Option<&CancellationToken>,
    executable: Option<&Path>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    let platform = artifact.platform.as_str();
    let directory = directory(data, version, platform);
    let parent = directory.parent().expect("runtime parent");
    let mut publication_parents = Vec::new();
    for path in parent.ancestors() {
        let path = if path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            path
        };
        publication_parents.push(path.to_owned());
        if path.is_dir() {
            break;
        }
    }
    crate::store::create_private_directory(parent)?;
    let lock_path = parent.join(format!(".{platform}.lock"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| Error::io(&lock_path, e))?;
    // Match the existing icon-installer contention budget; downloads retain
    // their separate 180-second limit. Do not wait forever for another setup.
    crate::store::wait_for_file_lock(&lock, &lock_path, false, cancel, Duration::from_secs(60))?;
    crate::cancel::check(cancel)?;
    if directory.exists() {
        verify(&directory, artifact, cancel, false)?;
        crate::store::sync_directory(parent)?;
        return Ok(
            json!({"version":version,"platform":platform,"executable":directory.join(&artifact.executable),"reused":true}),
        );
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| Error::io(parent, e))?;
    if let Some(path) = archive {
        let file = crate::store::open_regular_file(path).map_err(|error| {
            if error.code == "INVALID_DOCUMENT" {
                integrity("Runtime archive must be a regular file")
            } else {
                error
            }
        })?;
        copy_checked(file, &mut temporary, path, artifact.bytes, cancel)?;
    } else {
        eprintln!("Downloading Chromium {version} for {platform}");
        if let Some(executable) = executable {
            download::run(executable, &temporary, artifact.bytes, cancel)?;
        } else {
            if cancel.is_some() {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Cancellable runtime downloads require the lazypencil executable",
                ));
            }
            download::direct(artifact, &mut temporary)?;
        }
    }
    temporary
        .flush()
        .map_err(|e| Error::io(temporary.path(), e))?;
    if temporary
        .as_file()
        .metadata()
        .map_err(|e| Error::io(temporary.path(), e))?
        .len()
        != artifact.bytes
        || digest_with_cancellation(temporary.path(), artifact.bytes, cancel)? != artifact.sha256
    {
        return Err(integrity(
            "Chromium archive does not match its pinned SHA-256 and size",
        ));
    }
    let staging = tempfile::Builder::new()
        .prefix(".chromium-install-")
        .tempdir_in(parent)
        .map_err(|e| Error::io(parent, e))?;
    let mut archive = zip::ZipArchive::new(
        temporary
            .reopen()
            .map_err(|e| Error::io(temporary.path(), e))?,
    )
    .map_err(|e| integrity(format!("Cannot read runtime archive: {e}")))?;
    if archive.len() > 4096 {
        return Err(integrity("Runtime archive has too many entries"));
    }
    for index in 0..archive.len() {
        crate::cancel::check(cancel)?;
        let entry = archive
            .by_index(index)
            .map_err(|e| integrity(format!("Invalid runtime entry: {e}")))?;
        let name = entry
            .name()
            .map_err(|_| integrity("Runtime entry name is not valid UTF-8"))?
            .into_owned();
        let relative = Path::new(&name);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || name.contains(['\\', ':'])
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(integrity("Unsafe runtime archive path or symbolic link"));
        }
        let path = staging.path().join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(|e| Error::io(&path, e))?;
            continue;
        }
        let pinned = artifact
            .files
            .get(&name)
            .ok_or_else(|| integrity("Runtime archive contains an unpinned file"))?;
        if entry.size() != pinned.bytes {
            return Err(integrity("Runtime entry size differs from its pin"));
        }
        fs::create_dir_all(path.parent().expect("file parent")).map_err(|e| Error::io(&path, e))?;
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::io(&path, e))?;
        let copied = copy_checked(entry, &mut target, &path, pinned.bytes, cancel)?;
        if copied != pinned.bytes {
            return Err(integrity("Incomplete runtime entry"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if pinned.executable { 0o755 } else { 0o644 }),
            )
            .map_err(|e| Error::io(&path, e))?;
        }
        target.sync_all().map_err(|e| Error::io(&path, e))?;
    }
    // Persist all file data, permissions and nested directory entries before
    // exposing the verified installation through its final directory name.
    verify(staging.path(), artifact, cancel, true)?;
    crate::cancel::check(cancel)?;
    fs::rename(staging.path(), &directory).map_err(|e| Error::io(&directory, e))?;
    // Once renamed, finish the commit even if cancellation arrives. Sync the
    // newly created ancestor entries too, including the first existing parent.
    for path in publication_parents {
        crate::store::sync_directory(&path)?;
    }
    Ok(
        json!({"version":version,"platform":platform,"executable":directory.join(&artifact.executable),"reused":false,"source":artifact.url,"archive_sha256":artifact.sha256}),
    )
}

pub fn status(data: &Path) -> Value {
    let mut report = match installed(data) {
        Ok(browser) => {
            json!({"available":true,"managed":browser.version!="external-unverified","integrity_verified":browser.version!="external-unverified","version":browser.version,"platform":browser.platform,"executable":browser.executable})
        }
        Err(error) => json!({"available":false,"error":error}),
    };
    report["startup"] = json!("not_checked");
    report["host"] = host_status();
    report
}
fn verify(
    directory: &Path,
    artifact: &Artifact,
    cancel: Option<&CancellationToken>,
    durable: bool,
) -> Result<()> {
    crate::cancel::check(cancel)?;
    if fs::symlink_metadata(directory)
        .map_err(|e| Error::io(directory, e))?
        .file_type()
        .is_symlink()
    {
        return Err(integrity("Runtime directory is a symbolic link"));
    }
    for (relative, pinned) in &artifact.files {
        crate::cancel::check(cancel)?;
        let path = directory.join(relative);
        let mut parent = path.parent();
        while let Some(p) = parent.filter(|p| *p != directory) {
            let metadata = fs::symlink_metadata(p).map_err(|e| Error::io(p, e))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(integrity("Runtime parent is not a real directory"));
            }
            parent = p.parent();
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != pinned.bytes
            || digest_with_cancellation(&path, pinned.bytes, cancel)? != pinned.sha256
        {
            return Err(integrity(format!(
                "Installed runtime file differs from its pin: {relative}"
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if pinned.executable && metadata.permissions().mode() & 0o111 == 0 {
                return Err(integrity("Runtime executable has no execute permission"));
            }
        }
    }
    verify_contents(directory, directory, &artifact.files, cancel, durable)?;
    Ok(())
}

fn verify_contents(
    root: &Path,
    directory: &Path,
    files: &BTreeMap<String, PinnedFile>,
    cancel: Option<&CancellationToken>,
    durable: bool,
) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|e| Error::io(directory, e))? {
        crate::cancel::check(cancel)?;
        let path = entry.map_err(|e| Error::io(directory, e))?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
        if metadata.file_type().is_symlink() {
            return Err(integrity("Installed runtime contains a symbolic link"));
        }
        if metadata.is_dir() {
            verify_contents(root, &path, files, cancel, durable)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("walked child")
                .components()
                .map(|part| {
                    part.as_os_str()
                        .to_str()
                        .ok_or_else(|| integrity("Runtime file name is not UTF-8"))
                })
                .collect::<Result<Vec<_>>>()?
                .join("/");
            if !metadata.is_file() || !files.contains_key(&relative) {
                return Err(integrity("Installed runtime contains an unpinned file"));
            }
        }
    }
    crate::cancel::check(cancel)?;
    if durable {
        crate::store::sync_directory(directory)?;
    }
    Ok(())
}
#[cfg(test)]
fn digest(path: &Path, expected_bytes: u64) -> Result<String> {
    digest_with_cancellation(path, expected_bytes, None)
}
fn digest_with_cancellation(
    path: &Path,
    expected_bytes: u64,
    cancel: Option<&CancellationToken>,
) -> Result<String> {
    let mut file = crate::store::open_regular_file(path)?.take(expected_bytes.saturating_add(1));
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0;
    loop {
        crate::cancel::check(cancel)?;
        let count = match file.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::io(path, error)),
        };
        if count == 0 {
            crate::cancel::check(cancel)?;
            break;
        }
        bytes += count as u64;
        digest.update(&buffer[..count]);
    }
    if bytes != expected_bytes {
        return Err(integrity("Runtime file size changed while hashing"));
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn copy_checked(
    reader: impl Read,
    writer: &mut impl Write,
    path: &Path,
    expected_bytes: u64,
    cancel: Option<&CancellationToken>,
) -> Result<u64> {
    let mut reader = reader.take(expected_bytes.saturating_add(1));
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        crate::cancel::check(cancel)?;
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::io(path, error)),
        };
        crate::cancel::check(cancel)?;
        if count == 0 {
            return Ok(total);
        }
        total += count as u64;
        if total > expected_bytes {
            return Err(integrity("Runtime input exceeds its pinned size"));
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|error| Error::io(path, error))?;
    }
}

#[doc(hidden)]
pub fn serve_download_worker() -> Result<()> {
    if std::env::args_os().nth(2).is_some() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "The runtime download worker takes no arguments",
        ));
    }
    download::watch_owner()?;
    let manifest = manifest()?;
    let platform = platform()?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == platform)
        .ok_or_else(|| unavailable("No pinned artifact for this platform"))?;
    download::direct(artifact, &mut std::io::stdout().lock())
}
fn unavailable(message: impl Into<String>) -> Error {
    Error::new("RUNTIME_UNAVAILABLE", message)
}
fn integrity(message: impl Into<String>) -> Error {
    Error::new("RUNTIME_INTEGRITY", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_activates_only_a_complete_pinned_tree_and_retains_modified_data() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("archive.zip");
        let bytes = b"fixture browser";
        let mut zip = zip::ZipWriter::new(fs::File::create(&source).unwrap());
        zip.start_file(
            "app/nested/browser",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
        zip.finish().unwrap();
        let size = fs::metadata(&source).unwrap().len();
        let mut artifact = Artifact {
            platform: "fixture".into(),
            url: "https://invalid.example/archive.zip".into(),
            executable: "app/nested/browser".into(),
            sha256: digest(&source, size).unwrap(),
            bytes: size,
            files: BTreeMap::from([(
                "app/nested/browser".into(),
                PinnedFile {
                    sha256: crate::document::hash(bytes),
                    bytes: bytes.len() as u64,
                    executable: true,
                },
            )]),
        };
        let data = temp.path().join("fresh/data");
        artifact.files.get_mut("app/nested/browser").unwrap().sha256 = "0".repeat(64);
        assert_eq!(
            install(&data, "test", &artifact, Some(&source), None, None)
                .unwrap_err()
                .code,
            "RUNTIME_INTEGRITY"
        );
        let directory = directory(&data, "test", "fixture");
        assert!(!directory.exists());
        let parent = directory.parent().unwrap();
        assert_eq!(
            fs::read_dir(parent).unwrap().count(),
            1,
            "Only the installer lock remains after failure"
        );
        artifact.files.get_mut("app/nested/browser").unwrap().sha256 = crate::document::hash(bytes);
        assert_eq!(
            install(&data, "test", &artifact, Some(&source), None, None).unwrap()["reused"],
            false
        );
        assert_eq!(
            fs::read(directory.join(&artifact.executable)).unwrap(),
            bytes
        );
        assert_eq!(
            install(&data, "test", &artifact, Some(&source), None, None).unwrap()["reused"],
            true
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                fs::metadata(directory.join(&artifact.executable))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
        fs::write(directory.join(&artifact.executable), b"modified").unwrap();
        assert_eq!(
            install(&data, "test", &artifact, Some(&source), None, None)
                .unwrap_err()
                .code,
            "RUNTIME_INTEGRITY"
        );
        assert_eq!(
            fs::read(directory.join(&artifact.executable)).unwrap(),
            b"modified"
        );
    }

    #[test]
    fn cancellation_stops_a_transfer_before_writing_the_next_chunk() {
        struct Cancelling {
            token: CancellationToken,
        }
        impl Read for Cancelling {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                bytes.fill(7);
                self.token.cancel();
                Ok(bytes.len())
            }
        }
        let token = CancellationToken::new();
        let mut output = Vec::new();
        assert_eq!(
            copy_checked(
                Cancelling {
                    token: token.clone()
                },
                &mut output,
                Path::new("fixture"),
                1024 * 1024,
                Some(&token)
            )
            .unwrap_err()
            .code,
            "CANCELLED"
        );
        assert!(output.is_empty());
        assert_eq!(
            copy_checked(
                std::io::Cursor::new(b"too large"),
                &mut output,
                Path::new("fixture"),
                3,
                None
            )
            .unwrap_err()
            .code,
            "RUNTIME_INTEGRITY"
        );
        assert!(output.is_empty());
    }
    #[test]
    fn runtime_installation_lock_preserves_holder_and_observes_exhausted_budget() {
        let holder = tempfile::NamedTempFile::new().unwrap();
        let waiter = holder.reopen().unwrap();
        fs2::FileExt::lock_exclusive(holder.as_file()).unwrap();
        let acquire = || {
            crate::store::wait_for_file_lock(
                &waiter,
                holder.path(),
                false,
                None,
                std::time::Duration::ZERO,
            )
        };
        assert_eq!(acquire().unwrap_err().code, "LOCK_UNAVAILABLE");
        assert!(fs2::FileExt::try_lock_exclusive(&waiter).is_err());
        fs2::FileExt::unlock(holder.as_file()).unwrap();
        acquire().unwrap();
    }

    #[test]
    fn runtime_hash_requires_the_exact_pinned_length() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"abc").unwrap();
        assert_eq!(
            super::digest(file.path(), 3).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        for expected in [0, 2, 4] {
            assert_eq!(
                super::digest(file.path(), expected).unwrap_err().code,
                "RUNTIME_INTEGRITY"
            );
        }
    }
}
