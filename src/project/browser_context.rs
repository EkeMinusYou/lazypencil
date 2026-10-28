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

impl Scope {
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                "Unsupported copied browser scope version",
            ));
        }
        if self.url.is_empty() || self.url.len() > 1024 * 1024 {
            return Err(invalid("Invalid copied browser URL binding"));
        }
        Ok(())
    }
}

#[cfg(feature = "rendering")]
pub(super) fn has_context(node: &Value) -> bool {
    node.pointer("/metadata/lazypencil")
        .is_some_and(|namespace| {
            namespace.get("browser_scope").is_some() || namespace.get("browser_override").is_some()
        })
}

/// Reference preparation derives target types/themes before granting any
/// generated browser scope. Full nodes have already been remapped by rewrite.
#[cfg(feature = "rendering")]
pub(super) fn has_preview_contexts(node: &Value) -> bool {
    ((node["type"] == "ref" || node.get("type").is_none()) && has_context(node))
        || node
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| children.iter().any(has_preview_contexts))
        || node
            .get("descendants")
            .and_then(Value::as_object)
            .is_some_and(|changes| changes.values().any(has_preview_contexts))
}

#[cfg(feature = "rendering")]
pub(super) fn clear_preview_contexts(node: &mut Value) {
    if (node["type"] == "ref" || node.get("type").is_none())
        && let Some(properties) = node.as_object_mut()
    {
        super::override_context::remove_metadata_fields(
            properties,
            &["browser_scope", "browser_override"],
        );
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            clear_preview_contexts(child);
        }
    }
    if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
        for change in changes.values_mut() {
            clear_preview_contexts(change);
        }
    }
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

#[cfg(feature = "rendering")]
impl bindings::Bound {
    pub(super) fn generated_browser_context(
        &self,
        node: &mut Value,
        full: bool,
        inherited_bytes: &mut usize,
    ) -> Result<()> {
        self.generated_browser_context_at(node, full, None, inherited_bytes)
    }

    pub(super) fn generated_browser_context_at(
        &self,
        node: &mut Value,
        full: bool,
        inherited_url: Option<&str>,
        inherited_bytes: &mut usize,
    ) -> Result<()> {
        if self.values.assets.is_none() || node["enabled"] == false {
            return Ok(());
        }
        if node.get("url").is_none() && !has_context(node) {
            return Ok(());
        }
        let owns_url = node.get("url").is_some();
        let url = match node.get("url") {
            Some(value) => value.as_str(),
            None => inherited_url,
        }
        .ok_or_else(|| invalid("A generated browser needs a URL string"))?
        .to_owned();
        let old = override_value(node.as_object().expect("generated browser properties"))?;
        if full && old.is_some() {
            return Err(invalid(
                "Copied browser override envelopes require a ref node",
            ));
        }
        let mut metadata = old.as_ref().map_or_else(
            || node.get("metadata").cloned(),
            |value| value.metadata.clone(),
        );
        let mut explicit = None;
        if let Some(metadata) = &metadata {
            if !metadata.is_object() {
                return Err(invalid("Generated browser metadata must be an object"));
            }
            if let Some(value) = metadata.pointer("/lazypencil/browser_scope") {
                let scope: Scope = serde_json::from_value(value.clone())
                    .map_err(|_| invalid("Invalid copied browser scope"))?;
                scope.validate()?;
                explicit = Some(scope);
            }
        }
        if let Some(scope) = old.and_then(|value| value.scope) {
            scope.validate()?;
            explicit = Some(scope);
        }
        if !owns_url && explicit.is_none() {
            return Ok(());
        }
        if !owns_url && explicit.as_ref().is_some_and(|scope| scope.url != url) {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "An inactive generated metadata-only browser scope needs the original URL ownership context",
            ));
        }
        let scope = if let Some(mut scope) = explicit.filter(|scope| scope.url == url) {
            let map = self.values.browser_libraries.as_ref().ok_or_else(|| Error::new(
                    "UNSUPPORTED_FEATURE", "This copied script predates explicit browser scope capture; copy it again from its source"
                ))?;
            scope.library = map.get(&scope.library).cloned().or_else(|| {
                    map.values().any(|alias| alias == &scope.library).then(|| scope.library.clone())
                }).ok_or_else(|| Error::new("ASSET_NOT_FOUND", "A generated browser scope is outside the copied script resource namespaces"))?;
            Some(scope)
        } else if let Some(library) = &self.values.browser_library {
            Some(Scope {
                version: 1,
                url: url.clone(),
                library: library.clone(),
            })
        } else {
            if url::Url::parse(&url).is_ok_and(|url| url.scheme() == "file") {
                return Err(Error::new(
                    "ASSET_NOT_FOUND",
                    "A generated local browser needs the script's captured resource namespace",
                ));
            }
            None
        };
        if let Some(user_metadata) = metadata.take() {
            let mut properties = Map::from_iter([("metadata".into(), user_metadata)]);
            super::override_context::remove_metadata_fields(
                &mut properties,
                &["browser_scope", "browser_override"],
            );
            // The envelope retains the user's metadata; reserved contexts are
            // carried separately and cannot be resolved in the caller's scope.
            metadata = properties.remove("metadata");
        }
        if let Some(scope) = &scope {
            *inherited_bytes = inherited_bytes
                .saturating_add(serde_json::to_vec(scope).expect("browser scope").len() + 256);
            if *inherited_bytes > crate::script::MAX_OUTPUT_BYTES {
                return Err(invalid(
                    "Inherited browser scopes exceed the generated output budget",
                ));
            }
        }
        if full {
            let properties = node.as_object_mut().expect("generated browser properties");
            if let Some(metadata) = metadata {
                properties.insert("metadata".into(), metadata);
            } else {
                properties.remove("metadata");
            }
            if let Some(scope) = scope {
                super::scope::metadata_namespace(node)?.insert(
                    "browser_scope".into(),
                    serde_json::to_value(scope).expect("browser scope"),
                );
            }
        } else {
            node.as_object_mut()
                .expect("generated browser properties")
                .insert(
                    "metadata".into(),
                    json!({"type":"lazypencil","lazypencil":{"browser_override":{
                        "version":1,"metadata":metadata,"scope":scope
                    }}}),
                );
        }
        Ok(())
    }
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
        scope.validate()?;
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
                target.properties["url"].clone()
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

    #[cfg(feature = "rendering")]
    #[test]
    fn generated_browser_scopes_preserve_metadata_bound_aliases_and_output_limits() {
        let mut bound = bindings::Bound {
            origin: PathBuf::from("/project/main.pen"),
            inputs_bound: false,
            values: serde_json::from_value(json!({"version":1,"script_uri":"draw.js","assets":{},"browser_library":"captured"})).unwrap(),
        };
        let mut node = json!({"type":"browser","url":"FILE:///page.html?literal=$keep#anchor","metadata":{"type":"test","keep":"$keep"}});
        let original = node.clone();
        let mut bytes = 0;
        bound
            .generated_browser_context(&mut node, true, &mut bytes)
            .unwrap();
        assert_eq!(node["metadata"]["keep"], "$keep");
        assert_eq!(
            node.pointer("/metadata/lazypencil/browser_scope/library")
                .unwrap(),
            "captured"
        );
        assert_eq!(
            node.pointer("/metadata/lazypencil/browser_scope/url")
                .unwrap(),
            &original["url"]
        );
        assert!(bytes > 0);
        let mut bytes = crate::script::MAX_OUTPUT_BYTES;
        assert_eq!(
            bound
                .generated_browser_context(&mut original.clone(), true, &mut bytes)
                .unwrap_err()
                .code,
            "INVALID_DOCUMENT"
        );
        let mut partial = json!({"url":"https://example.test/"});
        bound.values.browser_library = None;
        bound
            .generated_browser_context(&mut partial, false, &mut 0)
            .unwrap();
        assert!(
            partial
                .pointer("/metadata/lazypencil/browser_override/metadata")
                .unwrap()
                .is_null()
        );
        assert!(
            partial
                .pointer("/metadata/lazypencil/browser_override/scope")
                .unwrap()
                .is_null()
        );
        assert_eq!(
            bound
                .generated_browser_context(&mut original.clone(), true, &mut 0)
                .unwrap_err()
                .code,
            "ASSET_NOT_FOUND"
        );
        bound.values.browser_library = Some("captured".into());
        assert_eq!(
            bound
                .generated_browser_context(&mut node, true, &mut 0)
                .unwrap_err()
                .code,
            "UNSUPPORTED_FEATURE"
        );
    }

    #[cfg(feature = "rendering")]
    #[test]
    fn explicit_generated_browser_contexts_keep_alias_authority_and_inactive_url_semantics() {
        let bound = bindings::Bound {
            origin: PathBuf::from("/project/main.pen"),
            inputs_bound: false,
            values: serde_json::from_value(json!({"version":1,"script_uri":"draw.js","assets":{},
                "browser_library":"root-copy","browser_libraries":{"":"root-copy","base:leaf":"leaf-copy"}})).unwrap(),
        };
        let url = "file:///page.html?literal=$keep#anchor";
        for alias in ["base:leaf", "leaf-copy", ""] {
            let scope = json!({"version":1,"url":url,"library":alias});
            for envelope in [false, true] {
                let metadata = json!({"type":"test","keep":"$literal"});
                let mut changes = if envelope {
                    json!({"metadata":{"type":"lazypencil","lazypencil":{"browser_override":{"version":1,"metadata":metadata,"scope":scope}}}})
                } else {
                    json!({"metadata":{"type":"test","keep":"$literal","lazypencil":{"browser_scope":scope}}})
                };
                bound
                    .generated_browser_context_at(&mut changes, false, Some(url), &mut 0)
                    .unwrap();
                let rewritten = override_value(changes.as_object().unwrap())
                    .unwrap()
                    .unwrap();
                assert_eq!(rewritten.metadata.unwrap(), metadata);
                let scope = rewritten.scope.unwrap();
                assert_eq!(scope.url, url);
                assert_eq!(
                    scope.library,
                    if alias.is_empty() {
                        "root-copy"
                    } else {
                        "leaf-copy"
                    }
                );
            }
        }
        let mut stale = json!({"metadata":{"type":"test","lazypencil":{"browser_scope":{"version":1,"url":"file:///old.html","library":"not-declared"}}}});
        assert_eq!(
            bound
                .generated_browser_context_at(&mut stale, false, Some(url), &mut 0)
                .unwrap_err()
                .code,
            "UNSUPPORTED_FEATURE"
        );
        stale["url"] = json!(url);
        bound
            .generated_browser_context(&mut stale, false, &mut 0)
            .unwrap();
        assert_eq!(
            override_value(stale.as_object().unwrap())
                .unwrap()
                .unwrap()
                .scope
                .unwrap()
                .library,
            "root-copy"
        );
        for (scope, code) in [
            (
                json!({"version":1,"url":url,"library":"not-declared"}),
                "ASSET_NOT_FOUND",
            ),
            (
                json!({"version":1,"url":url,"library":"../outside.pen"}),
                "ASSET_NOT_FOUND",
            ),
            (
                json!({"version":2,"url":"file:///old.html","library":""}),
                "UNSUPPORTED_VERSION",
            ),
            (
                json!({"version":1,"url":"","library":""}),
                "INVALID_DOCUMENT",
            ),
        ] {
            let mut node = json!({"type":"browser","url":url,"metadata":{"lazypencil":{"browser_scope":scope}}});
            assert_eq!(
                bound
                    .generated_browser_context(&mut node, true, &mut 0)
                    .unwrap_err()
                    .code,
                code
            );
        }
        let mut node = json!({"type":"browser","url":url,"metadata":{"type":"lazypencil","lazypencil":{"browser_override":{"version":1,"scope":null,"metadata":null}}}});
        assert_eq!(
            bound
                .generated_browser_context(&mut node, true, &mut 0)
                .unwrap_err()
                .code,
            "INVALID_DOCUMENT"
        );
        let mut node = json!({"type":"browser","url":url,"metadata":{"lazypencil":{"browser_scope":{"version":1,"url":url,"library":"base:leaf"}}}});
        let mut bytes = crate::script::MAX_OUTPUT_BYTES;
        assert_eq!(
            bound
                .generated_browser_context(&mut node, true, &mut bytes)
                .unwrap_err()
                .code,
            "INVALID_DOCUMENT"
        );
    }

    #[test]
    fn browser_urls_do_not_expand_dollar_prefixed_strings() {
        let project = Project::local(
            Document::parse(
                json!({"version":"2.21","children":[
                    {"id":"page","type":"browser","url":"$not_a_variable","width":32,"height":16}
                ]})
                .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            project.resolve().unwrap().roots[0].properties["url"],
            "$not_a_variable"
        );
    }

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
