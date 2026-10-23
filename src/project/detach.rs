use super::*;

impl Project {
    // Temporary IDs separate equal names in different documents. They never reach disk:
    // the operation allocates ordinary document IDs after the projection is complete.
    pub(super) fn detached_id(&self, origin: &Path, id: &str) -> String {
        if origin == self.root {
            id.to_owned()
        } else {
            let mut name = format!(
                "detached_{}_{}",
                hash(origin.as_os_str().as_encoded_bytes()),
                id
            );
            while self.root_document().locations().contains_key(&name) {
                name.insert(0, '_');
            }
            name
        }
    }

    pub(super) fn detached_scope(&self, origin: &Path) -> Result<String> {
        let mut queue = VecDeque::from([(self.root.clone(), String::new())]);
        let mut visited = BTreeSet::new();
        while let Some((path, scope)) = queue.pop_front() {
            self.check_cancelled()?;
            if path == origin {
                return Ok(scope);
            }
            if !visited.insert(path.clone()) {
                continue;
            }
            for (alias, target) in self.imports.get(&path).into_iter().flatten() {
                let scope = if scope.is_empty() {
                    alias.clone()
                } else {
                    format!("{scope}:{alias}")
                };
                queue.push_back((target.clone(), scope));
            }
        }
        Err(invalid(
            "A detached dependency must belong to a declared library",
        ))
    }

    fn detached_reference(&self, origin: &Path, reference: &str, variable: bool) -> Result<String> {
        if origin == self.root {
            return Ok(reference.to_owned());
        }
        let qualified = format!("{}:{reference}", self.detached_scope(origin)?);
        let original = if variable {
            self.lookup_variable(origin, reference)?
        } else {
            self.lookup_node(origin, reference)?
        };
        let relocated = if variable {
            self.lookup_variable(&self.root, &qualified)?
        } else {
            self.lookup_node(&self.root, &qualified)?
        };
        if original != relocated {
            return Err(Error::new(
                "INVALID_OPERATION",
                "A qualified library name collides with a local name; rename it before detaching",
            ));
        }
        Ok(qualified)
    }

    fn detached_variables(&self, origin: &Path, value: &Value) -> Result<Value> {
        self.check_cancelled()?;
        Ok(match value {
            Value::String(text) if text.starts_with('$') && text.len() > 1 => {
                json!(format!(
                    "${}",
                    self.detached_reference(origin, &text[1..], true)?
                ))
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| self.detached_variables(origin, v))
                    .collect::<Result<_>>()?,
            ),
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            key.clone(),
                            if key == "uniforms" && value.is_object() {
                                Value::Object(
                                    value
                                        .as_object()
                                        .expect("uniform map")
                                        .iter()
                                        .map(|(key, value)| {
                                            Ok((
                                                key.clone(),
                                                self.detached_variables(origin, value)?,
                                            ))
                                        })
                                        .collect::<Result<_>>()?,
                                )
                            } else if literal_property(key) {
                                value.clone()
                            } else {
                                self.detached_variables(origin, value)?
                            },
                        ))
                    })
                    .collect::<Result<_>>()?,
            ),
            _ => value.clone(),
        })
    }

    fn detached_uri(&self, origin: &Path, value: &mut Value) -> Result<()> {
        if origin == self.root || value == "" {
            return Ok(());
        }
        let uri = value
            .as_str()
            .ok_or_else(|| invalid("An asset URI must be a string"))?;
        let target = self.asset_path_for_edit(origin, uri)?;
        let base = self
            .root
            .parent()
            .ok_or_else(|| invalid("Missing document directory"))?;
        let base_parts = base.components().collect::<Vec<_>>();
        let target_parts = target.components().collect::<Vec<_>>();
        let common = base_parts
            .iter()
            .zip(&target_parts)
            .take_while(|(a, b)| a == b)
            .count();
        let mut relative = PathBuf::new();
        if common == 0 {
            relative = target;
        } else {
            for _ in &base_parts[common..] {
                relative.push("..");
            }
            for part in &target_parts[common..] {
                relative.push(part.as_os_str());
            }
        }
        let uri = if relative.is_absolute() {
            relative
                .to_str()
                .ok_or_else(|| invalid("Detached asset paths must be UTF-8"))?
                .to_owned()
        } else {
            relative
                .components()
                .map(|part| {
                    part.as_os_str()
                        .to_str()
                        .ok_or_else(|| invalid("Detached asset paths must be UTF-8"))
                })
                .collect::<Result<Vec<_>>>()?
                .join("/")
        };
        *value = json!(uri);
        Ok(())
    }

    fn detached_assets(&self, origin: &Path, object: &mut Map<String, Value>) -> Result<()> {
        if let Some(uri) = object.get_mut("scriptUri") {
            self.detached_uri(origin, uri)?;
        }
        for key in ["fill", "stroke", "effect"] {
            if let Some(payload) = object.get_mut(key) {
                let fills = match payload {
                    Value::Array(items) => items.iter_mut().collect::<Vec<_>>(),
                    item => vec![item],
                };
                for fill in fills {
                    let shader = fill["type"] == "shader";
                    if fill["type"] == "image" || shader {
                        if let Some(url) = fill.get_mut("url") {
                            self.detached_uri(origin, url)?;
                        }
                        if shader
                            && let Some(uniforms) =
                                fill.get_mut("uniforms").and_then(Value::as_object_mut)
                        {
                            for value in uniforms.values_mut().flat_map(shader_values_mut) {
                                if value.as_str().is_some_and(|s| {
                                    !s.is_empty()
                                        && !s.starts_with(['#', '$'])
                                        && !s.starts_with("pencil:")
                                }) {
                                    self.detached_uri(origin, value)?;
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn relocate_detached_node(&self, origin: &Path, node: &Value) -> Result<Value> {
        self.check_cancelled()?;
        self.relocate_detached_properties(origin, node, true)
    }

    fn relocate_detached_properties(
        &self,
        origin: &Path,
        node: &Value,
        is_node: bool,
    ) -> Result<Value> {
        let mut result = node
            .as_object()
            .ok_or_else(|| invalid("A node or override must be an object"))?
            .clone();
        if is_node {
            let id = node["id"]
                .as_str()
                .ok_or_else(|| invalid("A node needs an ID"))?;
            result.insert("id".into(), json!(self.detached_id(origin, id)));
        }
        for (key, value) in &mut result {
            if !literal_property(key) && key != "children" {
                *value = self.detached_variables(origin, value)?;
            }
        }
        if let Some(reference) = node.get("ref").and_then(Value::as_str) {
            result.insert(
                "ref".into(),
                json!(self.detached_reference(origin, reference, false)?),
            );
        }
        if let Some(slot) = result.get_mut("slot").and_then(Value::as_array_mut) {
            for reference in slot {
                if let Some(name) = reference.as_str() {
                    let name = if self.lookup_node(origin, name).is_ok() {
                        self.detached_reference(origin, name, false)?
                    } else if origin == self.root {
                        name.to_owned()
                    } else {
                        format!("{}:{name}", self.detached_scope(origin)?)
                    };
                    *reference = json!(name);
                }
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            result.insert(
                "children".into(),
                Value::Array(
                    children
                        .iter()
                        .map(|child| self.relocate_detached_node(origin, child))
                        .collect::<Result<_>>()?,
                ),
            );
        }
        if let Some(overrides) = node.get("descendants").and_then(Value::as_object) {
            let mut expanded = self.expand(origin, node, None, &[], &mut Vec::new(), &mut 0)?;
            let mut relocated = Map::new();
            for (path, changes) in overrides {
                let parts = path.split('/').collect::<Vec<_>>();
                let mut keys = Vec::new();
                for index in 0..parts.len() {
                    let target = find_descendant_mut(&mut expanded, &parts[..=index])
                        .ok_or_else(|| invalid("Missing inherited override during detach"))?;
                    keys.push(self.detached_id(&target.component_source, &target.component_key));
                }
                relocated.insert(
                    keys.join("/"),
                    self.relocate_detached_properties(
                        origin,
                        changes,
                        changes.get("type").is_some(),
                    )?,
                );
            }
            result.insert("descendants".into(), Value::Object(relocated));
        }
        self.detached_assets(origin, &mut result)?;
        Ok(Value::Object(result))
    }

    pub(super) fn align_detached_themes(
        &self,
        node: &mut Value,
        original: &ResolvedNode,
        inherited: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.retain_detached_script_scope(node, original)?;
        let mut actual = self.default_theme(&self.root)?;
        actual.extend(inherited.clone());
        if let Some(theme) = node.get("theme").and_then(Value::as_object) {
            for (axis, value) in theme {
                actual.insert(
                    axis.clone(),
                    value
                        .as_str()
                        .ok_or_else(|| invalid("Theme values must be strings"))?
                        .into(),
                );
            }
        }
        for (axis, value) in &original.theme {
            if actual.get(axis) != Some(value) {
                node.as_object_mut()
                    .expect("node")
                    .entry("theme")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| invalid("theme must be an object"))?
                    .insert(axis.clone(), json!(value));
            }
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
            for child in children {
                let original_child = original
                    .children
                    .iter()
                    .find(|source| {
                        let (path, id) = source
                            .instance_identity
                            .as_ref()
                            .map(|(p, id)| (p.as_path(), id.as_str()))
                            .unwrap_or((&source.source_path, &source.source_id));
                        child["id"] == self.detached_id(path, id)
                    })
                    .ok_or_else(|| invalid("Missing source of a detached child"))?;
                self.align_detached_themes(child, original_child, &original.theme)?;
            }
        }
        if let Some(overrides) = node.get_mut("descendants").and_then(Value::as_object_mut) {
            fn find<'a>(
                project: &Project,
                node: &'a ResolvedNode,
                parts: &[&str],
            ) -> Option<&'a ResolvedNode> {
                let (first, rest) = parts.split_first()?;
                for child in &node.children {
                    if child.component_key == *first
                        || project.detached_id(&child.component_source, &child.component_key)
                            == *first
                    {
                        return if rest.is_empty() {
                            Some(child)
                        } else if child.is_instance {
                            find(project, child, rest)
                        } else {
                            None
                        };
                    }
                    if !child.is_instance
                        && let Some(found) = find(project, child, parts)
                    {
                        return Some(found);
                    }
                }
                None
            }
            for (path, changes) in overrides {
                let target = find(self, original, &path.split('/').collect::<Vec<_>>())
                    .ok_or_else(|| invalid("Missing source of a detached override"))?;
                if changes.get("type").is_some() {
                    // Only differences from the already inherited theme become explicit.
                    self.align_detached_themes(changes, target, &target.theme)?;
                } else if let Some(children) =
                    changes.get_mut("children").and_then(Value::as_array_mut)
                {
                    for child in children {
                        let source = target
                            .children
                            .iter()
                            .find(|source| {
                                let (path, id) = source
                                    .instance_identity
                                    .as_ref()
                                    .map(|(p, id)| (p.as_path(), id.as_str()))
                                    .unwrap_or((&source.source_path, &source.source_id));
                                child["id"] == self.detached_id(path, id)
                            })
                            .ok_or_else(|| invalid("Missing source of detached slot content"))?;
                        self.align_detached_themes(child, source, &target.theme)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn bind_detached_internal_refs(&self, node: &mut Value) -> Result<()> {
        let mut source_ids = BTreeMap::new();
        for (source, document) in &self.documents {
            if source != &self.root {
                for id in document.locations().keys() {
                    source_ids.insert(self.detached_id(source, id), id.clone());
                }
            }
        }
        fn collect(value: &Value, ids: &mut BTreeSet<String>) {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                ids.insert(id.into());
            }
            if let Some(object) = value.as_object() {
                for (key, value) in object {
                    if matches!(key.as_str(), "children" | "descendants") {
                        collect(value, ids);
                    }
                }
                if value.get("type").is_none() {
                    for value in object.values() {
                        collect(value, ids);
                    }
                }
            } else if let Some(array) = value.as_array() {
                for value in array {
                    collect(value, ids);
                }
            }
        }
        fn visit(
            project: &Project,
            value: &mut Value,
            ids: &BTreeSet<String>,
            source_ids: &BTreeMap<String, String>,
        ) -> Result<()> {
            if let Some(object) = value.as_object_mut() {
                let properties = object.get("type").is_none();
                if let Some(reference) = object.get_mut("ref") {
                    bind(project, reference, ids)?;
                }
                if let Some(slot) = object.get_mut("slot").and_then(Value::as_array_mut) {
                    for reference in slot {
                        bind(project, reference, ids)?;
                    }
                }
                if let Some(overrides) =
                    object.get_mut("descendants").and_then(Value::as_object_mut)
                {
                    let original = std::mem::take(overrides);
                    for (path, changes) in original {
                        let path = path
                            .split('/')
                            .map(|part| {
                                if ids.contains(part) {
                                    return part.to_owned();
                                }
                                source_ids
                                    .get(part)
                                    .cloned()
                                    .unwrap_or_else(|| part.to_owned())
                            })
                            .collect::<Vec<_>>()
                            .join("/");
                        overrides.insert(path, changes);
                    }
                }
                for (key, value) in object {
                    if properties || matches!(key.as_str(), "children" | "descendants") {
                        visit(project, value, ids, source_ids)?;
                    }
                }
            } else if let Some(array) = value.as_array_mut() {
                for value in array {
                    visit(project, value, ids, source_ids)?;
                }
            }
            Ok(())
        }
        fn bind(project: &Project, value: &mut Value, ids: &BTreeSet<String>) -> Result<()> {
            let Some(reference) = value.as_str() else {
                return Ok(());
            };
            // Slot suggestions may name a component that is not currently available.
            // Actual ref nodes have already been validated by the expanded projection.
            let Ok((source, definition)) = project.lookup_node(&project.root, reference) else {
                return Ok(());
            };
            let id = project.detached_id(&source, definition["id"].as_str().expect("indexed node"));
            if ids.contains(&id) {
                *value = json!(id);
            }
            Ok(())
        }
        let mut ids = BTreeSet::new();
        collect(node, &mut ids);
        visit(self, node, &ids, &source_ids)
    }

    pub(crate) fn check_detached_semantics(
        &self,
        after: &Project,
        address: &NodeAddress,
    ) -> Result<()> {
        fn find<'a>(nodes: &'a [ResolvedNode], address: &NodeAddress) -> Option<&'a ResolvedNode> {
            for node in nodes {
                if node.address.node_id == address.node_id
                    && node.address.instance_path == address.instance_path
                {
                    return Some(node);
                }
                if let Some(found) = find(&node.children, address) {
                    return Some(found);
                }
            }
            None
        }
        fn asset_uri(project: &Project, origin: &Path, value: &mut Value) -> Result<()> {
            if let Some(uri) = value.as_str()
                && !uri.is_empty()
            {
                *value = json!(project.asset_path_for_edit(origin, uri)?);
            }
            Ok(())
        }
        fn properties(project: &Project, node: &ResolvedNode) -> Result<Map<String, Value>> {
            let mut result = node.properties.clone();
            if node.kind() == "script" {
                super::scope::without_script_scope(&mut result);
            }
            for key in ["id", "reusable", "slot", "theme"] {
                result.remove(key);
            }
            if let Some(uri) = result.get_mut("scriptUri") {
                asset_uri(project, node.property_source("scriptUri"), uri)?;
            }
            for key in ["fill", "stroke", "effect"] {
                if let Some(value) = result.get_mut(key) {
                    let fills = match value {
                        Value::Array(items) => items.iter_mut().collect::<Vec<_>>(),
                        item => vec![item],
                    };
                    for fill in fills {
                        let shader = fill["type"] == "shader";
                        if fill["type"] == "image" || shader {
                            if let Some(uri) = fill.get_mut("url") {
                                asset_uri(project, node.property_source(key), uri)?;
                            }
                            if shader
                                && let Some(uniforms) =
                                    fill.get_mut("uniforms").and_then(Value::as_object_mut)
                            {
                                for value in uniforms.values_mut().flat_map(shader_values_mut) {
                                    if value.as_str().is_some_and(|s| {
                                        !s.is_empty()
                                            && !s.starts_with(['#', '$'])
                                            && !s.starts_with("pencil:")
                                    }) {
                                        asset_uri(project, node.property_source(key), value)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(result)
        }
        fn equivalent(
            before: &Project,
            old: &ResolvedNode,
            after: &Project,
            new: &ResolvedNode,
        ) -> Result<bool> {
            if old.kind() == "script"
                && (old.script_origin() != new.script_origin()
                    || old.properties.contains_key("inputs")
                        && old.script_input_origin() != new.script_input_origin())
            {
                return Ok(false);
            }
            if old.theme != new.theme
                || old.children.len() != new.children.len()
                || properties(before, old)? != properties(after, new)?
            {
                return Ok(false);
            }
            for (old, new) in old.children.iter().zip(&new.children) {
                if old.is_instance != new.is_instance || !equivalent(before, old, after, new)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        let old = self.resolve_for_edit()?;
        let new = after.resolve_for_edit()?;
        let old = find(&old.roots, address)
            .ok_or_else(|| invalid("Missing original detached instance"))?;
        let new = find(&new.roots, address)
            .ok_or_else(|| invalid("Missing materialized detached instance"))?;
        if !equivalent(self, old, after, new)? {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "Detaching this instance would change its resolved properties, themes, nested references, or asset origins",
            ));
        }
        self.verify_unchanged()?;
        after.verify_unchanged()
    }
}
