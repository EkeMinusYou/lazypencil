//! A copied URL keeps its captured document namespace independently of metadata.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scope {
    pub version: u32,
    pub url: String,
    pub library: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Override {
    pub version: u32,
    pub metadata: Option<Value>,
    pub scope: Option<Scope>,
}

pub(super) fn override_value(changes: &Map<String, Value>) -> Result<Option<Override>> {
    let value: Option<Override> = super::override_context::parse(changes, "browser_override")?;
    if value.as_ref().is_some_and(|value| value.version != 1) {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported copied browser override version",
        ));
    }
    Ok(value)
}

impl Project {
    pub fn browser_node_resources(&self, node: &ResolvedNode) -> Result<BrowserResources> {
        if node.kind() != "browser" {
            return Err(invalid("Browser resources require a browser node"));
        }
        let url = node
            .properties
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("A browser node needs a URL"))?;
        self.browser_page_resources(node.browser_origin(), url)
    }

    pub(super) fn bind_browser_scope(&self, node: &mut ResolvedNode) -> Result<()> {
        node.browser_scope = None;
        if node.kind() != "browser" {
            return Ok(());
        }
        if node
            .properties
            .get("metadata")
            .and_then(|metadata| metadata.pointer("/lazypencil/browser_override"))
            .is_some()
        {
            return Err(invalid(
                "Copied browser override envelopes require a ref node",
            ));
        }
        let Some(value) = node
            .properties
            .get("metadata")
            .and_then(|metadata| metadata.pointer("/lazypencil/browser_scope"))
        else {
            return Ok(());
        };
        let scope: Scope = serde_json::from_value(value.clone())
            .map_err(|_| invalid("Invalid copied browser scope"))?;
        node.browser_scope =
            self.browser_scope_value(node, &scope, node.property_source("metadata"))?;
        Ok(())
    }

    pub(super) fn browser_scope_value(
        &self,
        node: &ResolvedNode,
        scope: &Scope,
        owner: &Path,
    ) -> Result<Option<PathBuf>> {
        if scope.version != 1 {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                "Unsupported copied browser scope version",
            ));
        }
        if scope.url.is_empty() || scope.url.len() > 1024 * 1024 {
            return Err(invalid("Invalid copied browser URL binding"));
        }
        if node.properties.get("url").and_then(Value::as_str) != Some(scope.url.as_str()) {
            return Ok(None);
        }
        self.scope_document(owner, &scope.library).map(Some)
    }

    pub(super) fn retain_detached_browser_scope(
        &self,
        value: &mut Value,
        original: &ResolvedNode,
    ) -> Result<()> {
        if value["type"] != "browser" {
            return Ok(());
        }
        let properties = value.as_object_mut().expect("browser");
        if let Some(metadata) = original.properties.get("metadata") {
            properties.insert("metadata".into(), metadata.clone());
        } else {
            properties.remove("metadata");
        }
        super::override_context::remove_metadata_fields(
            properties,
            &["browser_scope", "browser_override"],
        );
        if self
            .browser_resources
            .contains_key(original.browser_origin())
        {
            let scope = Scope {
                version: 1,
                url: value["url"]
                    .as_str()
                    .ok_or_else(|| invalid("A detached browser needs a URL"))?
                    .into(),
                library: self.detached_scope(original.browser_origin())?,
            };
            let namespace = super::scope::metadata_namespace(value)?;
            namespace.insert(
                "browser_scope".into(),
                serde_json::to_value(scope).expect("browser scope"),
            );
        }
        Ok(())
    }
}

impl Project {
    pub(super) fn relocate_browser_override(
        &self,
        node: &Value,
        result: &mut Map<String, Value>,
        target: Option<&ResolvedNode>,
    ) -> Result<()> {
        let Some(target) = target.filter(|target| target.kind() == "browser") else {
            return Ok(());
        };
        if node["type"] == "browser" {
            return Ok(());
        }
        let old = override_value(node.as_object().expect("browser properties"))?;
        let metadata = old
            .as_ref()
            .map_or(node.get("metadata"), |old| old.metadata.as_ref());
        let reserved = metadata
            .and_then(|metadata| metadata.pointer("/lazypencil/browser_scope"))
            .is_some();
        if node.get("url").is_none() && old.is_none() && !reserved {
            return Ok(());
        }
        let scope = if self.browser_resources.contains_key(target.browser_origin())
            && (node.get("url").is_some()
                || reserved
                || old.as_ref().is_some_and(|old| old.scope.is_some()))
        {
            let url = if let Some(url) = result.get("url") {
                url.clone()
            } else {
                self.detached_variables(target.property_source("url"), &target.properties["url"])?
            };
            Some(Scope {
                version: 1,
                url: url
                    .as_str()
                    .ok_or_else(|| invalid("A detached browser needs a URL"))?
                    .into(),
                library: self.detached_scope(target.browser_origin())?,
            })
        } else {
            None
        };
        if let Some(metadata) = metadata {
            result.insert("metadata".into(), metadata.clone());
        } else {
            result.remove("metadata");
        }
        super::override_context::remove_metadata_fields(
            result,
            &["browser_scope", "browser_override"],
        );
        let metadata = result.remove("metadata");
        result.insert(
            "metadata".into(),
            json!({"type":"lazypencil","lazypencil":{"browser_override":{
                "version":1,"metadata":metadata,"scope":scope
            }}}),
        );
        Ok(())
    }
}

pub(crate) fn revoke<'a>(
    document: &mut Document,
    pointer: &str,
    keys: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let keys = keys.into_iter().collect::<BTreeSet<_>>();
    if !keys.contains("url") || keys.contains("metadata") {
        return Ok(());
    }
    for (parent, field) in [
        (format!("{pointer}/metadata/lazypencil"), "browser_scope"),
        (
            format!("{pointer}/metadata/lazypencil/browser_override"),
            "scope",
        ),
        (
            format!("{pointer}/metadata/lazypencil/browser_override/metadata/lazypencil"),
            "browser_scope",
        ),
    ] {
        if document
            .value()
            .pointer(&format!("{parent}/{field}"))
            .is_some()
        {
            document.remove_property(&parent, field)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_scope_validates_versions_and_uses_declared_aliases_only_when_active() {
        let scene = |scope: Value| {
            Project::local(Document::parse(json!({"version":"2.21","children":[
            {"id":"page","type":"browser","url":"https://example.test/","width":32,"height":16,
             "metadata":{"type":"test","lazypencil":{"browser_scope":scope}}}
        ]}).to_string()).unwrap()).unwrap().resolve()
        };
        let mut scope =
            json!({"version":1,"url":"https://example.test/","library":"../../private.pen"});
        assert_eq!(scene(scope.clone()).unwrap_err().code, "INVALID_DOCUMENT");
        scope["url"] = json!("https://stale.test/");
        assert!(
            scene(scope.clone()).unwrap().roots[0]
                .browser_scope
                .is_none()
        );
        scope["version"] = json!(2);
        assert_eq!(
            scene(scope.clone()).unwrap_err().code,
            "UNSUPPORTED_VERSION"
        );
        scope["version"] = json!(1);
        scope["path"] = json!("/private");
        assert_eq!(scene(scope).unwrap_err().code, "INVALID_DOCUMENT");
    }
}
