//! Portable payload storage preserves URL identity without rewriting web code.

use super::*;
use serde::{Deserialize, Serialize};

const MAX_CATALOG: usize = 8 * 1024 * 1024;
type CapturedFile = (PathBuf, Arc<[u8]>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Declaration {
    uri: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u32,
    files: Vec<Member>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    url: String,
    mime: Option<String>,
    sha256: String,
    bytes: usize,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn declaration(value: &Value) -> Result<Declaration> {
    let result: Declaration = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Invalid browser resource bundle declaration"))?;
    if result.uri.len() > 4096
        || result.uri.contains("://")
        || result.uri.starts_with("file:")
        || !digest(&result.sha256)
    {
        return Err(invalid("Invalid browser resource bundle path or digest"));
    }
    crate::managed::validate_bundle_names([result.uri.as_str()], None)?;
    if Path::new(&result.uri)
        .file_name()
        .and_then(|name| name.to_str())
        != Some("resources.json")
    {
        return Err(invalid("Browser resource bundles require resources.json"));
    }
    Ok(result)
}

impl Project {
    fn browser_bundle_file(
        &mut self,
        origin: &Path,
        uri: &str,
        private: &[PathBuf],
        maximum: usize,
    ) -> Result<Option<CapturedFile>> {
        let Some(path) = self.browser_reference(origin, uri, "browser_resource", private)? else {
            return Ok(None);
        };
        let bytes = if let Some(file) = self.overlay.get(&path) {
            file.bytes.clone()
        } else if let Some(asset) = self.assets.get(&path) {
            asset.bytes.clone()
        } else {
            filesystem::read(&path, maximum, self.cancel.as_ref())?.into()
        };
        if bytes.len() > maximum {
            return Err(invalid("Browser bundle member exceeds its byte budget"));
        }
        if !self.assets.contains_key(&path) {
            self.capture_bytes(path.clone(), "browser_resource", bytes.clone())?;
        }
        Ok(Some((path, bytes)))
    }

    pub(super) fn capture_browser_bundle(
        &mut self,
        origin: &Path,
        value: &Value,
    ) -> Result<Resources> {
        let declaration = declaration(value)?;
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
        let Some((path, bytes)) = self.browser_bundle_file(
            origin,
            &declaration.uri,
            &resources.private_roots,
            MAX_CATALOG,
        )?
        else {
            resources.complete = false;
            return Ok(resources);
        };
        resources.catalog = Some(path);
        if hash(&bytes) != declaration.sha256 {
            return Err(Error::new(
                "ASSET_CONFLICT",
                "Browser resource catalogue content changed",
            ));
        }
        let catalog: Catalog = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Invalid browser resource catalogue"))?;
        if catalog.version != 1 {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                "Unsupported browser resource catalogue version",
            ));
        }
        if catalog.files.len() > MAX_FILES {
            return Err(invalid("Browser resources exceed 4096 files"));
        }
        let parent = Path::new(&declaration.uri)
            .parent()
            .expect("catalogue parent");
        let mut names = 0usize;
        let mut seen = BTreeSet::new();
        for member in catalog.files {
            self.check_cancelled()?;
            names = names.saturating_add(member.url.len());
            if !digest(&member.sha256)
                || member.bytes > MAX_FILE
                || member.url.len() > 4096
                || names > 4 * MAX_NAMES
                || crate::cdp::local_file_url(&member.url)? != member.url
                || !seen.insert(member.url.clone())
            {
                return Err(invalid(
                    "Invalid or duplicate browser resource catalogue member",
                ));
            }
            resources.logical_bytes = resources.logical_bytes.saturating_add(member.bytes);
            if resources.logical_bytes > MAX_PROJECT_BYTES {
                return Err(invalid(
                    "Browser resources exceed 128 MiB including URL aliases",
                ));
            }
            let uri = parent.join(format!("{}.blob", member.sha256));
            let Some((path, bytes)) = self.browser_bundle_file(
                origin,
                uri.to_str()
                    .ok_or_else(|| invalid("Browser bundle paths must be UTF-8"))?,
                &resources.private_roots,
                MAX_FILE,
            )?
            else {
                resources.complete = false;
                continue;
            };
            if bytes.len() != member.bytes || hash(&bytes) != member.sha256 {
                return Err(Error::new(
                    "ASSET_CONFLICT",
                    "Browser resource payload content changed",
                ));
            }
            resources.files.insert(
                member.url,
                Resource {
                    path,
                    mime: member.mime,
                    bytes,
                },
            );
        }
        Ok(resources)
    }

    pub(crate) fn pack_browser_resources(
        &self,
        origin: &Path,
        destination: &Path,
    ) -> Result<(Value, Vec<crate::managed::File>)> {
        self.check_cancelled()?;
        self.verify_unchanged()?;
        let resources = self
            .browser_resources
            .get(origin)
            .ok_or_else(|| invalid("No browser resource roots are declared for this document"))?;
        if !resources.complete {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "Cannot pack incomplete browser resources",
            ));
        }
        let mut members = BTreeMap::new();
        let mut files = Vec::new();
        for (url, resource) in &resources.files {
            self.check_cancelled()?;
            let sha256 = hash(&resource.bytes);
            members
                .entry(format!("{sha256}.blob"))
                .or_insert_with(|| resource.bytes.clone());
            files.push(Member {
                url: url.clone(),
                mime: resource.mime.clone(),
                sha256,
                bytes: resource.bytes.len(),
            });
        }
        let catalog = serde_json::to_vec(&Catalog { version: 1, files })
            .map_err(|_| invalid("Cannot encode browser resource catalogue"))?;
        if catalog.len() > MAX_CATALOG {
            return Err(invalid("Browser resource catalogue exceeds 8 MiB"));
        }
        let sha256 = hash(&catalog);
        members.insert("resources.json".into(), Arc::from(catalog));
        let files = crate::managed::File::bundle(destination, members, self.cancel.as_ref())?;
        if files.iter().any(|file| {
            resources
                .directories
                .iter()
                .any(|directory| file.path.starts_with(&directory.path))
        }) {
            return Err(Error::new(
                "ASSET_CONFLICT",
                "Managed browser output must be outside the captured source directories",
            ));
        }
        let uri = &files
            .iter()
            .find(|file| {
                file.path.file_name().and_then(|name| name.to_str()) == Some("resources.json")
            })
            .expect("catalogue member")
            .uri;
        Ok((
            json!({"version":1,"bundle":{"uri":uri,"sha256":sha256}}),
            files,
        ))
    }
}
