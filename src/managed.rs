use crate::{Error, Result, document::hash};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::NamedTempFile;

pub(crate) type Overlay = BTreeMap<PathBuf, File>;

/// Validate projected names before any filesystem operation. A copier can choose
/// portable storage without treating an existing-file conflict as a path alias.
pub(crate) fn validate_bundle_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let names = names.into_iter().collect::<BTreeSet<_>>();
    let mut portable = BTreeMap::new();
    for name in &names {
        crate::cancel::check(cancel)?;
        let path = Path::new(name);
        if name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
            || name.is_empty()
            || *name == "bundle.json"
            || name.contains('\\')
            || name.chars().any(char::is_control)
            || path.components().count() > 128
            || path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid managed bundle member path",
            ));
        }
        for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
            let original = ancestor.to_str().expect("UTF-8 member");
            if portable
                .insert(original.to_lowercase(), original.to_owned())
                .is_some_and(|previous| previous != original)
                || ancestor != path && names.contains(original)
            {
                return Err(Error::new(
                    "ASSET_CONFLICT",
                    "Managed bundle paths collide as files or portable names",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn normalize(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !result.pop() {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "A staged reference escapes its root",
                    ));
                }
            }
            part => result.push(part),
        }
    }
    Ok(result)
}

#[derive(Clone)]
pub(crate) struct File {
    pub path: PathBuf,
    pub uri: String,
    pub bytes: Arc<[u8]>,
    base: PathBuf,
    expected_uri: String,
    revision: String,
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
        Self::at(document, uri, bytes)
    }

    pub fn bundle(
        document: &Path,
        members: BTreeMap<String, Arc<[u8]>>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<Self>> {
        if members.is_empty() || members.len() > 4097 {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Managed bundles require 1–4097 members including their resource catalogue",
            ));
        }
        let mut total_bytes = 0usize;
        let mut path_bytes = 0usize;
        let mut inventory = BTreeMap::new();
        validate_bundle_names(members.keys().map(String::as_str), cancel)?;
        for (name, bytes) in &members {
            crate::cancel::check(cancel)?;
            if bytes.len() > 64 * 1024 * 1024 {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Invalid managed bundle member",
                ));
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            path_bytes = path_bytes.saturating_add(name.len());
            if total_bytes > 128 * 1024 * 1024 || path_bytes > 1024 * 1024 {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Managed bundle exceeds its byte or path budget",
                ));
            }
            inventory.insert(
                name,
                serde_json::json!({"sha256":hash(bytes),"bytes":bytes.len()}),
            );
        }
        let index: Arc<[u8]> =
            serde_json::to_vec(&serde_json::json!({"version":1,"files":inventory}))
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Cannot encode managed bundle index"))?
                .into();
        let directory = format!("assets/lazypencil/{}.bundle", hash(&index));
        members
            .into_iter()
            .chain(std::iter::once(("bundle.json".into(), index)))
            .map(|(name, bytes)| {
                crate::cancel::check(cancel)?;
                Self::at(document, format!("{directory}/{name}"), bytes)
            })
            .collect()
    }

    fn at(document: &Path, uri: String, bytes: Arc<[u8]>) -> Result<Self> {
        let base = document
            .parent()
            .ok_or_else(|| {
                Error::new(
                    "INVALID_OPERATION",
                    "Managed imports require a document path",
                )
            })?
            .to_path_buf();
        let path = base.join(&uri);
        let file = Self {
            path,
            expected_uri: uri.clone(),
            uri,
            revision: hash(&bytes),
            bytes,
            base,
        };
        file.verify(false)?;
        Ok(file)
    }

    pub fn verify(&self, required: bool) -> Result<()> {
        if self.uri != self.expected_uri
            || self.path != self.base.join(&self.expected_uri)
            || hash(&self.bytes) != self.revision
        {
            return Err(Error::new(
                "ASSET_CONFLICT",
                "A managed import lost its immutable content binding",
            ));
        }
        let base = &self.base;
        if crate::paths::canonical(base)? != self.base {
            return Err(Error::new(
                "ASSET_CONFLICT",
                "The managed import's document directory changed its target",
            ));
        }
        for path in self.directories() {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(Error::new(
                        "ASSET_CONFLICT",
                        "Managed import directories must be real directories",
                    ));
                }
                Ok(_) => {
                    if crate::paths::canonical(&path)? != path {
                        return Err(Error::new(
                            "ASSET_CONFLICT",
                            "Managed directories must retain their exact physical paths",
                        ));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {}
                Err(e) => return Err(Error::io(&path, e)),
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

    pub fn for_document(&self, document: &Path) -> Result<()> {
        if document.parent() != Some(self.base.as_path()) {
            return Err(Error::new(
                "ASSET_CONFLICT",
                "A staged import belongs to another document directory",
            ));
        }
        self.verify(false)
    }

    fn directories(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut path = self.base.clone();
        for part in Path::new(&self.expected_uri)
            .parent()
            .expect("managed parent")
            .components()
        {
            path.push(part);
            paths.push(path.clone());
        }
        paths
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
        for path in self.directories() {
            crate::cancel::check(cancel)?;
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(Error::io(&path, e)),
            }
            let metadata = fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::new(
                    "ASSET_CONFLICT",
                    "Managed import directories must be real directories",
                ));
            }
            if crate::paths::canonical(&path)? != path {
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
        for path in self
            .directories()
            .iter()
            .rev()
            .chain(std::iter::once(&self.base))
        {
            crate::store::sync_directory(path)?;
        }
        Ok(reused)
    }
}
