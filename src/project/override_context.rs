//! Shared validation for metadata envelopes that preserve execution inheritance.

use super::*;
use serde::de::DeserializeOwned;

const CONTEXT_FIELDS: &[&str] = &[
    "script_scope",
    "script_bindings",
    "script_input_context",
    "script_override",
    "browser_scope",
    "browser_override",
];

/// An ordinary metadata edit does not replace an execution binding. Explicit
/// reserved fields still let the caller replace the context deliberately.
pub(crate) fn preserve_metadata(
    document: &mut Document,
    pointer: &str,
    metadata: Option<&Value>,
    unset: bool,
) -> Result<bool> {
    if metadata.is_none() && !unset {
        return Ok(false);
    }
    if metadata
        .and_then(|value| value.get("lazypencil"))
        .is_some_and(|namespace| {
            CONTEXT_FIELDS
                .iter()
                .any(|field| namespace.get(*field).is_some())
        })
    {
        return Ok(false);
    }
    let Some(previous) = document
        .value()
        .pointer(pointer)
        .and_then(|node| node.pointer("/metadata/lazypencil"))
    else {
        return Ok(false);
    };
    let envelope = ["script_override", "browser_override"]
        .into_iter()
        .find(|field| previous.get(*field).is_some());
    if let Some(field) = envelope {
        document.set_property(
            &format!("{pointer}/metadata/lazypencil/{field}"),
            "metadata",
            metadata.unwrap_or(&Value::Null),
        )?;
        return Ok(true);
    }
    let contexts = CONTEXT_FIELDS
        .iter()
        .filter_map(|field| {
            previous
                .get(*field)
                .map(|value| ((*field).to_owned(), value.clone()))
        })
        .collect::<Map<_, _>>();
    if contexts.is_empty() {
        return Ok(false);
    }
    let mut metadata = metadata
        .cloned()
        .unwrap_or_else(|| json!({"type":"lazypencil"}));
    metadata
        .as_object_mut()
        .ok_or_else(|| invalid("Execution metadata must be an object"))?
        .entry("lazypencil")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("The lazypencil metadata namespace must be an object"))?
        .extend(contexts);
    document.set_property(pointer, "metadata", &metadata)?;
    Ok(true)
}

pub(super) fn parse<T: DeserializeOwned>(
    changes: &Map<String, Value>,
    field: &str,
) -> Result<Option<T>> {
    let Some(value) = changes
        .get("metadata")
        .and_then(|metadata| metadata.get("lazypencil"))
        .and_then(|namespace| namespace.get(field))
    else {
        return Ok(None);
    };
    if value.to_string().len() > 4 * 1024 * 1024 {
        return Err(invalid("Copied execution override exceeds 4 MiB"));
    }
    let metadata = changes["metadata"].as_object().expect("override owner");
    if metadata.len() != 2
        || metadata.get("type").and_then(Value::as_str) != Some("lazypencil")
        || metadata["lazypencil"]
            .as_object()
            .is_none_or(|namespace| namespace.len() != 1)
    {
        return Err(invalid(
            "Copied execution override must use its dedicated metadata envelope",
        ));
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| invalid("Invalid copied execution override"))
}

pub(super) fn remove_metadata_fields(properties: &mut Map<String, Value>, fields: &[&str]) {
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
        for field in fields {
            namespace.remove(*field);
        }
        if namespace.is_empty() {
            metadata.remove("lazypencil");
        }
    }
    if metadata.len() == 1 && metadata.get("type").and_then(Value::as_str) == Some("lazypencil") {
        properties.remove("metadata");
    }
}
