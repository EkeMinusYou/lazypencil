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

mod diagnostics;
pub(crate) use diagnostics::host_status;

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
    verify(&directory, artifact)?;
    Ok(InstalledBrowser {
        executable: directory.join(&artifact.executable),
        version: manifest.version,
        platform: platform.into(),
    })
}

/// A local archive is accepted only when it matches the same pinned official hash.
pub fn setup(data: &Path, archive: Option<&Path>) -> Result<Value> {
    let manifest = manifest()?;
    let platform = platform()?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == platform)
        .ok_or_else(|| unavailable("No pinned artifact for this platform"))?;
    let directory = directory(data, &manifest.version, platform);
    let parent = directory.parent().expect("runtime parent");
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
    crate::store::wait_for_file_lock(&lock, &lock_path, false, None, Duration::from_secs(60))?;
    if directory.exists() {
        verify(&directory, artifact)?;
        return Ok(
            json!({"version":manifest.version,"platform":platform,"executable":directory.join(&artifact.executable),"reused":true}),
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
        std::io::copy(&mut file.take(artifact.bytes + 1), &mut temporary)
            .map_err(|e| Error::io(path, e))?;
    } else {
        eprintln!("Downloading Chromium {} for {platform}", manifest.version);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(180)))
            .build()
            .into();
        let mut response = agent
            .get(&artifact.url)
            .call()
            .map_err(|e| unavailable(format!("Chromium download failed: {e}")))?;
        std::io::copy(
            &mut response.body_mut().as_reader().take(artifact.bytes + 1),
            &mut temporary,
        )
        .map_err(|e| unavailable(format!("Chromium download failed: {e}")))?;
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
        || digest(temporary.path(), artifact.bytes)? != artifact.sha256
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
        let copied = std::io::copy(&mut entry.take(pinned.bytes + 1), &mut target)
            .map_err(|e| Error::io(&path, e))?;
        if copied != pinned.bytes {
            return Err(integrity("Incomplete runtime entry"));
        }
        target.sync_all().map_err(|e| Error::io(&path, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if pinned.executable { 0o755 } else { 0o644 }),
            )
            .map_err(|e| Error::io(&path, e))?;
        }
    }
    verify(staging.path(), artifact)?;
    fs::rename(staging.path(), &directory).map_err(|e| Error::io(&directory, e))?;
    crate::store::sync_directory(parent)?;
    Ok(
        json!({"version":manifest.version,"platform":platform,"executable":directory.join(&artifact.executable),"reused":false,"source":artifact.url,"archive_sha256":artifact.sha256}),
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
fn verify(directory: &Path, artifact: &Artifact) -> Result<()> {
    if fs::symlink_metadata(directory)
        .map_err(|e| Error::io(directory, e))?
        .file_type()
        .is_symlink()
    {
        return Err(integrity("Runtime directory is a symbolic link"));
    }
    for (relative, pinned) in &artifact.files {
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
            || digest(&path, pinned.bytes)? != pinned.sha256
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
    verify_contents(directory, directory, &artifact.files)?;
    Ok(())
}

fn verify_contents(
    root: &Path,
    directory: &Path,
    files: &BTreeMap<String, PinnedFile>,
) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|e| Error::io(directory, e))? {
        let path = entry.map_err(|e| Error::io(directory, e))?.path();
        let metadata = fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
        if metadata.file_type().is_symlink() {
            return Err(integrity("Installed runtime contains a symbolic link"));
        }
        if metadata.is_dir() {
            verify_contents(root, &path, files)?;
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
    Ok(())
}
fn digest(path: &Path, expected_bytes: u64) -> Result<String> {
    let mut file = crate::store::open_regular_file(path)?.take(expected_bytes.saturating_add(1));
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut buffer).map_err(|e| Error::io(path, e))?;
        if count == 0 {
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
fn unavailable(message: impl Into<String>) -> Error {
    Error::new("RUNTIME_UNAVAILABLE", message)
}
fn integrity(message: impl Into<String>) -> Error {
    Error::new("RUNTIME_INTEGRITY", message)
}

#[cfg(test)]
mod tests {
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
