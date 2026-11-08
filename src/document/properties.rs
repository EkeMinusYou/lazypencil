//! Apply disjoint property edits with one CST capture and one final validation.
use super::*;
use std::ops::Range;

pub(crate) struct PropertyEdit {
    pub pointer: String,
    pub set: serde_json::Map<String, Value>,
    pub unset: Vec<String>,
}

impl Document {
    pub(crate) fn edit_properties(&mut self, edits: &[PropertyEdit]) -> Result<()> {
        if edits
            .iter()
            .all(|edit| edit.set.is_empty() && edit.unset.is_empty())
        {
            return Ok(());
        }
        let root = parse_cst(&self.text)?;
        let objects = edits
            .iter()
            .map(|edit| {
                cst_at(&root, &edit.pointer)?
                    .as_object()
                    .ok_or_else(|| invalid("Property target is not an object"))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut selected = Vec::new();
        for object in &objects {
            selected.push(CstNode::from(object.clone()));
            for property in object.properties() {
                let node = CstNode::from(property.clone());
                selected.push(node.clone());
                if let Some(value) = property.value() {
                    selected.push(value);
                }
                if let Some(comma) = node.trailing_comma() {
                    selected.push(comma.into());
                }
                if let Some(comma) = node
                    .previous_siblings()
                    .find(|s| !s.is_trivia())
                    .filter(CstNode::is_comma)
                {
                    selected.push(comma);
                }
            }
        }
        let spans = Spans::new(&root, &selected);
        let newline = if self.text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut changes = Vec::new();
        for (edit, object) in edits.iter().zip(objects) {
            let removed = edit
                .unset
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let mut additions = Vec::new();
            let mut previous = None;
            for property in object.properties() {
                let key = property
                    .decoded_name()
                    .ok_or_else(|| invalid("Invalid property name"))?;
                let node = CstNode::from(property.clone());
                if removed.contains(key.as_str()) {
                    changes.push((spans.get(&node), String::new()));
                    if let Some(comma) = node.trailing_comma() {
                        changes.push((spans.get(&comma.into()), String::new()));
                    } else if let Some(comma) = node
                        .previous_siblings()
                        .find(|s| !s.is_trivia())
                        .filter(CstNode::is_comma)
                    {
                        changes.push((spans.get(&comma), String::new()));
                    }
                } else {
                    previous = Some(spans.get(&node));
                    if let Some(value) = edit.set.get(&key) {
                        changes.push((
                            spans.get(
                                &property
                                    .value()
                                    .ok_or_else(|| invalid("Missing property value"))?,
                            ),
                            value.to_string(),
                        ));
                    }
                }
            }
            for (key, value) in &edit.set {
                if removed.contains(key.as_str()) || object.get(key).is_none() {
                    additions.push(format!("{}: {value}", Value::String(key.clone())));
                }
            }
            if !additions.is_empty() {
                let container = spans.get(&object.into());
                let separator = separator_at(&self.text, &container, previous.as_ref(), newline);
                let content = additions.join(&format!(",{separator}"));
                let (position, addition) = if let Some(previous) = previous {
                    (previous.end, format!(",{separator}{content}"))
                } else if self.text[container.clone()].contains('\n') {
                    (
                        container.end - 1,
                        format!(
                            "{separator}{content}\n{}",
                            indent_at(&self.text, container.start)
                        ),
                    )
                } else {
                    (container.end - 1, content)
                };
                changes.push((position..position, addition));
            }
        }
        changes.sort_by_key(|(range, _)| (range.start, range.end));
        changes.dedup();
        for pair in changes.windows(2) {
            if pair[0].0.end > pair[1].0.start {
                return Err(invalid(
                    "Property edits overlap; apply dependent changes separately",
                ));
            }
        }
        self.replace_ranges(changes)
    }
}

fn separator_at(
    text: &str,
    container: &Range<usize>,
    previous: Option<&Range<usize>>,
    newline: &str,
) -> String {
    if !text[container.clone()].contains('\n') {
        return " ".into();
    }
    let base = indent_at(text, container.start);
    let indent = previous
        .map(|range| indent_at(text, range.start))
        .filter(|indent| indent.len() > base.len())
        .unwrap_or_else(|| format!("{base}  "));
    format!("{newline}{indent}")
}

/// Capture selected offsets in one leaf walk instead of rescanning preceding siblings
/// for every property. CST paths are stable for this immutable preparation phase.
struct Spans(BTreeMap<Vec<usize>, Range<usize>>);
impl Spans {
    fn path(node: &CstNode) -> Vec<usize> {
        let mut path = Vec::new();
        let mut node = node.clone();
        while let Some(parent) = node.parent() {
            path.push(node.child_index());
            node = parent.into();
        }
        path.reverse();
        path
    }
    fn new(root: &CstRootNode, selected: &[CstNode]) -> Self {
        let mut spans = Self(
            selected
                .iter()
                .map(|node| (Self::path(node), 0..0))
                .collect(),
        );
        spans.walk(&root.clone().into(), &mut Vec::new(), 0);
        spans
    }
    fn walk(&mut self, node: &CstNode, path: &mut Vec<usize>, start: usize) -> usize {
        let children = node.children();
        let mut end = start;
        if children.is_empty() {
            end += node.to_string().len();
        } else {
            for (index, child) in children.iter().enumerate() {
                path.push(index);
                end = self.walk(child, path, end);
                path.pop();
            }
        }
        if let Some(range) = self.0.get_mut(path) {
            *range = start..end;
        }
        end
    }
    fn get(&self, node: &CstNode) -> Range<usize> {
        self.0[&Self::path(node)].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_properties_preserve_values_comments_and_unrelated_source_for_every_removal_subset() {
        for newline in ["\n", "\r\n"] {
            for trailing in ["", ","] {
                let source = format!("{{\n\"version\":\"2.22\",\"children\":[],\n\"metadata\":{{\n // 日本語\n \"a\":1.000e+3, /* keep */ \"b\":2, \"c\":3{trailing}\n}},\"untouched\":1.200e+4\n}}\n").replace('\n', newline);
                for mask in 0..8 {
                    for set in [json!({}), json!({"a":7,"new/key~":"日本語"})] {
                        let mut document = Document::parse(source.clone()).unwrap();
                        let mut sequential = document.clone();
                        let unset = ["a", "b", "c"]
                            .into_iter()
                            .enumerate()
                            .filter(|(i, _)| mask & (1 << i) != 0)
                            .map(|(_, key)| key.to_string())
                            .collect::<Vec<_>>();
                        for key in &unset {
                            sequential.remove_property("/metadata", key).unwrap();
                        }
                        for (key, value) in set.as_object().unwrap() {
                            sequential.set_property("/metadata", key, value).unwrap();
                        }
                        document
                            .edit_properties(&[PropertyEdit {
                                pointer: "/metadata".into(),
                                set: set.as_object().unwrap().clone(),
                                unset,
                            }])
                            .unwrap();
                        assert_eq!(
                            document.value(),
                            sequential.value(),
                            "mask={mask}, source={source}"
                        );
                        for retained in ["// 日本語", "/* keep */", "\"untouched\":1.200e+4"] {
                            assert!(document.text().contains(retained));
                        }
                        if mask & 1 == 0 && set.get("a").is_none() {
                            assert!(document.text().contains("1.000e+3"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn bulk_properties_keep_parent_child_offsets_and_leave_failed_edits_atomic() {
        let source = r#"{"version":"2.22","children":[{"id":"parent","type":"frame","width":1,"children":[{"id":"child","type":"rectangle","width":2}]}]}"#;
        let mut document = Document::parse(source.into()).unwrap();
        let edits = [("parent", 10), ("child", 20)]
            .into_iter()
            .map(|(id, width)| PropertyEdit {
                pointer: document.location(id).unwrap().pointer.clone(),
                set: json!({"width":width,"name":"日本語"})
                    .as_object()
                    .unwrap()
                    .clone(),
                unset: vec![],
            })
            .collect::<Vec<_>>();
        document.edit_properties(&edits).unwrap();
        assert_eq!(document.node("parent").unwrap()["width"], 10);
        assert_eq!(document.node("child").unwrap()["width"], 20);
        let saved = document.text().to_string();
        assert!(
            document
                .edit_properties(&[PropertyEdit {
                    pointer: "/children/0".into(),
                    set: json!({"children":null}).as_object().unwrap().clone(),
                    unset: vec![]
                }])
                .is_err()
        );
        assert_eq!(document.text(), saved);
    }
}
