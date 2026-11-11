//! Explicit detail reads and bounded, shallow node discovery for MCP clients.
use super::*;
use crate::project::{NodeAddress, ResolvedNode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tokio_util::sync::CancellationToken;

pub const MAX_SEARCH_BYTES: usize = 64 * 1024;
// The actor adds its document ID and scopes the snapshot/cursor tokens.
const SEARCH_PAYLOAD_BYTES: usize = MAX_SEARCH_BYTES - 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeTarget {
    pub node_id: String,
    #[serde(default)]
    pub instance_path: Vec<String>,
}
impl NodeTarget {
    fn validate(&self) -> Result<()> {
        if self.node_id.is_empty()
            || self.instance_path.len() > 128
            || self
                .instance_path
                .iter()
                .any(|p| p.is_empty() || p.contains('/'))
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "A target requires a nonempty node_id and at most 128 nonempty instance_path IDs without '/'",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadNodesRequest {
    pub node_id: String,
    #[serde(default)]
    pub instance_path: Vec<String>,
    #[serde(default)]
    pub calculated: bool,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub expand_instances: bool,
    #[serde(default)]
    pub library_alias: Option<String>,
    /// Include this many levels of descendants; zero reads only the target.
    #[serde(default)]
    #[schemars(range(min = 0, max = 16))]
    pub depth: usize,
}
impl ReadNodesRequest {
    pub fn validate(&self) -> Result<()> {
        self.target().validate()?;
        validate_view(
            self.calculated,
            self.snapshot_id.as_deref(),
            self.expand_instances,
            self.library_alias.as_deref(),
        )?;
        if self.depth > 16 {
            return Err(Error::new("INVALID_ARGUMENT", "Node depth must be 0..16"));
        }
        Ok(())
    }
    fn target(&self) -> NodeTarget {
        NodeTarget {
            node_id: self.node_id.clone(),
            instance_path: self.instance_path.clone(),
        }
    }
    fn inspect(&self) -> InspectNodesRequest {
        InspectNodesRequest {
            calculated: self.calculated,
            snapshot_id: self.snapshot_id.clone(),
            node_id: Some(self.node_id.clone()),
            instance_path: self.instance_path.clone(),
            expand_instances: self.expand_instances,
            library_alias: self.library_alias.clone(),
            name: None,
            kind: None,
            depth: self.depth,
            limit: 1,
            cursor: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchNodesRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Include this target itself and all descendants in the selected view.
    #[serde(default)]
    pub root: Option<NodeTarget>,
    #[serde(default)]
    pub calculated: bool,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub expand_instances: bool,
    #[serde(default)]
    pub library_alias: Option<String>,
    #[serde(default = "search_limit")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}
fn search_limit() -> usize {
    20
}
impl SearchNodesRequest {
    pub fn validate(&self) -> Result<()> {
        if !(1..=100).contains(&self.limit) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Search limit must be 1..100",
            ));
        }
        if let Some(root) = &self.root {
            root.validate()?;
        }
        validate_view(
            self.calculated,
            self.snapshot_id.as_deref(),
            self.expand_instances,
            self.library_alias.as_deref(),
        )
    }
    fn expanded(&self) -> bool {
        self.expand_instances
            || self
                .root
                .as_ref()
                .is_some_and(|r| !r.instance_path.is_empty())
    }
}
fn validate_view(
    calculated: bool,
    snapshot: Option<&str>,
    expanded: bool,
    library: Option<&str>,
) -> Result<()> {
    if (calculated || snapshot.is_some()) && (expanded || library.is_some()) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Calculated queries cannot use expand_instances or library_alias",
        ));
    }
    Ok(())
}
fn detail(mut value: Value) -> Value {
    let object = value.as_object_mut().expect("node result");
    object.remove("total_matches");
    object.remove("next_cursor");
    value
}
pub fn read_store_nodes(
    store: &Store,
    fonts: &[PathBuf],
    request: &ReadNodesRequest,
) -> Result<Value> {
    read_store_nodes_with_cancellation(store, fonts, request, None)
}
pub fn read_store_nodes_with_cancellation(
    store: &Store,
    fonts: &[PathBuf],
    request: &ReadNodesRequest,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    request.validate()?;
    inspect_store_nodes_with_cancellation(store, fonts, &request.inspect(), cancel).map(detail)
}
#[cfg(feature = "rendering")]
pub fn read_calculated_nodes(
    project: &Project,
    snapshot: &crate::layout::LayoutSnapshot,
    request: &ReadNodesRequest,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    request.validate()?;
    inspect_calculated_nodes(project, snapshot, &request.inspect(), cancel).map(detail)
}

#[derive(Serialize)]
struct TargetRef<'a> {
    node_id: &'a str,
    instance_path: &'a [String],
}
impl<'a> From<&'a NodeAddress> for TargetRef<'a> {
    fn from(a: &'a NodeAddress) -> Self {
        Self {
            node_id: &a.node_id,
            instance_path: &a.instance_path,
        }
    }
}
#[derive(Serialize)]
struct Candidate<'a> {
    id: Cow<'a, str>,
    read_target: TargetRef<'a>,
    name: Option<&'a str>,
    name_truncated: bool,
    #[serde(rename = "type")]
    kind: &'a str,
    parent: Option<TargetRef<'a>>,
    children_count: usize,
    bounds: Option<&'a crate::geometry::Rect>,
    enabled: Option<bool>,
    origin: &'static str,
    generated_by: Option<&'a NodeAddress>,
}
fn short_name(name: Option<&str>) -> (Option<&str>, bool) {
    match name.and_then(|s| s.char_indices().nth(256).map(|(i, _)| (s, i))) {
        Some((s, i)) => (Some(&s[..i]), true),
        None => (name, false),
    }
}

struct SearchPage<'a> {
    request: &'a SearchNodesRequest,
    filter: NodeFilter<'a>,
    result: Value,
    identity: String,
    offset: usize,
    total: usize,
    nodes: Vec<Value>,
    full: bool,
    cancel: Option<&'a CancellationToken>,
}
impl<'a> SearchPage<'a> {
    fn new(
        request: &'a SearchNodesRequest,
        mut metadata: Value,
        cancel: Option<&'a CancellationToken>,
    ) -> Result<Self> {
        request.validate()?;
        crate::cancel::check(cancel)?;
        let identity = crate::document::hash(json!({"metadata":metadata,"name":request.name,"type":request.kind,"root":request.root}).to_string().as_bytes());
        let offset = crate::query::page_offset(
            &identity,
            request.cursor.as_deref(),
            request.limit,
            usize::MAX,
        )?;
        metadata["nodes"] = json!([]);
        metadata["total_matches"] = json!(0);
        metadata["next_cursor"] = Value::Null;
        Ok(Self {
            request,
            filter: NodeFilter::new(request.name.as_deref(), request.kind.as_deref()),
            result: metadata,
            identity,
            offset,
            total: 0,
            nodes: vec![],
            full: false,
            cancel,
        })
    }
    fn matches(&mut self, name: Option<&str>, kind: Option<&str>) -> Result<bool> {
        crate::cancel::check(self.cancel)?;
        if !self.filter.matches(name, kind) {
            return Ok(false);
        }
        let index = self.total;
        self.total += 1;
        Ok(index >= self.offset && !self.full && self.nodes.len() < self.request.limit)
    }
    fn push(&mut self, candidate: &Candidate<'_>) -> Result<()> {
        // Count borrowed strings before allocating a candidate JSON object.
        if crate::query::checked_serialized_size(
            candidate,
            SEARCH_PAYLOAD_BYTES,
            self.cancel,
            "Search candidate exceeds 64 KiB",
        )
        .is_err()
        {
            crate::cancel::check(self.cancel)?;
            if self.nodes.is_empty() {
                return Err(oversized());
            }
            self.full = true;
            return Ok(());
        }
        let value = serde_json::to_value(candidate)
            .map_err(|e| Error::new("INTERNAL_ERROR", e.to_string()))?;
        self.nodes.push(value);
        // Reserve room for the final count and cursor before deciding the page boundary.
        let size = crate::query::serialized_size(&self.result)
            + crate::query::serialized_size(&self.nodes)
            + self.identity.len()
            + 128;
        if size > SEARCH_PAYLOAD_BYTES {
            self.nodes.pop();
            if self.nodes.is_empty() {
                return Err(oversized());
            }
            self.full = true;
        }
        Ok(())
    }
    fn finish(mut self) -> Result<Value> {
        crate::cancel::check(self.cancel)?;
        if self.offset > self.total {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Cursor offset is outside the search result",
            ));
        }
        let next = self.offset + self.nodes.len();
        self.result["total_matches"] = json!(self.total);
        self.result["next_cursor"] = if next < self.total {
            json!(format!("{}:{next}", self.identity))
        } else {
            Value::Null
        };
        self.result["nodes"] = json!(self.nodes);
        crate::query::check_response_size(
            &self.result,
            SEARCH_PAYLOAD_BYTES,
            self.cancel,
            "Search result exceeds 64 KiB",
        )?;
        Ok(self.result)
    }
}
fn oversized() -> Error {
    Error::new(
        "INVALID_ARGUMENT",
        "One search candidate cannot fit in the 64 KiB response budget; identifiers are never truncated",
    )
}
fn compact_status(project: &Project) -> Value {
    json!({"assets_complete":project.resolution_error().is_none().then(|| project.assets_complete()),
        "asset_diagnostic_count":project.asset_diagnostic_count(),
        "dependency_error":null,
        "resolution_error":project.resolution_error().map(|error| error.code),
        "details_tool":"get_document"})
}
fn metadata(
    revision: String,
    dependency: Value,
    library: &Option<String>,
    view: &str,
    status: Value,
) -> Value {
    json!({"revision":revision,"dependency_revision":dependency,"library_alias":library,"view":view,"status":status})
}

pub fn search_store_nodes_with_cancellation(
    store: &Store,
    fonts: &[PathBuf],
    request: &SearchNodesRequest,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    request.validate()?;
    crate::cancel::check(cancel)?;
    if request.calculated || request.snapshot_id.is_some() {
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "Calculated searches require an MCP calculation snapshot",
        ));
    }
    let document = store.read()?;
    let revision = document.revision();
    let result = match project_with_overlay_and_cancellation(
        store,
        document.clone(),
        fonts,
        &Default::default(),
        cancel,
    ) {
        Ok(project) => search_project_nodes(&project, request, cancel)?,
        Err(error)
            if error.code != "CANCELLED"
                && request.library_alias.is_none()
                && !request.expanded() =>
        {
            // Broken dependencies must not hide the stored nodes needed for repair.
            // No dependency snapshot exists, so a continuation cannot promise stable inputs.
            if request.cursor.is_some() {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Dependencies are unavailable; repair them before continuing a stored search",
                ));
            }
            let status = json!({"assets_complete":null,"asset_diagnostic_count":null,"dependency_error":error.code,"resolution_error":null,"details_tool":"get_document"});
            let mut page = SearchPage::new(
                request,
                metadata(revision.clone(), Value::Null, &None, "stored", status),
                cancel,
            )?;
            visit_stored_page(&document, &mut page)?;
            page.finish()?
        }
        Err(error) => return Err(error),
    };
    store::require_revision(&store.read()?, &revision)?;
    crate::cancel::check(cancel)?;
    Ok(result)
}
fn visit_stored_page(document: &Document, page: &mut SearchPage<'_>) -> Result<()> {
    let request = page.request;
    let mut visit = |node: &Value| -> Result<()> {
        if page.matches(node["name"].as_str(), node["type"].as_str())? {
            let id = node["id"].as_str().expect("ID");
            let (name, name_truncated) = short_name(node["name"].as_str());
            page.push(&Candidate {
                id: Cow::Borrowed(id),
                read_target: TargetRef {
                    node_id: id,
                    instance_path: &[],
                },
                name,
                name_truncated,
                kind: node["type"].as_str().expect("type"),
                parent: document
                    .location(id)?
                    .parent_id
                    .as_deref()
                    .map(|p| TargetRef {
                        node_id: p,
                        instance_path: &[],
                    }),
                children_count: node["children"].as_array().map_or(0, Vec::len),
                bounds: None,
                enabled: None,
                origin: "stored",
                generated_by: None,
            })?;
        }
        Ok(())
    };
    if let Some(root) = &request.root {
        walk_stored(document.node(&root.node_id)?, &mut visit)?;
    } else {
        for root in document.value()["children"].as_array().expect("roots") {
            walk_stored(root, &mut visit)?;
        }
    }
    Ok(())
}
fn walk_stored(node: &Value, visit: &mut impl FnMut(&Value) -> Result<()>) -> Result<()> {
    visit(node)?;
    for child in node["children"].as_array().into_iter().flatten() {
        walk_stored(child, visit)?;
    }
    if node["type"] == "ref" {
        walk_overrides(node, visit)?;
    }
    Ok(())
}
fn walk_overrides(node: &Value, visit: &mut impl FnMut(&Value) -> Result<()>) -> Result<()> {
    for value in node
        .get("descendants")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(serde_json::Map::values)
    {
        if value.get("type").is_some() {
            walk_stored(value, visit)?;
        } else {
            for child in value["children"].as_array().into_iter().flatten() {
                walk_stored(child, visit)?;
            }
            walk_overrides(value, visit)?;
        }
    }
    Ok(())
}
fn same_target(target: &NodeTarget, address: &NodeAddress) -> bool {
    target.node_id == address.node_id && target.instance_path == address.instance_path
}
pub fn search_project_nodes(
    project: &Project,
    request: &SearchNodesRequest,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    request.validate()?;
    if request.calculated || request.snapshot_id.is_some() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Use a retained calculation for calculated searches",
        ));
    }
    let view = if request.expanded() {
        "expanded"
    } else {
        "stored"
    };
    let mut page = SearchPage::new(
        request,
        metadata(
            project.root_document().revision(),
            json!(project.dependency_revision()),
            &request.library_alias,
            view,
            compact_status(project),
        ),
        cancel,
    )?;
    if request.expanded() {
        // A scoped query expands only its owning stored subtree. Unrelated broken
        // references must not prevent inspecting a valid target for repair.
        let outer_parent;
        let roots = if let Some(target) = &request.root {
            let document = project.library_document(request.library_alias.as_deref())?;
            let location = document.location(&target.node_id).map_err(|_| {
                Error::new("NODE_NOT_FOUND", "Search root is not in the expanded tree")
            })?;
            outer_parent = location.parent_id.as_ref().map(|id| NodeAddress {
                node_id: id.clone(),
                instance_path: vec![],
            });
            vec![project.expanded_library_node(
                request.library_alias.as_deref(),
                &NodeAddress {
                    node_id: target.node_id.clone(),
                    instance_path: vec![],
                },
            )?]
        } else {
            outer_parent = None;
            project.expanded_library_roots(request.library_alias.as_deref())?
        };
        let mut found = request.root.is_none();
        for root in &roots {
            walk_expanded(
                root,
                outer_parent.as_ref(),
                request.root.is_none(),
                &mut found,
                &mut page,
            )?;
        }
        if !found {
            return Err(Error::new(
                "NODE_NOT_FOUND",
                "Search root is not in the expanded tree",
            ));
        }
    } else {
        let document = project.library_document(request.library_alias.as_deref())?;
        visit_stored_page(document, &mut page)?;
    }
    let result = page.finish()?;
    project.verify_unchanged()?;
    Ok(result)
}
fn walk_expanded(
    node: &ResolvedNode,
    parent: Option<&NodeAddress>,
    inside: bool,
    found: &mut bool,
    page: &mut SearchPage<'_>,
) -> Result<()> {
    crate::cancel::check(page.cancel)?;
    let selected = page
        .request
        .root
        .as_ref()
        .is_some_and(|r| same_target(r, &node.address));
    *found |= selected;
    let inside = inside || selected;
    if inside
        && page.matches(
            node.properties.get("name").and_then(Value::as_str),
            Some(node.kind()),
        )?
    {
        let (name, name_truncated) =
            short_name(node.properties.get("name").and_then(Value::as_str));
        page.push(&Candidate {
            id: Cow::Owned(node.id()),
            read_target: (&node.address).into(),
            name,
            name_truncated,
            kind: node.kind(),
            parent: parent.map(Into::into),
            children_count: node.children.len(),
            bounds: None,
            enabled: None,
            origin: if node.address.instance_path.is_empty() {
                "stored"
            } else {
                "inherited"
            },
            generated_by: None,
        })?;
    }
    for child in &node.children {
        walk_expanded(child, Some(&node.address), inside, found, page)?;
    }
    Ok(())
}

#[cfg(feature = "rendering")]
pub fn search_calculated_nodes(
    project: &Project,
    snapshot: &crate::layout::LayoutSnapshot,
    request: &SearchNodesRequest,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    request.validate()?;
    if snapshot.revision != project.root_document().revision()
        || snapshot.dependency_revision != project.dependency_revision()
        || request
            .snapshot_id
            .as_ref()
            .is_some_and(|id| id != &snapshot.id)
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "Search and calculation refer to different captured inputs",
        ));
    }
    let mut meta = metadata(
        snapshot.revision.clone(),
        json!(snapshot.dependency_revision),
        &request.library_alias,
        "calculated",
        compact_status(project),
    );
    meta["snapshot_id"] = json!(snapshot.id);
    meta["runtime_dependency_revision"] = json!(crate::document::hash(
        serde_json::to_string(&snapshot.runtime_dependencies)
            .expect("runtime dependencies")
            .as_bytes()
    ));
    let mut page = SearchPage::new(request, meta, cancel)?;
    let selected = request
        .root
        .as_ref()
        .map(|root| {
            snapshot
                .items
                .iter()
                .position(|item| same_target(root, &item.node.address))
                .ok_or_else(|| {
                    Error::new("NODE_NOT_FOUND", "Search root is not in this calculation")
                })
        })
        .transpose()?;
    // Preorder layout items keep each subtree contiguous; a stack avoids rescanning ancestors.
    let mut scope_end = snapshot.items.len();
    if let Some(root) = selected {
        let mut stack = snapshot.items[root].children.clone();
        scope_end = root + 1;
        while let Some(index) = stack.pop() {
            crate::cancel::check(cancel)?;
            scope_end = scope_end.max(index + 1);
            stack.extend_from_slice(&snapshot.items[index].children);
        }
    }
    for item in &snapshot.items[selected.unwrap_or(0)..scope_end] {
        if page.matches(
            item.node.properties.get("name").and_then(Value::as_str),
            Some(item.node.kind()),
        )? {
            let (name, name_truncated) =
                short_name(item.node.properties.get("name").and_then(Value::as_str));
            page.push(&Candidate {
                id: Cow::Borrowed(&item.record.id),
                read_target: (&item.node.address).into(),
                name,
                name_truncated,
                kind: item.node.kind(),
                parent: item
                    .parent
                    .map(|p| (&snapshot.items[p].node.address).into()),
                children_count: item.children.len(),
                bounds: Some(&item.record.visual_bounds),
                enabled: Some(item.record.enabled),
                origin: if item.node.generated_by.is_some() {
                    "generated"
                } else if item.node.address.instance_path.is_empty() {
                    "stored"
                } else {
                    "inherited"
                },
                generated_by: item.node.generated_by.as_ref(),
            })?;
        }
    }
    let result = page.finish()?;
    project.verify_unchanged()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_search_cancellation_and_invalid_views_fail_before_work() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Store::init(
            temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        let request: SearchNodesRequest = serde_json::from_value(json!({})).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            search_store_nodes_with_cancellation(&store, &[], &request, Some(&cancel))
                .unwrap_err()
                .code,
            "CANCELLED"
        );
        for query in [
            json!({"calculated":true,"expand_instances":true}),
            json!({"snapshot_id":"id","library_alias":"ui"}),
            json!({"limit":0}),
            json!({"limit":101}),
            json!({"root":{"node_id":""}}),
        ] {
            let request: SearchNodesRequest = serde_json::from_value(query).unwrap();
            assert_eq!(request.validate().unwrap_err().code, "INVALID_ARGUMENT");
        }
        let detail: ReadNodesRequest =
            serde_json::from_value(json!({"node_id":"known","depth":17})).unwrap();
        assert_eq!(detail.validate().unwrap_err().code, "INVALID_ARGUMENT");
    }

    #[test]
    fn node_search_cursor_offsets_must_stay_in_range() {
        let request: SearchNodesRequest = serde_json::from_value(json!({"limit":1})).unwrap();
        let meta = json!({"revision":"one","view":"stored"});
        let page = SearchPage::new(&request, meta.clone(), None).unwrap();
        let mut invalid: SearchNodesRequest =
            serde_json::from_value(json!({"cursor":format!("{}:1",page.identity)})).unwrap();
        assert_eq!(
            SearchPage::new(&invalid, meta.clone(), None)
                .unwrap()
                .finish()
                .unwrap_err()
                .code,
            "INVALID_ARGUMENT"
        );
        invalid.cursor = Some("malformed".into());
        assert!(SearchPage::new(&invalid, meta, None).is_err());
    }
}
