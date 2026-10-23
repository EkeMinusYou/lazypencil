//! Read the same expanded, script-generated tree used by layout and PNG output.
use super::ReadNodesRequest;
use crate::{Error, Result, layout::LayoutSnapshot, project::Project};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};
use std::io::{self, Write};
use tokio_util::sync::CancellationToken;

const MAX_BYTES: usize = 4 * 1024 * 1024;

pub fn validate_calculated_nodes_request(request: &ReadNodesRequest) -> Result<()> {
    if request.depth > 16 || !(1..=500).contains(&request.limit) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Node depth must be 0..16 and limit 1..500",
        ));
    }
    if request.library_alias.is_some() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "A calculation contains the bound document's effective tree; library_alias selects uncalculated library definitions",
        ));
    }
    if request.instance_path.len() > 128
        || request
            .instance_path
            .iter()
            .any(|part| part.is_empty() || part.contains('/'))
        || (!request.instance_path.is_empty() && request.node_id.is_none())
    {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "An instance path requires a node ID and at most 128 nonempty IDs without '/'",
        ));
    }
    Ok(())
}

pub fn read_calculated_nodes(
    project: &Project,
    snapshot: &LayoutSnapshot,
    request: &ReadNodesRequest,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    validate_calculated_nodes_request(request)?;
    crate::cancel::check(cancel)?;
    if snapshot.revision != project.root_document().revision()
        || snapshot.dependency_revision != project.dependency_revision()
        || request
            .snapshot_id
            .as_deref()
            .is_some_and(|id| id != snapshot.id)
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "The node query and calculation do not belong to the same captured project",
        ));
    }
    let mut query = serde_json::to_value(request).expect("node query");
    for key in [
        "cursor",
        "limit",
        "calculated",
        "snapshot_id",
        "expand_instances",
    ] {
        query.as_object_mut().expect("object").remove(key);
    }
    let identity = crate::document::hash(
        json!({"snapshot":snapshot.id,"view":"calculated","query":query})
            .to_string()
            .as_bytes(),
    );
    let name = request.name.as_ref().map(|name| name.to_lowercase());
    let mut found = request.node_id.is_none();
    let mut matches = Vec::new();
    for (index, item) in snapshot.items.iter().enumerate() {
        crate::cancel::check(cancel)?;
        if let Some(id) = &request.node_id {
            let selected = if request.instance_path.is_empty() {
                &item.record.id == id
            } else {
                &item.record.address.node_id == id
                    && item.record.address.instance_path == request.instance_path
            };
            if !selected {
                continue;
            }
            found = true;
        }
        if request
            .kind
            .as_deref()
            .is_some_and(|kind| kind != item.node.kind())
        {
            continue;
        }
        if name.as_ref().is_some_and(|name| {
            !item
                .node
                .properties
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|value| value.to_lowercase().contains(name))
        }) {
            continue;
        }
        matches.push(index);
    }
    if !found {
        return Err(Error::new(
            "NODE_NOT_FOUND",
            "The requested node is not in this calculation",
        ));
    }
    let offset = crate::query::page_offset(
        &identity,
        request.cursor.as_deref(),
        request.limit,
        matches.len(),
    )?;
    let page = matches
        .iter()
        .skip(offset)
        .take(request.limit)
        .map(|index| Node {
            snapshot,
            index: *index,
            depth: request.depth,
        })
        .collect::<Vec<_>>();
    let next = (offset + page.len() < matches.len())
        .then(|| format!("{identity}:{}", offset + page.len()));
    let response = Page {
        metadata: snapshot.metadata(),
        nodes: page,
        total_matches: matches.len(),
        next_cursor: next,
        view: "calculated",
        variables_resolved: true,
    };
    // Count borrowed data before constructing Values. Overlapping subtrees and
    // large generated properties must not allocate an unbounded response first.
    let mut counter = Counter {
        used: 0,
        exceeded: false,
        cancel,
    };
    if serde_json::to_writer(&mut counter, &response).is_err() {
        crate::cancel::check(cancel)?;
        if counter.exceeded {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Calculated node response exceeds 4 MiB; reduce depth or limit",
            ));
        }
        return Err(Error::new(
            "INTERNAL_ERROR",
            "Cannot serialize calculated nodes",
        ));
    }
    crate::cancel::check(cancel)?;
    let value = serde_json::to_value(&response)
        .map_err(|_| Error::new("INTERNAL_ERROR", "Cannot construct calculated node result"))?;
    project.verify_unchanged()?;
    crate::cancel::check(cancel)?;
    Ok(value)
}

#[derive(Serialize)]
struct Page<'a> {
    #[serde(flatten)]
    metadata: Value,
    view: &'static str,
    variables_resolved: bool,
    nodes: Vec<Node<'a>>,
    total_matches: usize,
    next_cursor: Option<String>,
}

struct Node<'a> {
    snapshot: &'a LayoutSnapshot,
    index: usize,
    depth: usize,
}
impl Serialize for Node<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let item = &self.snapshot.items[self.index];
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &item.record.id)?;
        map.serialize_entry("address", &item.node.address)?;
        map.serialize_entry("source_path", &item.node.source_path)?;
        map.serialize_entry("source_id", &item.node.source_id)?;
        map.serialize_entry("is_instance", &item.node.is_instance)?;
        map.serialize_entry("properties", &item.node.properties)?;
        map.serialize_entry("property_sources", &item.node.property_sources)?;
        map.serialize_entry("theme", &item.node.theme)?;
        map.serialize_entry("generated_by", &item.node.generated_by)?;
        map.serialize_entry("layout", &item.record)?;
        if self.depth == 0 {
            map.serialize_entry("children_count", &item.children.len())?;
        } else {
            map.serialize_entry(
                "children",
                &Children {
                    snapshot: self.snapshot,
                    indices: &item.children,
                    depth: self.depth - 1,
                },
            )?;
        }
        map.end()
    }
}
struct Children<'a> {
    snapshot: &'a LayoutSnapshot,
    indices: &'a [usize],
    depth: usize,
}
impl Serialize for Children<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.indices.len()))?;
        for index in self.indices {
            sequence.serialize_element(&Node {
                snapshot: self.snapshot,
                index: *index,
                depth: self.depth,
            })?;
        }
        sequence.end()
    }
}

struct Counter<'a> {
    used: usize,
    exceeded: bool,
    cancel: Option<&'a CancellationToken>,
}
impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // Interrupted errors would make write_all retry forever after cancellation.
        if self.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(io::Error::other("Query cancelled"));
        }
        if bytes.len() > MAX_BYTES - self.used {
            self.exceeded = true;
            return Err(io::Error::other("Node response limit"));
        }
        self.used += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
