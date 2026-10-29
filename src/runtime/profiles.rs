//! Explicitly created Chromium profiles, separate from ordinary browser data.

use crate::{Error, Result};
#[cfg(unix)]
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
};

#[cfg(unix)]
const MARKER: &str = "lazypencil-profile.json";

#[cfg(all(unix, feature = "rendering"))]
pub(crate) mod pages;

#[cfg(all(unix, feature = "rendering"))]
pub fn serve_page_worker() -> Result<()> {
    pages::serve_worker()
}

#[derive(Clone)]
pub struct Profile {
    directory: PathBuf,
    identity: String,
}

#[cfg(unix)]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: u32,
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_path_hash: Option<String>,
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

#[cfg(unix)]
fn failure(stage: &str) -> Error {
    Error::new(
        "RUNTIME_UNAVAILABLE",
        "Dedicated browser profile is unavailable",
    )
    .details(serde_json::json!({"stage":stage}))
}

#[cfg(unix)]
fn private_metadata(path: &Path, directory: bool) -> Result<fs::Metadata> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = fs::symlink_metadata(path).map_err(|_| failure("profile_metadata"))?;
    // SAFETY: geteuid has no arguments or memory-access requirements.
    let owned = metadata.uid() == unsafe { libc::geteuid() };
    if metadata.file_type().is_symlink()
        || metadata.is_dir() != directory
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
        || metadata.permissions().mode() & 0o077 != 0
        || !owned
    {
        return Err(failure("profile_permissions"));
    }
    Ok(metadata)
}

#[cfg(unix)]
fn private_file(path: &Path, create: bool) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            private_metadata(path, false)?;
        }
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(failure("profile_file")),
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let file = options.open(path).map_err(|_| failure("profile_file"))?;
    private_metadata(path, false)?;
    if !file
        .metadata()
        .map_err(|_| failure("profile_file"))?
        .is_file()
    {
        return Err(failure("profile_file"));
    }
    Ok(file)
}

#[cfg(unix)]
fn read_identity(directory: &Path) -> Result<Identity> {
    let mut bytes = Vec::new();
    private_file(&directory.join(MARKER), false)?
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| failure("profile_read_marker"))?;
    if bytes.len() > 4096 {
        return Err(failure("profile_marker"));
    }
    let identity: Identity =
        serde_json::from_slice(&bytes).map_err(|_| failure("profile_marker"))?;
    if identity.version != 1
        || uuid::Uuid::parse_str(&identity.id).is_err()
        || identity.browser_path_hash.as_ref().is_some_and(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(failure("profile_marker"));
    }
    Ok(identity)
}

impl Profile {
    #[cfg(unix)]
    pub fn create(data: &Path, name: &str) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if !valid_name(name) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Profile names need 1..64 lowercase ASCII letters, digits or hyphens, starting with a letter or digit",
            ));
        }
        fs::create_dir_all(data).map_err(|_| failure("profile_data_directory"))?;
        let data = crate::paths::canonical(data)?;
        let root = data.join("browser-profiles");
        if let Err(error) = fs::DirBuilder::new().mode(0o700).create(&root)
            && error.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(failure("profile_root"));
        }
        private_metadata(&root, true)?;
        let directory = root.join(name);
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Self::open(data.as_path(), name);
            }
            Err(_) => return Err(failure("profile_create")),
        }
        let identity = Identity {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            browser_path_hash: None,
        };
        // Never recursively delete a caller-provided path or adopt an existing
        // unmarked directory. A partial initialization remains diagnosable.
        let mut marker = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(directory.join(MARKER))
            .map_err(|_| failure("profile_create_marker"))?;
        marker
            .write_all(&serde_json::to_vec(&identity).expect("profile identity"))
            .and_then(|()| marker.sync_all())
            .map_err(|_| failure("profile_create_marker"))?;
        File::open(&directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| failure("profile_create_marker"))?;
        Self::from_directory(&directory)
    }

    #[cfg(unix)]
    pub fn open(data: &Path, name: &str) -> Result<Self> {
        if !valid_name(name) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid dedicated browser profile name",
            ));
        }
        let root = crate::paths::canonical(data)?.join("browser-profiles");
        private_metadata(&root, true)?;
        Self::from_directory(&root.join(name))
    }

    #[cfg(unix)]
    pub(crate) fn from_directory(directory: &Path) -> Result<Self> {
        private_metadata(directory, true)?;
        let directory = crate::paths::canonical(directory)?;
        let identity = read_identity(&directory)?;
        Ok(Self {
            directory,
            identity: identity.id,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    #[cfg(unix)]
    pub(crate) fn lease(&self) -> Result<Lease> {
        let current = Self::from_directory(&self.directory)?;
        if current.identity != self.identity {
            return Err(failure("profile_identity"));
        }
        let lock = private_file(&self.directory.join("lazypencil-profile.lock"), true)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            Error::new(
                "RUNTIME_UNAVAILABLE",
                "Dedicated browser profile cannot be locked; close its other browser session",
            )
            .details(serde_json::json!({"stage":"profile_lock","os_error":error.raw_os_error()}))
        })?;
        // The guardian retains this file until every owned browser child has
        // exited, including after abrupt host termination.
        Ok(Lease { _lock: lock })
    }

    #[cfg(unix)]
    pub(crate) fn lease_for_browser(&self, executable: &Path) -> Result<Lease> {
        use std::os::unix::ffi::OsStrExt;
        let lease = self.lease()?;
        let executable = crate::paths::canonical(executable)?;
        let expected = crate::document::hash(executable.as_os_str().as_bytes());
        let mut identity = read_identity(&self.directory)?;
        if identity.id != self.identity {
            return Err(failure("profile_identity"));
        }
        match &identity.browser_path_hash {
            Some(actual) if actual == &expected => return Ok(lease),
            Some(_) => return Err(failure("profile_browser_identity")),
            None => identity.browser_path_hash = Some(expected),
        }
        // Bind before the browser opens its persistent stores, under the same
        // lease. Full Chrome and headless shell need not share cookie encryption
        // or profile formats even when their Chromium major versions match.
        let mut marker = tempfile::NamedTempFile::new_in(&self.directory)
            .map_err(|_| failure("profile_bind_browser"))?;
        marker
            .write_all(&serde_json::to_vec(&identity).expect("profile identity"))
            .and_then(|()| marker.as_file().sync_all())
            .map_err(|_| failure("profile_bind_browser"))?;
        marker
            .persist(self.directory.join(MARKER))
            .map_err(|_| failure("profile_bind_browser"))?;
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| failure("profile_bind_browser"))?;
        Ok(lease)
    }

    #[cfg(not(unix))]
    pub fn create(_data: &Path, _name: &str) -> Result<Self> {
        Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "Dedicated browser profiles require a supported Unix host",
        ))
    }

    #[cfg(not(unix))]
    pub fn open(_data: &Path, _name: &str) -> Result<Self> {
        Self::create(_data, _name)
    }
}

#[cfg(unix)]
pub(crate) struct Lease {
    _lock: File,
}

pub fn login(
    data: &Path,
    name: &str,
    executable: &Path,
    url: &str,
    timeout_seconds: u64,
    headless: bool,
) -> Result<()> {
    if !(1..=3600).contains(&timeout_seconds) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Login timeout must be within 1..3600 seconds",
        ));
    }
    let parsed = url::Url::parse(url)
        .map_err(|_| Error::new("INVALID_ARGUMENT", "Login needs an HTTP(S) URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(Error::new("INVALID_ARGUMENT", "Login needs an HTTP(S) URL"));
    }
    let profile = Profile::open(data, name)?;
    let executable = crate::paths::canonical(executable)?;
    if !executable.is_file() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Login browser must be an executable file",
        ));
    }
    let acquisition = super::installed(data)?;
    if acquisition.executable != executable {
        return Err(Error::new(
            "RUNTIME_UNAVAILABLE",
            "Login and acquisition must use the same browser executable; set LAZYPENCIL_CHROMIUM to the full browser used for login",
        ).details(serde_json::json!({"stage":"profile_browser_identity"})));
    }
    if !headless && acquisition.version != "external-unverified" {
        return Err(Error::new(
            "RUNTIME_UNAVAILABLE",
            "Visible login needs a full Chrome/Chromium browser; configure LAZYPENCIL_CHROMIUM and --browser to use that same executable",
        ).details(serde_json::json!({"stage":"profile_login_browser"})));
    }
    let installed = super::InstalledBrowser {
        executable,
        version: "external-unverified".into(),
        platform: super::platform()?.into(),
    };
    let cancel = crate::cancel::token();
    let budget = crate::cdp::Budget::new(
        std::time::Duration::from_secs(timeout_seconds),
        Some(&cancel),
    );
    let launch = crate::cdp::Budget::new(
        budget.remaining()?.min(std::time::Duration::from_secs(30)),
        Some(&cancel),
    );
    let mut browser =
        crate::cdp::Browser::launch_profile(&installed, &profile, &launch, None, !headless)?;
    browser.login(url, &budget)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};

    #[test]
    fn dedicated_profiles_are_explicit_private_and_exclusively_leased() {
        let temp = tempfile::tempdir().unwrap();
        let profile = Profile::create(temp.path(), "work-1").unwrap();
        assert_eq!(
            Profile::create(temp.path(), "work-1").unwrap().identity,
            profile.identity
        );
        let lease = profile.lease().unwrap();
        assert!(
            Profile::open(temp.path(), "work-1")
                .unwrap()
                .lease()
                .is_err()
        );
        drop(lease);
        profile.lease().unwrap();
        let browser = temp.path().join("browser");
        fs::write(&browser, b"first browser fixture").unwrap();
        drop(profile.lease_for_browser(&browser).unwrap());
        let marker = fs::read(profile.directory().join(MARKER)).unwrap();
        let alias = temp.path().join("browser-alias");
        symlink(&browser, &alias).unwrap();
        drop(profile.lease_for_browser(&alias).unwrap());
        let other = temp.path().join("other-browser");
        fs::write(&other, b"different browser fixture").unwrap();
        assert!(profile.lease_for_browser(&other).is_err());
        assert_eq!(fs::read(profile.directory().join(MARKER)).unwrap(), marker);
        drop(
            Profile::open(temp.path(), "work-1")
                .unwrap()
                .lease_for_browser(&browser)
                .unwrap(),
        );
        for name in ["", "..", "../work", "Work", "/work", "-work", "a/b"] {
            assert!(Profile::create(temp.path(), name).is_err());
        }
        let root = temp.path().join("browser-profiles");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join("unowned"))
            .unwrap();
        assert!(Profile::create(temp.path(), "unowned").is_err());
        symlink(profile.directory(), root.join("linked")).unwrap();
        assert!(Profile::open(temp.path(), "linked").is_err());
        fs::set_permissions(profile.directory(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Profile::open(temp.path(), "work-1").is_err());
    }

    #[test]
    fn profile_markers_and_locks_reject_aliases_foreign_files_and_replaced_identities() {
        let temp = tempfile::tempdir().unwrap();
        let profile = Profile::create(temp.path(), "work").unwrap();
        let outside = temp.path().join("outside-secret");
        fs::write(&outside, b"outside contents").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        let lock = profile.directory().join("lazypencil-profile.lock");
        symlink(&outside, &lock).unwrap();
        assert!(profile.lease().is_err());
        fs::remove_file(&lock).unwrap();
        fs::hard_link(&outside, &lock).unwrap();
        assert!(profile.lease().is_err());
        fs::remove_file(&lock).unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"outside contents");
        let marker = profile.directory().join(MARKER);
        fs::write(
            &marker,
            serde_json::to_vec(&Identity {
                version: 1,
                id: uuid::Uuid::new_v4().to_string(),
                browser_path_hash: None,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(profile.lease().is_err());
        Profile::open(temp.path(), "work").unwrap().lease().unwrap();
        fs::write(&marker, b"not a profile marker").unwrap();
        assert!(Profile::open(temp.path(), "work").is_err());
    }
}
