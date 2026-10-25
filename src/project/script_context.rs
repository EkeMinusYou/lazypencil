//! Keep copied input expressions independent of generated-code namespaces.

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inputs {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    pub values: BTreeMap<String, Value>,
    pub variables: BTreeMap<String, String>,
    pub components: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Bound {
    pub origin: PathBuf,
    pub values: Inputs,
}

impl Bound {
    pub fn equivalent(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.values.values == other.values.values
            && self.values.variables == other.values.variables
            && self.values.components == other.values.components
    }

    pub fn expression(&self, name: &str, value: &Value) -> Result<Value> {
        let value = self.values.values.get(name).unwrap_or(value);
        if let Some(variable) = value.as_str().and_then(|v| v.strip_prefix('$'))
            && !self
                .values
                .variables
                .values()
                .any(|target| target == variable)
        {
            return Err(invalid(
                "A copied input variable is absent from its captured namespace",
            ));
        }
        Ok(value.clone())
    }

    #[cfg(feature = "rendering")]
    pub fn component<'a>(&'a self, value: &'a str) -> Result<&'a str> {
        self.values
            .components
            .get(value)
            .map(String::as_str)
            .or_else(|| {
                self.values
                    .components
                    .values()
                    .any(|target| target == value)
                    .then_some(value)
            })
            .ok_or_else(|| {
                invalid("A copied input reference is absent from its captured namespace")
            })
    }
}

pub(super) fn bind(
    project: &Project,
    node: &ResolvedNode,
    value: &Value,
    owner: &Path,
) -> Result<Option<Bound>> {
    if value.to_string().len() > 4 * 1024 * 1024 {
        return Err(invalid("Copied input context exceeds 4 MiB"));
    }
    let values: Inputs = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Invalid copied input context"))?;
    if values.version != 1 {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported copied input context version",
        ));
    }
    let valid = |v: &str| !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control);
    if values.variables.len() + values.components.len() + values.values.len() > MAX_NODES
        || values.values.keys().any(|v| !valid(v))
        || [&values.variables, &values.components]
            .into_iter()
            .flat_map(BTreeMap::iter)
            .any(|(from, to)| !valid(from) || !valid(to))
    {
        return Err(invalid("Invalid copied input context names"));
    }
    let origin = project.scope_document(owner, values.library.as_deref().unwrap_or(""));
    let raw = node.properties.get("inputs").and_then(Value::as_object);
    let matches = raw.is_some_and(|raw| {
        raw.len() == values.values.len()
            && values.values.iter().all(|(name, expected)| {
                let Some(actual) = raw.get(name) else {
                    return false;
                };
                if actual == expected {
                    return true;
                }
                if let (Some(actual), Some(expected)) = (actual.as_str(), expected.as_str())
                    && (values
                        .components
                        .get(actual)
                        .is_some_and(|target| target == expected)
                        || values.components.values().any(|v| v == expected)
                            && origin.as_ref().ok().is_some_and(|origin| {
                                project
                                    .lookup_node(node.property_source("inputs"), actual)
                                    .ok()
                                    .zip(project.lookup_node(origin, expected).ok())
                                    .is_some_and(|((a, first), (b, second))| {
                                        a == b && first["id"] == second["id"]
                                    })
                            }))
                {
                    return true;
                }
                let Some(actual) = actual.as_str().and_then(|v| v.strip_prefix('$')) else {
                    return false;
                };
                let Some(expected) = expected.as_str().and_then(|v| v.strip_prefix('$')) else {
                    return false;
                };
                values
                    .variables
                    .get(actual)
                    .is_some_and(|target| target == expected)
                    || project
                        .variable_key(node.property_source("inputs"), actual)
                        .ok()
                        .zip(
                            origin
                                .as_ref()
                                .ok()
                                .and_then(|origin| project.variable_key(origin, expected).ok()),
                        )
                        .is_some_and(|(a, b)| a == b)
            })
    });
    if !matches {
        return Ok(None);
    }
    let origin = origin?;
    for target in values.components.values() {
        project.check_cancelled()?;
        if project.documents[&origin].node(target)?["reusable"] != true {
            return Err(invalid("A copied input target must be reusable"));
        }
    }
    for target in values.variables.values() {
        project.check_cancelled()?;
        if project.documents[&origin].value()["variables"]
            .get(target)
            .is_none()
        {
            return Err(invalid("A copied input variable is missing"));
        }
    }
    Ok(Some(Bound { origin, values }))
}

pub(crate) fn revoke<'a>(
    document: &mut Document,
    pointer: &str,
    keys: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let keys = keys.into_iter().collect::<BTreeSet<_>>();
    if keys.contains("metadata") || !keys.contains("inputs") && !keys.contains("scriptUri") {
        return Ok(());
    }
    let root = format!("{pointer}/metadata/lazypencil");
    if ![
        "script_input_context",
        "script_bindings",
        "script_scope",
        "script_override",
    ]
    .into_iter()
    .any(|field| {
        document
            .value()
            .pointer(&format!("{root}/{field}"))
            .is_some()
    }) {
        return Ok(());
    }
    for namespace in [
        root.clone(),
        format!("{root}/script_override/metadata/lazypencil"),
    ] {
        let mut fields = Vec::new();
        if keys.contains("inputs") {
            fields.push("script_input_context");
        }
        if keys.contains("scriptUri") {
            fields.extend(["script_bindings", "script_scope"]);
        }
        for field in fields {
            if document
                .value()
                .pointer(&format!("{namespace}/{field}"))
                .is_some()
            {
                document.remove_property(&namespace, field)?;
            }
        }
        if keys.contains("inputs") {
            for (parent, field) in [
                ("script_bindings", "inputs_hash"),
                ("script_scope", "inputs"),
            ] {
                let path = format!("{namespace}/{parent}");
                if document
                    .value()
                    .pointer(&format!("{path}/{field}"))
                    .is_some()
                {
                    document.remove_property(&path, field)?;
                }
            }
        }
    }
    let envelope = format!("{root}/script_override");
    for (key, field) in [("inputs", "input"), ("scriptUri", "output")] {
        if keys.contains(key)
            && document
                .value()
                .pointer(&format!("{envelope}/{field}"))
                .is_some()
        {
            document.remove_property(&envelope, field)?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Override {
    version: u32,
    pub metadata: Option<Value>,
    pub output: Option<Value>,
    pub input: Option<Value>,
}

pub(super) fn metadata_contexts(metadata: Option<&Value>) -> (bool, bool) {
    let namespace = metadata.and_then(|value| value.get("lazypencil"));
    let output = namespace.is_some_and(|value| {
        value.get("script_bindings").is_some() || value.get("script_scope").is_some()
    });
    let input = namespace.is_some_and(|value| {
        value.get("script_input_context").is_some()
            || value
                .pointer("/script_scope/inputs")
                .is_some_and(|v| !v.is_null())
            || value
                .pointer("/script_bindings/inputs_hash")
                .is_some_and(|v| !v.is_null())
    });
    (output, input)
}

pub(super) fn override_value(changes: &Map<String, Value>) -> Result<Option<Override>> {
    let Some(value) = changes
        .get("metadata")
        .and_then(|v| v.pointer("/lazypencil/script_override"))
    else {
        return Ok(None);
    };
    if value.to_string().len() > 4 * 1024 * 1024 {
        return Err(invalid("Copied script override exceeds 4 MiB"));
    }
    let metadata = changes["metadata"].as_object().expect("override owner");
    if metadata.len() != 2
        || metadata.get("type").and_then(Value::as_str) != Some("lazypencil")
        || metadata["lazypencil"]
            .as_object()
            .is_none_or(|v| v.len() != 1)
    {
        return Err(invalid(
            "Copied script override must use its dedicated metadata envelope",
        ));
    }
    let value: Override = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Invalid copied script override"))?;
    if value.version != 1 {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported copied script override version",
        ));
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Value {
        json!({"version":1,"values":{"selected":"copy"},"variables":{},"components":{"old":"copy"}})
    }

    fn scene(context: Value, selected: &str) -> Result<ResolvedScene> {
        Project::local(Document::parse(json!({"version":"2.21","children":[
            {"id":"copy","type":"rectangle","reusable":true,"width":10,"height":10},
            {"id":"ordinary","type":"rectangle","width":10,"height":10},
            {"id":"script","type":"script","scriptUri":"draw.js","width":10,"height":10,"inputs":{"selected":selected},
             "metadata":{"type":"test","lazypencil":{"script_input_context":context}}}
        ]}).to_string())?)?.resolve()
    }

    #[test]
    fn input_context_validates_active_targets_and_ignores_stale_namespace_destinations() {
        assert!(
            scene(context(), "old").unwrap().roots[2]
                .input_context
                .is_some()
        );
        for (section, value) in [
            ("components", json!({"old":"ordinary"})),
            ("variables", json!({"old":"missing"})),
            ("library", json!("undeclared")),
        ] {
            let mut context = context();
            context[section] = value;
            assert!(scene(context.clone(), "copy").is_err(), "{context}");
            assert!(
                scene(context, "caller").unwrap().roots[2]
                    .input_context
                    .is_none()
            );
        }
        let mut unknown = context();
        unknown["version"] = json!(2);
        assert_eq!(
            scene(unknown, "caller").unwrap_err().code,
            "UNSUPPORTED_VERSION"
        );
        let mut malformed = context();
        malformed["unexpected"] = json!(true);
        assert_eq!(
            scene(malformed, "caller").unwrap_err().code,
            "INVALID_DOCUMENT"
        );
    }

    #[test]
    fn overriding_code_and_inputs_together_revokes_all_contexts_without_changing_user_metadata() {
        for (carrier, input_context) in [(false, false), (false, true), (true, true)] {
            let mut namespace =
                json!({"script_bindings":{"inputs_hash":"old"},"script_scope":{"inputs":{}}});
            if input_context {
                namespace["script_input_context"] = context();
            }
            let metadata = if carrier {
                json!({"type":"lazypencil","lazypencil":{"script_override":{"version":1,"metadata":{"type":"test","keep":[1,2],"lazypencil":namespace},"input":context(),"output":{"version":1}}}})
            } else {
                json!({"type":"test","keep":[1,2],"lazypencil":namespace})
            };
            let mut document = Document::parse(json!({"version":"2.21","children":[{"id":"script","type":"script","scriptUri":"draw.js","width":10,"height":10,"metadata":metadata}]}).to_string()).unwrap();
            revoke(&mut document, "/children/0", ["inputs", "scriptUri"]).unwrap();
            let metadata = &document.node("script").unwrap()["metadata"];
            let ordinary = if carrier {
                let envelope = &metadata["lazypencil"]["script_override"];
                assert!(envelope.get("input").is_none() && envelope.get("output").is_none());
                &envelope["metadata"]
            } else {
                metadata
            };
            assert_eq!(ordinary["keep"], json!([1, 2]));
            for key in ["script_input_context", "script_bindings", "script_scope"] {
                assert!(ordinary["lazypencil"].get(key).is_none());
            }
        }
    }
}
