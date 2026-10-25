//! Read canonical paths through held, non-following directory descriptors.

use crate::{Error, Result};
use serde::Serialize;
use std::path::Path;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) enum Kind {
    File,
    Directory,
    Link,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Entry {
    pub name: String,
    pub kind: Kind,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod native {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        fs::File,
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };

    fn opened(fd: libc::c_int, path: &Path) -> Result<File> {
        if fd < 0 {
            Err(Error::io(path, std::io::Error::last_os_error()))
        } else {
            // The successful open returned a fresh, uniquely owned descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    fn name(value: &std::ffi::OsStr) -> Result<CString> {
        CString::new(value.as_bytes())
            .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid browser resource path"))
    }

    fn directory(path: &Path, cancel: Option<&CancellationToken>) -> Result<File> {
        crate::cancel::check(cancel)?;
        if !path.is_absolute() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Browser resource paths must be canonical and absolute",
            ));
        }
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut current = opened(unsafe { libc::open(c"/".as_ptr(), flags) }, path)?;
        for component in path.components() {
            crate::cancel::check(cancel)?;
            match component {
                Component::RootDir => (),
                Component::Normal(part) => {
                    let part = name(part)?;
                    current = opened(
                        unsafe { libc::openat(current.as_raw_fd(), part.as_ptr(), flags) },
                        path,
                    )?;
                }
                _ => {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "Browser resource paths must be canonical",
                    ));
                }
            }
        }
        Ok(current)
    }

    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // fdopendir took ownership of the descriptor on success.
            unsafe { libc::closedir(self.0) };
        }
    }

    fn errno() -> *mut libc::c_int {
        #[cfg(target_os = "linux")]
        {
            unsafe { libc::__errno_location() }
        }
        #[cfg(target_os = "macos")]
        {
            unsafe { libc::__error() }
        }
    }

    pub(super) fn entries(
        path: &Path,
        limit: usize,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<Entry>> {
        let file = directory(path, cancel)?;
        let fd = file.into_raw_fd();
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(Error::io(path, error));
        }
        let directory = Directory(raw);
        let mut entries = Vec::new();
        loop {
            crate::cancel::check(cancel)?;
            unsafe { *errno() = 0 };
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                let error = unsafe { *errno() };
                if error != 0 {
                    return Err(Error::io(path, std::io::Error::from_raw_os_error(error)));
                }
                break;
            }
            let raw_name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if matches!(raw_name.to_bytes(), b"." | b"..") {
                continue;
            }
            if entries.len() >= limit {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Browser resource directory exceeds its entry budget",
                ));
            }
            let name = raw_name
                .to_str()
                .map_err(|_| {
                    Error::new("INVALID_ARGUMENT", "Browser resource names must be UTF-8")
                })?
                .to_owned();
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            let status = unsafe {
                libc::fstatat(
                    libc::dirfd(directory.0),
                    raw_name.as_ptr(),
                    metadata.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if status != 0 {
                return Err(Error::io(path, std::io::Error::last_os_error()));
            }
            let kind = match unsafe { metadata.assume_init() }.st_mode & libc::S_IFMT {
                libc::S_IFREG => Kind::File,
                libc::S_IFDIR => Kind::Directory,
                libc::S_IFLNK => Kind::Link,
                _ => {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "Browser resources must be regular files, directories, or guarded symbolic links",
                    ));
                }
            };
            entries.push(Entry { name, kind });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub(super) fn regular(
        path: &Path,
        limit: usize,
        cancel: Option<&CancellationToken>,
    ) -> Result<File> {
        let parent = directory(
            path.parent()
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Missing resource parent"))?,
            cancel,
        )?;
        let name = name(
            path.file_name()
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Missing resource filename"))?,
        )?;
        let file = opened(
            unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            },
            path,
        )?;
        let metadata = file.metadata().map_err(|error| Error::io(path, error))?;
        if !metadata.is_file() || metadata.len() > limit as u64 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid or oversized browser resource file",
            ));
        }
        Ok(file)
    }

    pub(super) fn read(
        path: &Path,
        limit: usize,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>> {
        let mut file = regular(path, limit, cancel)?;
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            crate::cancel::check(cancel)?;
            let read = match file.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.map_err(|error| Error::io(path, error))?,
            };
            if read == 0 {
                break;
            }
            if bytes.len().saturating_add(read) > limit {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Browser resource file exceeds its byte budget",
                ));
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Ok(bytes)
    }
}

pub(super) fn entries(
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<Entry>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        native::entries(path, limit, cancel)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, limit, cancel);
        Err(Error::new(
            "RUNTIME_UNAVAILABLE",
            "Guarded local browser resources are not available on this platform",
        ))
    }
}

pub(super) fn read(
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        native::read(path, limit, cancel)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, limit, cancel);
        Err(Error::new(
            "RUNTIME_UNAVAILABLE",
            "Guarded local browser resources are not available on this platform",
        ))
    }
}

pub(super) fn revision(
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        crate::store::content::hash_reader(
            native::regular(path, limit, cancel)?,
            path,
            limit,
            cancel,
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path, limit, cancel);
        Err(Error::new(
            "RUNTIME_UNAVAILABLE",
            "Guarded local browser resources are not available on this platform",
        ))
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn canonical_file_and_directory_reads_refuse_later_symbolic_substitution() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let inside = root.join("inside");
        let outside = root.join("outside");
        fs::create_dir(&inside).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(inside.join("data"), b"captured").unwrap();
        fs::write(outside.join("data"), b"private!").unwrap();
        let path = inside.join("data");
        assert_eq!(read(&path, 32, None).unwrap(), b"captured");
        assert_eq!(
            revision(&path, 32, None).unwrap(),
            crate::document::hash(b"captured")
        );
        fs::rename(&path, inside.join("saved")).unwrap();
        symlink(outside.join("data"), &path).unwrap();
        assert!(read(&path, 32, None).is_err());
        assert!(revision(&path, 32, None).is_err());
        fs::remove_file(&path).unwrap();
        fs::rename(inside.join("saved"), &path).unwrap();
        fs::rename(&inside, root.join("old-inside")).unwrap();
        symlink(&outside, &inside).unwrap();
        assert!(entries(&inside, 32, None).is_err());
        assert!(read(&path, 32, None).is_err());
        assert!(revision(&path, 32, None).is_err());
    }

    #[test]
    fn secure_reads_keep_limits_empty_inputs_and_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let path = root.join("empty");
        fs::write(&path, []).unwrap();
        assert!(read(&path, 0, None).unwrap().is_empty());
        assert_eq!(
            revision(&path, 0, None).unwrap(),
            crate::document::hash(&[])
        );
        fs::write(&path, b"not empty").unwrap();
        assert!(read(&path, 8, None).is_err());
        assert!(revision(&path, 8, None).is_err());
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            entries(&root, 32, Some(&cancel)).unwrap_err().code,
            "CANCELLED"
        );
        assert_eq!(
            read(&path, 32, Some(&cancel)).unwrap_err().code,
            "CANCELLED"
        );
        assert_eq!(
            revision(&path, 32, Some(&cancel)).unwrap_err().code,
            "CANCELLED"
        );
    }
}
