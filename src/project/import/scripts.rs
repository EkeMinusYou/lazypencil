use super::*;
use crate::script::header::{Header, InputKind};

mod composition;
use composition::Budget;

pub(super) struct Scope {
    documents: BTreeSet<PathBuf>,
    components: BTreeMap<String, Key>,
    variables: BTreeMap<String, Key>,
    pub(super) assets: BTreeMap<String, String>,
}

pub(super) struct Info {
    pub(super) header: Header,
    resolved: ResolvedNode,
    uri: String,
}

impl Copier<'_> {
    pub(super) fn script_info(&self, origin: &Path, node: &Value) -> Result<Option<Info>> {
        if !matches!(node["type"].as_str(), Some("script" | "ref")) {
            return Ok(None);
        }
        let resolved = self
            .project
            .expand(origin, node, None, &[], &mut Vec::new(), &mut 0)?;
        if resolved.kind() != "script" {
            return Ok(None);
        }
        let uri = resolved
            .properties
            .get("scriptUri")
            .and_then(Value::as_str)
            .ok_or_else(|| copy_error("A copied script needs scriptUri"))?
            .to_owned();
        let path = self
            .project
            .resolve_file(resolved.property_source("scriptUri"), &uri)?;
        let code = std::str::from_utf8(&self.project.asset(&path)?.bytes)
            .map_err(|_| copy_error("Copied script source must be UTF-8"))?;
        Ok(Some(Info {
            header: Header::parse(code)?,
            resolved,
            uri,
        }))
    }

    pub(super) fn scan_script(&mut self, origin: &Path, node: &Value) -> Result<Option<Info>> {
        let Some(info) = self.script_info(origin, node)? else {
            return Ok(None);
        };
        self.script_scope(info.resolved.script_origin())?;
        self.script_scope(info.resolved.script_input_origin())?;
        if let Some(bound) = &info.resolved.script_bindings {
            self.script_scope(&bound.origin)?;
            if let Some(assets) = &bound.values.assets {
                for (key, uri) in assets {
                    let (kind, _) = key.split_once(':').expect("validated asset binding");
                    let path = self.project.resolve_file(&bound.origin, uri)?;
                    self.project.capture_asset(&path, kind)?;
                    self.script_images
                        .insert((bound.origin.clone(), uri.clone(), kind.into()));
                }
            }
        }
        let source_key = self.key(&(
            origin.into(),
            node["id"].as_str().expect("script ID").into(),
        ))?;
        if let Some(assets) = self.options.script_assets.get(&source_key).cloned() {
            if assets.len() > 4096 {
                return Err(copy_error("A script asset manifest exceeds 4096 entries"));
            }
            self.used_script_assets.insert(source_key);
            for asset in assets {
                self.project.check_cancelled()?;
                let kind = match asset.kind {
                    crate::operations::ScriptAssetKind::Image => "image",
                    crate::operations::ScriptAssetKind::Shader => "shader",
                    crate::operations::ScriptAssetKind::Script => "script",
                };
                super::super::bindings::asset_key(kind, &asset.uri)?;
                let owner = info.resolved.script_origin();
                let path = self.project.resolve_file(owner, &asset.uri)?;
                self.project.capture_asset(&path, kind)?;
                self.script_images
                    .insert((owner.into(), asset.uri, kind.into()));
            }
        }
        if node["type"] == "ref"
            && node.get("scriptUri").is_some()
            && node.get("metadata").is_none()
        {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "A copied source override needs a separately preserved metadata inheritance scope",
            ));
        }
        Ok(Some(info))
    }

    fn script_scope(&mut self, origin: &Path) -> Result<()> {
        if self.script_scopes.contains_key(origin) {
            return Ok(());
        }
        let mut queue = VecDeque::from([(origin.to_path_buf(), String::new(), BTreeSet::new())]);
        let mut documents = BTreeSet::new();
        let mut component_names = BTreeSet::new();
        let mut variable_names = BTreeSet::new();
        let mut bytes = 0usize;
        let mut paths = 0usize;
        while let Some((path, prefix, mut active)) = queue.pop_front() {
            self.project.check_cancelled()?;
            paths += 1;
            bytes = bytes.saturating_add(prefix.len());
            if paths > 4096 {
                return Err(copy_error(
                    "The complete script namespace exceeds 4096 library paths",
                ));
            }
            if !active.insert(path.clone()) {
                continue;
            }
            documents.insert(path.clone());
            let document = &self.project.documents[&path];
            for id in document.locations().keys() {
                if document.node(id)?["reusable"] == true {
                    component_names.insert(id.clone());
                    component_names.insert(format!("{prefix}{id}"));
                    bytes = bytes.saturating_add(prefix.len() + 2 * id.len());
                    if bytes > 4 * 1024 * 1024 {
                        return Err(copy_error(
                            "Script component names exceed the namespace byte budget",
                        ));
                    }
                }
            }
            for name in document
                .value()
                .get("variables")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(Map::keys)
            {
                variable_names.insert(name.clone());
                variable_names.insert(format!("{prefix}{name}"));
                bytes = bytes.saturating_add(prefix.len() + 2 * name.len());
                if bytes > 4 * 1024 * 1024 {
                    return Err(copy_error(
                        "Script variable names exceed the namespace byte budget",
                    ));
                }
            }
            if bytes > 4 * 1024 * 1024
                || component_names.len() + variable_names.len() > MAX_NODES
                || prefix.len() > 4096
                || active.len() > 128
            {
                return Err(copy_error(
                    "The complete script namespace exceeds its bounded import budget",
                ));
            }
            for (alias, path) in &self.project.imports[&path] {
                if queue.len() >= 4096 {
                    return Err(copy_error(
                        "The complete script namespace queue exceeds 4096 library paths",
                    ));
                }
                queue.push_back((path.clone(), format!("{prefix}{alias}:"), active.clone()));
            }
        }
        let components = component_names
            .into_iter()
            .filter_map(|name| {
                self.project
                    .lookup_node(origin, &name)
                    .ok()
                    .filter(|(_, definition)| definition["reusable"] == true)
                    .map(|(path, definition)| {
                        (
                            name,
                            (
                                path,
                                definition["id"].as_str().expect("definition ID").into(),
                            ),
                        )
                    })
            })
            .collect();
        let variables = variable_names
            .into_iter()
            .filter_map(|name| {
                self.project
                    .variable_key(origin, &name)
                    .ok()
                    .map(|key| (name, key))
            })
            .collect();
        self.script_scopes.insert(
            origin.into(),
            Scope {
                documents: documents.clone(),
                components,
                variables,
                assets: BTreeMap::new(),
            },
        );
        for path in documents {
            self.source_axes(&path)?;
            let document = self.project.documents[&path].clone();
            for id in document.locations().keys() {
                if document.node(id)?["reusable"] == true {
                    self.queue.push_back((path.clone(), id.clone()));
                }
            }
            for name in document
                .value()
                .get("variables")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(Map::keys)
            {
                self.variable(&path, name)?;
            }
        }
        Ok(())
    }

    pub(super) fn capture_script_images(&mut self) -> Result<()> {
        if self.script_scopes.is_empty() {
            return Ok(());
        }
        for (key, definition) in self.var_values.clone() {
            self.project.check_cancelled()?;
            if definition["type"] != "string" {
                continue;
            }
            for value in samplers::definition_values(&definition["value"]) {
                let Some(uri) = value.as_str().filter(|v| {
                    !v.is_empty()
                        && v.len() <= 4096
                        && !v.starts_with(['$', '#'])
                        && !v.starts_with("pencil:")
                }) else {
                    continue;
                };
                let Ok(path) = self.project.resolve_file(&key.0, uri) else {
                    continue;
                };
                let Ok(file) = crate::store::open_regular_file(&path) else {
                    continue;
                };
                let mut header = Vec::new();
                file.take(12)
                    .read_to_end(&mut header)
                    .map_err(|e| Error::io(&path, e))?;
                let size = header.len();
                if header[..size].starts_with(b"\x89PNG\r\n\x1a\n")
                    || header[..size].starts_with(&[0xff, 0xd8, 0xff])
                    || size == 12 && &header[..4] == b"RIFF" && &header[8..] == b"WEBP"
                {
                    self.project.capture_asset(&path, "image")?;
                    self.sampler_vars.insert(key.clone());
                    self.script_images
                        .insert((key.0.clone(), uri.into(), "image".into()));
                }
            }
        }
        Ok(())
    }

    pub(super) fn prepare_script_assets(&mut self) -> Result<()> {
        if !self.script_scopes.is_empty() {
            let mut axes: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            for ((_, name), target) in &self.axes {
                axes.entry(name).or_default().insert(target);
            }
            if axes.values().any(|targets| targets.len() > 1) {
                return Err(copy_error(
                    "Shared script theme axes need one common target; explicitly reuse the same axis across source scopes",
                ));
            }
        }
        let mut total = 0usize;
        for origin in self.script_scopes.keys().cloned().collect::<Vec<_>>() {
            let mut budget = Budget::new(total);
            let documents = self.script_scopes[&origin].documents.clone();
            let mut assets = Vec::new();
            for path in &documents {
                collect_assets(self.project.documents[path].value(), path, &mut assets)?;
            }
            assets.extend(
                self.script_images
                    .iter()
                    .filter(|(path, _, _)| documents.contains(path))
                    .cloned(),
            );
            let mut map = BTreeMap::new();
            for (owner, uri, kind) in assets {
                self.project.check_cancelled()?;
                let path = self.project.resolve_file(&owner, &uri)?;
                let target = self.asset(&owner, &uri, &kind)?;
                let relative = relative_uri(&origin, &path)?;
                for uri in [Some(relative), (owner == origin).then_some(uri)]
                    .into_iter()
                    .flatten()
                {
                    let key = super::super::bindings::asset_key(&kind, &uri)?;
                    if let Some(previous) = map.get(&key) {
                        if previous != &target {
                            return Err(Error::new(
                                "ASSET_CONFLICT",
                                "Script asset paths have conflicting copied bindings",
                            ));
                        }
                    } else {
                        budget.pair(&key, &target)?;
                        map.insert(key, target.clone());
                    }
                }
            }
            self.script_scopes.get_mut(&origin).expect("scope").assets = map;
            total = total.saturating_add(budget.bytes());
        }
        Ok(())
    }

    fn component_bindings(&self, origin: &Path, budget: &mut Budget) -> Result<Map<String, Value>> {
        self.script_scopes[origin]
            .components
            .iter()
            .map(|(name, key)| {
                let target = self.node_target(key)?;
                budget.pair(name, &target)?;
                Ok((name.clone(), json!(target)))
            })
            .collect()
    }

    fn override_paths(
        &self,
        root: &ResolvedNode,
        from: &str,
        to: &str,
        result: &mut Map<String, Value>,
        budget: &mut Budget,
    ) -> Result<()> {
        for child in &root.children {
            self.project.check_cancelled()?;
            let key = (child.component_source.clone(), child.component_key.clone());
            let source = format!("{from}{}", child.component_key);
            let target = format!("{to}{}", self.node_target(&key)?);
            budget.pair(&source, &target)?;
            result.insert(source.clone(), json!(target));
            if child.is_instance {
                self.override_paths(
                    child,
                    &format!("{source}/"),
                    &format!("{target}/"),
                    result,
                    budget,
                )?;
            } else {
                self.override_paths(child, from, to, result, budget)?;
            }
        }
        Ok(())
    }

    pub(super) fn rewrite_script(
        &mut self,
        origin: &Path,
        node: &Value,
        result: &mut Map<String, Value>,
        info: Option<&Info>,
    ) -> Result<()> {
        let Some(info) = info else { return Ok(()) };
        if node["type"] == "ref" && node.get("metadata").is_none() {
            if let Some(inputs) = node.get("inputs").and_then(Value::as_object) {
                for input in &info.header.inputs {
                    if !matches!(input.kind, InputKind::Ref) {
                        continue;
                    }
                    let Some(value) = inputs
                        .get(&input.name)
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                    else {
                        continue;
                    };
                    if value.starts_with('$') {
                        return Err(Error::new(
                            "UNSUPPORTED_FEATURE",
                            "A variable ref override needs a separately preserved input binding scope",
                        ));
                    }
                    let (path, definition) = self
                        .project
                        .lookup_node(info.resolved.script_input_origin(), value)?;
                    result.get_mut("inputs").expect("inputs")[&input.name] = json!(
                        self.node_target(&(path, definition["id"].as_str().expect("ID").into()))?
                    );
                }
            }
            return Ok(());
        }
        let mut budget = Budget::new(self.script_binding_bytes);
        let output = info.resolved.script_origin();
        let input = info.resolved.script_input_origin();
        let uri = self.asset(
            info.resolved.property_source("scriptUri"),
            &info.uri,
            "script",
        )?;
        if node["type"] == "script" || node.get("scriptUri").is_some() {
            result.insert("scriptUri".into(), json!(uri));
        }
        let mut bindings = if let Some(bound) = &info.resolved.script_bindings {
            let key = self.key(&(
                origin.into(),
                node["id"].as_str().expect("script ID").into(),
            ))?;
            let additional = self
                .options
                .script_assets
                .get(&key)
                .cloned()
                .unwrap_or_default();
            self.compose_bindings(bound, &uri, input, output, &additional, &mut budget)?
        } else {
            let mut descendants = Map::new();
            for (name, key) in &self.script_scopes[output].components {
                if self.reuse.contains_key(key) {
                    continue;
                }
                let definition = self.project.documents[&key.0].node(&key.1)?;
                let expanded =
                    self.project
                        .expand(&key.0, definition, None, &[], &mut Vec::new(), &mut 0)?;
                let mut paths = Map::new();
                budget.key(name)?;
                self.override_paths(&expanded, "", "", &mut paths, &mut budget)?;
                descendants.insert(name.clone(), Value::Object(paths));
            }
            let variables = self.script_scopes[output]
                .variables
                .iter()
                .map(|(name, key)| {
                    budget.pair(name, &self.vars[key])?;
                    Ok((name.clone(), json!(self.vars[key])))
                })
                .collect::<Result<Map<_, _>>>()?;
            let mut themes = Map::new();
            for (_, name) in self.axes.keys() {
                let target = self.axis_target(output, name)?;
                budget.pair(name, &target)?;
                themes.insert(name.clone(), json!(target));
            }
            for (key, target) in &self.script_scopes[output].assets {
                budget.pair(key, target)?;
            }
            json!({"version":1,"script_uri":uri,"components":self.component_bindings(output,&mut budget)?,"input_components":self.component_bindings(input,&mut budget)?,
                "variables":variables,"themes":themes,"descendants":descendants,"assets":self.script_scopes[output].assets})
        };
        let inputs_hash = result.get("inputs").map(|value| {
            let mut value = value.clone();
            value.sort_all_objects();
            hash(value.to_string().as_bytes())
        });
        bindings["inputs_hash"] = json!(inputs_hash);
        self.script_binding_bytes = self.script_binding_bytes.saturating_add(budget.bytes());
        let metadata = result
            .entry("metadata")
            .or_insert_with(|| json!({"type":"lazypencil"}))
            .as_object_mut()
            .ok_or_else(|| copy_error("Script metadata must be an object"))?;
        let namespace = metadata
            .entry("lazypencil")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| copy_error("The lazypencil namespace must be an object"))?;
        namespace.remove("script_scope");
        namespace.insert("script_bindings".into(), bindings);
        let _ = origin;
        Ok(())
    }
}

fn relative_uri(origin: &Path, target: &Path) -> Result<String> {
    let parent = origin
        .parent()
        .ok_or_else(|| copy_error("A script scope needs a bound path"))?;
    for ancestor in parent.ancestors() {
        if let Ok(tail) = target.strip_prefix(ancestor) {
            let mut parts = vec![
                "..".to_owned();
                parent
                    .strip_prefix(ancestor)
                    .expect("ancestor")
                    .components()
                    .count()
            ];
            for part in tail.components() {
                parts.push(
                    part.as_os_str()
                        .to_str()
                        .ok_or_else(|| copy_error("Script asset paths must be UTF-8"))?
                        .to_owned(),
                );
            }
            return Ok(parts.join("/"));
        }
    }
    Err(copy_error(
        "A script asset cannot be made relative to its namespace",
    ))
}
