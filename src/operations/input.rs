//! Actionable errors shared by CLI and MCP editing inputs.
use super::*;
use serde::de::Error as _;

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged, deny_unknown_fields)]
        enum Address {
            Id(String),
            Result {
                result_of: String,
            },
            Node {
                node_id: String,
                #[serde(default)]
                instance_path: Vec<String>,
            },
        }
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<Address>(value) {
            Ok(Address::Id(id)) => Ok(Target::Id(id)),
            Ok(Address::Result { result_of }) => Ok(Target::Result { result_of }),
            Ok(Address::Node {
                node_id,
                instance_path,
            }) => Ok(Target::Node {
                node_id,
                instance_path,
            }),
            Err(_) => Err(D::Error::custom(
                "target/parent must be an ID string, {\"result_of\":\"client_ref\"}, or {\"node_id\":\"stored ID\",\"instance_path\":[\"child ID\"]}; node_id must be a string. For a new instance, insert it in one batch and use its returned binding for descendant overrides in the next batch",
            )),
        }
    }
}

pub(super) fn parse_operations(value: Value) -> Result<Vec<Operation>> {
    let values: Vec<Value> = serde_json::from_value(value)
        .map_err(|_| Error::new("INVALID_ARGUMENT", "operations must be an array"))?;
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            serde_json::from_value(value).map_err(|error| {
                let mut result =
                    Error::new("INVALID_ARGUMENT", format!("operations[{index}]: {error}"));
                result.details = Some(json!({"operation_index": index}));
                result
            })
        })
        .collect()
}

pub(super) fn deserialize_operations<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Operation>, D::Error> {
    parse_operations(Value::deserialize(deserializer)?).map_err(D::Error::custom)
}

pub(super) fn operation_error(mut error: Error, index: usize) -> Error {
    error.message = format!("operations[{index}]: {}", error.message);
    let mut details = error.details.take().unwrap_or_else(|| json!({}));
    if let Some(object) = details.as_object_mut() {
        object.insert("operation_index".into(), json!(index));
    }
    error.details = Some(details);
    error
}

pub(super) fn check_update_fields(set: &Map<String, Value>, unset: &[String]) -> Result<()> {
    if let Some(key) = set
        .keys()
        .chain(unset.iter())
        .find(|key| matches!(key.as_str(), "id" | "type" | "children" | "descendants"))
    {
        let mut error = invalid(format!(
            "update cannot change {key:?}; use insert/delete/move for stored children, set_override for instance properties, or replace_override/set_slot_children for instance structure"
        ));
        error.details = Some(json!({"field": key, "guidance": "operations"}));
        return Err(error);
    }
    Ok(())
}
