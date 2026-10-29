//! Explicit, bounded web roots captured as immutable project inputs.

use super::*;
use std::fs;

mod bundle;
pub(crate) mod filesystem;
use filesystem::Entry;

const MAX_FILES: usize = 4096;
const MAX_DIRECTORIES: usize = 4096;
const MAX_NAMES: usize = 1024 * 1024;
const MAX_FILE: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserResourceRoot {
    /// Original canonical file directory URL, including its trailing slash.
    #[schemars(length(min = 1, max = 4096))]
    pub url: String,
    /// Physical directory relative to the owning .pen file, within allowed roots.
    #[schemars(length(min = 1, max = 4096))]
    pub path: String,
}

struct Root {
    url: url::Url,
    path: String,
}

struct Directory {
    path: PathBuf,
    entries: Vec<Entry>,
}

struct Resource {
    path: PathBuf,
    mime: Option<String>,
    bytes: Arc<[u8]>,
}

pub(super) struct Resources {
    files: BTreeMap<String, Resource>,
    directories: Vec<Directory>,
    revision: String,
    responses: Option<Arc<crate::cdp::LocalFiles>>,
    logical_bytes: usize,
    private_roots: Vec<PathBuf>,
    complete: bool,
    catalog: Option<PathBuf>,
}

/// One document's captured URL namespace. It contains no filesystem fallback.
pub struct BrowserResources {
    pub revision: String,
    pub files: Arc<crate::cdp::LocalFiles>,
}

fn declarations(document: &Document) -> Result<Vec<Root>> {
    let Some(value) = document
        .value()
        .pointer("/metadata/lazypencil/browser_resources")
    else {
        return Ok(Vec::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| invalid("Browser resources must be an object"))?;
    if object.len() != 2
        || !object.contains_key("version")
        || !(object.contains_key("roots") || object.contains_key("bundle"))
    {
        return Err(invalid("Invalid browser resource fields"));
    }
    if value["version"] != 1 {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported browser resource version",
        ));
    }
    if let Some(value) = object.get("bundle") {
        bundle::declaration(value)?;
        return Ok(Vec::new());
    }
    let roots = value["roots"]
        .as_array()
        .filter(|roots| roots.len() <= 128)
        .ok_or_else(|| invalid("Browser resources require at most 128 roots"))?;
    let mut result: Vec<Root> = Vec::new();
    let mut names = 0usize;
    for root in roots {
        let object = root
            .as_object()
            .ok_or_else(|| invalid("A browser root must be an object"))?;
        if object.len() != 2 || !object.contains_key("url") || !object.contains_key("path") {
            return Err(invalid("A browser root requires URL and path"));
        }
        let value = root["url"]
            .as_str()
            .ok_or_else(|| invalid("A browser root URL must be a string"))?;
        let key = crate::cdp::local_file_url(value)?;
        let url = url::Url::parse(&key).expect("validated local URL");
        let path = root["path"]
            .as_str()
            .filter(|path| {
                !path.is_empty()
                    && path.len() <= 4096
                    && !path.contains("://")
                    && !path.starts_with("file:")
                    && !path.chars().any(char::is_control)
            })
            .ok_or_else(|| invalid("Invalid browser root path"))?;
        names = names.saturating_add(value.len()).saturating_add(path.len());
        if value.len() > 4096 || names > MAX_NAMES || key != value || !url.path().ends_with('/') {
            return Err(invalid(
                "Browser roots need canonical local directory URLs without queries or fragments",
            ));
        }
        if result.iter().any(|other| {
            key.starts_with(other.url.as_str()) || other.url.as_str().starts_with(&key)
        }) {
            return Err(invalid("Browser resource URL roots must not overlap"));
        }
        result.push(Root {
            url,
            path: path.into(),
        });
    }
    Ok(result)
}

pub(super) fn validate(document: &Document) -> Result<()> {
    declarations(document).map(|_| ())
}

impl Project {
    pub(super) fn capture_browser_resources(&mut self, origin: &Path) -> Result<()> {
        let roots = declarations(&self.documents[origin])?;
        let packed = self.documents[origin]
            .value()
            .pointer("/metadata/lazypencil/browser_resources/bundle")
            .cloned();
        if let Some(value) = packed {
            let mut resources = self.capture_browser_bundle(origin, &value)?;
            resources.finish()?;
            self.browser_resources.insert(origin.into(), resources);
            return Ok(());
        }
        if roots.is_empty()
            && self.documents[origin]
                .value()
                .pointer("/metadata/lazypencil/browser_resources")
                .is_none()
        {
            return Ok(());
        }
        let mut resources = Resources {
            files: BTreeMap::new(),
            directories: Vec::new(),
            revision: String::new(),
            responses: None,
            logical_bytes: 0,
            private_roots: self.browser_private_roots()?,
            complete: true,
            catalog: None,
        };
        let mut names = 0usize;
        for root in roots {
            self.check_cancelled()?;
            let path = self.reference_path(origin, &root.path)?;
            if self
                .browser_reference(origin, &root.path, "browser_root", &resources.private_roots)?
                .is_none()
            {
                resources.complete = false;
                continue;
            }
            self.capture_browser_directory(
                origin,
                &path,
                &root.url,
                &mut resources,
                &mut BTreeSet::new(),
                &mut names,
            )?;
        }
        resources.finish()?;
        self.browser_resources.insert(origin.into(), resources);
        Ok(())
    }

    fn capture_browser_directory(
        &mut self,
        origin: &Path,
        requested: &Path,
        url: &url::Url,
        resources: &mut Resources,
        active: &mut BTreeSet<PathBuf>,
        names: &mut usize,
    ) -> Result<()> {
        self.check_cancelled()?;
        if active.len() >= 128 || resources.directories.len() >= MAX_DIRECTORIES {
            return Err(invalid(
                "Browser resource directories exceed their depth or count budget",
            ));
        }
        let uri = requested
            .to_str()
            .ok_or_else(|| invalid("Browser resource paths must be UTF-8"))?;
        let Some(canonical) =
            self.browser_reference(origin, uri, "browser_root", &resources.private_roots)?
        else {
            resources.complete = false;
            return Ok(());
        };
        if !active.insert(canonical.clone()) {
            return Err(invalid(
                "Browser resource directories contain a symbolic-link cycle",
            ));
        }
        let entries = filesystem::entries(
            &canonical,
            MAX_FILES + MAX_DIRECTORIES,
            self.cancel.as_ref(),
        )?;
        let mut children = Vec::new();
        for entry in &entries {
            self.check_cancelled()?;
            *names = names.saturating_add(entry.name.len());
            if *names > MAX_NAMES {
                return Err(invalid("Browser resource names exceed 1 MiB"));
            }
            let path = requested.join(&entry.name);
            let mut child = url.clone();
            child
                .path_segments_mut()
                .map_err(|_| invalid("Invalid browser directory URL"))?
                .pop_if_empty()
                .push(&entry.name);
            if child.as_str().len() > 4096 {
                return Err(invalid("A browser resource URL exceeds 4096 bytes"));
            }
            let Some(target) = self.browser_reference(
                origin,
                path.to_str()
                    .ok_or_else(|| invalid("Browser resource paths must be UTF-8"))?,
                "browser_resource",
                &resources.private_roots,
            )?
            else {
                resources.complete = false;
                continue;
            };
            let metadata =
                fs::symlink_metadata(&target).map_err(|error| Error::io(&target, error))?;
            if metadata.is_dir() {
                child.path_segments_mut().expect("directory URL").push("");
                children.push((path, child));
            } else if metadata.is_file() {
                if resources.files.len() >= MAX_FILES {
                    return Err(invalid("Browser resources exceed 4096 files"));
                }
                self.runtime_assets.remove(&target);
                let bytes = if let Some(document) = self.documents.get(&target) {
                    if let Some(bytes) = self.browser_document_bytes.get(&target) {
                        bytes.clone()
                    } else {
                        let bytes = if let Some(asset) = self
                            .assets
                            .get(&target)
                            .filter(|asset| asset.bytes.as_ref() == document.text().as_bytes())
                        {
                            asset.bytes.clone()
                        } else {
                            self.byte_count = self.byte_count.saturating_add(document.text().len());
                            if self.byte_count > MAX_PROJECT_BYTES {
                                return Err(invalid("Project dependencies exceed 128 MiB"));
                            }
                            Arc::from(document.text().as_bytes())
                        };
                        self.browser_document_bytes
                            .insert(target.clone(), bytes.clone());
                        bytes
                    }
                } else if let Some(existing) = self.assets.get(&target) {
                    // The final input verification checks the live file once;
                    // URL aliases share an already captured immutable payload.
                    existing.bytes.clone()
                } else {
                    let bytes: Arc<[u8]> =
                        filesystem::read(&target, MAX_FILE, self.cancel.as_ref())?.into();
                    self.capture_bytes(target.clone(), "browser_resource", bytes.clone())?;
                    bytes
                };
                if bytes.len() > MAX_FILE {
                    return Err(invalid("A browser resource exceeds 32 MiB"));
                }
                resources.logical_bytes = resources.logical_bytes.saturating_add(bytes.len());
                if resources.logical_bytes > MAX_PROJECT_BYTES {
                    return Err(invalid(
                        "Browser resources exceed 128 MiB including URL aliases",
                    ));
                }
                resources.files.insert(
                    child.to_string(),
                    Resource {
                        path: target,
                        mime: mime(&entry.name).map(String::from),
                        bytes,
                    },
                );
            } else {
                return Err(invalid(
                    "Browser resource links must target regular files or directories",
                ));
            }
        }
        resources.directories.push(Directory {
            path: canonical.clone(),
            entries,
        });
        for (path, url) in children {
            self.capture_browser_directory(origin, &path, &url, resources, active, names)?;
        }
        active.remove(&canonical);
        Ok(())
    }

    /// Return an immutable namespace only for a URL explicitly declared by
    /// its property owner. Another document's matching URL grants no access.
    pub fn browser_page_resources(&self, origin: &Path, url: &str) -> Result<BrowserResources> {
        self.check_cancelled()?;
        let key = crate::cdp::local_file_url(url)?;
        let resources = self
            .browser_resources
            .get(origin)
            .filter(|resources| resources.files.contains_key(&key))
            .ok_or_else(|| {
                Error::new(
                    "ASSET_NOT_FOUND",
                    "Local browser URL is outside its document's captured resource roots",
                )
            })?;
        Ok(BrowserResources {
            revision: resources.revision.clone(),
            files: resources
                .responses
                .as_ref()
                .expect("nonempty captured namespace")
                .clone(),
        })
    }

    pub(super) fn verify_browser_resources(&self) -> Result<()> {
        if self.browser_resources.is_empty() {
            return Ok(());
        }
        let private = self.browser_private_roots()?;
        for resources in self.browser_resources.values() {
            for path in resources.paths() {
                self.check_cancelled()?;
                forbid_private(path, &private).map_err(|_| {
                    Error::new(
                        "REVISION_CONFLICT",
                        "Browser private resource boundaries changed; reload the project",
                    )
                })?;
            }
            for directory in &resources.directories {
                self.check_cancelled()?;
                forbid_private(&directory.path, &private).map_err(|_| {
                    Error::new(
                        "REVISION_CONFLICT",
                        "Browser private resource boundaries changed; reload the project",
                    )
                })?;
                let current = filesystem::entries(
                    &directory.path,
                    MAX_FILES + MAX_DIRECTORIES,
                    self.cancel.as_ref(),
                )
                .map_err(|error| {
                    if error.code == "CANCELLED" {
                        error
                    } else {
                        Error::new(
                            "REVISION_CONFLICT",
                            "Browser resource directory changed; reload the project",
                        )
                    }
                })?;
                if current != directory.entries {
                    return Err(Error::new(
                        "REVISION_CONFLICT",
                        "Browser resource directory membership changed; reload the project",
                    ));
                }
            }
        }
        Ok(())
    }

    fn browser_private_roots(&self) -> Result<Vec<PathBuf>> {
        self.icon_data_dir
            .iter()
            .cloned()
            .chain(std::iter::once(crate::store::default_lock_dir()?))
            .map(|path| {
                let path = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()
                        .map_err(|error| Error::io(&path, error))?
                        .join(path)
                };
                match crate::paths::canonical(&path) {
                    Ok(target) => Ok(target),
                    Err(error) => crate::paths::MissingPath::capture(&path)?
                        .map(|missing| missing.target)
                        .ok_or(error),
                }
            })
            .collect()
    }

    fn browser_reference(
        &mut self,
        origin: &Path,
        uri: &str,
        kind: &str,
        private: &[PathBuf],
    ) -> Result<Option<PathBuf>> {
        match self.resolve_file(origin, uri) {
            Ok(target) => {
                forbid_private(&target, private)?;
                Ok(Some(target))
            }
            Err(error) if error.code == "IO_ERROR" => {
                let requested = self.reference_path(origin, uri)?;
                if let Some(missing) = crate::paths::MissingPath::capture(&requested)? {
                    forbid_private(&missing.target, private)?;
                }
                self.capture_missing_reference(origin, uri, kind, error)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

impl Resources {
    fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files
            .values()
            .map(|file| file.path.as_path())
            .chain(self.catalog.as_deref())
    }

    fn finish(&mut self) -> Result<()> {
        let inventory = self.files.iter().map(|(url, file)| {
            json!({"url":url,"mime":file.mime,"bytes":file.bytes.len(),"sha256":hash(&file.bytes)})
        }).collect::<Vec<_>>();
        self.revision = hash(&serde_json::to_vec(&inventory).expect("serializable inventory"));
        if !self.files.is_empty() {
            self.responses = Some(Arc::new(crate::cdp::LocalFiles::with_optional_mime(
                self.files
                    .iter()
                    .map(|(url, file)| (url.clone(), file.mime.clone(), file.bytes.clone())),
            )?));
        }
        Ok(())
    }
}

fn forbid_private(path: &Path, private: &[PathBuf]) -> Result<()> {
    if private
        .iter()
        .any(|root| path.starts_with(root) || root.starts_with(path))
    {
        return Err(invalid(
            "Browser resources must not overlap lazypencil's private data or lock directories",
        ));
    }
    Ok(())
}

pub(super) fn revision(project: &Project) -> Option<Value> {
    (!project.browser_resources.is_empty()).then(|| {
        json!(
            project
                .browser_resources
                .iter()
                .map(|(path, resources)| (path, &resources.revision))
                .collect::<Vec<_>>()
        )
    })
}

pub(super) fn protected_paths(project: &Project) -> BTreeSet<&Path> {
    project
        .browser_resources
        .values()
        .flat_map(|resources| resources.paths())
        .collect()
}

pub(super) fn file_revision(
    path: &Path,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<String> {
    filesystem::revision(path, MAX_FILE, cancel).map_err(|error| {
        if error.code == "CANCELLED" {
            error
        } else {
            Error::new(
                "REVISION_CONFLICT",
                "Browser resource changed; reload the project",
            )
        }
    })
}

fn mime(name: &str) -> Option<&'static str> {
    Some(
        match name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" | "mjs" => "text/javascript",
            "json" | "map" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            "wasm" => "application/wasm",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            "txt" => "text/plain",
            _ => return None,
        },
    )
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn documents_shared_by_multiple_url_roots_retain_one_payload_allocation() {
        let temp = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let store = Store::init(
            temp.path().join("scene.pen"),
            Some(state.path().join("private")),
        )
        .unwrap();
        fs::write(store.path(), json!({"version":"2.21","metadata":{"lazypencil":{"browser_resources":{
            "version":1,"roots":[{"url":"file:///first/","path":"."},{"url":"file:///second/","path":"."}]
        }}},"children":[]}).to_string()).unwrap();
        let project = Project::load(&store).unwrap();
        let resources = &project.browser_resources[store.path()];
        assert!(Arc::ptr_eq(
            &resources.files["file:///first/scene.pen"].bytes,
            &resources.files["file:///second/scene.pen"].bytes
        ));
        assert!(Arc::ptr_eq(
            &resources.files["file:///first/scene.pen"].bytes,
            &project.browser_document_bytes[store.path()]
        ));
    }
}
