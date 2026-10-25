use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptScope {
    version: u32,
    script_uri: String,
    library: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    inputs: Option<InputScope>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputScope {
    hash: String,
    library: String,
}

impl Project {
    pub(super) fn scope_document(&self, owner: &Path, alias: &str) -> Result<PathBuf> {
        if alias.len() > 4096 || alias.chars().any(char::is_control) {
            return Err(invalid("A script scope needs a declared library alias"));
        }
        let mut path = owner.to_path_buf();
        if !alias.is_empty() {
            let parts = alias.split(':').collect::<Vec<_>>();
            if parts.len() > 128 {
                return Err(invalid("A script scope exceeds the library depth limit"));
            }
            for part in parts {
                path = self
                    .imports
                    .get(&path)
                    .and_then(|imports| imports.get(part))
                    .cloned()
                    .ok_or_else(|| {
                        invalid("A script scope refers to an unknown declared library")
                    })?;
            }
        }
        Ok(path)
    }

    pub(super) fn bind_script_scope(&self, node: &mut ResolvedNode) -> Result<()> {
        node.script_scope = None;
        node.script_input_scope = None;
        node.input_context = None;
        node.script_bindings = super::bindings::bind(self, node)?;
        if node.kind() != "script" {
            return Ok(());
        }
        if let Some(value) = node
            .properties
            .get("metadata")
            .and_then(|v| v.pointer("/lazypencil/script_input_context"))
        {
            node.input_context =
                super::script_context::bind(self, node, value, node.property_source("metadata"))?;
        }
        let Some(value) = node
            .properties
            .get("metadata")
            .and_then(|metadata| metadata.get("lazypencil"))
            .and_then(|namespace| namespace.get("script_scope"))
        else {
            return Ok(());
        };
        let scope: ScriptScope = serde_json::from_value(value.clone())
            .map_err(|_| invalid("Invalid lazypencil script scope metadata"))?;
        if scope.version != 1 {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                "Unsupported lazypencil script scope version",
            ));
        }
        if scope.script_uri.len() > 4096
            || scope.inputs.as_ref().is_some_and(|inputs| {
                inputs.hash.len() != 64
                    || !inputs
                        .hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err(invalid("Invalid script scope URI or input hash"));
        }
        let owner = node.property_source("metadata");
        // Source and input edits invalidate their bindings independently.
        let origin = if node.properties.get("scriptUri").and_then(Value::as_str)
            == Some(scope.script_uri.as_str())
        {
            Some(self.scope_document(owner, &scope.library)?)
        } else {
            None
        };
        let inputs_origin = if let Some(inputs) = scope.inputs {
            if node
                .properties
                .get("inputs")
                .is_some_and(|value| value_hash(value) == inputs.hash)
            {
                Some(self.scope_document(owner, &inputs.library)?)
            } else {
                None
            }
        } else {
            None
        };
        node.script_scope = origin;
        node.script_input_scope = inputs_origin;
        Ok(())
    }

    pub(super) fn retain_detached_script_scope(
        &self,
        value: &mut Value,
        original: &ResolvedNode,
    ) -> Result<()> {
        if value["type"] != "script" {
            return Ok(());
        }
        // A materialized script owns its effective user metadata. Partial ref
        // envelopes describe inheritance and must not become ordinary metadata.
        let properties = value.as_object_mut().expect("script");
        if let Some(metadata) = original.properties.get("metadata") {
            properties.insert("metadata".into(), metadata.clone());
        } else {
            properties.remove("metadata");
        }
        if let Some(context) = &original.input_context {
            let alias = self.detached_scope(&context.origin)?;
            let mut context = context.values.clone();
            context.library = (!alias.is_empty()).then_some(alias);
            let metadata = value
                .as_object_mut()
                .expect("script")
                .entry("metadata")
                .or_insert_with(|| json!({"type":"lazypencil"}))
                .as_object_mut()
                .ok_or_else(|| invalid("Script metadata must be an object"))?;
            let namespace = metadata
                .entry("lazypencil")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| invalid("The lazypencil namespace must be an object"))?;
            namespace.insert(
                "script_input_context".into(),
                serde_json::to_value(context).expect("input context"),
            );
        } else if let Some(namespace) = value
            .pointer_mut("/metadata/lazypencil")
            .and_then(Value::as_object_mut)
        {
            namespace.remove("script_input_context");
        }
        let retained = if let Some(bound) = &original.script_bindings {
            let mut values = bound.values.clone();
            values.script_uri = value["scriptUri"]
                .as_str()
                .ok_or_else(|| invalid("A detached script requires scriptUri"))?
                .into();
            let scope = self.detached_scope(&bound.origin)?;
            values.library = (!scope.is_empty()).then_some(scope);
            // A revoked same-JSON input must not become bound again on save.
            values.inputs_hash = bound
                .inputs_bound
                .then(|| value.get("inputs").map(value_hash))
                .flatten();
            Some(serde_json::to_value(values).expect("script bindings"))
        } else {
            None
        };
        if let Some(namespace) = value
            .get_mut("metadata")
            .and_then(|v| v.get_mut("lazypencil"))
            .and_then(Value::as_object_mut)
        {
            namespace.remove("script_bindings");
            if let Some(bindings) = retained {
                namespace.insert("script_bindings".into(), bindings);
            }
        }
        let origin = original.script_origin();
        let inputs_origin = original.script_input_origin();
        if origin == self.root && (value.get("inputs").is_none() || inputs_origin == self.root) {
            return Ok(());
        }
        let scope = ScriptScope {
            version: 1,
            script_uri: value["scriptUri"]
                .as_str()
                .ok_or_else(|| invalid("A detached script requires scriptUri"))?
                .into(),
            library: self.detached_scope(origin)?,
            inputs: value
                .get("inputs")
                .map(|inputs| -> Result<InputScope> {
                    Ok(InputScope {
                        hash: value_hash(inputs),
                        library: self.detached_scope(inputs_origin)?,
                    })
                })
                .transpose()?,
        };
        let metadata = value
            .as_object_mut()
            .expect("node")
            .entry("metadata")
            .or_insert_with(|| json!({"type":"lazypencil"}))
            .as_object_mut()
            .ok_or_else(|| invalid("Script scope metadata requires an object"))?;
        let namespace = metadata
            .entry("lazypencil")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| invalid("The lazypencil metadata namespace must be an object"))?;
        namespace.insert(
            "script_scope".into(),
            serde_json::to_value(scope).expect("scope"),
        );
        Ok(())
    }
}

fn value_hash(value: &Value) -> String {
    // Object key order is immaterial to an input binding.
    let mut normalized = value.clone();
    normalized.sort_all_objects();
    hash(normalized.to_string().as_bytes())
}

pub(super) fn without_script_scope(properties: &mut Map<String, Value>) {
    let Some(metadata) = properties
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if let Some(namespace) = metadata
        .get_mut("lazypencil")
        .and_then(Value::as_object_mut)
    {
        namespace.remove("script_scope");
        if namespace.is_empty() {
            metadata.remove("lazypencil");
        }
    }
    if metadata.len() == 1 && metadata.get("type").and_then(Value::as_str) == Some("lazypencil") {
        properties.remove("metadata");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_unknown_script_scopes_cannot_resolve_arbitrary_paths() {
        for (scope, code) in [
            (
                json!({"version":2,"script_uri":"draw.js","library":""}),
                "UNSUPPORTED_VERSION",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","library":"../../outside.pen"}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","library":"file:///outside.pen"}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","library":"","extra":true}),
                "INVALID_DOCUMENT",
            ),
            (
                json!({"version":1,"script_uri":"draw.js","library":"","inputs":{"hash":"bad","library":""}}),
                "INVALID_DOCUMENT",
            ),
        ] {
            let document = Document::parse(json!({"version":"2.21","children":[{"id":"script","type":"script","scriptUri":"draw.js","metadata":{"type":"test","lazypencil":{"script_scope":scope}}}]}).to_string()).unwrap();
            let project = Project::local(document).unwrap();
            assert_eq!(project.resolve().unwrap_err().code, code);
        }
    }
}
