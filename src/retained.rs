//! Conservative accounting of owned JSON payloads, independent of serialization.
use serde_json::Value;
pub(crate) fn json_bytes(value: &Value) -> usize {
    std::mem::size_of::<Value>().saturating_add(match value {
        Value::String(value) => value.capacity(),
        Value::Array(values) => {
            values.iter().map(json_bytes).sum::<usize>()
                + (values.capacity() - values.len()) * std::mem::size_of::<Value>()
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                key.capacity()
                    .saturating_add(96)
                    .saturating_add(json_bytes(value))
            })
            .sum(),
        _ => 0,
    })
}
