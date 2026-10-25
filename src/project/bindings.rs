//! Remap a script's generated references without rewriting its JavaScript.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bindings {
    pub(super) version: u32,
    pub(super) script_uri: String,
    #[serde(default)]
    pub(super) inputs_hash: Option<String>,
    #[serde(default)]
    pub(super) components: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) input_components: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) variables: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) themes: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) descendants: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) assets: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Bound {
    pub(super) origin: PathBuf,
    pub(super) inputs_bound: bool,
    pub(super) values: Bindings,
}

fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

pub(super) fn bind(project: &Project, node: &ResolvedNode) -> Result<Option<Bound>> {
    if node.kind() != "script" {
        return Ok(None);
    }
    let Some(value) = node
        .properties
        .get("metadata")
        .and_then(|value| value.get("lazypencil"))
        .and_then(|value| value.get("script_bindings"))
    else {
        return Ok(None);
    };
    if value.to_string().len() > 4 * 1024 * 1024 {
        return Err(invalid("Script bindings exceed 4 MiB"));
    }
    let values: Bindings = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Invalid lazypencil script bindings"))?;
    if values.version != 1 {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported script bindings version",
        ));
    }
    let mut maps = vec![&values.components, &values.variables, &values.themes];
    if let Some(inputs) = &values.input_components {
        maps.push(inputs);
    }
    if let Some(assets) = &values.assets {
        maps.push(assets);
    }
    let count = maps.iter().map(|map| map.len()).sum::<usize>()
        + values
            .descendants
            .values()
            .map(BTreeMap::len)
            .sum::<usize>();
    if count > MAX_NODES
        || !valid_name(&values.script_uri)
        || maps
            .iter()
            .flat_map(|map| map.iter())
            .any(|(key, value)| !valid_name(key) || !valid_name(value))
        || values.descendants.iter().any(|(key, paths)| {
            !values.components.contains_key(key)
                || paths
                    .iter()
                    .any(|(from, to)| !valid_name(from) || !valid_name(to))
        })
        || values.inputs_hash.as_ref().is_some_and(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(invalid("Invalid or oversized script binding names"));
    }
    // An explicit source override must not revive a stale binding, even when
    // another document uses an identical URI. Same-value instance overrides
    // are separately revoked by apply_properties.
    if node.properties.get("scriptUri").and_then(Value::as_str) != Some(values.script_uri.as_str())
    {
        return Ok(None);
    }
    let origin = node.property_source("metadata").to_path_buf();
    let document = &project.documents[&origin];
    for target in values.components.values() {
        project.check_cancelled()?;
        if document.node(target)?["reusable"] != true {
            return Err(invalid(
                "A script binding must target a local reusable component",
            ));
        }
    }
    if let Some(inputs) = &values.input_components {
        for target in inputs.values() {
            if document.node(target)?["reusable"] != true {
                return Err(invalid(
                    "A saved input binding must target a local reusable component",
                ));
            }
        }
    }
    if let Some(assets) = &values.assets {
        for key in assets.keys() {
            let (kind, uri) = key
                .split_once(':')
                .ok_or_else(|| invalid("A script asset binding needs a kind and URI"))?;
            if asset_key(kind, uri)? != *key {
                return Err(invalid("Script asset binding keys must be normalized"));
            }
        }
    }
    for (section, names) in [("variables", &values.variables), ("themes", &values.themes)] {
        for target in names.values() {
            project.check_cancelled()?;
            if document.value()[section].get(target).is_none() {
                return Err(invalid(
                    "A script binding target is missing from its document",
                ));
            }
        }
    }
    let inputs_bound = values.inputs_hash.as_ref().is_some_and(|expected| {
        node.properties.get("inputs").is_some_and(|inputs| {
            let mut inputs = inputs.clone();
            inputs.sort_all_objects();
            hash(inputs.to_string().as_bytes()) == *expected
        })
    });
    Ok(Some(Bound {
        origin,
        inputs_bound,
        values,
    }))
}

#[cfg(feature = "rendering")]
impl Bound {
    pub(super) fn input_component<'a>(&'a self, name: &'a str, saved: bool) -> Result<&'a str> {
        if saved && let Some(map) = &self.values.input_components {
            return map
                .get(name)
                .map(String::as_str)
                .or_else(|| map.values().any(|value| value == name).then_some(name))
                .ok_or_else(|| {
                    invalid("A saved input reference is absent from the script bindings")
                });
        }
        self.component(name)
    }

    fn asset(&self, kind: &str, uri: &str) -> Result<String> {
        let Some(assets) = &self.values.assets else {
            return Ok(uri.into());
        };
        if uri.is_empty() {
            return Ok(uri.into());
        }
        assets.get(&asset_key(kind,uri)?).cloned()
            .or_else(||assets.iter().any(|(key,target)|key.starts_with(&format!("{kind}:")) && target==uri).then(||uri.to_owned()))
            .ok_or_else(||Error::new("ASSET_NOT_FOUND","A generated file is outside the copied script asset manifest; include it in script_assets when copying"))
    }

    fn assets(&self, node: &mut Value) -> Result<()> {
        if self.values.assets.is_none() || node["enabled"] == false {
            return Ok(());
        }
        if node["type"] == "browser"
            && node
                .get("href")
                .and_then(Value::as_str)
                .is_some_and(|uri| uri.starts_with("file:"))
        {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "Copied file browser URLs need an independent page-resource closure",
            ));
        }
        if node["type"] == "ref" && node.get("scriptUri").is_some() {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "Generated source overrides require a separate script binding scope",
            ));
        }
        if let Some(uri) = node.get_mut("scriptUri")
            && let Some(value) = uri.as_str()
        {
            *uri = json!(self.asset("script", value)?);
        }
        for property in ["fill", "stroke", "effect"] {
            if let Some(value) = node.get_mut(property) {
                for fill in shader_values_mut(value) {
                    if fill["enabled"] == false {
                        continue;
                    }
                    let kind = match fill["type"].as_str() {
                        Some("image") => "image",
                        Some("shader") => "shader",
                        _ => continue,
                    };
                    if let Some(uri) = fill.get_mut("url")
                        && let Some(value) = uri.as_str()
                    {
                        *uri = json!(self.asset(kind, value)?);
                    }
                    if kind == "shader"
                        && let Some(uniforms) =
                            fill.get_mut("uniforms").and_then(Value::as_object_mut)
                    {
                        for value in uniforms.values_mut().flat_map(shader_values_mut) {
                            if let Some(uri) = value.as_str().filter(|uri| {
                                !uri.is_empty()
                                    && !uri.starts_with(['$', '#'])
                                    && !uri.starts_with("pencil:")
                            }) {
                                *value = json!(self.asset("image", uri)?);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn component<'a>(&'a self, name: &'a str) -> Result<&'a str> {
        self.values
            .components
            .get(name)
            .map(String::as_str)
            .or_else(|| {
                self.values
                    .components
                    .values()
                    .any(|target| target == name)
                    .then_some(name)
            })
            .ok_or_else(|| {
                invalid("A generated component reference is absent from the script bindings")
            })
    }

    fn value(&self, value: &mut Value) -> Result<()> {
        match value {
            Value::String(text) if text.starts_with('$') && text.len() > 1 => {
                let name = &text[1..];
                let target = self
                    .values
                    .variables
                    .get(name)
                    .or_else(|| {
                        self.values
                            .variables
                            .values()
                            .find(|target| target.as_str() == name)
                    })
                    .ok_or_else(|| {
                        invalid("A generated variable reference is absent from the script bindings")
                    })?;
                *text = format!("${target}");
            }
            Value::Array(values) => {
                for value in values {
                    self.value(value)?;
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    if key == "uniforms" || key == "inputs" {
                        if let Some(values) = value.as_object_mut() {
                            for value in values.values_mut() {
                                self.value(value)?;
                            }
                        } else {
                            self.value(value)?;
                        }
                    } else if key != "children" && !literal_property(key) {
                        self.value(value)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn rewrite(
        &self,
        node: &mut Value,
        handles: &BTreeMap<String, (PathBuf, String)>,
        inherited_bytes: &mut usize,
    ) -> Result<()> {
        self.assets(node)?;
        let original_reference = node.get("ref").and_then(Value::as_str).map(str::to_owned);
        if node["type"] == "ref" {
            let name = original_reference
                .as_deref()
                .ok_or_else(|| invalid("A generated ref needs its target"))?;
            if !handles.contains_key(name) {
                node["ref"] = json!(self.component(name)?);
            }
        }
        if let Some(theme) = node.get_mut("theme").and_then(Value::as_object_mut) {
            let mut remapped = Map::new();
            for (axis, value) in std::mem::take(theme) {
                let target = self.values.themes.get(&axis).cloned().unwrap_or(axis);
                if remapped.insert(target, value).is_some() {
                    return Err(invalid("Generated theme axes collide after script binding"));
                }
            }
            *theme = remapped;
        }
        self.value(node)?;
        if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
            for child in children {
                self.rewrite(child, handles, inherited_bytes)?;
            }
        }
        if let Some(slot) = node.get_mut("slot").and_then(Value::as_array_mut) {
            for value in slot {
                if let Some(target) = value
                    .as_str()
                    .and_then(|name| self.values.components.get(name))
                {
                    *value = json!(target);
                }
            }
        }
        if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
            let paths = original_reference
                .as_ref()
                .and_then(|name| self.values.descendants.get(name));
            let mut rewritten = Map::new();
            for (path, mut properties) in std::mem::take(changes) {
                let path = if original_reference
                    .as_ref()
                    .is_some_and(|name| self.values.components.contains_key(name))
                {
                    paths
                        .and_then(|paths| paths.get(&path))
                        .cloned()
                        .ok_or_else(|| {
                            invalid("A generated override path is absent from the script bindings")
                        })?
                } else {
                    path
                };
                self.rewrite(&mut properties, handles, inherited_bytes)?;
                if rewritten.insert(path, properties).is_some() {
                    return Err(invalid(
                        "Generated override paths collide after script binding",
                    ));
                }
            }
            *changes = rewritten;
        }
        if node["type"] == "script" && self.values.assets.is_some() && node["enabled"] != false {
            *inherited_bytes = inherited_bytes
                .saturating_add(serde_json::to_vec(&self.values).expect("bindings").len())
                .saturating_add(8192);
            if *inherited_bytes > crate::script::MAX_OUTPUT_BYTES {
                return Err(invalid(
                    "Inherited script bindings exceed the generated output budget",
                ));
            }
            let mut values = self.values.clone();
            values.script_uri = node["scriptUri"]
                .as_str()
                .ok_or_else(|| invalid("A generated script needs scriptUri"))?
                .into();
            values.input_components = None;
            values.inputs_hash = node.get("inputs").map(|value| {
                let mut value = value.clone();
                value.sort_all_objects();
                hash(value.to_string().as_bytes())
            });
            let metadata = node
                .as_object_mut()
                .expect("script")
                .entry("metadata")
                .or_insert_with(|| json!({"type":"lazypencil"}))
                .as_object_mut()
                .ok_or_else(|| invalid("Generated script metadata must be an object"))?;
            let namespace = metadata
                .entry("lazypencil")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| invalid("Generated script namespace must be an object"))?;
            if namespace.contains_key("script_scope") || namespace.contains_key("script_bindings") {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Generated explicit script scopes need independent scope composition",
                ));
            }
            namespace.insert(
                "script_bindings".into(),
                serde_json::to_value(values).expect("bindings"),
            );
        }
        Ok(())
    }
}

pub(super) fn asset_key(kind: &str, uri: &str) -> Result<String> {
    if !matches!(kind, "image" | "shader" | "script")
        || !valid_name(uri)
        || uri.contains("://")
        || uri.starts_with("file:")
    {
        return Err(invalid(
            "A copied script asset needs a local file URI and known kind",
        ));
    }
    let path = Path::new(uri);
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // a/.. may follow a symlink. Preserve that traversal spelling;
                // the copy planner separately records its verified target.
                parts.push("..".to_owned());
            }
            std::path::Component::RootDir => {}
            std::path::Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| invalid("Script asset paths must be UTF-8"))?
                    .to_owned(),
            ),
            _ => return Err(invalid("Unsupported script asset path prefix")),
        }
    }
    Ok(format!(
        "{kind}:{}{}",
        if path.is_absolute() { "/" } else { "" },
        parts.join("/")
    ))
}

#[cfg(test)]
mod asset_tests {
    use super::*;

    #[test]
    fn asset_keys_remove_dot_spelling_without_merging_parent_traversal_origins() {
        assert_eq!(
            asset_key("image", "./pixel.png").unwrap(),
            "image:pixel.png"
        );
        assert_eq!(
            asset_key("script", "../nested.js").unwrap(),
            "script:../nested.js"
        );
        assert_ne!(
            asset_key("image", "a/../pixel.png").unwrap(),
            asset_key("image", "pixel.png").unwrap()
        );
        assert_eq!(
            asset_key("image", "a/../pixel.png").unwrap(),
            "image:a/../pixel.png"
        );
    }

    #[test]
    fn asset_keys_do_not_grant_schemes_unknown_kinds_or_control_paths() {
        for (kind, uri) in [
            ("image", "https://example.com/image.png"),
            ("script", "file:///outside.js"),
            ("binary", "asset.bin"),
            ("image", "bad\nname.png"),
            ("image", ""),
        ] {
            assert!(asset_key(kind, uri).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(bindings: Value, uri: &str) -> Project {
        Project::local(Document::parse(json!({"version":"2.21", "variables":{"copied.tone":{"type":"color","value":"#0000ff"}},
            "themes":{"copied.mode":["light","dark"]},"children":[
                {"id":"copy","type":"rectangle","reusable":true,"width":10,"height":10},
                {"id":"not-reusable","type":"rectangle","width":10,"height":10},
                {"id":"generator","type":"script","scriptUri":uri,"width":10,"height":10,
                 "inputs":{"selected":"old"},"metadata":{"type":"example","lazypencil":{"script_bindings":bindings}}}
            ]}).to_string()).unwrap()).unwrap()
    }

    fn bindings() -> Value {
        json!({"version":1,"script_uri":"draw.js","components":{"old":"copy"},
            "variables":{"old.tone":"copied.tone"},"themes":{"mode":"copied.mode"}})
    }

    #[test]
    fn binding_validation_requires_local_targets_and_a_known_bounded_format() {
        for (value, code) in [
            (
                json!({"version":2,"script_uri":"draw.js"}),
                "UNSUPPORTED_VERSION",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","unexpected":true}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","components":{"old":"missing"}}),
                "INVALID_OPERATION",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","components":{"old":"not-reusable"}}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","variables":{"old":"missing"}}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","themes":{"old":"missing"}}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","inputs_hash":"bad"}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","descendants":{"unknown":{"old":"new"}}}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","variables":{"":"copied.tone"}}),
                "INVALID_DOCUMENT",
            ),
        ] {
            let result = project(value.clone(), "draw.js").resolve();
            assert_eq!(result.unwrap_err().code, code, "{value}");
        }
    }

    #[test]
    fn changed_code_disables_stale_targets_but_does_not_bypass_format_validation() {
        let mut value = bindings();
        value["components"]["old"] = json!("removed");
        let scene = project(value, "replacement.js").resolve().unwrap();
        assert!(scene.roots[2].script_bindings.is_none());
        let mut value = bindings();
        value["version"] = json!(9);
        assert_eq!(
            project(value, "replacement.js").resolve().unwrap_err().code,
            "UNSUPPORTED_VERSION"
        );
    }

    #[test]
    fn saved_input_bindings_are_bound_to_canonical_json_and_cannot_grant_paths() {
        let mut value = bindings();
        value["inputs_hash"] = json!(hash(br#"{"selected":"old"}"#));
        let scene = project(value.clone(), "draw.js").resolve().unwrap();
        assert!(
            scene.roots[2]
                .script_bindings
                .as_ref()
                .unwrap()
                .inputs_bound
        );
        value["inputs_hash"] = json!("0".repeat(64));
        let scene = project(value, "draw.js").resolve().unwrap();
        assert!(
            !scene.roots[2]
                .script_bindings
                .as_ref()
                .unwrap()
                .inputs_bound
        );
        let mut value = bindings();
        value["components"]["old"] = json!("../outside.pen:component");
        assert!(project(value, "draw.js").resolve().is_err());
    }
}
