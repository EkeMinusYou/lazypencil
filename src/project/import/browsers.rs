use super::super::browser_context::{self, Scope};
use super::*;

pub(super) struct Info {
    resolved: ResolvedNode,
    origin: Option<PathBuf>,
}

impl Copier<'_> {
    pub(super) fn dynamic_info(
        &mut self,
        origin: &Path,
        node: &Value,
    ) -> Result<Option<DynamicInfo>> {
        if !matches!(node["type"].as_str(), Some("script" | "browser" | "ref")) {
            return Ok(None);
        }
        let resolved = self
            .project
            .expand(origin, node, None, &[], &mut Vec::new(), &mut 0)?;
        self.dynamic_info_resolved(
            resolved,
            (origin.into(), node["id"].as_str().expect("node ID").into()),
        )
    }

    pub(super) fn dynamic_info_resolved(
        &mut self,
        resolved: ResolvedNode,
        source: Key,
    ) -> Result<Option<DynamicInfo>> {
        match resolved.kind() {
            "script" => self
                .script_info_resolved(resolved, source)
                .map(|info| Some(DynamicInfo::Script(info))),
            "browser" => {
                let has_url = resolved
                    .properties
                    .get("url")
                    .and_then(Value::as_str)
                    .is_some_and(|url| !url.is_empty());
                let origin = (has_url
                    && self
                        .project
                        .browser_resources
                        .contains_key(resolved.browser_origin()))
                .then(|| resolved.browser_origin().to_path_buf());
                if origin.is_none()
                    && (resolved.browser_scope.is_some()
                        || resolved
                            .properties
                            .get("url")
                            .and_then(Value::as_str)
                            .and_then(|url| url::Url::parse(url).ok())
                            .is_some_and(|url| url.scheme() == "file"))
                {
                    return Err(Error::new(
                        "ASSET_NOT_FOUND",
                        "A copied local browser requires a declared resource namespace",
                    ));
                }
                Ok(Some(DynamicInfo::Browser(Info { resolved, origin })))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn copied_browser_scope(&mut self, origin: &Path) -> Result<String> {
        if let Some(alias) = self.browser_scopes.get(origin) {
            return Ok(alias.clone());
        }
        let destination = self
            .path
            .ok_or_else(|| copy_error("Copying browser resources requires a bound document"))?;
        let (mut declaration, files) = self.project.pack_browser_resources(origin, destination)?;
        let catalog = files
            .iter()
            .find(|file| file.uri.ends_with("/resources.json"))
            .expect("browser catalogue");
        let anchor = destination
            .parent()
            .expect("bound document directory")
            .join("assets/lazypencil/browser.lib.pen");
        declaration["bundle"]["uri"] = json!(super::scripts::relative_uri(&anchor, &catalog.path)?);
        let document = json!({"version":self.project.root_document().version(),"metadata":{"lazypencil":{"browser_resources":declaration}},"children":[]});
        let library = crate::managed::File::new(
            destination,
            serde_json::to_vec(&document)
                .map_err(|_| copy_error("Cannot encode copied browser scope"))?
                .into(),
            "lib.pen",
        )?;
        let alias = self.bind_copied_library(
            &library,
            format!("lazypencil-web-{}", &hash(library.uri.as_bytes())[..16]),
        );
        for file in files.into_iter().chain(std::iter::once(library)) {
            self.files.insert(file.path.clone(), file);
        }
        self.browser_scopes.insert(origin.into(), alias.clone());
        Ok(alias)
    }

    pub(super) fn rewrite_browser(
        &mut self,
        node: &Value,
        result: &mut Map<String, Value>,
        info: Option<&Info>,
    ) -> Result<()> {
        let Some(info) = info else {
            return Ok(());
        };
        let old = browser_context::override_value(node.as_object().expect("browser properties"))?;
        let full = node["type"] == "browser";
        let metadata = old
            .as_ref()
            .map_or(node.get("metadata"), |value| value.metadata.as_ref());
        let owns_scope = full
            || node.get("url").is_some()
            || metadata
                .and_then(|value| value.pointer("/lazypencil/browser_scope"))
                .is_some()
            || old.as_ref().is_some_and(|value| value.scope.is_some());
        if !full && !owns_scope && metadata.is_none() {
            if old.is_some() {
                result.remove("metadata");
            }
            return Ok(());
        }
        if let Some(metadata) = metadata {
            result.insert("metadata".into(), metadata.clone());
        } else {
            result.remove("metadata");
        }
        super::super::override_context::remove_metadata_fields(
            result,
            &["browser_scope", "browser_override"],
        );
        let scope = if owns_scope && let Some(origin) = &info.origin {
            let url = if let Some(url) = result.get("url") {
                url.clone()
            } else {
                info.resolved.properties["url"].clone()
            };
            Some(Scope {
                version: 1,
                url: url
                    .as_str()
                    .ok_or_else(|| copy_error("A copied browser needs a URL"))?
                    .into(),
                library: self.copied_browser_scope(origin)?,
            })
        } else {
            None
        };
        if full {
            if let Some(scope) = scope {
                let metadata = result
                    .entry("metadata")
                    .or_insert_with(|| json!({"type":"lazypencil"}))
                    .as_object_mut()
                    .ok_or_else(|| copy_error("Browser metadata must be an object"))?;
                metadata
                    .entry("lazypencil")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| copy_error("The lazypencil namespace must be an object"))?
                    .insert(
                        "browser_scope".into(),
                        serde_json::to_value(scope).expect("browser scope"),
                    );
            }
        } else if owns_scope {
            let metadata = result.remove("metadata");
            result.insert(
                "metadata".into(),
                json!({"type":"lazypencil","lazypencil":{"browser_override":{
                    "version":1,"metadata":metadata,"scope":scope
                }}}),
            );
        }
        Ok(())
    }
}
