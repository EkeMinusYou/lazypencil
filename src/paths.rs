use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Decode an explicit local file URL without granting access to its target.
/// Callers must still capture canonical bindings and enforce project roots.
pub(crate) fn local_file_url(uri: &str) -> Result<PathBuf> {
    let invalid = || {
        Error::new(
            "ASSET_NOT_FOUND",
            "Expected an absolute local file URL without credentials, query, or fragment",
        )
    };
    if !uri
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
        || uri.chars().any(char::is_control)
        || uri.contains('\\')
    {
        return Err(invalid());
    }
    let url = url::Url::parse(uri).map_err(|_| invalid())?;
    if url.scheme() != "file"
        || !url.username().is_empty()
        || url.password().is_some()
        || url
            .host_str()
            .is_some_and(|host| !host.is_empty() && host != "localhost")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let path = url.to_file_path().map_err(|_| invalid())?;
    if !path.is_absolute()
        || path.to_str().is_none()
        || path.as_os_str().as_encoded_bytes().contains(&0)
    {
        return Err(invalid());
    }
    Ok(path)
}

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
    // Preserve the selected directory entry. Resolving an opened descriptor again
    // with F_GETPATH can return another hard link to the same inode on macOS.
    let canonical = std::fs::canonicalize(path).map_err(|e| crate::Error::io(path, e))?;
    if canonical.to_str().is_none() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Paths must be UTF-8 to be represented by the JSON API",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_file_urls_decode_once_without_treating_reserved_filename_bytes_as_url_fields() {
        let path = std::env::temp_dir().join("画像 %20 # ?.png");
        let url = url::Url::from_file_path(&path).unwrap();
        assert_eq!(local_file_url(url.as_str()).unwrap(), path);
        let upper = url.as_str().replacen("file:", "FILE:", 1);
        assert_eq!(local_file_url(&upper).unwrap(), path);
        let local_host = url.as_str().replacen("file:///", "file://localhost/", 1);
        assert_eq!(local_file_url(&local_host).unwrap(), path);
        for invalid in [
            format!("{url}?query"),
            format!("{url}#fragment"),
            "file:relative.png".into(),
            "file://remote.example/image.png".into(),
            "file://user:secret@localhost/image.png".into(),
            "file:///bad%00name.png".into(),
            "file:///bad%ffname.png".into(),
            "file:///bad\nname.png".into(),
            "file:///bad\\name.png".into(),
            "https://example.com/image.png".into(),
        ] {
            assert_eq!(
                local_file_url(&invalid).unwrap_err().code,
                "ASSET_NOT_FOUND"
            );
        }
    }
}
