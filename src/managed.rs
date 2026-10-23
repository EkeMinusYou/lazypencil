use crate::{Error, Result, document::hash};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::NamedTempFile;

pub(crate) type Overlay = BTreeMap<PathBuf, Arc<[u8]>>;

#[derive(Clone)]
pub(crate) struct File {
    pub path: PathBuf,
    pub uri: String,
    pub bytes: Arc<[u8]>,
}

impl File {
    pub fn new(document: &Path, bytes: Arc<[u8]>, extension: &str) -> Result<Self> {
        if bytes.is_empty()
            || bytes.len() > 64 * 1024 * 1024
            || !matches!(
                extension,
                "png" | "jpg" | "jpeg" | "webp" | "js" | "glsl" | "lib.pen"
            )
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid managed import size or extension",
            ));
        }
        let uri = format!("assets/lazypencil/{}.{extension}", hash(&bytes));
        let path = document
            .parent()
            .ok_or_else(|| {
                Error::new(
                    "INVALID_OPERATION",
                    "Managed imports require a document path",
                )
            })?
            .join(&uri);
        let file = Self { path, uri, bytes };
        file.verify(false)?;
        Ok(file)
    }

    pub fn verify(&self, required: bool) -> Result<()> {
        let directory = self.path.parent().expect("managed directory");
        let base = directory
            .parent()
            .and_then(Path::parent)
            .expect("document directory");
        if crate::paths::canonical(base)? != base {
            return Err(Error::new(
                "ASSET_CONFLICT",
                "The managed import's document directory changed its target",
            ));
        }
        for path in [directory.parent().expect("assets directory"), directory] {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(Error::new(
                        "ASSET_CONFLICT",
                        "Managed import directories must be real directories",
                    ));
                }
                Ok(_) => {
                    if crate::paths::canonical(path)? != path {
                        return Err(Error::new(
                            "ASSET_CONFLICT",
                            "Managed directories must retain their exact physical paths",
                        ));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {}
                Err(e) => return Err(Error::io(path, e)),
            }
        }
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() != self.bytes.len() as u64
                {
                    return Err(Error::new(
                        "ASSET_CONFLICT",
                        "A managed import exists with a different type or length",
                    ));
                }
                if fs::read(&self.path)
                    .map_err(|e| Error::io(&self.path, e))?
                    .as_slice()
                    != self.bytes.as_ref()
                {
                    return Err(Error::new(
                        "ASSET_CONFLICT",
                        "A managed import was changed; it will not be overwritten",
                    ));
                }
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => Ok(()),
            Err(e) => Err(Error::io(&self.path, e)),
        }
    }

    pub fn publish(
        &self,
        store: &crate::store::Store,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<bool> {
        crate::cancel::check(cancel)?;
        // Readers of an absent dependency hold this same path's shared lock.
        // Take no document lock here: a saving document may already own it.
        let _lock = store.lock_publication(&self.path, cancel)?;
        self.verify(false)?;
        let directory = self.path.parent().expect("managed directory");
        for path in [directory.parent().expect("assets directory"), directory] {
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Error::io(path, e)),
            }
            let metadata = fs::symlink_metadata(path).map_err(|e| Error::io(path, e))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::new(
                    "ASSET_CONFLICT",
                    "Managed import directories must be real directories",
                ));
            }
            if crate::paths::canonical(path)? != path {
                return Err(Error::new(
                    "ASSET_CONFLICT",
                    "Managed directories must retain their exact physical paths",
                ));
            }
        }
        let mut temporary =
            NamedTempFile::new_in(directory).map_err(|e| Error::io(directory, e))?;
        for chunk in self.bytes.chunks(256 * 1024) {
            crate::cancel::check(cancel)?;
            temporary
                .write_all(chunk)
                .map_err(|error| Error::io(&self.path, error))?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| Error::io(&self.path, e))?;
        crate::cancel::check(cancel)?;
        let reused = match temporary.persist_noclobber(&self.path) {
            Ok(_) => false,
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => true,
            Err(e) => return Err(Error::io(&self.path, e.error)),
        };
        self.verify(true)?;
        crate::store::sync_directory(directory)?;
        crate::store::sync_directory(directory.parent().expect("assets directory"))?;
        crate::store::sync_directory(
            directory
                .parent()
                .and_then(Path::parent)
                .expect("document directory"),
        )?;
        Ok(reused)
    }
}
