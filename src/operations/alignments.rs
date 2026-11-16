//! Preserve existing legacy spellings while emitting canonical new properties.
use super::*;
use crate::document::PropertyEdit;

pub(super) fn canonicalize_new_values(
    before: &Document,
    after: &mut Document,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let mut edits = Vec::new();
    for (id, location) in after.locations() {
        crate::cancel::check(cancel)?;
        collect(
            after
                .value()
                .pointer(&location.pointer)
                .expect("indexed node"),
            before.node(id).ok(),
            &location.pointer,
            &mut edits,
        );
    }
    after.edit_properties(&edits)
}

fn collect(node: &Value, before: Option<&Value>, pointer: &str, edits: &mut Vec<PropertyEdit>) {
    if node.get("textAlignVertical").and_then(Value::as_str) == Some("center")
        && before
            .and_then(|node| node.get("textAlignVertical"))
            .and_then(Value::as_str)
            != Some("center")
    {
        edits.push(PropertyEdit {
            pointer: pointer.into(),
            set: serde_json::from_value(json!({"textAlignVertical":"middle"}))
                .expect("property map"),
            unset: Vec::new(),
        });
    }
    if let Some(overrides) = node.get("descendants").and_then(Value::as_object) {
        for (path, changes) in overrides {
            // Replacement nodes and their children have their own index entries.
            if changes.get("type").is_none() {
                collect(
                    changes,
                    before
                        .and_then(|node| node.get("descendants"))
                        .and_then(|overrides| overrides.get(path)),
                    &format!("{pointer}/descendants/{}", pointer_key(path)),
                    edits,
                );
            }
        }
    }
}
