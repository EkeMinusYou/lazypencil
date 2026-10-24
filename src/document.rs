use crate::{Error, Result};
use jsonc_parser::{
    ParseOptions,
    cst::{CstNode, CstRootNode},
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

mod codec;
pub use codec::{Codec, SUPPORTED_DOCUMENT_VERSIONS};

pub const DOCUMENT_VERSION: &str = Codec::DEFAULT.version();
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_JSON_DEPTH: usize = 128;
pub const MAX_NODES: usize = 100_000;

pub const NODE_TYPES: &[&str] = &[
    "frame",
    "group",
    "rectangle",
    "ellipse",
    "polygon",
    "path",
    "text",
    "note",
    "prompt",
    "context",
    "icon",
    "script",
    "browser",
    "ref",
];

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NodeLocation {
    pub pointer: String,
    pub parent_id: Option<String>,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub struct Document {
    text: String,
    value: Value,
    nodes: BTreeMap<String, NodeLocation>,
}

impl Document {
    pub fn parse(text: String) -> Result<Self> {
        check_input(&text)?;
        let root = parse_cst(&text)?;
        let node = root
            .value()
            .ok_or_else(|| invalid("The document is empty"))?;
        reject_duplicate_keys(&node)?;
        let value = node
            .to_serde_value()
            .ok_or_else(|| invalid("Invalid JSON value"))?;
        if !value.is_object() {
            return Err(invalid("A .pen document must be an object"));
        }
        if !value.get("version").is_some_and(Value::is_string) {
            return Err(invalid("The document must declare its version"));
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("The document must contain a children array"))?;
        let mut nodes = BTreeMap::new();
        index_nodes(children, "/children", None, &mut nodes)?;
        Ok(Self { text, value, nodes })
    }

    pub fn empty() -> Self {
        Self::parse(format!(
            "{{\n  \"version\": \"{DOCUMENT_VERSION}\",\n  \"children\": []\n}}\n"
        ))
        .expect("the built-in empty document is valid")
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn value(&self) -> &Value {
        &self.value
    }
    pub fn revision(&self) -> String {
        hash(self.text.as_bytes())
    }
    pub fn version(&self) -> &str {
        self.value["version"].as_str().unwrap_or_default()
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn locations(&self) -> &BTreeMap<String, NodeLocation> {
        &self.nodes
    }

    pub fn codec(&self) -> Option<Codec> {
        Codec::parse(self.version())
    }

    pub fn ensure_writable(&self) -> Result<()> {
        if self.codec().is_none() {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                format!(
                    "Version {} is read-only; writable versions are {}",
                    self.version(),
                    SUPPORTED_DOCUMENT_VERSIONS.join(", ")
                ),
            ));
        }
        Ok(())
    }

    pub fn node(&self, id: &str) -> Result<&Value> {
        let location = self.location(id)?;
        self.value
            .pointer(&location.pointer)
            .ok_or_else(|| invalid("The node index is inconsistent"))
    }

    pub fn location(&self, id: &str) -> Result<&NodeLocation> {
        self.nodes
            .get(id)
            .ok_or_else(|| Error::new("INVALID_OPERATION", format!("Node {id:?} does not exist")))
    }

    pub fn known_node(&self, id: &str) -> Result<&Value> {
        let node = self.node(id)?;
        if !NODE_TYPES.contains(&node["type"].as_str().unwrap_or_default()) {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                format!("Cannot edit unknown node type at {id}"),
            ));
        }
        Ok(node)
    }

    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut result = Vec::new();
        if self.codec().is_none() {
            result.push(Diagnostic {
                code: "UNSUPPORTED_VERSION".into(),
                severity: "error",
                message: format!("Version {} is read-only", self.version()),
                node_id: None,
            });
        }
        for (id, location) in &self.nodes {
            let node = self.value.pointer(&location.pointer).expect("indexed node");
            if !NODE_TYPES.contains(&node["type"].as_str().unwrap_or_default()) {
                result.push(Diagnostic {
                    code: "UNSUPPORTED_FEATURE".into(),
                    severity: "warning",
                    message: format!("Unknown node type {} is preserved", node["type"]),
                    node_id: Some(id.clone()),
                });
            }
        }
        result
    }

    pub fn summary(&self) -> Value {
        json!({
            "version": self.version(), "revision": self.revision(),
            "node_count": self.node_count(), "writable": self.codec().is_some(),
            "roots": self.value["children"].as_array().into_iter().flatten().map(|n| json!({
                "id": n["id"], "name": n.get("name"), "type": n["type"],
            })).collect::<Vec<_>>(), "diagnostics": self.diagnostics(),
        })
    }

    pub fn raw(&self, pointer: &str) -> Result<String> {
        let root = parse_cst(&self.text)?;
        Ok(cst_at(&root, pointer)?.to_string())
    }

    pub fn set_property(&mut self, object_pointer: &str, key: &str, value: &Value) -> Result<()> {
        self.set_property_raw(object_pointer, key, &value.to_string())
    }

    pub fn set_property_raw(&mut self, object_pointer: &str, key: &str, raw: &str) -> Result<()> {
        let root = parse_cst(&self.text)?;
        let object = cst_at(&root, object_pointer)?
            .as_object()
            .ok_or_else(|| invalid("Property target is not an object"))?;
        if let Some(prop) = object.get(key) {
            let node = prop
                .value()
                .ok_or_else(|| invalid("Missing property value"))?;
            self.replace_ranges(vec![(span(&node), raw.to_string())])
        } else {
            let properties = object.properties();
            let content = format!("{}: {raw}", Value::String(key.to_string()));
            let node: CstNode = object.into();
            let previous = properties.last().map(|p| CstNode::from(p.clone()));
            self.append_content(&node, previous.as_ref(), &content)
        }
    }

    pub fn remove_property(&mut self, object_pointer: &str, key: &str) -> Result<()> {
        let root = parse_cst(&self.text)?;
        let object = cst_at(&root, object_pointer)?
            .as_object()
            .ok_or_else(|| invalid("Property target is not an object"))?;
        if let Some(property) = object.get(key) {
            self.remove_cst_node(&property.into())?;
        }
        Ok(())
    }

    pub fn insert_element(&mut self, array_pointer: &str, index: usize, raw: &str) -> Result<()> {
        let root = parse_cst(&self.text)?;
        let array = cst_at(&root, array_pointer)?
            .as_array()
            .ok_or_else(|| invalid("Insertion target is not an array"))?;
        let elements = array.elements();
        if index > elements.len() {
            return Err(Error::new(
                "INVALID_OPERATION",
                "Insertion index is out of range",
            ));
        }
        if let Some(next) = elements.get(index) {
            let position = span(next).start;
            let separator = separator(&self.text, &array.clone().into(), Some(next));
            self.replace_ranges(vec![(position..position, format!("{raw},{separator}"))])
        } else {
            self.append_content(&array.into(), elements.last(), raw)
        }
    }

    pub fn remove_element(&mut self, pointer: &str) -> Result<()> {
        let root = parse_cst(&self.text)?;
        let node = cst_at(&root, pointer)?;
        if node.parent().is_none_or(|p| p.as_array().is_none()) {
            return Err(Error::new(
                "INVALID_OPERATION",
                "Deletion target is not an array element",
            ));
        }
        self.remove_cst_node(&node)
    }

    fn remove_cst_node(&mut self, node: &CstNode) -> Result<()> {
        let mut changes = vec![(span(node), String::new())];
        if let Some(comma) = node.trailing_comma() {
            changes.push((span(&comma.into()), String::new()));
        } else if let Some(comma) = node
            .previous_siblings()
            .find(|s| !s.is_trivia())
            .filter(CstNode::is_comma)
        {
            changes.push((span(&comma), String::new()));
        }
        self.replace_ranges(changes)
    }

    fn append_content(
        &mut self,
        container: &CstNode,
        previous: Option<&CstNode>,
        content: &str,
    ) -> Result<()> {
        let (position, addition) = if let Some(previous) = previous {
            (
                span(previous).end,
                format!(
                    ",{}{content}",
                    separator(&self.text, container, Some(previous))
                ),
            )
        } else {
            let range = span(container);
            let position = range.end - 1;
            let multiline = self.text[range.clone()].contains('\n');
            let addition = if multiline {
                format!(
                    "{}{}\n{}",
                    separator(&self.text, container, None),
                    content,
                    indent_at(&self.text, range.start)
                )
            } else {
                content.to_string()
            };
            (position, addition)
        };
        self.replace_ranges(vec![(position..position, addition)])
    }

    fn replace_ranges(&mut self, mut changes: Vec<(std::ops::Range<usize>, String)>) -> Result<()> {
        changes.sort_by_key(|change| std::cmp::Reverse(change.0.start));
        let mut text = self.text.clone();
        for (range, replacement) in changes {
            text.replace_range(range, &replacement);
        }
        *self = Self::parse(text)?;
        Ok(())
    }
}

pub fn hash(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}

pub fn pointer_key(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_DOCUMENT", message)
}

fn parse_cst(text: &str) -> Result<CstRootNode> {
    let options = ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    };
    CstRootNode::parse(text, &options).map_err(|e| invalid(e.to_string()))
}

fn check_input(text: &str) -> Result<()> {
    if text.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("Document exceeds the 64 MiB limit"));
    }
    let bytes = text.as_bytes();
    let (mut i, mut depth) = (0, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        break;
                    }
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                    i += 1;
                }
                i += 1;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_DEPTH {
                    return Err(invalid("JSON nesting exceeds the depth limit"));
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

fn reject_duplicate_keys(root: &CstNode) -> Result<()> {
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if let Some(object) = node.as_object() {
            let mut names = HashSet::new();
            for prop in object.properties() {
                let name = prop
                    .decoded_name()
                    .ok_or_else(|| invalid("Invalid property name"))?;
                if !names.insert(name.clone()) {
                    return Err(invalid(format!("Duplicate property {name:?}")));
                }
            }
        }
        stack.extend(node.children());
    }
    Ok(())
}

fn index_nodes(
    children: &[Value],
    pointer: &str,
    parent: Option<&str>,
    nodes: &mut BTreeMap<String, NodeLocation>,
) -> Result<()> {
    for (index, node) in children.iter().enumerate() {
        index_node(node, &format!("{pointer}/{index}"), index, parent, nodes)?;
    }
    Ok(())
}

fn index_node(
    node: &Value,
    path: &str,
    index: usize,
    parent: Option<&str>,
    nodes: &mut BTreeMap<String, NodeLocation>,
) -> Result<()> {
    let id = node
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && !s.contains('/'))
        .ok_or_else(|| invalid(format!("{path} requires a nonempty ID without '/'")))?;
    if !node.get("type").is_some_and(Value::is_string) {
        return Err(invalid(format!("Node {id} requires a type")));
    }
    if nodes
        .insert(
            id.into(),
            NodeLocation {
                pointer: path.into(),
                parent_id: parent.map(str::to_string),
                index,
            },
        )
        .is_some()
    {
        return Err(invalid(format!("Duplicate node ID {id:?}")));
    }
    if nodes.len() > MAX_NODES {
        return Err(invalid("Document exceeds the node limit"));
    }
    if let Some(children) = node.get("children") {
        let children = children
            .as_array()
            .ok_or_else(|| invalid(format!("Node {id} has non-array children")))?;
        index_nodes(children, &format!("{path}/children"), Some(id), nodes)?;
    }
    if node["type"] == "ref" {
        index_overrides(node, path, Some(id), nodes)?;
    }
    Ok(())
}

fn index_overrides(
    node: &Value,
    path: &str,
    parent: Option<&str>,
    nodes: &mut BTreeMap<String, NodeLocation>,
) -> Result<()> {
    if let Some(overrides) = node.get("descendants") {
        let overrides = overrides
            .as_object()
            .ok_or_else(|| invalid("descendants must be an object"))?;
        for (key, changes) in overrides {
            if !changes.is_object() {
                return Err(invalid("Descendant overrides must be objects"));
            }
            let pointer = format!("{path}/descendants/{}", pointer_key(key));
            if changes.get("type").is_some() {
                index_node(changes, &pointer, 0, parent, nodes)?;
            } else {
                if let Some(children) = changes.get("children") {
                    let children = children
                        .as_array()
                        .ok_or_else(|| invalid("Override children must be an array"))?;
                    index_nodes(children, &format!("{pointer}/children"), parent, nodes)?;
                }
                index_overrides(changes, &pointer, parent, nodes)?;
            }
        }
    }
    Ok(())
}

fn cst_at(root: &CstRootNode, pointer: &str) -> Result<CstNode> {
    let mut node = root.value().ok_or_else(|| invalid("Missing document"))?;
    if pointer.is_empty() {
        return Ok(node);
    }
    for part in pointer
        .strip_prefix('/')
        .ok_or_else(|| invalid("Invalid JSON pointer"))?
        .split('/')
    {
        let part = part.replace("~1", "/").replace("~0", "~");
        node = if let Some(object) = node.as_object() {
            object.get(&part).and_then(|p| p.value())
        } else if let Some(array) = node.as_array() {
            part.parse::<usize>()
                .ok()
                .and_then(|i| array.elements().get(i).cloned())
        } else {
            None
        }
        .ok_or_else(|| invalid(format!("No value at {pointer}")))?;
    }
    Ok(node)
}

fn span(node: &CstNode) -> std::ops::Range<usize> {
    let mut start = 0;
    let mut current = node.clone();
    while let Some(parent) = current.parent() {
        start += parent
            .children()
            .iter()
            .take(current.child_index())
            .map(|c| c.to_string().len())
            .sum::<usize>();
        current = parent.into();
    }
    start..start + node.to_string().len()
}

fn indent_at(text: &str, offset: usize) -> String {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..offset]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn separator(text: &str, container: &CstNode, previous: Option<&CstNode>) -> String {
    if !container.to_string().contains('\n') {
        return " ".into();
    }
    let base = indent_at(text, span(container).start);
    let indent = previous
        .map(|p| indent_at(text, span(p).start))
        .filter(|s| s.len() > base.len())
        .unwrap_or_else(|| format!("{base}  "));
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    format!("{newline}{indent}")
}
