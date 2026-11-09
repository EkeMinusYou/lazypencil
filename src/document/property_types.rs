//! Check boolean attributes without interpreting open metadata or input maps.
use super::*;

const VARIABLE_BOOLEANS: &[&str] = &[
    "enabled",
    "flipX",
    "flipY",
    "clip",
    "underline",
    "strikethrough",
];
const LITERAL_BOOLEANS: &[&str] = &["reusable", "layoutIncludeStroke", "placeholder"];

pub(crate) fn validate_authored_booleans(node: &Value, id: &str) -> Result<()> {
    validate_properties(node.as_object().expect("indexed object"), id, true)?;
    if matches!(node.get("type").and_then(Value::as_str), None | Some("ref"))
        && let Some(overrides) = node.get("descendants").and_then(Value::as_object)
    {
        for (path, changes) in overrides {
            // Replacement nodes and their children have their own document index entries.
            if changes.get("type").is_none() {
                validate_authored_booleans(changes, &format!("{id}/{path}"))?;
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_resolved_booleans(
    properties: &serde_json::Map<String, Value>,
    id: &str,
) -> Result<()> {
    validate_properties(properties, id, false)
}

fn validate_properties(
    properties: &serde_json::Map<String, Value>,
    id: &str,
    allow_variables: bool,
) -> Result<()> {
    for key in VARIABLE_BOOLEANS {
        check(
            properties.get(*key),
            || format!("{id}.{key}"),
            allow_variables,
        )?;
    }
    for key in LITERAL_BOOLEANS {
        check(properties.get(*key), || format!("{id}.{key}"), false)?;
    }
    for key in ["fill", "stroke", "effect"] {
        if let Some(value) = properties.get(key) {
            if let Some(values) = value.as_array() {
                for (index, value) in values.iter().enumerate() {
                    check(
                        value.get("enabled"),
                        || format!("{id}.{key}[{index}].enabled"),
                        allow_variables,
                    )?;
                }
            } else {
                check(
                    value.get("enabled"),
                    || format!("{id}.{key}.enabled"),
                    allow_variables,
                )?;
            }
        }
    }
    Ok(())
}

fn check(
    value: Option<&Value>,
    attribute: impl FnOnce() -> String,
    allow_variables: bool,
) -> Result<()> {
    if let Some(value) = value
        && !value.is_boolean()
        && !(allow_variables
            && value
                .as_str()
                .is_some_and(|value| value.starts_with('$') && value.len() > 1))
    {
        return Err(Error::new(
            "INVALID_DOCUMENT",
            format!(
                "{} must be a boolean{}",
                attribute(),
                if allow_variables { " or variable" } else { "" }
            ),
        ));
    }
    Ok(())
}
