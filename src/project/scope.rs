use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScriptScope {
    pub version: u32,
    pub script_uri: String,
    pub library: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<InputScope>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InputScope {
    pub hash: String,
    pub library: String,
}

pub(super) fn parse_scope(value: &Value) -> Result<ScriptScope> {
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
    Ok(scope)
}

/// Drop only a provably inactive generated scope before URI remapping can make
/// its old spelling match a copied asset. Ref inputs may still be inherited.
#[cfg(feature = "rendering")]
pub(super) fn discard_inactive_generated_scope(node: &mut Value) -> Result<()> {
    let Some(value) = node.pointer("/metadata/lazypencil/script_scope") else {
        return Ok(());
    };
    let scope = parse_scope(value)?;
    let output_active =
        node.get("scriptUri").and_then(Value::as_str) == Some(scope.script_uri.as_str());
    let inputs_inactive = scope.inputs.as_ref().is_none_or(|inputs| {
        node.get("inputs")
            .map(|value| value_hash(value) != inputs.hash)
            .unwrap_or(node["type"] == "script")
    });
    if !output_active && inputs_inactive {
        node.pointer_mut("/metadata/lazypencil")
            .and_then(Value::as_object_mut)
            .expect("validated script scope namespace")
            .remove("script_scope");
    }
    Ok(())
}

impl Project {
    pub(super) fn scope_document(&self, owner: &Path, alias: &str) -> Result<PathBuf> {
        if alias.len() > 4096 || alias.chars().any(char::is_control) {
            return Err(invalid("An execution scope needs a declared library alias"));
        }
        let mut path = owner.to_path_buf();
        if !alias.is_empty() {
            let parts = alias.split(':').collect::<Vec<_>>();
            if parts.len() > 128 {
                return Err(invalid(
                    "An execution scope exceeds the library depth limit",
                ));
            }
            for part in parts {
                path = self
                    .imports
                    .get(&path)
                    .and_then(|imports| imports.get(part))
                    .cloned()
                    .ok_or_else(|| {
                        invalid("An execution scope refers to an unknown declared library")
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
        let scope = parse_scope(value)?;
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
            metadata_namespace(value)?.insert(
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
            let mut values = bound.original().clone();
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
        if let Some(bindings) = retained {
            metadata_namespace(value)?.insert("script_bindings".into(), bindings);
        } else if let Some(namespace) = value
            .get_mut("metadata")
            .and_then(|v| v.get_mut("lazypencil"))
            .and_then(Value::as_object_mut)
        {
            namespace.remove("script_bindings");
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
        metadata_namespace(value)?.insert(
            "script_scope".into(),
            serde_json::to_value(scope).expect("scope"),
        );
        Ok(())
    }
}

pub(super) fn metadata_namespace(node: &mut Value) -> Result<&mut Map<String, Value>> {
    node.as_object_mut()
        .expect("node")
        .entry("metadata")
        .or_insert_with(|| json!({"type":"lazypencil"}))
        .as_object_mut()
        .ok_or_else(|| invalid("Node metadata must be an object"))?
        .entry("lazypencil")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("The lazypencil metadata namespace must be an object"))
}

pub(super) fn value_hash(value: &Value) -> String {
    // Object key order is immaterial to an input binding.
    let mut normalized = value.clone();
    normalized.sort_all_objects();
    hash(normalized.to_string().as_bytes())
}

pub(super) fn without_script_scope(properties: &mut Map<String, Value>) {
    super::override_context::remove_metadata_fields(properties, &["script_scope"]);
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
