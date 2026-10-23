use crate::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct MissingPath {
    pub requested: PathBuf,
    pub target: PathBuf,
    anchor_requested: PathBuf,
    anchor: PathBuf,
    tail: PathBuf,
    links: Vec<(PathBuf, PathBuf)>,
}

impl MissingPath {
    pub fn revision(&self) -> String {
        crate::document::hash(
            serde_json::json!({"availability":"missing","path":self.requested,"target":self.target})
                .to_string()
                .as_bytes(),
        )
    }
    pub fn capture(path: &Path) -> Result<Option<Self>> {
        let requested = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| Error::io(path, error))?
                .join(path)
        };
        if requested.to_str().is_none() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Missing references require UTF-8 paths",
            ));
        }
        match std::fs::canonicalize(&requested) {
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(&requested, error)),
        }
        if requested.components().count() > 256 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Missing references may contain at most 256 path components",
            ));
        }
        Self::capture_absent(&requested, 0).map(Some)
    }

    fn capture_absent(path: &Path, depth: usize) -> Result<Self> {
        if depth > 40 {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "Missing reference exceeds the symbolic link limit",
            ));
        }
        for ancestor in path.ancestors() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) => {
                    let tail = path
                        .strip_prefix(ancestor)
                        .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid missing reference"))?;
                    if tail.components().any(|part| {
                        !matches!(
                            part,
                            std::path::Component::Normal(_) | std::path::Component::CurDir
                        )
                    }) {
                        return Err(Error::new(
                            "ASSET_NOT_FOUND",
                            "A missing reference cannot traverse an absent parent directory",
                        ));
                    }
                    let tail = tail
                        .components()
                        .filter(|part| !matches!(part, std::path::Component::CurDir))
                        .collect::<PathBuf>();
                    match std::fs::canonicalize(ancestor) {
                        Ok(_) => {
                            if !std::fs::metadata(ancestor).is_ok_and(|metadata| metadata.is_dir())
                                || tail.as_os_str().is_empty()
                            {
                                return Err(changed(path));
                            }
                            let anchor = canonical(ancestor)?;
                            return Ok(Self {
                                requested: path.into(),
                                target: anchor.join(&tail),
                                anchor_requested: ancestor.into(),
                                anchor,
                                tail,
                                links: vec![],
                            });
                        }
                        Err(error)
                            if error.kind() == std::io::ErrorKind::NotFound
                                && metadata.file_type().is_symlink() =>
                        {
                            let link = std::fs::read_link(ancestor)
                                .map_err(|error| Error::io(ancestor, error))?;
                            if link.to_str().is_none() {
                                return Err(Error::new(
                                    "INVALID_ARGUMENT",
                                    "Missing symbolic links must use UTF-8 targets",
                                ));
                            }
                            let target = if link.is_absolute() {
                                link.clone()
                            } else {
                                ancestor.parent().ok_or_else(|| changed(path))?.join(&link)
                            };
                            let target = target.join(tail);
                            if target.components().count() > 256 {
                                return Err(changed(path));
                            }
                            let mut snapshot = Self::capture_absent(&target, depth + 1)?;
                            snapshot.requested = path.into();
                            snapshot.links.push((ancestor.into(), link));
                            return Ok(snapshot);
                        }
                        Err(error) => return Err(Error::io(ancestor, error)),
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::io(ancestor, error)),
            }
        }
        Err(changed(path))
    }

    pub fn verify(&self) -> Result<()> {
        if !canonical(&self.anchor_requested).is_ok_and(|path| path == self.anchor) {
            return Err(changed(&self.requested));
        }
        for (path, target) in &self.links {
            if !std::fs::read_link(path).is_ok_and(|current| current == *target) {
                return Err(changed(&self.requested));
            }
        }
        // Newly created ordinary directories retain the same absent file identity.
        // A new link or junction would redirect a reference that was previously absent.
        let mut path = self.anchor.clone();
        let components = self.tail.components().collect::<Vec<_>>();
        for component in &components[..components.len().saturating_sub(1)] {
            path.push(component.as_os_str());
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(changed(&self.requested));
                    }
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if metadata.file_attributes() & 0x400 != 0 {
                            return Err(changed(&self.requested));
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => return Err(changed(&self.requested)),
            }
        }
        match std::fs::symlink_metadata(&self.target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(changed(&self.requested)),
        }
    }

    pub fn current_target(&self) -> Result<PathBuf> {
        self.verify()?;
        Self::capture(&self.requested)
            .map_err(|_| changed(&self.requested))?
            .map(|snapshot| snapshot.target)
            .ok_or_else(|| changed(&self.requested))
    }
}

fn changed(path: &Path) -> Error {
    Error::new(
        "REVISION_CONFLICT",
        "A missing reference changed; reload the document",
    )
    .details(serde_json::json!({"path":path}))
}

pub fn canonical(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    if path.to_str().is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Paths must be UTF-8 to be represented by the JSON API",
        ));
    }
    let canonical = std::fs::canonicalize(path).map_err(|e| crate::Error::io(path, e))?;
    #[cfg(target_os = "macos")]
    let canonical = {
        use std::{
            fs::OpenOptions,
            os::{
                fd::AsRawFd,
                unix::{ffi::OsStringExt, fs::OpenOptionsExt},
            },
        };
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&canonical)
            .map_err(|e| Error::io(path, e))?;
        let mut buffer = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: The descriptor is open and the buffer has PATH_MAX bytes, as F_GETPATH requires.
        let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
        if result < 0 {
            return Err(Error::io(path, std::io::Error::last_os_error()));
        }
        let length = buffer
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Error::new("IO_ERROR", "Canonical path exceeds PATH_MAX"))?;
        buffer.truncate(length);
        PathBuf::from(std::ffi::OsString::from_vec(buffer))
    };
    if canonical.to_str().is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Paths must be UTF-8 to be represented by the JSON API",
        ));
    }
    Ok(canonical)
}
