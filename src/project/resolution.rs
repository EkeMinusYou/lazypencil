//! Expand instances and resolve authored values against captured namespaces.

use super::*;

impl Project {
    pub fn resolve(&self) -> Result<ResolvedScene> {
        self.require_assets()?;
        self.resolve_impl(true)
    }

    pub(crate) fn resolve_for_edit(&self) -> Result<ResolvedScene> {
        self.resolve_impl(true)
    }

    pub(super) fn prepare_icons(
        &mut self,
        libraries: BTreeSet<String>,
        runtime: bool,
    ) -> Result<()> {
        let libraries = libraries
            .into_iter()
            .filter(|name| !self.icons.contains(name))
            .collect();
        let (registry, files) =
            crate::icons::Registry::prepare(libraries, self.icon_data_dir.as_deref());
        for (path, bytes) in files {
            if self.assets.contains_key(&path) {
                continue;
            }
            self.byte_count += bytes.len();
            if self.byte_count > MAX_PROJECT_BYTES {
                return Err(invalid("Project dependencies exceed 128 MiB"));
            }
            if runtime {
                self.runtime_assets.insert(path.clone());
            }
            self.assets.insert(
                path,
                Asset {
                    revision: hash(&bytes),
                    bytes,
                    kind: "icon_pack".into(),
                },
            );
        }
        self.icons.extend(registry);
        Ok(())
    }
    pub(super) fn bind_icons(&self, node: &mut ResolvedNode) {
        if node.kind() == "icon" {
            let icon = self.icons.resolve(&node.properties);
            node.icon_metadata = Some(
                icon.as_ref()
                    .map(|icon| icon.metadata())
                    .unwrap_or_else(|error| json!({"error":error})),
            );
            node.icon = Some(icon);
        }
        for child in &mut node.children {
            self.bind_icons(child);
        }
    }
    pub(super) fn resolve_impl(&self, icons: bool) -> Result<ResolvedScene> {
        self.resolve_bounded(icons, &mut Expansion::default())
    }

    pub(super) fn resolve_bounded(
        &self,
        icons: bool,
        count: &mut Expansion,
    ) -> Result<ResolvedScene> {
        for document in self.documents.values() {
            self.check_cancelled()?;
            crate::operations::validate_local_with_cancellation(document, self.cancel.as_ref())?;
        }
        let mut roots = Vec::new();
        for node in self.root_document().value()["children"]
            .as_array()
            .expect("parsed document")
        {
            let mut resolved = self.expand(&self.root, node, None, &[], &mut Vec::new(), count)?;
            assign_addresses(&mut resolved, None, &[]);
            self.resolve_properties(&mut resolved, &self.default_theme(&self.root)?, count)?;
            if icons {
                self.bind_icons(&mut resolved);
            }
            roots.push(resolved);
        }
        self.verify_unchanged()?;
        Ok(ResolvedScene {
            revision: self.root_document().revision(),
            dependency_revision: self.dependency_revision(),
            dependencies: self.dependencies(),
            runtime_dependencies: self.runtime_dependencies(),
            roots,
        })
    }

    pub fn diagnostics(&self) -> Vec<crate::document::Diagnostic> {
        self.documents
            .values()
            .flat_map(|document| {
                document
                    .diagnostics()
                    .into_iter()
                    .chain(document.property_diagnostics())
            })
            .collect()
    }

    pub(super) fn expand(
        &self,
        origin: &Path,
        node: &Value,
        instance: Option<&str>,
        path: &[String],
        active: &mut Vec<(PathBuf, String)>,
        count: &mut Expansion,
    ) -> Result<ResolvedNode> {
        self.check_cancelled()?;
        count.nodes += 1;
        if count.nodes > MAX_NODES || active.len() > 128 || path.len() > 256 {
            return Err(invalid(
                "Expanded document exceeds its node or reference depth limit",
            ));
        }
        let id = node["id"]
            .as_str()
            .ok_or_else(|| invalid("A component child requires an ID"))?;
        count.reserve(instance.unwrap_or(id).len())?;
        for part in path {
            count.key(part)?;
        }
        let address = NodeAddress {
            node_id: instance.unwrap_or(id).into(),
            instance_path: if instance.is_some() {
                path.to_vec()
            } else {
                Vec::new()
            },
        };
        if node["type"] == "ref" {
            let reference = node["ref"]
                .as_str()
                .ok_or_else(|| invalid("An instance requires a component reference"))?;
            let (source, definition) = self.lookup_node_ref(origin, reference)?;
            if definition["reusable"] != true {
                return Err(invalid(format!("Component {reference:?} is not reusable")));
            }
            let key = (
                source.clone(),
                definition["id"]
                    .as_str()
                    .expect("indexed definition")
                    .to_string(),
            );
            if active.contains(&key) {
                return Err(Error::new(
                    "REFERENCE_CYCLE",
                    format!("Component reference cycle at {reference}"),
                ));
            }
            active.push(key);
            let root = instance.unwrap_or(id);
            let mut resolved = self.expand(&source, definition, Some(root), path, active, count)?;
            resolved.address = address;
            resolved.component_key = id.into();
            resolved.component_source = origin.into();
            resolved.instance_identity = Some((origin.into(), id.into()));
            resolved.is_instance = true;
            let props = node
                .as_object()
                .ok_or_else(|| invalid("Instance must be an object"))?;
            self.apply_properties(&mut resolved, props, origin, active, count)?;
            if let Some(overrides) = node.get("descendants") {
                let overrides = overrides
                    .as_object()
                    .ok_or_else(|| invalid("descendants must be an object"))?;
                let mut entries = overrides.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(path, _)| path.split('/').count());
                for (path, changes) in entries {
                    let parts = path.split('/').collect::<Vec<_>>();
                    let target = find_descendant_mut(&mut resolved, &parts).ok_or_else(|| {
                        invalid(format!("Missing descendant override path {path:?}"))
                    })?;
                    if changes.get("type").is_some() {
                        let old_key = target.component_key.clone();
                        let old_source = target.component_source.clone();
                        let old_address = target.address.clone();
                        let mut replacement = self.expand(
                            origin,
                            changes,
                            Some(&old_address.node_id),
                            &old_address.instance_path,
                            active,
                            count,
                        )?;
                        replacement.component_key = old_key;
                        replacement.component_source = old_source;
                        replacement.address = old_address;
                        *target = replacement;
                    } else {
                        self.apply_properties(
                            target,
                            changes
                                .as_object()
                                .ok_or_else(|| invalid("An override must be an object"))?,
                            origin,
                            active,
                            count,
                        )?;
                    }
                }
            }
            active.pop();
            return Ok(resolved);
        }
        // Children are expanded separately. Cloning their entire subtrees at
        // every ancestor multiplies embedded image bytes by the tree depth.
        count.reserve(std::mem::size_of::<ResolvedNode>())?;
        count.reserve(origin.as_os_str().len().saturating_mul(4))?;
        let properties: Map<String, Value> = node
            .as_object()
            .ok_or_else(|| invalid("Node must be an object"))?
            .iter()
            .filter(|(key, _)| key.as_str() != "children")
            .map(|(key, value)| {
                count.key(key)?;
                count.key(key)?;
                count.reserve(origin.as_os_str().len())?;
                Ok((key.clone(), count.clone_value(value)?))
            })
            .collect::<Result<_>>()?;
        let children = node.get("children");
        let property_sources = properties
            .keys()
            .map(|key| (key.clone(), origin.into()))
            .collect();
        let mut resolved = ResolvedNode {
            address,
            source_path: origin.into(),
            source_id: id.into(),
            component_key: id.into(),
            component_source: origin.into(),
            instance_identity: None,
            is_instance: false,
            properties,
            property_sources,
            script_scope: None,
            script_input_scope: None,
            script_bindings: None,
            input_context: None,
            browser_scope: None,
            browser_url_scope: None,
            #[cfg(feature = "rendering")]
            script_inputs_resolved: false,
            theme: BTreeMap::new(),
            children: Vec::new(),
            generated_by: None,
            icon: None,
            icon_metadata: None,
        };
        self.bind_script_scope(&mut resolved)?;
        self.bind_browser_scope(&mut resolved)?;
        if let Some(children) = children {
            for child in children
                .as_array()
                .ok_or_else(|| invalid("children must be an array"))?
            {
                let mut child_path = path.to_vec();
                child_path.push(
                    child["id"]
                        .as_str()
                        .ok_or_else(|| invalid("Missing child ID"))?
                        .into(),
                );
                resolved.children.push(self.expand(
                    origin,
                    child,
                    instance,
                    &child_path,
                    active,
                    count,
                )?);
            }
        }
        Ok(resolved)
    }

    pub(super) fn apply_properties(
        &self,
        node: &mut ResolvedNode,
        changes: &Map<String, Value>,
        origin: &Path,
        active: &mut Vec<(PathBuf, String)>,
        count: &mut Expansion,
    ) -> Result<()> {
        let carrier = script_context::override_value(changes)?;
        let browser_carrier = browser_context::override_value(changes)?;
        if carrier.is_some() && node.kind() != "script" {
            return Err(invalid("Copied script overrides require a script target"));
        }
        if browser_carrier.is_some() && node.kind() != "browser" {
            return Err(invalid("Copied browser overrides require a browser target"));
        }
        let carrier_metadata = carrier
            .as_ref()
            .map(|carrier| carrier.metadata.as_ref())
            .or_else(|| {
                browser_carrier
                    .as_ref()
                    .map(|carrier| carrier.metadata.as_ref())
            });
        let metadata_changed =
            carrier_metadata.map_or(changes.contains_key("metadata"), |value| value.is_some());
        for (key, value) in changes {
            if matches!(
                key.as_str(),
                "id" | "type" | "ref" | "descendants" | "children"
            ) {
                continue;
            }
            let value = if key == "metadata"
                && let Some(metadata) = carrier_metadata
            {
                let Some(value) = metadata else {
                    continue;
                };
                value
            } else {
                value
            };
            if key == "theme" {
                let map = value
                    .as_object()
                    .ok_or_else(|| invalid("theme must be an object"))?;
                let target = node.properties.entry("theme").or_insert_with(|| json!({}));
                target
                    .as_object_mut()
                    .ok_or_else(|| invalid("theme must be an object"))?
                    .extend({
                        count.value(value)?;
                        map.clone()
                    });
            } else {
                count.key(key)?;
                count.key(key)?;
                count.reserve(origin.as_os_str().len())?;
                node.properties
                    .insert(key.clone(), count.clone_value(value)?);
            }
            node.property_sources.insert(key.clone(), origin.into());
        }
        if let Some(children) = changes.get("children") {
            node.children.clear();
            for child in children
                .as_array()
                .ok_or_else(|| invalid("Replacement children must be an array"))?
            {
                let mut path = node.address.instance_path.clone();
                path.push(
                    child["id"]
                        .as_str()
                        .ok_or_else(|| invalid("Replacement child requires an ID"))?
                        .into(),
                );
                node.children.push(self.expand(
                    origin,
                    child,
                    Some(&node.address.node_id),
                    &path,
                    active,
                    count,
                )?);
            }
        }
        // Ordinary metadata does not own execution context. Reset explicit code
        // and input edits independently, then bind only reserved context fields.
        if changes.contains_key("scriptUri") {
            node.script_scope = None;
            node.script_bindings = None;
        }
        if changes.contains_key("inputs") {
            node.input_context = None;
            node.script_input_scope = None;
            if let Some(bindings) = &mut node.script_bindings {
                bindings.inputs_bound = false;
            }
        }
        if metadata_changed {
            let (output, input) =
                script_context::metadata_contexts(node.properties.get("metadata"));
            if output || input {
                let previous_output =
                    (!output).then(|| (node.script_bindings.take(), node.script_scope.take()));
                let previous_input =
                    (!input).then(|| (node.input_context.take(), node.script_input_scope.take()));
                self.bind_script_scope(node)?;
                if let Some((mut bindings, scope)) = previous_output {
                    if input && let Some(bindings) = &mut bindings {
                        bindings.inputs_bound = false;
                    }
                    node.script_bindings = bindings;
                    node.script_scope = scope;
                }
                if let Some((context, scope)) = previous_input {
                    node.input_context = context;
                    node.script_input_scope = scope;
                }
            }
        }
        if let Some(carrier) = carrier {
            if let Some(value) = carrier.output {
                node.script_bindings = bindings::bind_value(self, node, &value, origin)?;
                node.script_scope = None;
            }
            if let Some(value) = carrier.input {
                node.input_context = script_context::bind(self, node, &value, origin)?;
            }
        }
        if changes.contains_key("url") {
            node.browser_scope = None;
            node.browser_url_scope = None;
        }
        if metadata_changed
            && node
                .properties
                .get("metadata")
                .and_then(|value| value.pointer("/lazypencil/browser_scope"))
                .is_some()
        {
            self.bind_browser_scope(node)?;
        }
        if let Some(scope) = browser_carrier.and_then(|carrier| carrier.scope) {
            self.apply_browser_scope(node, &scope, origin)?;
        }
        Ok(())
    }

    pub(super) fn default_theme(&self, source: &Path) -> Result<BTreeMap<String, String>> {
        let mut result = BTreeMap::new();
        if let Some(axes) = self.documents[source].value().get("themes") {
            for (name, values) in axes
                .as_object()
                .ok_or_else(|| invalid("themes must be an object"))?
            {
                let first = values
                    .as_array()
                    .and_then(|v| v.first())
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid(format!("Theme axis {name:?} needs a nonempty string array"))
                    })?;
                if !values
                    .as_array()
                    .expect("checked array")
                    .iter()
                    .all(Value::is_string)
                {
                    return Err(invalid("Theme values must be strings"));
                }
                result.insert(name.clone(), first.into());
            }
        }
        Ok(result)
    }

    pub(super) fn resolve_properties(
        &self,
        node: &mut ResolvedNode,
        inherited: &BTreeMap<String, String>,
        budget: &mut Expansion,
    ) -> Result<()> {
        self.check_cancelled()?;
        let theme = self.effective_theme(node, inherited)?;
        node.theme = theme.clone();
        let is_script = node.kind() == "script";
        if is_script {
            // Input names belong to the captured header. Stale saved controls
            // must not resolve variables or create runtime dependencies.
            let header = node
                .properties
                .get("scriptUri")
                .and_then(Value::as_str)
                .and_then(|uri| {
                    self.resolve_file(node.property_source("scriptUri"), uri)
                        .ok()
                })
                .and_then(|path| self.assets.get(&path))
                .and_then(|asset| std::str::from_utf8(&asset.bytes).ok())
                .and_then(|source| crate::script::header::Header::parse(source).ok());
            if let Some(values) = node.properties.get("inputs") {
                let values = values
                    .as_object()
                    .ok_or_else(|| invalid("Script inputs must be an object"))?;
                let values = match &header {
                    Some(header) => {
                        self.resolve_script_inputs_bounded(node, header, values, budget)?
                    }
                    // Generated code may not be captured yet. Its declared
                    // inputs are resolved once after capture, before execution.
                    None => {
                        budget.object(values)?;
                        values.clone()
                    }
                };
                node.properties
                    .insert("inputs".into(), Value::Object(values));
            }
            #[cfg(feature = "rendering")]
            {
                node.script_inputs_resolved = header.is_some();
            }
        }
        for (key, value) in &mut node.properties {
            if literal_property(key) || key == "inputs" && is_script {
                continue;
            }
            let origin = node.property_sources.get(key).unwrap_or(&node.source_path);
            *value = self.resolve_value_bounded(origin, value, &theme, &mut Vec::new(), budget)?;
        }
        for child in &mut node.children {
            self.resolve_properties(child, &theme, budget)?;
        }
        Ok(())
    }

    pub(crate) fn resolve_script_inputs_bounded(
        &self,
        node: &ResolvedNode,
        header: &crate::script::header::Header,
        values: &Map<String, Value>,
        budget: &mut Expansion,
    ) -> Result<Map<String, Value>> {
        values
            .iter()
            .map(|(name, value)| {
                budget.key(name)?;
                let value = if header.inputs.iter().any(|input| &input.name == name) {
                    budget.value(value)?;
                    let (origin, value) = if let Some(context) = &node.input_context {
                        (context.origin.as_path(), context.expression(name, value)?)
                    } else {
                        (node.property_source("inputs"), value.clone())
                    };
                    self.resolve_value_bounded(
                        origin,
                        &value,
                        &node.theme,
                        &mut Vec::new(),
                        budget,
                    )?
                } else {
                    budget.clone_value(value)?
                };
                Ok((name.clone(), value))
            })
            .collect()
    }

    pub(super) fn effective_theme(
        &self,
        node: &ResolvedNode,
        inherited: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>> {
        self.check_cancelled()?;
        let mut theme = self.default_theme(&node.source_path)?;
        theme.extend(inherited.clone());
        if let Some(overrides) = node.properties.get("theme") {
            for (axis, value) in overrides
                .as_object()
                .ok_or_else(|| invalid("theme must be an object"))?
            {
                theme.insert(
                    axis.clone(),
                    value
                        .as_str()
                        .ok_or_else(|| invalid("Theme axis values must be strings"))?
                        .into(),
                );
            }
        }
        Ok(theme)
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn prepare_script_refs(
        &mut self,
        node: &ResolvedNode,
        header: &crate::script::header::Header,
        mut inputs: Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        for input in &header.inputs {
            if matches!(input.kind, crate::script::header::InputKind::Ref)
                && let Some(value) = inputs.get(&input.name)
            {
                let saved = node
                    .properties
                    .get("inputs")
                    .and_then(|values| values.get(&input.name))
                    .is_some_and(|value| !value.is_null());
                let origin = if saved {
                    node.script_input_origin()
                } else {
                    node.script_origin()
                };
                let reference = value.as_str().expect("validated ref input");
                if reference.is_empty() {
                    continue;
                }
                let (origin, reference) = if self.runtime_refs.contains_key(reference) {
                    (origin, reference)
                } else if saved && let Some(context) = &node.input_context {
                    (context.origin.as_path(), context.component(reference)?)
                } else if let Some(bindings) = &node.script_bindings
                    && (!saved || bindings.inputs_bound)
                {
                    (
                        bindings.origin.as_path(),
                        bindings.input_component(reference, saved)?,
                    )
                } else {
                    (origin, reference)
                };
                let (source, definition) = self.lookup_node(origin, reference)?;
                if definition["reusable"] != true {
                    return Err(invalid(
                        "Script ref input must reference a reusable component",
                    ));
                }
                let same_scope = self
                    .lookup_node(node.script_origin(), reference)
                    .is_ok_and(|(path, other)| path == source && other["id"] == definition["id"]);
                if !same_scope
                    || node.script_bindings.is_some()
                    || value.as_str() != Some(reference)
                {
                    let id = definition["id"]
                        .as_str()
                        .expect("definition ID")
                        .to_string();
                    let handle = format!(
                        "@runtime/component/{}",
                        hash(json!([source, id]).to_string().as_bytes())
                    );
                    self.runtime_refs.insert(handle.clone(), (source, id));
                    inputs.insert(input.name.clone(), json!(handle));
                }
            }
        }
        Ok(inputs)
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn generated_children(
        &mut self,
        script: &ResolvedNode,
        nodes: &[Value],
        budget: &mut Expansion,
    ) -> Result<Vec<ResolvedNode>> {
        let origin = script
            .script_bindings
            .as_ref()
            .map(|bindings| bindings.origin.clone())
            .unwrap_or_else(|| script.script_origin().to_path_buf());
        for node in nodes {
            budget.value(node)?;
        }
        let mut nodes = nodes.to_vec();
        let mut counter = 0;
        let mut ids = BTreeSet::new();
        let mut generated_names = BTreeMap::new();
        for node in &mut nodes {
            assign_generated_ids(node, &mut counter, &mut ids, &mut generated_names)?;
        }
        if let Some(bindings) = &script.script_bindings {
            let mut inherited_bytes = 0;
            let mut contexts = Vec::with_capacity(nodes.len());
            for node in &mut nodes {
                self.check_cancelled()?;
                contexts.push(bindings.rewrite(
                    node,
                    &self.runtime_refs,
                    &mut inherited_bytes,
                    self,
                    &generated_names,
                )?);
            }
            for (node, context) in nodes.iter_mut().zip(&contexts) {
                self.bind_generated_ref_contexts(
                    node,
                    bindings,
                    &script.theme,
                    &mut inherited_bytes,
                    context,
                )?;
            }
        } else {
            for node in &mut nodes {
                bindings::rewrite_generated_paths(
                    self,
                    node,
                    &origin,
                    &self.runtime_refs,
                    &generated_names,
                )?;
            }
        }
        let mut result = Vec::new();
        let mut generated = 0;
        for node in nodes {
            let mut node = self.expand(&origin, &node, None, &[], &mut Vec::new(), budget)?;
            self.resolve_properties(&mut node, &script.theme, budget)?;
            mark_generated(&mut node, &script.address, &script.id(), &mut generated);
            result.push(node);
        }
        let mut assets = Vec::new();
        let mut libraries = BTreeSet::new();
        for node in &result {
            collect_resolved_assets(node, &mut assets)?;
            collect_icon_libraries(node, &mut libraries);
        }
        self.prepare_icons(libraries, true)?;
        for node in &mut result {
            self.bind_icons(node);
        }
        for (origin, uri, kind) in assets {
            let path = if kind == "image" {
                self.resolve_image(&origin, &uri)?
            } else {
                self.resolve_file(&origin, &uri)?
            };
            if !self.assets.contains_key(&path) {
                if kind == "image" {
                    self.capture_image(&origin, &uri)?;
                } else {
                    self.capture_asset(&path, &kind)?;
                }
                self.runtime_assets.insert(path);
            }
        }
        self.verify_unchanged()?;
        Ok(result)
    }

    #[cfg(feature = "rendering")]
    pub(super) fn resolve_value(
        &self,
        origin: &Path,
        value: &Value,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
    ) -> Result<Value> {
        self.resolve_value_bounded(origin, value, theme, active, &mut Expansion::default())
    }

    pub(super) fn resolve_value_bounded(
        &self,
        origin: &Path,
        value: &Value,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
        budget: &mut Expansion,
    ) -> Result<Value> {
        self.check_cancelled()?;
        budget.reserve(std::mem::size_of::<Value>())?;
        Ok(match value {
            Value::String(value) if value.starts_with('$') && value.len() > 1 => {
                self.variable_bounded(origin, &value[1..], theme, active, None, budget)?
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(k, v)| {
                        budget.key(k)?;
                        Ok((
                            k.clone(),
                            if k == "uniforms" && v.is_object() {
                                Value::Object(
                                    v.as_object()
                                        .expect("checked object")
                                        .iter()
                                        .map(|(name, value)| {
                                            budget.key(name)?;
                                            Ok((
                                                name.clone(),
                                                if object.get("type").and_then(Value::as_str)
                                                    == Some("shader")
                                                {
                                                    self.resolve_shader_uniform(
                                                        origin, value, theme, active, budget,
                                                    )?
                                                } else {
                                                    self.resolve_value_bounded(
                                                        origin, value, theme, active, budget,
                                                    )?
                                                },
                                            ))
                                        })
                                        .collect::<Result<_>>()?,
                                )
                            } else if literal_property(k) {
                                budget.clone_value(v)?
                            } else {
                                self.resolve_value_bounded(origin, v, theme, active, budget)?
                            },
                        ))
                    })
                    .collect::<Result<_>>()?,
            ),
            Value::Array(array) => Value::Array(
                array
                    .iter()
                    .map(|v| self.resolve_value_bounded(origin, v, theme, active, budget))
                    .collect::<Result<_>>()?,
            ),
            _ => budget.clone_value(value)?,
        })
    }

    pub(super) fn resolve_shader_uniform(
        &self,
        origin: &Path,
        value: &Value,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
        budget: &mut Expansion,
    ) -> Result<Value> {
        budget.reserve(std::mem::size_of::<Value>())?;
        if let Value::Array(values) = value {
            return Ok(Value::Array(
                values
                    .iter()
                    .map(|value| self.resolve_shader_uniform(origin, value, theme, active, budget))
                    .collect::<Result<_>>()?,
            ));
        }
        let Some(name) = value
            .as_str()
            .and_then(|value| value.strip_prefix('$'))
            .filter(|name| !name.is_empty())
        else {
            return self.resolve_value_bounded(origin, value, theme, active, budget);
        };
        let mut source = origin.to_path_buf();
        let result =
            self.variable_bounded(origin, name, theme, active, Some(&mut source), budget)?;
        if let Some(uri) = result
            .as_str()
            .filter(|uri| !uri.is_empty() && !uri.starts_with('#') && !uri.starts_with("pencil:"))
        {
            // Anchor the value without opening inactive assets. Capture and rendering
            // perform canonicalization, root checks, and dependency verification.
            let uri = self.sampler_uri(&source, uri);
            if let Ok(path) = self.reference_path(&source, uri) {
                return Ok(json!(
                    path.to_str()
                        .ok_or_else(|| invalid("Shader image paths must be UTF-8"))?
                ));
            }
        }
        Ok(result)
    }

    pub(super) fn variable_bounded(
        &self,
        origin: &Path,
        name: &str,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
        file_origin: Option<&mut PathBuf>,
        budget: &mut Expansion,
    ) -> Result<Value> {
        self.check_cancelled()?;
        let (source, definition) = self.lookup_variable_ref(origin, name)?;
        let key = (source.clone(), name.to_string());
        if active.contains(&key) || active.len() > 128 {
            return Err(Error::new(
                "REFERENCE_CYCLE",
                format!("Variable alias cycle at ${name}"),
            ));
        }
        active.push(key);
        let raw = definition
            .get("value")
            .ok_or_else(|| invalid(format!("Variable {name:?} has no value")))?;
        let value = if let Some(values) = raw.as_array() {
            let mut selected = None;
            for entry in values {
                let matches = match entry.get("theme") {
                    None => true,
                    Some(condition) => {
                        let condition = condition
                            .as_object()
                            .ok_or_else(|| invalid("Variable theme condition must be an object"))?;
                        condition.iter().all(|(k, v)| {
                            v.as_str()
                                .is_some_and(|v| theme.get(k).is_some_and(|t| t == v))
                        })
                    }
                };
                if matches {
                    selected = entry.get("value");
                }
            }
            selected
                .ok_or_else(|| invalid(format!("Variable {name:?} has no value for this theme")))?
        } else {
            raw
        };
        let result = if let Some(alias) = value
            .as_str()
            .and_then(|value| value.strip_prefix('$'))
            .filter(|name| !name.is_empty())
        {
            self.variable_bounded(&source, alias, theme, active, file_origin, budget)?
        } else {
            if let Some(origin) = file_origin {
                *origin = source.clone();
            }
            self.resolve_value_bounded(&source, value, theme, active, budget)?
        };
        active.pop();
        let valid = match definition["type"].as_str() {
            Some("number") => result.is_number(),
            Some("string" | "color") => result.is_string(),
            Some("boolean") => result.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(invalid(format!(
                "Variable {name:?} resolves to the wrong type"
            )));
        }
        Ok(result)
    }
}
