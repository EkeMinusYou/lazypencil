//! Apply one edit at a time while retaining one atomic batch state.

use super::*;

struct Batch<'a> {
    edited: Document,
    bindings: BTreeMap<String, String>,
    changed: BTreeSet<String>,
    generated: Map<String, Value>,
    context: Context<'a>,
}

pub(super) fn apply(
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
    let mut batch = Batch {
        edited: document.clone(),
        bindings: BTreeMap::new(),
        changed: BTreeSet::new(),
        generated: Map::new(),
        context: Context {
            path: project_path,
            data,
            cancel,
            overlay: BTreeMap::new(),
            files: Vec::new(),
            sources: Vec::new(),
        },
    };
    let mut index = 0;
    while index < operations.len() {
        let mut edits = Vec::new();
        let mut ids = HashSet::new();
        while let Some(operation) = operations.get(index) {
            let Some((id, edit)) = batch.plain_update(operation)? else {
                break;
            };
            if !ids.insert(id.clone()) {
                break;
            }
            batch.changed.insert(id);
            edits.push(edit);
            index += 1;
        }
        if !edits.is_empty() {
            batch.edited.edit_properties(&edits)?;
        } else {
            batch.apply_one(index, &operations[index])?;
            index += 1;
        }
    }
    batch.finish()
}

impl Batch<'_> {
    fn plain_update(
        &self,
        operation: &Operation,
    ) -> Result<Option<(String, crate::document::PropertyEdit)>> {
        crate::cancel::check(self.context.cancel)?;
        let Operation::Update { target, set, unset } = operation else {
            return Ok(None);
        };
        if !resolve_address(target, &self.bindings)?
            .instance_path
            .is_empty()
        {
            return Ok(None);
        }
        let id = resolve_target(target, &self.bindings)?;
        let node = self.edited.known_node(&id)?;
        if node.pointer("/metadata/lazypencil").is_some() {
            return Ok(None);
        }
        if set
            .keys()
            .chain(unset.iter())
            .any(|key| matches!(key.as_str(), "id" | "type" | "children" | "descendants"))
        {
            return Err(invalid(
                "Update cannot change identity, children, or descendant overrides; use explicit structural operations",
            ));
        }
        Ok(Some((
            id.clone(),
            crate::document::PropertyEdit {
                pointer: self.edited.location(&id)?.pointer.clone(),
                set: set
                    .iter()
                    .map(|(key, value)| {
                        resolve_bindings(value, &self.bindings).map(|value| (key.clone(), value))
                    })
                    .collect::<Result<_>>()?,
                unset: unset.clone(),
            },
        )))
    }

    fn apply_one(&mut self, index: usize, operation: &Operation) -> Result<()> {
        crate::cancel::check(self.context.cancel)?;
        match operation {
            Operation::SetBrowserResources { roots } => {
                for (pointer, key) in [("", "metadata"), ("/metadata", "lazypencil")] {
                    let value = self
                        .edited
                        .value()
                        .pointer(pointer)
                        .and_then(|value| value.get(key));
                    match value {
                        None => self.edited.set_property(pointer, key, &json!({}))?,
                        Some(value) if value.is_object() => {}
                        Some(_) => {
                            return Err(invalid(
                                "Browser resources require object metadata and a lazypencil namespace",
                            ));
                        }
                    }
                }
                self.edited.set_property(
                    "/metadata/lazypencil",
                    "browser_resources",
                    &json!({"version":1,"roots":roots}),
                )?;
            }
            Operation::PackBrowserResources {} => {
                let path = self.context.path.ok_or_else(|| {
                    invalid("Packing browser resources requires a bound document path")
                })?;
                let project = project_for(&self.edited, &self.context)?;
                let (declaration, files) = project.pack_browser_resources(path, path)?;
                let assets = files.iter().map(|file| json!({"uri":file.uri,"hash":hash(&file.bytes),"bytes":file.bytes.len()})).collect::<Vec<_>>();
                for file in files {
                    self.context.stage(file, true)?;
                }
                self.edited.set_property(
                    "/metadata/lazypencil",
                    "browser_resources",
                    &declaration,
                )?;
                self.generated.insert(
                    index.to_string(),
                    json!({"assets":assets,"browser_resources":declaration}),
                );
                self.context.record_source(project)?;
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
                    &mut self.edited,
                    &mut self.context,
                    import::Request {
                        source,
                        mode: *mode,
                        options,
                        expected: expected_source_revision.as_deref(),
                        parent: parent.as_ref(),
                        index: *at,
                    },
                    &self.bindings,
                )?;
                bind(client_ref, &id, &mut self.bindings)?;
                self.generated.insert(index.to_string(), result);
                self.changed.insert(id);
            }
            Operation::Insert {
                parent,
                index: at,
                node,
                client_ref,
            } => {
                let node = resolve_bindings(node, &self.bindings)?;
                ensure_known_subtree(&node)?;
                let mut node = node.clone();
                let mapping = allocate_ids(&mut node)?;
                remap_references(&mut node, &mapping);
                let id = node["id"].as_str().expect("allocated ID").to_string();
                let (pointer, position) =
                    destination(&mut self.edited, parent.as_ref(), *at, &self.bindings)?;
                self.edited
                    .insert_element(&pointer, position, &node.to_string())?;
                bind(client_ref, &id, &mut self.bindings)?;
                self.generated
                    .insert(index.to_string(), json!({"id":id,"id_map":mapping}));
                self.changed.insert(id);
            }
            Operation::Update { target, set, unset } => {
                let address = resolve_address(target, &self.bindings)?;
                if !address.instance_path.is_empty() {
                    update_override(
                        &mut self.edited,
                        &address,
                        set,
                        unset,
                        &self.bindings,
                        &self.context,
                    )?;
                    self.changed.insert(address.node_id);
                    return Ok(());
                }
                let id = resolve_target(target, &self.bindings)?;
                self.edited.known_node(&id)?;
                if set
                    .keys()
                    .chain(unset.iter())
                    .any(|k| matches!(k.as_str(), "id" | "type" | "children" | "descendants"))
                {
                    return Err(invalid(
                        "Update cannot change identity, children, or descendant overrides; use explicit structural operations",
                    ));
                }
                let pointer = self.edited.location(&id)?.pointer.clone();
                apply_property_changes(&mut self.edited, &pointer, set, unset, &self.bindings)?;
                self.changed.insert(id);
            }
            Operation::Delete { target } => {
                let id = resolve_target(target, &self.bindings)?;
                ensure_known_subtree(self.edited.known_node(&id)?)?;
                let pointer = self.edited.location(&id)?.pointer.clone();
                self.edited.remove_element(&pointer)?;
                self.changed.insert(id);
            }
            Operation::Move {
                target,
                parent,
                index: at,
            } => {
                let id = resolve_target(target, &self.bindings)?;
                ensure_known_subtree(self.edited.known_node(&id)?)?;
                if let Some(parent) = parent {
                    let mut ancestor = Some(resolve_target(parent, &self.bindings)?);
                    while let Some(current) = ancestor {
                        if current == id {
                            return Err(invalid(
                                "A node cannot move into itself or its descendants",
                            ));
                        }
                        ancestor = self.edited.location(&current)?.parent_id.clone();
                    }
                }
                let raw = self.edited.raw(&self.edited.location(&id)?.pointer)?;
                let pointer = self.edited.location(&id)?.pointer.clone();
                self.edited.remove_element(&pointer)?;
                let (destination, position) =
                    destination(&mut self.edited, parent.as_ref(), *at, &self.bindings)?;
                self.edited.insert_element(&destination, position, &raw)?;
                self.changed.insert(id);
            }
            Operation::Copy {
                target,
                parent,
                index: at,
                client_ref,
            } => {
                let source = resolve_target(target, &self.bindings)?;
                ensure_known_subtree(self.edited.known_node(&source)?)?;
                let raw = self.edited.raw(&self.edited.location(&source)?.pointer)?;
                let mut template = self.edited.node(&source)?.clone();
                let mapping = allocate_ids(&mut template)?;
                if Project::has_copied_script_inputs(&template) {
                    let mut project = project_for(&self.edited, &self.context)?;
                    project.remap_copied_script_inputs(
                        self.edited.node(&source)?,
                        &mut template,
                        &mapping,
                    )?;
                    self.context.record_source(project)?;
                }
                remap_references(&mut template, &mapping);
                let id = template["id"].as_str().expect("allocated ID").to_string();
                let mut copy = Document::parse(format!(
                    "{{\"version\":\"{}\",\"children\":[{raw}]}}",
                    self.edited.version()
                ))?;
                let original = copy.value()["children"][0].clone();
                patch_value(&mut copy, "/children/0", &original, &template)?;
                let raw = copy.raw("/children/0")?;
                let (pointer, position) =
                    destination(&mut self.edited, parent.as_ref(), *at, &self.bindings)?;
                self.edited.insert_element(&pointer, position, &raw)?;
                bind(client_ref, &id, &mut self.bindings)?;
                self.generated
                    .insert(index.to_string(), json!({"id":id,"id_map":mapping}));
                self.changed.insert(id);
            }
            Operation::DetachInstance { target, client_ref } => {
                let address = resolve_address(target, &self.bindings)?;
                let mut before_project = project_for(&self.edited, &self.context)?;
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
                    let (owner, key) = override_location(&mut self.edited, &address)?;
                    let key = key.expect("inherited path");
                    // The projection already includes deeper outer overrides. Remove them
                    // before publishing the replacement, whose children have new IDs.
                    let prefix = format!("{key}/");
                    let stale = self
                        .edited
                        .value()
                        .pointer(&owner)
                        .and_then(Value::as_object)
                        .expect("override map")
                        .keys()
                        .filter(|path| path.starts_with(&prefix))
                        .cloned()
                        .collect::<Vec<_>>();
                    for path in stale {
                        self.edited.remove_property(&owner, &path)?;
                    }
                    self.edited.set_property(&owner, &key, &node)?;
                } else {
                    let before = self.edited.known_node(&id)?.clone();
                    let pointer = self.edited.location(&id)?.pointer.clone();
                    patch_value(&mut self.edited, &pointer, &before, &node)?;
                }
                let mut after_project = project_for(&self.edited, &self.context)?;
                before_project.check_detached_semantics(&mut after_project, &address)?;
                bind(client_ref, &id, &mut self.bindings)?;
                let mut entry = json!({"id":id,"id_map":mapping});
                if inherited {
                    entry["address"] = json!(address);
                }
                self.generated.insert(index.to_string(), entry);
                self.changed.insert(address.node_id);
            }
            Operation::SetVariable { name, definition } => {
                variable_name(name)?;
                if self.edited.value().get("variables").is_none() {
                    self.edited.set_property("", "variables", &json!({}))?;
                }
                self.edited.set_property("/variables", name, definition)?;
            }
            Operation::RemoveVariable { name } => {
                variable_name(name)?;
                if self.edited.value().get("variables").is_some() {
                    self.edited.remove_property("/variables", name)?;
                }
            }
            Operation::AddImport { alias, uri } => {
                import_alias(alias)?;
                if uri.is_empty() {
                    return Err(invalid("A library URI must be nonempty"));
                }
                if self.edited.value().get("imports").is_none() {
                    self.edited.set_property("", "imports", &json!({}))?;
                }
                if self.edited.value()["imports"]
                    .get(alias)
                    .is_some_and(|v| v != uri)
                {
                    return Err(invalid(
                        "The library alias is already bound to another URI; remove it explicitly first",
                    ));
                }
                self.edited.set_property("/imports", alias, &json!(uri))?;
            }
            Operation::RemoveImport { alias } => {
                import_alias(alias)?;
                if self.edited.value().get("imports").is_some() {
                    self.edited.remove_property("/imports", alias)?;
                }
            }
            Operation::SetOverride { target, set, unset } => {
                let address = resolve_address(target, &self.bindings)?;
                require_instance(&self.edited, &address)?;
                update_override(
                    &mut self.edited,
                    &address,
                    set,
                    unset,
                    &self.bindings,
                    &self.context,
                )?;
                self.changed.insert(address.node_id);
            }
            Operation::ResetOverride { target, properties } => {
                let address = resolve_address(target, &self.bindings)?;
                require_instance(&self.edited, &address)?;
                reset_override(&mut self.edited, &address, properties.as_deref())?;
                self.changed.insert(address.node_id);
            }
            Operation::ReplaceOverride {
                target,
                node,
                client_ref,
            } => {
                let address = resolve_address(target, &self.bindings)?;
                require_instance(&self.edited, &address)?;
                if address.instance_path.is_empty() {
                    return Err(invalid(
                        "replace_override requires an inherited descendant address",
                    ));
                }
                effective_node(&self.edited, &address, &self.context)?;
                let mut node = resolve_bindings(node, &self.bindings)?;
                ensure_known_subtree(&node)?;
                let mapping = allocate_ids(&mut node)?;
                remap_references(&mut node, &mapping);
                let id = node["id"].as_str().expect("allocated ID").to_string();
                let (owner, key) = override_location(&mut self.edited, &address)?;
                self.edited.set_property(
                    &owner,
                    key.as_deref().expect("descendant path"),
                    &node,
                )?;
                bind(client_ref, &id, &mut self.bindings)?;
                self.generated
                    .insert(index.to_string(), json!({"id":id,"id_map":mapping}));
                self.changed.insert(address.node_id);
            }
            Operation::SetSlotChildren {
                target,
                children,
                client_refs,
            } => {
                let address = resolve_address(target, &self.bindings)?;
                let effective = effective_node(&self.edited, &address, &self.context)?;
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
                wrapper = resolve_bindings(&wrapper, &self.bindings)?;
                ensure_known_subtree(&wrapper)?;
                let mapping = allocate_ids(&mut wrapper)?;
                remap_references(&mut wrapper, &mapping);
                let children = wrapper["children"].as_array().expect("child array");
                let ids = children
                    .iter()
                    .map(|node| node["id"].as_str().expect("allocated ID").to_string())
                    .collect::<Vec<_>>();
                for (alias, id) in client_refs.iter().zip(&ids) {
                    bind(alias, id, &mut self.bindings)?;
                }
                let pointer = property_target(&mut self.edited, &address)?;
                self.edited
                    .set_property(&pointer, "children", &json!(children))?;
                self.generated
                    .insert(index.to_string(), json!({"ids":ids,"id_map":mapping}));
                self.changed.insert(address.node_id);
            }
            Operation::SetTheme { target, set, unset } => {
                let address = resolve_address(target, &self.bindings)?;
                if !address.instance_path.is_empty() {
                    effective_node(&self.edited, &address, &self.context)?;
                }
                for axis in set.keys().chain(unset.iter()) {
                    if axis.is_empty() || axis.contains(':') {
                        return Err(invalid("Theme axes must be nonempty and contain no ':'"));
                    }
                }
                let pointer = property_target(&mut self.edited, &address)?;
                let mut theme = match self
                    .edited
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
                    self.edited.remove_property(&pointer, "theme")?;
                } else {
                    self.edited.set_property(&pointer, "theme", &json!(theme))?;
                }
                self.changed.insert(address.node_id);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<PreparedBatch> {
        crate::cancel::check(self.context.cancel)?;
        validate_local_with_cancellation(&self.edited, self.context.cancel)?;
        if self.context.path.is_some()
            || self
                .edited
                .value()
                .get("imports")
                .and_then(Value::as_object)
                .is_none_or(|imports| imports.is_empty())
        {
            project_for(&self.edited, &self.context)?.validate_structure()?;
        }
        let diagnostics = self.edited.property_diagnostics();
        Ok(PreparedBatch {
            document: self.edited,
            result: json!({"changed_ids": self.changed, "generated_ids":self.generated,"bindings":self.bindings,"diagnostics":diagnostics}),
            sources: self.context.sources,
            files: self.context.files,
            overlay: self.context.overlay,
        })
    }
}
