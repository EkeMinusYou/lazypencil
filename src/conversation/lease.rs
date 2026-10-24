//! Connection ownership that expires even when the host cannot run destructors.

use crate::{Error, Result};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const PREFIX: &str = "lazypencil-connection-v1\n";

pub(crate) struct Lease {
    file: File,
    path: PathBuf,
    id: String,
}

impl Lease {
    pub(crate) fn create(manifest: &Path) -> Result<Self> {
        let path = owner_path(manifest);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|error| Error::io(&path, error))?;
        let mut lease = Self {
            id: uuid::Uuid::new_v4().to_string(),
            path,
            file,
        };
        FileExt::try_lock_exclusive(&lease.file).map_err(|error| Error::io(&lease.path, error))?;
        lease
            .file
            .write_all(content(&lease.id).as_bytes())
            .and_then(|()| lease.file.sync_all())
            .map_err(|error| Error::io(&lease.path, error))?;
        Ok(lease)
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        let _ = remove_owned(&self.path, &self.file);
    }
}

pub(crate) fn require_live(manifest: &Path, id: Option<&str>) -> Result<()> {
    let id = id.ok_or_else(expired)?;
    let file = read_owner(manifest, id).map_err(|_| expired())?;
    match FileExt::try_lock_shared(&file) {
        Err(error) if crate::store::lock_contended(&error) => Ok(()),
        _ => Err(expired()),
    }
}

/// Only a shared probe is held during cleanup. Another reader must still see
/// an expired owner, rather than mistake the cleanup lock for a living host.
pub(crate) fn remove_stale(manifest: &Path, id: &str) -> Result<bool> {
    require_id(id)?;
    let path = owner_path(manifest);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            remove_regular_manifest(manifest)?;
            return Ok(true);
        }
        Err(error) => return Err(Error::io(&path, error)),
        Ok(metadata) if !metadata.is_file() => return Err(expired()),
        Ok(_) => {}
    }
    let file = read_owner(manifest, id)?;
    match FileExt::try_lock_shared(&file) {
        Err(error) if crate::store::lock_contended(&error) => return Ok(false),
        Err(error) => return Err(Error::io(&path, error)),
        Ok(()) => {}
    }
    remove_regular_manifest(manifest)?;
    remove_owned(&path, &file)?;
    Ok(true)
}

fn read_owner(manifest: &Path, id: &str) -> Result<File> {
    require_id(id)?;
    let (file, bytes) = open_owner(manifest)?;
    if bytes != content(id).as_bytes() {
        return Err(expired());
    }
    Ok(file)
}

pub(crate) fn remove_orphan(manifest: &Path) -> Result<bool> {
    if !matches!(fs::symlink_metadata(manifest), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(false);
    }
    let (file, bytes) = open_owner(manifest)?;
    let id = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| text.strip_prefix(PREFIX))
        .and_then(|text| text.strip_suffix('\n'))
        .ok_or_else(expired)?;
    require_id(id)?;
    match FileExt::try_lock_shared(&file) {
        Err(error) if crate::store::lock_contended(&error) => return Ok(false),
        Err(error) => return Err(Error::io(&owner_path(manifest), error)),
        Ok(()) => {}
    }
    if !matches!(fs::symlink_metadata(manifest), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(false);
    }
    remove_owned(&owner_path(manifest), &file)?;
    Ok(true)
}

fn require_id(id: &str) -> Result<()> {
    if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id) {
        Ok(())
    } else {
        Err(expired())
    }
}

fn open_owner(manifest: &Path) -> Result<(File, Vec<u8>)> {
    let path = owner_path(manifest);
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| Error::io(&path, error))?;
    if !file
        .metadata()
        .map_err(|error| Error::io(&path, error))?
        .is_file()
    {
        return Err(expired());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(128)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(&path, error))?;
    Ok((file, bytes))
}

fn owner_path(manifest: &Path) -> PathBuf {
    manifest.with_extension("owner")
}

fn content(id: &str) -> String {
    format!("{PREFIX}{id}\n")
}

fn remove_regular_manifest(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            fs::remove_file(path).map_err(|error| Error::io(path, error))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(expired()),
        Err(error) => Err(Error::io(path, error)),
    }
}

fn remove_owned(path: &Path, file: &File) -> Result<()> {
    let owned = file.metadata().map_err(|error| Error::io(path, error))?;
    match fs::symlink_metadata(path) {
        Ok(current)
            if current.is_file()
                && current.dev() == owned.dev()
                && current.ino() == owned.ino() =>
        {
            fs::remove_file(path).map_err(|error| Error::io(path, error))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(expired()),
        Err(error) => Err(Error::io(path, error)),
    }
}

fn expired() -> Error {
    Error::new(
        "AGENT_ERROR",
        "The conversation connection is no longer active; reconnect to issue a new grant",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn only_the_matching_living_owner_authorizes_access_and_cleanup_cannot_revive_it() {
        let temp = TempDir::new().unwrap();
        let manifest = temp.path().join("grant.json");
        let lease = Lease::create(&manifest).unwrap();
        fs::write(&manifest, b"saved-manifest").unwrap();
        require_live(&manifest, Some(lease.id())).unwrap();
        assert!(require_live(&manifest, None).is_err());
        assert!(require_live(&manifest, Some(&uuid::Uuid::new_v4().to_string())).is_err());
        assert!(!remove_stale(&manifest, lease.id()).unwrap());
        // A killed process releases this same kernel lock without removing files.
        FileExt::unlock(&lease.file).unwrap();
        assert!(require_live(&manifest, Some(lease.id())).is_err());
        let probe = read_owner(&manifest, lease.id()).unwrap();
        FileExt::try_lock_shared(&probe).unwrap();
        assert!(require_live(&manifest, Some(lease.id())).is_err());
        assert!(remove_stale(&manifest, lease.id()).unwrap());
        assert!(!manifest.exists());
        assert!(!owner_path(&manifest).exists());
    }

    #[test]
    fn owner_files_cannot_be_symlinks_or_blocking_pipes_and_replacements_survive_drop() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let temp = TempDir::new().unwrap();
        let manifest = temp.path().join("grant.json");
        let lease = Lease::create(&manifest).unwrap();
        let path = owner_path(&manifest);
        let outside = temp.path().join("unrelated");
        let preserved = content(lease.id());
        fs::write(&outside, preserved.as_bytes()).unwrap();
        let outside_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&outside)
            .unwrap();
        FileExt::try_lock_exclusive(&outside_file).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(require_live(&manifest, Some(lease.id())).is_err());
        assert!(remove_stale(&manifest, lease.id()).is_err());
        fs::remove_file(&path).unwrap();
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: The owned C string names a fresh path inside this test directory.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let probe_path = manifest.clone();
        let probe_id = lease.id().to_string();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(require_live(&probe_path, Some(&probe_id)).is_err());
        });
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("Owner pipe must not block")
        );
        assert!(remove_stale(&manifest, lease.id()).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();
        drop(lease);
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(fs::read(&outside).unwrap(), preserved.as_bytes());
    }

    #[test]
    fn incomplete_publication_only_removes_an_unowned_recognized_owner_file() {
        let temp = TempDir::new().unwrap();
        let manifest = temp.path().join("grant.json");
        let lease = Lease::create(&manifest).unwrap();
        assert!(!remove_orphan(&manifest).unwrap());
        FileExt::unlock(&lease.file).unwrap();
        assert!(remove_orphan(&manifest).unwrap());
        fs::write(owner_path(&manifest), b"unrecognized-data").unwrap();
        assert!(remove_orphan(&manifest).is_err());
        assert_eq!(
            fs::read(owner_path(&manifest)).unwrap(),
            b"unrecognized-data"
        );
        fs::write(&manifest, b"preserved").unwrap();
        assert!(remove_stale(&manifest, "invalid").is_err());
        assert_eq!(fs::read(manifest).unwrap(), b"preserved");
    }
}
