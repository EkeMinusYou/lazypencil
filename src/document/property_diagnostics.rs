//! Non-destructive diagnostics for unrecognized authored node properties.
use super::*;

// Shared across node types: refs may override any property of their definition.
// Metadata, inputs and other open maps are intentionally not inspected as nodes.
const NODE_PROPERTIES: &[&str] = &[
    "id",
    "type",
    "name",
    "context",
    "reusable",
    "theme",
    "enabled",
    "opacity",
    "flipX",
    "flipY",
    "layoutPosition",
    "metadata",
    "rotation",
    "x",
    "y",
    "width",
    "height",
    "layout",
    "gap",
    "layoutIncludeStroke",
    "padding",
    "justifyContent",
    "alignItems",
    "fill",
    "stroke",
    "strokeWidth",
    "strokeAlignment",
    "strokeLinecap",
    "strokeLinejoin",
    "effect",
    "cornerRadius",
    "innerRadius",
    "startAngle",
    "sweepAngle",
    "polygonCount",
    "fillRule",
    "geometry",
    "viewBox",
    "fontFamily",
    "fontSize",
    "fontWeight",
    "letterSpacing",
    "fontStyle",
    "underline",
    "lineHeight",
    "textAlign",
    "textAlignVertical",
    "strikethrough",
    "href",
    "content",
    "textGrowth",
    "children",
    "clip",
    "placeholder",
    "slot",
    "color",
    "library",
    "icon",
    "weight",
    "scriptUri",
    "inputs",
    "url",
    "deviceId",
    "zoom",
    "scrollX",
    "scrollY",
    "ref",
    "descendants",
];

pub(crate) fn node_property_diagnostics(
    properties: &serde_json::Map<String, Value>,
    node_id: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for key in properties.keys() {
        if NODE_PROPERTIES.contains(&key.as_str()) {
            continue;
        }
        let suggestion = NODE_PROPERTIES
            .iter()
            .find(|known| one_edit_apart(key, known));
        let hint = suggestion
            .map(|known| format!(" Did you mean {known:?}?"))
            .unwrap_or_default();
        diagnostics.push(Diagnostic {
            code: "UNKNOWN_PROPERTY".into(),
            severity: "warning",
            node_id: Some(node_id.into()),
            message: format!(
                "Property {key:?} is not recognized; it is preserved but may have no effect.{hint}"
            ),
        });
    }
    diagnostics
}

// Include adjacent transpositions without an unbounded edit-distance matrix.
fn one_edit_apart(value: &str, known: &str) -> bool {
    if value.len().abs_diff(known.len()) > 1 || !value.is_ascii() {
        return false;
    }
    let (a, b) = (value.as_bytes(), known.as_bytes());
    let prefix = a.iter().zip(b).take_while(|(a, b)| a == b).count();
    if a.len() == b.len() {
        prefix == a.len()
            || a[prefix + 1..] == b[prefix + 1..]
            || prefix + 1 < a.len()
                && a[prefix] == b[prefix + 1]
                && a[prefix + 1] == b[prefix]
                && a[prefix + 2..] == b[prefix + 2..]
    } else if a.len() > b.len() {
        a[prefix + 1..] == b[prefix..]
    } else {
        a[prefix..] == b[prefix + 1..]
    }
}

impl Document {
    pub(crate) fn property_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for (id, location) in &self.nodes {
            let node = self.value.pointer(&location.pointer).expect("indexed node");
            if NODE_TYPES.contains(&node["type"].as_str().unwrap_or_default()) {
                diagnostics.extend(node_property_diagnostics(node.as_object().unwrap(), id));
                override_diagnostics(node, id, &mut diagnostics);
            }
        }
        diagnostics
    }
}

fn override_diagnostics(node: &Value, owner: &str, diagnostics: &mut Vec<Diagnostic>) {
    if let Some(overrides) = node.get("descendants").and_then(Value::as_object) {
        for (path, changes) in overrides {
            // Replacement nodes and their children are already in the document index.
            if changes.get("type").is_none() {
                let address = format!("{owner}/{path}");
                if let Some(properties) = changes.as_object() {
                    diagnostics.extend(node_property_diagnostics(properties, &address));
                }
                override_diagnostics(changes, &address, diagnostics);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_warnings_preserve_extensions_and_ignore_open_map_keys() {
        let source = json!({"version":"2.22","children":[
            {"id":"title","type":"text","fontSize":24,"fontSzie":42,"futureFeature":true,
             "metadata":{"fontSzie":9},"inputs":{"custom":1}},
            {"id":"instance","type":"ref","ref":"title","descendants":{"label":{"fontWieght":"700"}}},
            {"id":"future","type":"future","anything":true}
        ]}).to_string();
        let document = Document::parse(source.clone()).unwrap();
        let warnings = document.property_diagnostics();
        assert_eq!(warnings.len(), 3);
        assert!(
            warnings
                .iter()
                .all(|warning| warning.code == "UNKNOWN_PROPERTY" && warning.severity == "warning")
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("Did you mean \"fontSize\"?"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.node_id.as_deref() == Some("instance/label"))
        );
        assert_eq!(document.text(), source);
        assert_eq!(document.node("title").unwrap()["fontSize"], 24);
    }

    #[test]
    fn typo_suggestions_handle_insertions_deletions_case_and_transpositions() {
        for typo in ["fontSzie", "fontSiz", "fontSizee", "fontsize"] {
            assert!(one_edit_apart(typo, "fontSize"), "{typo}");
        }
        for other in ["unrelated", "日本語", "", "fontSzzzz"] {
            assert!(!one_edit_apart(other, "fontSize"), "{other}");
        }
    }
}
