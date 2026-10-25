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
    pub(super) resolved: ResolvedNode,
    uri: String,
    source: Key,
}

impl Copier<'_> {
    pub(super) fn script_info_resolved(
        &mut self,
        resolved: ResolvedNode,
        source: Key,
    ) -> Result<Info> {
        let uri = resolved
            .properties
            .get("scriptUri")
            .and_then(Value::as_str)
            .ok_or_else(|| copy_error("A copied script needs scriptUri"))?
            .to_owned();
        let path = self
            .project
            .resolve_file(resolved.property_source("scriptUri"), &uri)?;
        self.project.capture_asset(&path, "script")?;
        let code = std::str::from_utf8(&self.project.asset(&path)?.bytes)
            .map_err(|_| copy_error("Copied script source must be UTF-8"))?;
        Ok(Info {
            header: Header::parse(code)?,
            resolved,
            uri,
            source,
        })
    }

    pub(super) fn scan_script_info(&mut self, info: &Info) -> Result<()> {
        self.script_scope(info.resolved.script_origin())?;
        self.script_scope(info.resolved.script_input_origin())?;
        if info.resolved.properties.contains_key("inputs") {
            self.script_scope(info.resolved.property_source("inputs"))?;
        }
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
        let source_key = self.key(&info.source)?;
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
        Ok(())
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
                let Ok(path) = self
                    .project
                    .resolve_file(&key.0, self.project.sampler_uri(&key.0, uri))
                else {
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
                let source_uri = if kind == "image" {
                    self.project.sampler_uri(&owner, &uri).to_owned()
                } else {
                    uri.clone()
                };
                let path = self.project.resolve_file(&owner, &source_uri)?;
                let target = self.asset(&owner, &source_uri, &kind)?;
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

    pub(super) fn script_input_expression(
        &self,
        origin: &Path,
        node: &ResolvedNode,
        name: &str,
        value: &Value,
    ) -> Result<(PathBuf, Value)> {
        if let Some(context) = &node.input_context {
            Ok((
                context.origin.clone(),
                context.values.values.get(name).unwrap_or(value).clone(),
            ))
        } else {
            Ok((origin.into(), value.clone()))
        }
    }

    pub(super) fn copied_input_value(&self, info: &Info, name: &str, raw: &Value) -> Result<Value> {
        let node = &info.resolved;
        let (origin, value) =
            self.script_input_expression(node.property_source("inputs"), node, name, raw)?;
        if let Some(variable) = value.as_str().and_then(|v| v.strip_prefix('$')) {
            let known = node
                .input_context
                .as_ref()
                .is_none_or(|context| context.values.variables.values().any(|v| v == variable));
            if known
                && let Ok(key) = self.project.variable_key(&origin, variable)
                && let Some(target) = self.vars.get(&key)
            {
                return Ok(json!(format!("${target}")));
            }
            return Ok(value);
        }
        if info
            .header
            .inputs
            .iter()
            .any(|input| input.name == name && matches!(input.kind, InputKind::Ref))
            && let Some(reference) = value.as_str().filter(|v| !v.is_empty())
        {
            let (origin, reference) = if let Some(context) = &node.input_context {
                let target = context
                    .values
                    .components
                    .get(reference)
                    .map(String::as_str)
                    .unwrap_or(reference);
                (context.origin.as_path(), target)
            } else if let Some(bound) = &node.script_bindings
                && bound.inputs_bound
            {
                let map = bound
                    .values
                    .input_components
                    .as_ref()
                    .unwrap_or(&bound.values.components);
                (
                    bound.origin.as_path(),
                    map.get(reference).map(String::as_str).unwrap_or(reference),
                )
            } else {
                (node.script_input_origin(), reference)
            };
            let (path, definition) = self.project.lookup_node(origin, reference)?;
            return Ok(json!(self.node_target(&(
                path,
                definition["id"].as_str().expect("ref ID").into()
            ))?));
        }
        Ok(value)
    }

    fn input_context(&self, info: &Info, budget: &mut Budget) -> Result<Option<Value>> {
        let node = &info.resolved;
        let Some(values) = node.properties.get("inputs").and_then(Value::as_object) else {
            return Ok(None);
        };
        let variables = if let Some(old) = &node.input_context {
            composition::mapped(&old.values.variables, budget, |name| {
                self.vars
                    .get(&(old.origin.clone(), name.into()))
                    .cloned()
                    .ok_or_else(|| copy_error("Missing copied input variable"))
            })?
        } else {
            self.script_scopes[node.property_source("inputs")]
                .variables
                .iter()
                .map(|(name, key)| {
                    budget.pair(name, &self.vars[key])?;
                    Ok((name.clone(), json!(self.vars[key])))
                })
                .collect::<Result<Map<_, _>>>()?
        };
        let components = if let Some(old) = &node.input_context {
            composition::mapped(&old.values.components, budget, |id| {
                self.node_target(&(old.origin.clone(), id.into()))
            })?
        } else if let Some(bound) = &node.script_bindings
            && bound.inputs_bound
        {
            composition::mapped(
                bound
                    .values
                    .input_components
                    .as_ref()
                    .unwrap_or(&bound.values.components),
                budget,
                |id| self.node_target(&(bound.origin.clone(), id.into())),
            )?
        } else {
            self.component_bindings(node.script_input_origin(), budget)?
        };
        let values = values
            .iter()
            .map(|(name, value)| {
                let value = self.copied_input_value(info, name, value)?;
                budget.key(name)?;
                budget.charge(value.to_string().len() + 4)?;
                Ok((name.clone(), value))
            })
            .collect::<Result<Map<_, _>>>()?;
        Ok(Some(
            json!({"version":1,"values":values,"variables":variables,"components":components}),
        ))
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
        node: &Value,
        result: &mut Map<String, Value>,
        info: Option<&Info>,
    ) -> Result<()> {
        let Some(info) = info else { return Ok(()) };
        let old_carrier = super::super::script_context::override_value(
            node.as_object().expect("script properties"),
        )?;
        let metadata = old_carrier
            .as_ref()
            .map_or(node.get("metadata").is_some(), |v| v.metadata.is_some());
        let raw_metadata = old_carrier
            .as_ref()
            .map_or(node.get("metadata"), |v| v.metadata.as_ref());
        let (metadata_output, metadata_input) =
            super::super::script_context::metadata_contexts(raw_metadata);
        let full = node["type"] == "script";
        let output = full
            || metadata_output
            || node.get("scriptUri").is_some()
            || old_carrier.as_ref().is_some_and(|v| v.output.is_some());
        let input_context = full
            || metadata_input
            || node.get("inputs").is_some()
            || old_carrier.as_ref().is_some_and(|v| v.input.is_some());
        if !full && !metadata && !output && !input_context {
            if old_carrier.is_some() {
                result.remove("metadata");
            }
            return Ok(());
        }
        if metadata {
            result.insert(
                "metadata".into(),
                info.resolved
                    .properties
                    .get("metadata")
                    .cloned()
                    .unwrap_or(json!({"type":"lazypencil"})),
            );
        } else {
            result.remove("metadata");
        }
        let mut budget = Budget::new(self.script_binding_bytes);
        let output_scope = info.resolved.script_origin();
        let input = info.resolved.script_input_origin();
        let uri = self.asset(
            info.resolved.property_source("scriptUri"),
            &info.uri,
            "script",
        )?;
        if node["type"] == "script" || node.get("scriptUri").is_some() {
            result.insert("scriptUri".into(), json!(uri));
        }
        let mut bindings = if !output {
            Value::Null
        } else if let Some(bound) = &info.resolved.script_bindings {
            let key = self.key(&info.source)?;
            let additional = self
                .options
                .script_assets
                .get(&key)
                .cloned()
                .unwrap_or_default();
            self.compose_bindings(bound, &uri, input, output_scope, &additional, &mut budget)?
        } else {
            let mut descendants = Map::new();
            for (name, key) in &self.script_scopes[output_scope].components {
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
            let variables = self.script_scopes[output_scope]
                .variables
                .iter()
                .map(|(name, key)| {
                    budget.pair(name, &self.vars[key])?;
                    Ok((name.clone(), json!(self.vars[key])))
                })
                .collect::<Result<Map<_, _>>>()?;
            let mut themes = Map::new();
            for (_, name) in self.axes.keys() {
                let target = self.axis_target(output_scope, name)?;
                budget.pair(name, &target)?;
                themes.insert(name.clone(), json!(target));
            }
            for (key, target) in &self.script_scopes[output_scope].assets {
                budget.pair(key, target)?;
            }
            json!({"version":1,"script_uri":uri,"components":self.component_bindings(output_scope,&mut budget)?,"input_components":self.component_bindings(input,&mut budget)?,
                "variables":variables,"themes":themes,"descendants":descendants,"assets":self.script_scopes[output_scope].assets})
        };
        let inputs_hash = result.get("inputs").map(|value| {
            let mut value = value.clone();
            value.sort_all_objects();
            hash(value.to_string().as_bytes())
        });
        if output {
            bindings["inputs_hash"] = json!(inputs_hash);
        }
        let context = if input_context {
            self.input_context(info, &mut budget)?
        } else {
            None
        };
        self.script_binding_bytes = self.script_binding_bytes.saturating_add(budget.bytes());
        if !full {
            let mut user_metadata = result.remove("metadata");
            if user_metadata
                .as_ref()
                .is_some_and(|value| !value.is_object())
            {
                return Err(copy_error("Script metadata must be an object"));
            }
            if let Some(namespace) = user_metadata
                .as_mut()
                .and_then(|v| v.get_mut("lazypencil"))
                .and_then(Value::as_object_mut)
            {
                for key in [
                    "script_bindings",
                    "script_scope",
                    "script_input_context",
                    "script_override",
                ] {
                    namespace.remove(key);
                }
                if namespace.is_empty() {
                    user_metadata
                        .as_mut()
                        .expect("metadata")
                        .as_object_mut()
                        .expect("metadata object")
                        .remove("lazypencil");
                }
            }
            result.insert("metadata".into(),json!({"type":"lazypencil","lazypencil":{"script_override":{"version":1,"metadata":user_metadata,"output":if output {Some(bindings)} else {None},"input":context}}}));
            return Ok(());
        }
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
        namespace.remove("script_input_context");
        if let Some(context) = context {
            namespace.insert("script_input_context".into(), context);
        }
        Ok(())
    }
}

pub(super) fn relative_uri(origin: &Path, target: &Path) -> Result<String> {
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
