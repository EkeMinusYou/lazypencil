use crate::{
    Error, Result,
    document::{Document, NODE_TYPES, hash, pointer_key},
    project::{NodeAddress, Project},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use uuid::Uuid;

mod import;
pub use import::{
    ComponentSource, ImportMode, ImportOptions, NameResolution, ScriptAsset, ScriptAssetKind,
};

pub(crate) struct PreparedBatch {
    pub document: Document,
    pub result: Value,
    pub sources: Vec<Project>,
    pub files: Vec<crate::managed::File>,
    pub overlay: crate::managed::Overlay,
}

impl PreparedBatch {
    pub fn plain(document: Document, result: Value) -> Self {
        Self {
            document,
            result,
            sources: Vec::new(),
            files: Vec::new(),
            overlay: BTreeMap::new(),
        }
    }
    fn into_unmanaged(self) -> Result<(Document, Value)> {
        if !self.files.is_empty() {
            return Err(invalid(
                "Managed imports must be applied through the common document service",
            ));
        }
        for source in &self.sources {
            source.verify_unchanged()?;
        }
        Ok((self.document, self.result))
    }
}

struct Context<'a> {
    path: Option<&'a Path>,
    data: Option<&'a Path>,
    cancel: Option<&'a tokio_util::sync::CancellationToken>,
    overlay: crate::managed::Overlay,
    files: Vec<crate::managed::File>,
    sources: Vec<Project>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum Target {
    Id(String),
    Result {
        result_of: String,
    },
    Node {
        node_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        instance_path: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Insert {
        #[serde(default)]
        parent: Option<Target>,
        #[serde(default)]
        index: Option<usize>,
        node: Value,
        #[serde(default)]
        client_ref: Option<String>,
    },
    Update {
        target: Target,
        #[serde(default)]
        set: Map<String, Value>,
        #[serde(default)]
        unset: Vec<String>,
    },
    Delete {
        target: Target,
    },
    Move {
        target: Target,
        #[serde(default)]
        parent: Option<Target>,
        #[serde(default)]
        index: Option<usize>,
    },
    Copy {
        target: Target,
        #[serde(default)]
        parent: Option<Target>,
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        client_ref: Option<String>,
    },
    SetVariable {
        name: String,
        definition: Value,
    },
    RemoveVariable {
        name: String,
    },
    AddImport {
        alias: String,
        uri: String,
    },
    RemoveImport {
        alias: String,
    },
    /// Capture this document's declared local web resources into portable assets.
    PackBrowserResources {},
    SetOverride {
        target: Target,
        #[serde(default)]
        set: Map<String, Value>,
        #[serde(default)]
        unset: Vec<String>,
    },
    ResetOverride {
        target: Target,
        #[serde(default)]
        properties: Option<Vec<String>>,
    },
    ReplaceOverride {
        target: Target,
        node: Value,
        #[serde(default)]
        client_ref: Option<String>,
    },
    SetSlotChildren {
        target: Target,
        children: Vec<Value>,
        #[serde(default)]
        client_refs: Vec<Option<String>>,
    },
    SetTheme {
        target: Target,
        #[serde(default)]
        set: BTreeMap<String, String>,
        #[serde(default)]
        unset: Vec<String>,
    },
    DetachInstance {
        target: Target,
        #[serde(default)]
        client_ref: Option<String>,
    },
    ImportComponent {
        source: ComponentSource,
        mode: ImportMode,
        #[serde(default)]
        parent: Option<Target>,
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        options: ImportOptions,
        #[serde(default)]
        expected_source_revision: Option<String>,
        #[serde(default)]
        client_ref: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub request_id: String,
    pub expected_revision: String,
    #[serde(default)]
    pub expected_dependency_revision: Option<String>,
    #[serde(default)]
    pub expected_runtime_dependency_revision: Option<String>,
    pub operations: Vec<Operation>,
    #[serde(default)]
    pub dry_run: bool,
}

impl EditRequest {
    pub fn fingerprint(&self) -> String {
        let mut body = json!({"expected_revision": self.expected_revision, "expected_dependency_revision":self.expected_dependency_revision, "operations":self.operations});
        if let Some(revision) = &self.expected_runtime_dependency_revision {
            body["expected_runtime_dependency_revision"] = json!(revision);
        }
        hash(canonical_value(&body).to_string().as_bytes())
    }
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(k, v)| (k.clone(), canonical_value(v)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(array) => Value::Array(array.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

pub fn apply(document: &Document, operations: &[Operation]) -> Result<(Document, Value)> {
    apply_impl(document, operations, None, None, None)?.into_unmanaged()
}

pub fn apply_in_project(
    document: &Document,
    operations: &[Operation],
    path: &Path,
) -> Result<(Document, Value)> {
    apply_impl(document, operations, Some(path), None, None)?.into_unmanaged()
}

pub(crate) fn prepare_in_project(
    document: &Document,
    operations: &[Operation],
    path: &Path,
    data: &Path,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<PreparedBatch> {
    apply_impl(document, operations, Some(path), Some(data), cancel)
}

fn apply_impl(
    document: &Document,
    operations: &[Operation],
    project_path: Option<&Path>,
    data: Option<&Path>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<PreparedBatch> {
    crate::cancel::check(cancel)?;
    document.ensure_writable()?;
    if operations.len() > 1000 {
        return Err(invalid("A batch may contain at most 1000 operations"));
    }
    let mut edited = document.clone();
    let mut bindings = BTreeMap::<String, String>::new();
    let mut changed = BTreeSet::<String>::new();
    let mut generated = Map::new();
    let mut context = Context {
        path: project_path,
        data,
        cancel,
        overlay: BTreeMap::new(),
        files: Vec::new(),
        sources: Vec::new(),
    };
    for (index, operation) in operations.iter().enumerate() {
        crate::cancel::check(cancel)?;
        match operation {
            Operation::PackBrowserResources {} => {
                let path = context.path.ok_or_else(|| {
                    invalid("Packing browser resources requires a bound document path")
                })?;
                let project = project_for(&edited, &context)?;
                let (declaration, files) = project.pack_browser_resources(path, path)?;
                let assets = files.iter().map(|file| json!({"uri":file.uri,"hash":hash(&file.bytes),"bytes":file.bytes.len()})).collect::<Vec<_>>();
                for file in files {
                    context.stage(file, true)?;
                }
                edited.set_property("/metadata/lazypencil", "browser_resources", &declaration)?;
                generated.insert(
                    index.to_string(),
                    json!({"assets":assets,"browser_resources":declaration}),
                );
                context.record_source(project)?;
            }
            Operation::ImportComponent {
                source,
                mode,
                parent,
                index: at,
                options,
                expected_source_revision,
                client_ref,
            } => {
                let (id, result) = import::apply_import(
                    &mut edited,
                    &mut context,
                    source,
                    *mode,
                    options,
                    expected_source_revision.as_deref(),
                    parent.as_ref(),
                    *at,
                    &bindings,
                )?;
                bind(client_ref, &id, &mut bindings)?;
                generated.insert(index.to_string(), result);
                changed.insert(id);
            }
            Operation::Insert {
                parent,
                index: at,
                node,
                client_ref,
            } => {
                let node = resolve_bindings(node, &bindings)?;
                ensure_known_subtree(&node)?;
                let mut node = node.clone();
                let mapping = allocate_ids(&mut node)?;
                remap_references(&mut node, &mapping);
                let id = node["id"].as_str().expect("allocated ID").to_string();
                let (pointer, position) =
                    destination(&mut edited, parent.as_ref(), *at, &bindings)?;
                edited.insert_element(&pointer, position, &node.to_string())?;
                bind(client_ref, &id, &mut bindings)?;
                generated.insert(index.to_string(), json!({"id":id,"id_map":mapping}));
                changed.insert(id);
            }
            Operation::Update { target, set, unset } => {
                let address = resolve_address(target, &bindings)?;
                if !address.instance_path.is_empty() {
                    update_override(&mut edited, &address, set, unset, &bindings, &context)?;
                    changed.insert(address.node_id);
                    continue;
                }
                let id = resolve_target(target, &bindings)?;
                edited.known_node(&id)?;
                if set
                    .keys()
                    .chain(unset.iter())
                    .any(|k| matches!(k.as_str(), "id" | "type" | "children" | "descendants"))
                {
                    return Err(invalid(
                        "Update cannot change identity, children, or descendant overrides; use explicit structural operations",
                    ));
                }
                let pointer = edited.location(&id)?.pointer.clone();
                apply_property_changes(&mut edited, &pointer, set, unset, &bindings)?;
                changed.insert(id);
            }
            Operation::Delete { target } => {
                let id = resolve_target(target, &bindings)?;
                ensure_known_subtree(edited.known_node(&id)?)?;
                let pointer = edited.location(&id)?.pointer.clone();
                edited.remove_element(&pointer)?;
                changed.insert(id);
            }
            Operation::Move {
                target,
                parent,
                index: at,
            } => {
                let id = resolve_target(target, &bindings)?;
                ensure_known_subtree(edited.known_node(&id)?)?;
                if let Some(parent) = parent {
                    let mut ancestor = Some(resolve_target(parent, &bindings)?);
                    while let Some(current) = ancestor {
                        if current == id {
                            return Err(invalid(
                                "A node cannot move into itself or its descendants",
                            ));
                        }
                        ancestor = edited.location(&current)?.parent_id.clone();
                    }
                }
                let raw = edited.raw(&edited.location(&id)?.pointer)?;
                let pointer = edited.location(&id)?.pointer.clone();
                edited.remove_element(&pointer)?;
                let (destination, position) =
                    destination(&mut edited, parent.as_ref(), *at, &bindings)?;
                edited.insert_element(&destination, position, &raw)?;
                changed.insert(id);
            }
            Operation::Copy {
                target,
                parent,
                index: at,
                client_ref,
            } => {
                let source = resolve_target(target, &bindings)?;
                ensure_known_subtree(edited.known_node(&source)?)?;
                let raw = edited.raw(&edited.location(&source)?.pointer)?;
                let mut template = edited.node(&source)?.clone();
                let mapping = allocate_ids(&mut template)?;
                remap_references(&mut template, &mapping);
                let id = template["id"].as_str().expect("allocated ID").to_string();
                let mut copy =
                    Document::parse(format!("{{\"version\":\"2.21\",\"children\":[{raw}]}}"))?;
                let original = copy.value()["children"][0].clone();
                patch_value(&mut copy, "/children/0", &original, &template)?;
                let raw = copy.raw("/children/0")?;
                let (pointer, position) =
                    destination(&mut edited, parent.as_ref(), *at, &bindings)?;
                edited.insert_element(&pointer, position, &raw)?;
                bind(client_ref, &id, &mut bindings)?;
                generated.insert(index.to_string(), json!({"id":id,"id_map":mapping}));
                changed.insert(id);
            }
            Operation::DetachInstance { target, client_ref } => {
                let address = resolve_address(target, &bindings)?;
                let mut before_project = project_for(&edited, &context)?;
                let mut node = before_project.detached_template(&address)?;
                ensure_known_subtree(&node)?;
                let mut mapping = allocate_ids(&mut node)?;
                let inherited = !address.instance_path.is_empty();
                let id = if inherited {
                    node["id"].as_str().expect("allocated ID").to_owned()
                } else {
                    mapping.insert(address.node_id.clone(), address.node_id.clone());
                    node["id"] = json!(address.node_id);
                    address.node_id.clone()
                };
                remap_references(&mut node, &mapping);
                if inherited {
                    let (owner, key) = override_location(&mut edited, &address)?;
                    let key = key.expect("inherited path");
                    // The projection already includes deeper outer overrides. Remove them
                    // before publishing the replacement, whose children have new IDs.
                    let prefix = format!("{key}/");
                    let stale = edited
                        .value()
                        .pointer(&owner)
                        .and_then(Value::as_object)
                        .expect("override map")
                        .keys()
                        .filter(|path| path.starts_with(&prefix))
                        .cloned()
                        .collect::<Vec<_>>();
                    for path in stale {
                        edited.remove_property(&owner, &path)?;
                    }
                    edited.set_property(&owner, &key, &node)?;
                } else {
                    let before = edited.known_node(&id)?.clone();
                    let pointer = edited.location(&id)?.pointer.clone();
                    patch_value(&mut edited, &pointer, &before, &node)?;
                }
                let mut after_project = project_for(&edited, &context)?;
                before_project.check_detached_semantics(&mut after_project, &address)?;
                bind(client_ref, &id, &mut bindings)?;
                let mut entry = json!({"id":id,"id_map":mapping});
                if inherited {
                    entry["address"] = json!(address);
                }
                generated.insert(index.to_string(), entry);
                changed.insert(address.node_id);
            }
            Operation::SetVariable { name, definition } => {
                variable_name(name)?;
                if edited.value().get("variables").is_none() {
                    edited.set_property("", "variables", &json!({}))?;
                }
                edited.set_property("/variables", name, definition)?;
            }
            Operation::RemoveVariable { name } => {
                variable_name(name)?;
                if edited.value().get("variables").is_some() {
                    edited.remove_property("/variables", name)?;
                }
            }
            Operation::AddImport { alias, uri } => {
                import_alias(alias)?;
                if uri.is_empty() {
                    return Err(invalid("A library URI must be nonempty"));
                }
                if edited.value().get("imports").is_none() {
                    edited.set_property("", "imports", &json!({}))?;
                }
                if edited.value()["imports"]
                    .get(alias)
                    .is_some_and(|v| v != uri)
                {
                    return Err(invalid(
                        "The library alias is already bound to another URI; remove it explicitly first",
                    ));
                }
                edited.set_property("/imports", alias, &json!(uri))?;
            }
            Operation::RemoveImport { alias } => {
                import_alias(alias)?;
                if edited.value().get("imports").is_some() {
                    edited.remove_property("/imports", alias)?;
                }
            }
            Operation::SetOverride { target, set, unset } => {
                let address = resolve_address(target, &bindings)?;
                require_instance(&edited, &address)?;
                update_override(&mut edited, &address, set, unset, &bindings, &context)?;
                changed.insert(address.node_id);
            }
            Operation::ResetOverride { target, properties } => {
                let address = resolve_address(target, &bindings)?;
                require_instance(&edited, &address)?;
                reset_override(&mut edited, &address, properties.as_deref())?;
                changed.insert(address.node_id);
            }
            Operation::ReplaceOverride {
                target,
                node,
                client_ref,
            } => {
                let address = resolve_address(target, &bindings)?;
                require_instance(&edited, &address)?;
                if address.instance_path.is_empty() {
                    return Err(invalid(
                        "replace_override requires an inherited descendant address",
                    ));
                }
                effective_node(&edited, &address, &context)?;
                let mut node = resolve_bindings(node, &bindings)?;
                ensure_known_subtree(&node)?;
                let mapping = allocate_ids(&mut node)?;
                remap_references(&mut node, &mapping);
                let id = node["id"].as_str().expect("allocated ID").to_string();
                let (owner, key) = override_location(&mut edited, &address)?;
                edited.set_property(&owner, key.as_deref().expect("descendant path"), &node)?;
                bind(client_ref, &id, &mut bindings)?;
                generated.insert(index.to_string(), json!({"id":id,"id_map":mapping}));
                changed.insert(address.node_id);
            }
            Operation::SetSlotChildren {
                target,
                children,
                client_refs,
            } => {
                let address = resolve_address(target, &bindings)?;
                let effective = effective_node(&edited, &address, &context)?;
                if effective.kind() != "frame" {
                    return Err(invalid(
                        "Slot children can only replace the children of a frame",
                    ));
                }
                if !client_refs.is_empty() && client_refs.len() != children.len() {
                    return Err(invalid(
                        "client_refs must be empty or contain one entry per child",
                    ));
                }
                let mut wrapper = json!({"type":"frame","children":children});
                wrapper = resolve_bindings(&wrapper, &bindings)?;
                ensure_known_subtree(&wrapper)?;
                let mapping = allocate_ids(&mut wrapper)?;
                remap_references(&mut wrapper, &mapping);
                let children = wrapper["children"].as_array().expect("child array");
                let ids = children
                    .iter()
                    .map(|node| node["id"].as_str().expect("allocated ID").to_string())
                    .collect::<Vec<_>>();
                for (alias, id) in client_refs.iter().zip(&ids) {
                    bind(alias, id, &mut bindings)?;
                }
                let pointer = property_target(&mut edited, &address)?;
                edited.set_property(&pointer, "children", &json!(children))?;
                generated.insert(index.to_string(), json!({"ids":ids,"id_map":mapping}));
                changed.insert(address.node_id);
            }
            Operation::SetTheme { target, set, unset } => {
                let address = resolve_address(target, &bindings)?;
                if !address.instance_path.is_empty() {
                    effective_node(&edited, &address, &context)?;
                }
                for axis in set.keys().chain(unset.iter()) {
                    if axis.is_empty() || axis.contains(':') {
                        return Err(invalid("Theme axes must be nonempty and contain no ':'"));
                    }
                }
                let pointer = property_target(&mut edited, &address)?;
                let mut theme = match edited
                    .value()
                    .pointer(&pointer)
                    .and_then(|n| n.get("theme"))
                {
                    Some(v) => v
                        .as_object()
                        .cloned()
                        .ok_or_else(|| invalid("theme must be an object"))?,
                    None => Map::new(),
                };
                for axis in unset {
                    theme.remove(axis);
                }
                for (axis, value) in set {
                    theme.insert(axis.clone(), json!(value));
                }
                if theme.is_empty() {
                    edited.remove_property(&pointer, "theme")?;
                } else {
                    edited.set_property(&pointer, "theme", &json!(theme))?;
                }
                changed.insert(address.node_id);
            }
        }
    }
    crate::cancel::check(cancel)?;
    validate_local(&edited)?;
    if project_path.is_some()
        || edited
            .value()
            .get("imports")
            .and_then(Value::as_object)
            .is_none_or(|imports| imports.is_empty())
    {
        project_for(&edited, &context)?.validate_structure()?;
    }
    Ok(PreparedBatch {
        document: edited,
        result: json!({"changed_ids": changed, "generated_ids":generated,"bindings":bindings,"diagnostics":[]}),
        sources: context.sources,
        files: context.files,
        overlay: context.overlay,
    })
}

pub fn validate_local(document: &Document) -> Result<()> {
    let imports = document.value().get("imports").and_then(Value::as_object);
    let has_imports = imports.is_some_and(|v| !v.is_empty());
    let variables = document.value().get("variables").and_then(Value::as_object);
    if document.value().get("variables").is_some() && variables.is_none() {
        return Err(invalid("variables must be an object"));
    }
    for (name, definition) in variables.into_iter().flatten() {
        variable_name(name)?;
        validate_variable_definition(
            name,
            definition,
            variables.expect("variable map"),
            has_imports,
        )?;
        check_variable_cycle(name, variables.expect("variable map"), &mut HashSet::new())?;
    }
    for (id, location) in document.locations() {
        let node = document
            .value()
            .pointer(&location.pointer)
            .expect("indexed node");
        let kind = node["type"].as_str().unwrap_or_default();
        if !NODE_TYPES.contains(&kind) {
            continue;
        }
        if kind == "ref" {
            let target = node["ref"]
                .as_str()
                .ok_or_else(|| invalid(format!("Instance {id} needs a ref string")))?;
            if let Ok(definition) = document.node(target) {
                if definition["reusable"] != true {
                    return Err(invalid(format!(
                        "Instance {id} references a non-reusable node"
                    )));
                }
                check_ref_cycle(document, id, &mut HashSet::new())?;
            } else if !has_imports {
                return Err(invalid(format!(
                    "Instance {id} references missing component {target}"
                )));
            }
        }
        for key in [
            "x",
            "y",
            "rotation",
            "opacity",
            "fontSize",
            "letterSpacing",
            "lineHeight",
            "gap",
        ] {
            if let Some(value) = node.get(key) {
                if !value.is_number() && !value.as_str().is_some_and(|s| s.starts_with('$')) {
                    return Err(invalid(format!("{id}.{key} must be a number or variable")));
                }
                if let Some(n) = value.as_f64() {
                    if key == "opacity" && !(0.0..=1.0).contains(&n) {
                        return Err(invalid(format!("{id}.opacity is outside 0..1")));
                    }
                    if matches!(key, "fontSize" | "lineHeight") && n <= 0.0 {
                        return Err(invalid(format!("{id}.{key} must be positive")));
                    }
                }
            }
        }
        for key in ["width", "height"] {
            if let Some(value) = node.get(key) {
                let valid = value.as_f64().is_some_and(|n| n >= 0.0)
                    || value.as_str().is_some_and(valid_size);
                if !valid {
                    return Err(invalid(format!("{id}.{key} is not a valid size")));
                }
            }
        }
        for (key, allowed) in [
            ("layout", &["none", "horizontal", "vertical"][..]),
            ("layoutPosition", &["auto", "absolute"][..]),
            (
                "textGrowth",
                &["auto", "fixed-width", "fixed-width-height"][..],
            ),
            (
                "justifyContent",
                &["start", "center", "end", "space_between", "space_around"][..],
            ),
            ("alignItems", &["start", "center", "end"][..]),
        ] {
            if let Some(value) = node.get(key)
                && !value.as_str().is_some_and(|s| allowed.contains(&s))
            {
                return Err(invalid(format!("{id}.{key} has an unsupported value")));
            }
        }
        check_variable_uses(node, variables, has_imports)?;
    }
    Ok(())
}

fn destination(
    doc: &mut Document,
    parent: Option<&Target>,
    index: Option<usize>,
    bindings: &BTreeMap<String, String>,
) -> Result<(String, usize)> {
    let pointer = if let Some(parent) = parent {
        let id = resolve_target(parent, bindings)?;
        let node = doc.known_node(&id)?;
        if !matches!(node["type"].as_str(), Some("frame" | "group")) {
            return Err(invalid(
                "Only frames and groups accept direct child insertion",
            ));
        }
        let pointer = doc.location(&id)?.pointer.clone();
        if node.get("children").is_none() {
            doc.set_property(&pointer, "children", &json!([]))?;
        }
        format!("{pointer}/children")
    } else {
        "/children".into()
    };
    let length = doc
        .value()
        .pointer(&pointer)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Missing child array"))?
        .len();
    let index = index.unwrap_or(length);
    if index > length {
        return Err(invalid("Child index is out of range"));
    }
    Ok((pointer, index))
}

fn resolve_target(target: &Target, bindings: &BTreeMap<String, String>) -> Result<String> {
    match target {
        Target::Id(id) => Ok(id.clone()),
        Target::Node {
            node_id,
            instance_path,
        } if instance_path.is_empty() => Ok(node_id.clone()),
        Target::Node { .. } => Err(invalid(
            "Inherited descendants require override or slot operations; they cannot be moved or deleted as stored roots",
        )),
        Target::Result { result_of } => bindings
            .get(result_of)
            .cloned()
            .ok_or_else(|| invalid(format!("Unknown or forward client_ref {result_of:?}"))),
    }
}

fn resolve_address(target: &Target, bindings: &BTreeMap<String, String>) -> Result<NodeAddress> {
    let address = match target {
        Target::Node {
            node_id,
            instance_path,
        } => NodeAddress {
            node_id: node_id.clone(),
            instance_path: instance_path.clone(),
        },
        _ => NodeAddress {
            node_id: resolve_target(target, bindings)?,
            instance_path: Vec::new(),
        },
    };
    if address.instance_path.len() > 128
        || address
            .instance_path
            .iter()
            .any(|p| p.is_empty() || p.contains('/'))
    {
        return Err(invalid(
            "Instance paths must contain at most 128 nonempty IDs without '/'",
        ));
    }
    Ok(address)
}

fn project_for(document: &Document, context: &Context<'_>) -> Result<Project> {
    let data = context
        .data
        .map(Path::to_path_buf)
        .or_else(|| crate::store::default_data_dir().ok());
    match context.path {
        Some(path) => Project::from_document_with_overlay_and_cancellation(
            path,
            document.clone(),
            &[],
            data.as_deref(),
            &context.overlay,
            context.cancel,
        ),
        None => {
            let mut project = Project::local(document.clone())?;
            project.set_cancellation(context.cancel);
            Ok(project)
        }
    }
}

fn effective_node(
    document: &Document,
    address: &NodeAddress,
    context: &Context<'_>,
) -> Result<crate::project::ResolvedNode> {
    project_for(document, context)?.expanded_node(address)
}

fn require_instance(document: &Document, address: &NodeAddress) -> Result<()> {
    if document.known_node(&address.node_id)?["type"] != "ref" {
        return Err(invalid("An override target must be a stored instance"));
    }
    Ok(())
}

fn override_location(
    document: &mut Document,
    address: &NodeAddress,
) -> Result<(String, Option<String>)> {
    let pointer = document.location(&address.node_id)?.pointer.clone();
    if address.instance_path.is_empty() {
        return Ok((pointer, None));
    }
    require_instance(document, address)?;
    if document
        .node(&address.node_id)?
        .get("descendants")
        .is_none()
    {
        document.set_property(&pointer, "descendants", &json!({}))?;
    }
    Ok((
        format!("{pointer}/descendants"),
        Some(address.instance_path.join("/")),
    ))
}

fn property_target(document: &mut Document, address: &NodeAddress) -> Result<String> {
    document.known_node(&address.node_id)?;
    let (owner, key) = override_location(document, address)?;
    let Some(key) = key else {
        return Ok(owner);
    };
    if document
        .value()
        .pointer(&owner)
        .and_then(|v| v.get(&key))
        .is_none()
    {
        document.set_property(&owner, &key, &json!({}))?;
    }
    let pointer = format!("{owner}/{}", pointer_key(&key));
    if !document
        .value()
        .pointer(&pointer)
        .is_some_and(Value::is_object)
    {
        return Err(invalid("The override must be an object"));
    }
    Ok(pointer)
}

fn apply_property_changes(
    document: &mut Document,
    pointer: &str,
    set: &Map<String, Value>,
    unset: &[String],
    bindings: &BTreeMap<String, String>,
) -> Result<()> {
    let metadata = set
        .get("metadata")
        .map(|value| resolve_bindings(value, bindings))
        .transpose()?;
    let preserved_metadata = crate::project::preserve_execution_metadata(
        document,
        pointer,
        metadata.as_ref(),
        unset.iter().any(|key| key == "metadata"),
    )?;
    let keys = set
        .keys()
        .chain(unset.iter().filter(|key| {
            document
                .value()
                .pointer(pointer)
                .is_some_and(|node| node.get(key.as_str()).is_some())
        }))
        .filter(|key| !preserved_metadata || key.as_str() != "metadata")
        .map(String::as_str)
        .collect::<Vec<_>>();
    crate::project::revoke_copied_script_context(document, pointer, keys.iter().copied())?;
    crate::project::revoke_copied_browser_context(document, pointer, keys)?;
    for key in unset
        .iter()
        .filter(|key| !preserved_metadata || key.as_str() != "metadata")
    {
        document.remove_property(pointer, key)?;
    }
    for (key, value) in set
        .iter()
        .filter(|(key, _)| !preserved_metadata || key.as_str() != "metadata")
    {
        document.set_property(pointer, key, &resolve_bindings(value, bindings)?)?;
    }
    Ok(())
}

fn update_override(
    document: &mut Document,
    address: &NodeAddress,
    set: &Map<String, Value>,
    unset: &[String],
    bindings: &BTreeMap<String, String>,
    context: &Context<'_>,
) -> Result<()> {
    require_instance(document, address)?;
    if set.keys().chain(unset.iter()).any(|k| {
        matches!(
            k.as_str(),
            "id" | "type" | "ref" | "children" | "descendants"
        )
    }) {
        return Err(invalid(
            "Override updates cannot change identity or structure; use replacement or slot operations",
        ));
    }
    effective_node(document, address, context)?;
    let pointer = property_target(document, address)?;
    apply_property_changes(document, &pointer, set, unset, bindings)?;
    prune_empty_override(document, address)
}

fn reset_override(
    document: &mut Document,
    address: &NodeAddress,
    properties: Option<&[String]>,
) -> Result<()> {
    let root = document.location(&address.node_id)?.pointer.clone();
    if address.instance_path.is_empty() {
        let properties = properties
            .ok_or_else(|| invalid("Resetting root overrides requires explicit property names"))?;
        for key in properties {
            if matches!(key.as_str(), "id" | "type" | "ref") {
                return Err(invalid("Cannot reset instance identity"));
            }
        }
        return apply_property_changes(document, &root, &Map::new(), properties, &BTreeMap::new());
    }
    let key = address.instance_path.join("/");
    let owner = format!("{root}/descendants");
    let pointer = format!("{owner}/{}", pointer_key(&key));
    if document.value().pointer(&pointer).is_none() {
        return Ok(());
    }
    if let Some(properties) = properties {
        for property in properties {
            if matches!(property.as_str(), "id" | "type" | "ref") {
                return Err(invalid(
                    "Reset the complete replacement to restore inherited identity",
                ));
            }
        }
        apply_property_changes(
            document,
            &pointer,
            &Map::new(),
            properties,
            &BTreeMap::new(),
        )?;
        prune_empty_override(document, address)
    } else {
        document.remove_property(&owner, &key)
    }
}

fn prune_empty_override(document: &mut Document, address: &NodeAddress) -> Result<()> {
    if address.instance_path.is_empty() {
        return Ok(());
    }
    let owner = format!(
        "{}/descendants",
        document.location(&address.node_id)?.pointer
    );
    let key = address.instance_path.join("/");
    if document
        .value()
        .pointer(&owner)
        .and_then(|v| v.get(&key))
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        document.remove_property(&owner, &key)?;
    }
    Ok(())
}

fn import_alias(alias: &str) -> Result<()> {
    if alias.is_empty() || alias.contains([':', '/']) || alias.chars().any(char::is_control) {
        return Err(invalid(
            "Library aliases must be nonempty and contain no ':', '/', or control characters",
        ));
    }
    Ok(())
}

fn bind(alias: &Option<String>, id: &str, bindings: &mut BTreeMap<String, String>) -> Result<()> {
    if let Some(alias) = alias
        && (alias.is_empty()
            || alias.len() > 128
            || bindings.insert(alias.clone(), id.into()).is_some())
    {
        return Err(invalid(
            "client_ref must be nonempty and unique in the batch",
        ));
    }
    Ok(())
}

fn resolve_bindings(value: &Value, bindings: &BTreeMap<String, String>) -> Result<Value> {
    Ok(match value {
        Value::Object(object) if object.len() == 1 && object.contains_key("result_of") => {
            let alias = object["result_of"]
                .as_str()
                .ok_or_else(|| invalid("result_of must be a string"))?;
            json!(
                bindings
                    .get(alias)
                    .ok_or_else(|| invalid(format!("Unknown or forward client_ref {alias:?}")))?
            )
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(k, v)| Ok((k.clone(), resolve_bindings(v, bindings)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Array(array) => Value::Array(
            array
                .iter()
                .map(|v| resolve_bindings(v, bindings))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}

fn ensure_known_subtree(value: &Value) -> Result<()> {
    let kind = value["type"]
        .as_str()
        .ok_or_else(|| invalid("A node needs a type"))?;
    if !NODE_TYPES.contains(&kind) {
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            format!("Cannot structurally modify unknown node type {kind}"),
        ));
    }
    if let Some(children) = value.get("children") {
        for node in children
            .as_array()
            .ok_or_else(|| invalid("children must be an array"))?
        {
            ensure_known_subtree(node)?;
        }
    }
    if kind == "ref" {
        ensure_known_overrides(value)?;
    }
    Ok(())
}

fn ensure_known_overrides(value: &Value) -> Result<()> {
    if let Some(overrides) = value.get("descendants") {
        for changes in overrides
            .as_object()
            .ok_or_else(|| invalid("descendants must be an object"))?
            .values()
        {
            if changes.get("type").is_some() {
                ensure_known_subtree(changes)?;
            } else {
                for child in changes
                    .get("children")
                    .map(|v| {
                        v.as_array()
                            .ok_or_else(|| invalid("Override children must be an array"))
                    })
                    .transpose()?
                    .into_iter()
                    .flatten()
                {
                    ensure_known_subtree(child)?;
                }
                ensure_known_overrides(changes)?;
            }
        }
    }
    Ok(())
}

fn allocate_ids(node: &mut Value) -> Result<BTreeMap<String, String>> {
    fn visit(node: &mut Value, ids: &mut BTreeMap<String, String>) -> Result<()> {
        let object = node
            .as_object_mut()
            .ok_or_else(|| invalid("A node must be an object"))?;
        let id = format!("n{}", Uuid::new_v4().simple());
        if let Some(old) = object.get("id") {
            let old = old
                .as_str()
                .filter(|s| !s.is_empty() && !s.contains('/'))
                .ok_or_else(|| invalid("Template IDs must be nonempty strings without '/'"))?;
            if ids.insert(old.into(), id.clone()).is_some() {
                return Err(invalid("Duplicate template ID"));
            }
        }
        object.insert("id".into(), json!(id));
        if let Some(children) = object.get_mut("children") {
            for node in children
                .as_array_mut()
                .ok_or_else(|| invalid("children must be an array"))?
            {
                visit(node, ids)?;
            }
        }
        if object.get("type").is_some_and(|v| v == "ref") {
            visit_overrides(object, ids)?;
        }
        Ok(())
    }
    fn visit_overrides(
        object: &mut Map<String, Value>,
        ids: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        if let Some(overrides) = object.get_mut("descendants").and_then(Value::as_object_mut) {
            for value in overrides.values_mut() {
                if value.get("type").is_some() {
                    visit(value, ids)?;
                } else if let Some(object) = value.as_object_mut() {
                    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut)
                    {
                        for child in children {
                            visit(child, ids)?;
                        }
                    }
                    visit_overrides(object, ids)?;
                }
            }
        }
        Ok(())
    }
    let mut ids = BTreeMap::new();
    visit(node, &mut ids)?;
    Ok(ids)
}

fn remap_references(node: &mut Value, ids: &BTreeMap<String, String>) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    let is_ref = object.get("type").is_none_or(|v| v == "ref");
    if is_ref
        && let Some(reference) = object.get_mut("ref")
        && let Some(new) = reference.as_str().and_then(|s| ids.get(s))
    {
        *reference = json!(new);
    }
    if let Some(slot) = object.get_mut("slot").and_then(Value::as_array_mut) {
        for id in slot {
            if let Some(new) = id.as_str().and_then(|s| ids.get(s)) {
                *id = json!(new);
            }
        }
    }
    if let Some(children) = object.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            remap_references(child, ids);
        }
    }
    if is_ref && let Some(overrides) = object.get_mut("descendants").and_then(Value::as_object_mut)
    {
        let original = std::mem::take(overrides);
        for (path, mut value) in original {
            let path = path
                .split('/')
                .map(|part| ids.get(part).map_or(part, String::as_str))
                .collect::<Vec<_>>()
                .join("/");
            remap_references(&mut value, ids);
            overrides.insert(path, value);
        }
    }
}

fn patch_value(doc: &mut Document, pointer: &str, before: &Value, after: &Value) -> Result<()> {
    if before == after {
        return Ok(());
    }
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            for key in old.keys().filter(|k| !new.contains_key(*k)) {
                doc.remove_property(pointer, key)?;
            }
            for (key, value) in new {
                match old.get(key) {
                    Some(previous)
                        if (previous.is_object() && value.is_object())
                            || (previous.is_array()
                                && value.is_array()
                                && previous.as_array().map(Vec::len)
                                    == value.as_array().map(Vec::len)) =>
                    {
                        patch_value(
                            doc,
                            &format!("{pointer}/{}", pointer_key(key)),
                            previous,
                            value,
                        )?;
                    }
                    Some(previous) if previous == value => {}
                    _ => doc.set_property(pointer, key, value)?,
                }
            }
        }
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => {
            for (index, (old, new)) in old.iter().zip(new).enumerate() {
                patch_value(doc, &format!("{pointer}/{index}"), old, new)?;
            }
        }
        _ => {
            let (parent, key) = pointer
                .rsplit_once('/')
                .ok_or_else(|| invalid("Cannot replace the document root"))?;
            let key = key.replace("~1", "/").replace("~0", "~");
            if doc.value().pointer(parent).is_some_and(Value::is_object) {
                doc.set_property(parent, &key, after)?;
            } else {
                let index = key.parse().map_err(|_| invalid("Invalid array index"))?;
                doc.remove_element(pointer)?;
                doc.insert_element(parent, index, &after.to_string())?;
            }
        }
    }
    Ok(())
}

fn check_ref_cycle(document: &Document, id: &str, active: &mut HashSet<String>) -> Result<()> {
    if !active.insert(id.into()) {
        return Err(Error::new(
            "REFERENCE_CYCLE",
            format!("Component cycle at {id}"),
        ));
    }
    let node = document.node(id)?;
    if let Some(target) = node.get("ref").and_then(Value::as_str)
        && document.node(target).is_ok()
    {
        check_ref_cycle(document, target, active)?;
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            if let Some(id) = child["id"].as_str() {
                check_ref_cycle(document, id, active)?;
            }
        }
    }
    active.remove(id);
    Ok(())
}

fn check_variable_cycle(
    name: &str,
    variables: &Map<String, Value>,
    active: &mut HashSet<String>,
) -> Result<()> {
    if !active.insert(name.into()) {
        return Err(Error::new(
            "REFERENCE_CYCLE",
            format!("Variable cycle at {name}"),
        ));
    }
    if let Some(definition) = variables.get(name) {
        for reference in variable_references(&definition["value"]) {
            if variables.contains_key(&reference) {
                check_variable_cycle(&reference, variables, active)?;
            }
        }
    }
    active.remove(name);
    Ok(())
}

fn validate_variable_definition(
    name: &str,
    definition: &Value,
    variables: &Map<String, Value>,
    has_imports: bool,
) -> Result<()> {
    let kind = definition["type"]
        .as_str()
        .filter(|s| matches!(*s, "number" | "string" | "color" | "boolean"))
        .ok_or_else(|| invalid(format!("Variable {name} needs a supported type")))?;
    let value = definition
        .get("value")
        .ok_or_else(|| invalid(format!("Variable {name} needs a value")))?;
    let candidates =
        if let Some(values) = value.as_array() {
            if values.is_empty() {
                return Err(invalid(format!("Variable {name} has no theme values")));
            }
            let mut candidates = Vec::new();
            for entry in values {
                if let Some(theme) = entry.get("theme")
                    && !theme
                        .as_object()
                        .is_some_and(|v| v.values().all(Value::is_string))
                {
                    return Err(invalid(format!(
                        "Variable {name} has an invalid theme condition"
                    )));
                }
                candidates.push(entry.get("value").ok_or_else(|| {
                    invalid(format!("Variable {name} has a missing theme value"))
                })?);
            }
            candidates
        } else {
            vec![value]
        };
    for value in candidates {
        if let Some(alias) = value
            .as_str()
            .and_then(|s| s.strip_prefix('$'))
            .filter(|s| !s.is_empty())
        {
            if let Some(target) = variables.get(alias) {
                if target["type"] != kind {
                    return Err(invalid(format!(
                        "Variable {name} aliases {alias} with a different type"
                    )));
                }
            } else if !has_imports {
                return Err(invalid(format!(
                    "Variable {name} aliases missing variable {alias}"
                )));
            }
            continue;
        }
        let valid = match kind {
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "string" => value.is_string(),
            "color" => value.as_str().is_some_and(|s| {
                s.strip_prefix('#').is_some_and(|hex| {
                    matches!(hex.len(), 3 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
                })
            }),
            _ => false,
        };
        if !valid {
            return Err(invalid(format!(
                "Variable {name} has a value incompatible with {kind}"
            )));
        }
    }
    Ok(())
}

fn literal_variable_property(key: &str) -> bool {
    matches!(
        key,
        "theme"
            | "metadata"
            | "name"
            | "context"
            | "id"
            | "ref"
            | "scriptUri"
            | "url"
            | "geometry"
            | "href"
    )
}

fn variable_references(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => s
            .strip_prefix('$')
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.into()])
            .unwrap_or_default(),
        Value::Array(v) => v.iter().flat_map(variable_references).collect(),
        Value::Object(v) => v
            .iter()
            .filter(|(key, _)| !literal_variable_property(key))
            .flat_map(|(_, value)| variable_references(value))
            .collect(),
        _ => Vec::new(),
    }
}

fn node_variable_references(value: &Value) -> Vec<String> {
    let Some(properties) = value.as_object() else {
        return variable_references(value);
    };
    let defer_inputs = matches!(
        properties.get("type").and_then(Value::as_str),
        None | Some("script" | "ref")
    );
    properties
        .iter()
        // Script input names require their captured header. This also covers
        // instance/descendant overrides whose effective type is not local.
        .filter(|(key, _)| {
            (key.as_str() != "inputs" || !defer_inputs) && !literal_variable_property(key)
        })
        .flat_map(|(key, value)| match key.as_str() {
            "children" => value
                .as_array()
                .map(|children| children.iter().flat_map(node_variable_references).collect())
                .unwrap_or_else(|| variable_references(value)),
            "descendants" => value
                .as_object()
                .map(|overrides| {
                    overrides
                        .values()
                        .flat_map(node_variable_references)
                        .collect()
                })
                .unwrap_or_else(|| variable_references(value)),
            // An `inputs` uniform or another nested property is still checked.
            _ => variable_references(value),
        })
        .collect()
}

fn check_variable_uses(
    value: &Value,
    variables: Option<&Map<String, Value>>,
    has_imports: bool,
) -> Result<()> {
    if has_imports {
        return Ok(());
    }
    for name in node_variable_references(value) {
        if !variables.is_some_and(|v| v.contains_key(&name)) {
            return Err(invalid(format!("Missing variable ${name}")));
        }
    }
    Ok(())
}

fn valid_size(value: &str) -> bool {
    if value.starts_with('$') && value.len() > 1 {
        return true;
    }
    for prefix in ["fit_content", "fill_container"] {
        if value == prefix {
            return true;
        }
        if let Some(number) = value
            .strip_prefix(prefix)
            .and_then(|v| v.strip_prefix('('))
            .and_then(|v| v.strip_suffix(')'))
            && number
                .parse::<f64>()
                .is_ok_and(|n| n.is_finite() && n >= 0.0)
        {
            return true;
        }
    }
    false
}

fn variable_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains(':')
        || name.starts_with('$')
        || name.chars().any(char::is_control)
    {
        Err(invalid(
            "Variable names must be nonempty, unprefixed, and contain no ':' or control characters",
        ))
    } else {
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_OPERATION", message)
}
