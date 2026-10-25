//! Remap a script's generated references without rewriting its JavaScript.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Bindings {
    version: u32,
    script_uri: String,
    #[serde(default)]
    inputs_hash: Option<String>,
    #[serde(default)]
    components: BTreeMap<String, String>,
    #[serde(default)]
    variables: BTreeMap<String, String>,
    #[serde(default)]
    themes: BTreeMap<String, String>,
    #[serde(default)]
    descendants: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Bound {
    #[cfg(feature = "rendering")]
    pub(super) origin: PathBuf,
    pub(super) inputs_bound: bool,
    #[cfg(feature = "rendering")]
    values: Bindings,
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
    let maps = [&values.components, &values.variables, &values.themes];
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
        #[cfg(feature = "rendering")]
        origin,
        inputs_bound,
        #[cfg(feature = "rendering")]
        values,
    }))
}

#[cfg(feature = "rendering")]
impl Bound {
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
    ) -> Result<()> {
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
                self.rewrite(child, handles)?;
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
                self.rewrite(&mut properties, handles)?;
                if rewritten.insert(path, properties).is_some() {
                    return Err(invalid(
                        "Generated override paths collide after script binding",
                    ));
                }
            }
            *changes = rewritten;
        }
        Ok(())
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
