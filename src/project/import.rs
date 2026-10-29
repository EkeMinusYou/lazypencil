use super::*;
use crate::operations::{ImportOptions, NameResolution};
use uuid::Uuid;

mod browsers;
mod reuse;
mod samplers;
mod scripts;

enum DynamicInfo {
    Script(scripts::Info),
    Browser(browsers::Info),
}

impl DynamicInfo {
    fn script(&self) -> Option<&scripts::Info> {
        if let Self::Script(info) = self {
            Some(info)
        } else {
            None
        }
    }
    fn browser(&self) -> Option<&browsers::Info> {
        if let Self::Browser(info) = self {
            Some(info)
        } else {
            None
        }
    }
}

type Key = (PathBuf, String);
pub(crate) struct CopyPlan {
    pub definitions: Vec<Value>,
    pub imports: BTreeMap<String, Value>,
    pub variables: BTreeMap<String, Value>,
    pub themes: BTreeMap<String, Value>,
    pub files: Vec<crate::managed::File>,
    pub component_id: String,
    pub result: Value,
}

impl Project {
    pub(crate) fn attach_import_source(&mut self, alias: &str, uri: &str) -> Result<()> {
        let target = self.resolve_file(&self.root, uri)?;
        if !target
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pen"))
        {
            return Err(copy_error("Component imports require a .pen source"));
        }
        if self.imports[&self.root]
            .get(alias)
            .is_some_and(|existing| existing != &target)
        {
            return Err(copy_error(
                "The import alias is already bound to another library",
            ));
        }
        // This lookup edge is not a stored import. A copy may read a library that
        // depends on the consumer without introducing a cycle in the saved file.
        self.load_document(&target, None, &mut BTreeSet::new())?;
        self.imports
            .get_mut(&self.root)
            .expect("root imports")
            .insert(alias.into(), target);
        self.verify_unchanged()
    }
    pub(crate) fn component_reference(&self, alias: Option<&str>, id: &str) -> Result<String> {
        let path = self.library_path(alias)?;
        let node = self.documents[path].node(id)?;
        if node["reusable"] != true {
            return Err(copy_error("The import source must be a reusable component"));
        }
        let reference = alias.map_or_else(|| id.to_owned(), |alias| format!("{alias}:{id}"));
        let (resolved, actual) = self.lookup_node(&self.root, &reference)?;
        if &resolved != path || actual["id"] != id {
            return Err(copy_error(
                "A local node shadows the requested library reference",
            ));
        }
        self.expanded_library_node(
            alias,
            &NodeAddress {
                node_id: id.into(),
                instance_path: Vec::new(),
            },
        )?;
        Ok(reference)
    }

    pub(crate) fn copy_component(
        &mut self,
        alias: Option<&str>,
        id: &str,
        options: &ImportOptions,
        path: Option<&Path>,
    ) -> Result<CopyPlan> {
        let source = self.library_document(alias)?;
        if source.node(id)?["reusable"] != true {
            return Err(copy_error("The import source must be a reusable component"));
        }
        self.expanded_library_node(
            alias,
            &NodeAddress {
                node_id: id.into(),
                instance_path: Vec::new(),
            },
        )?;
        let selected = (self.library_path(alias)?.clone(), id.to_owned());
        for prefix in [&options.variable_prefix, &options.component_prefix]
            .into_iter()
            .flatten()
        {
            if prefix.is_empty() || prefix.len() > 128 || prefix.chars().any(char::is_control) {
                return Err(copy_error(
                    "Import prefixes must contain 1..128 bytes without control characters",
                ));
            }
        }
        let mut graph = Copier {
            project: self,
            options,
            path,
            owned: BTreeSet::new(),
            roots: BTreeSet::new(),
            queue: VecDeque::from([selected.clone()]),
            ids: BTreeMap::new(),
            reuse: BTreeMap::new(),
            reused_targets: BTreeMap::new(),
            reused_sources: BTreeMap::new(),
            reused_target_nodes: 0,
            reused_path_bytes: 0,
            names: BTreeMap::new(),
            vars: BTreeMap::new(),
            var_values: BTreeMap::new(),
            axes: BTreeMap::new(),
            axis_values: BTreeMap::new(),
            files: BTreeMap::new(),
            sampler_vars: BTreeSet::new(),
            sampler_bindings: BTreeMap::new(),
            copied_imports: BTreeMap::new(),
            used_components: BTreeSet::new(),
            used_variables: BTreeSet::new(),
            used_themes: BTreeSet::new(),
            script_scopes: BTreeMap::new(),
            script_images: BTreeSet::new(),
            used_script_assets: BTreeSet::new(),
            script_binding_bytes: 0,
            input_epoch: Uuid::new_v4().simple().to_string(),
            browser_scopes: BTreeMap::new(),
        };
        while let Some(key) = graph.queue.pop_front() {
            graph.project.check_cancelled()?;
            if graph.owned.contains(&key) || graph.reuse.contains_key(&key) {
                continue;
            }
            let node = graph.project.documents[&key.0].node(&key.1)?.clone();
            if node["reusable"] != true {
                return Err(copy_error(
                    "A copied reference must target a reusable definition",
                ));
            }
            if let Some(NameResolution::Reuse { target }) =
                options.components.get(&graph.key(&key)?)
            {
                graph.remember_reuse(&key, target)?;
                graph.used_components.insert(graph.key(&key)?);
                continue;
            }
            graph.roots.insert(key.clone());
            graph.scan_node(&key.0, &node)?;
        }
        let roots = graph
            .roots
            .iter()
            .filter(|key| {
                let mut parent = graph.project.documents[&key.0]
                    .location(&key.1)
                    .ok()
                    .and_then(|l| l.parent_id.clone());
                while let Some(id) = parent {
                    // A substituted ancestor no longer emits its original
                    // children. Independently referenced inner definitions
                    // must therefore remain standalone copied roots.
                    if graph.reuse.contains_key(&(key.0.clone(), id.clone())) {
                        break;
                    }
                    if graph.roots.contains(&(key.0.clone(), id.clone())) {
                        return false;
                    }
                    parent = graph.project.documents[&key.0]
                        .location(&id)
                        .ok()
                        .and_then(|l| l.parent_id.clone());
                }
                true
            })
            .cloned()
            .collect::<Vec<_>>();
        for key in &graph.owned {
            graph.project.check_cancelled()?;
            graph
                .ids
                .insert(key.clone(), format!("n{}", Uuid::new_v4().simple()));
        }
        graph.capture_script_images()?;
        graph.prepare_sampler_bindings()?;
        graph.prepare_script_assets()?;
        let mut definitions = Vec::new();
        for (origin, id) in &roots {
            let node = graph.project.documents[origin].node(id)?.clone();
            definitions.push(graph.rewrite_node(origin, &node)?);
        }
        let mut variables = BTreeMap::new();
        for (key, definition) in graph.var_values.clone() {
            let mut definition = graph.rewrite_value(&key.0, &definition)?;
            graph.rewrite_sampler_definition(&key, &mut definition);
            variables.insert(graph.vars[&key].clone(), definition);
        }
        for (provided, used) in [
            (&options.components, &graph.used_components),
            (&options.variables, &graph.used_variables),
            (&options.themes, &graph.used_themes),
        ] {
            if provided.keys().any(|key| !used.contains(key)) {
                return Err(copy_error(
                    "An import name resolution does not match a required source key",
                ));
            }
        }
        if options.component_paths.keys().any(|key| {
            !graph
                .reuse
                .keys()
                .any(|source| graph.key(source).is_ok_and(|value| &value == key))
        }) {
            return Err(copy_error(
                "Component paths must name an explicitly reused source component",
            ));
        }
        if options
            .script_assets
            .keys()
            .any(|key| !graph.used_script_assets.contains(key))
        {
            return Err(copy_error(
                "Additional script assets must name a copied script source key",
            ));
        }
        let component_id = graph.node_target(&selected)?;
        let mut map_bytes = 0usize;
        let ids = graph
            .ids
            .iter()
            .map(|(key, id)| graph.report_entry(key, id, &mut map_bytes))
            .collect::<Result<Map<_, _>>>()?;
        let variables_map = graph
            .vars
            .iter()
            .map(|(key, name)| graph.report_entry(key, name, &mut map_bytes))
            .collect::<Result<Map<_, _>>>()?;
        let themes_map = graph
            .axes
            .iter()
            .map(|(key, name)| graph.report_entry(key, name, &mut map_bytes))
            .collect::<Result<Map<_, _>>>()?;
        let reused = graph
            .reuse
            .iter()
            .map(|(key, id)| graph.report_entry(key, id, &mut map_bytes))
            .collect::<Result<Map<_, _>>>()?;
        let result = json!({"mode":"copy","id_map":ids,"reused_components":reused,"variable_map":variables_map,"theme_map":themes_map,
            "assets":graph.files.values().map(|file|json!({"uri":file.uri,"hash":hash(&file.bytes),"bytes":file.bytes.len()})).collect::<Vec<_>>()});
        Ok(CopyPlan {
            definitions,
            imports: graph.copied_imports,
            variables,
            themes: graph.axis_values,
            files: graph.files.into_values().collect(),
            component_id,
            result,
        })
    }

    pub(super) fn variable_key(&self, origin: &Path, name: &str) -> Result<Key> {
        if self.documents[origin]
            .value()
            .get("variables")
            .and_then(|v| v.get(name))
            .is_some()
        {
            return Ok((origin.into(), name.into()));
        }
        if let Some((alias, member)) = name.split_once(':')
            && let Some(path) = self
                .imports
                .get(origin)
                .and_then(|imports| imports.get(alias))
        {
            return self.variable_key(path, member);
        }
        let mut found = BTreeSet::new();
        for path in self
            .imports
            .get(origin)
            .into_iter()
            .flat_map(BTreeMap::values)
        {
            if self.documents[path]
                .value()
                .get("variables")
                .and_then(|v| v.get(name))
                .is_some()
            {
                found.insert(path.clone());
            }
        }
        if found.len() != 1 {
            return Err(copy_error("A copied variable is missing or ambiguous"));
        }
        Ok((
            found.into_iter().next().expect("single source"),
            name.into(),
        ))
    }
}

struct Copier<'a> {
    project: &'a mut Project,
    options: &'a ImportOptions,
    path: Option<&'a Path>,
    owned: BTreeSet<Key>,
    roots: BTreeSet<Key>,
    queue: VecDeque<Key>,
    ids: BTreeMap<Key, String>,
    reuse: BTreeMap<Key, String>,
    reused_targets: BTreeMap<String, BTreeMap<String, bool>>,
    reused_sources: BTreeMap<Key, reuse::Source>,
    reused_target_nodes: usize,
    reused_path_bytes: usize,
    names: BTreeMap<Key, String>,
    vars: BTreeMap<Key, String>,
    var_values: BTreeMap<Key, Value>,
    axes: BTreeMap<Key, String>,
    axis_values: BTreeMap<String, Value>,
    files: BTreeMap<PathBuf, crate::managed::File>,
    sampler_vars: BTreeSet<Key>,
    sampler_bindings: BTreeMap<Key, String>,
    copied_imports: BTreeMap<String, Value>,
    used_components: BTreeSet<String>,
    used_variables: BTreeSet<String>,
    used_themes: BTreeSet<String>,
    script_scopes: BTreeMap<PathBuf, scripts::Scope>,
    script_images: BTreeSet<(PathBuf, String, String)>,
    used_script_assets: BTreeSet<String>,
    script_binding_bytes: usize,
    input_epoch: String,
    browser_scopes: BTreeMap<PathBuf, String>,
}

impl Copier<'_> {
    fn bind_copied_library(&mut self, file: &crate::managed::File, mut alias: String) -> String {
        while self.project.imports[&self.project.root]
            .get(&alias)
            .is_some_and(|path| path != &file.path)
            || self.project.root_document().value()["variables"]
                .as_object()
                .is_some_and(|values| {
                    values
                        .keys()
                        .any(|name| name.starts_with(&format!("{alias}:")))
                })
            || self
                .copied_imports
                .get(&alias)
                .is_some_and(|uri| uri != file.uri.as_str())
        {
            alias.push('_');
        }
        self.copied_imports.insert(alias.clone(), json!(file.uri));
        alias
    }

    fn axis_target(&self, origin: &Path, axis: &str) -> Result<String> {
        if let Some(target) = self.axes.get(&(origin.into(), axis.into())) {
            return Ok(target.clone());
        }
        let targets = self
            .axes
            .iter()
            .filter(|((_, name), _)| name == axis)
            .map(|(_, target)| target.clone())
            .collect::<BTreeSet<_>>();
        match targets.len() {
            0 => Ok(axis.into()),
            1 => Ok(targets.into_iter().next().expect("single theme target")),
            _ => Err(copy_error(
                "An inherited theme axis has ambiguous copied names; reuse one common target",
            )),
        }
    }
    fn key(&self, key: &Key) -> Result<String> {
        let scope = self.project.detached_scope(&key.0)?;
        if scope.len().saturating_add(key.1.len()) > 4096 {
            return Err(copy_error(
                "A qualified import source key exceeds 4096 bytes",
            ));
        }
        fn escape(value: &str) -> String {
            value.replace('%', "%25").replace(':', "%3A")
        }
        let value = if scope.is_empty() {
            escape(&key.1)
        } else {
            format!(
                "{}:{}",
                scope.split(':').map(escape).collect::<Vec<_>>().join(":"),
                escape(&key.1)
            )
        };
        if value.len() > 4096 {
            return Err(copy_error(
                "An escaped import source key exceeds 4096 bytes",
            ));
        }
        Ok(value)
    }
    fn report_entry(&self, key: &Key, target: &str, bytes: &mut usize) -> Result<(String, Value)> {
        self.project.check_cancelled()?;
        let name = self.key(key)?;
        *bytes = bytes
            .saturating_add(name.len())
            .saturating_add(target.len())
            .saturating_add(16);
        if *bytes > 4 * 1024 * 1024 - 16384 {
            return Err(copy_error(
                "The copied ID and name report exceeds the batch response limit; import a smaller component",
            ));
        }
        Ok((name, json!(target)))
    }
    fn existing_component(&self, target: &str) -> Result<()> {
        if self.project.root_document().node(target)?["reusable"] != true {
            return Err(copy_error(
                "A reused component must be a local reusable definition",
            ));
        }
        Ok(())
    }
    fn node_target(&self, key: &Key) -> Result<String> {
        self.reuse
            .get(key)
            .or_else(|| self.ids.get(key))
            .cloned()
            .ok_or_else(|| copy_error("Missing copied component dependency"))
    }
    fn axis(&mut self, origin: &Path, name: &str, definition: &Value) -> Result<()> {
        self.project.check_cancelled()?;
        let key = (origin.into(), name.into());
        if self.axes.contains_key(&key) {
            return Ok(());
        }
        let source = self.key(&key)?;
        let policy = self.options.themes.get(&source);
        let target = match policy {
            Some(NameResolution::Rename { name }) => name.clone(),
            Some(NameResolution::Reuse { target }) => target.clone(),
            None => name.into(),
        };
        check_name(&target)?;
        if policy.is_some() {
            self.used_themes.insert(source.clone());
        }
        let existing = self
            .project
            .root_document()
            .value()
            .get("themes")
            .and_then(|v| v.get(&target))
            .or_else(|| self.axis_values.get(&target));
        let local = origin == self.project.root && target == name && policy.is_none();
        if matches!(policy, Some(NameResolution::Reuse { .. })) || local {
            let existing = existing
                .ok_or_else(|| copy_error("The requested theme reuse target does not exist"))?;
            let required = definition
                .as_array()
                .ok_or_else(|| copy_error("A theme axis must be an array"))?;
            if !required.iter().all(|value| {
                existing
                    .as_array()
                    .is_some_and(|values| values.contains(value))
            }) {
                return Err(copy_error("The reused theme axis lacks source values"));
            }
        } else {
            if existing.is_some() {
                return Err(conflict("theme", &source, &target));
            }
            self.axis_values.insert(target.clone(), definition.clone());
        }
        self.axes.insert(key, target);
        Ok(())
    }
    fn source_axes(&mut self, origin: &Path) -> Result<()> {
        let themes = self.project.documents[origin]
            .value()
            .get("themes")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (name, definition) in themes {
            self.axis(origin, &name, &definition)?;
        }
        Ok(())
    }
    fn variable(&mut self, origin: &Path, name: &str) -> Result<()> {
        self.project.check_cancelled()?;
        let key = self.project.variable_key(origin, name)?;
        if self.vars.contains_key(&key) {
            return Ok(());
        }
        let source = self.key(&key)?;
        let policy = self.options.variables.get(&source);
        let target = match policy {
            Some(NameResolution::Rename { name }) => name.clone(),
            Some(NameResolution::Reuse { target }) => target.clone(),
            None => format!(
                "{}{}",
                self.options.variable_prefix.as_deref().unwrap_or(""),
                key.1
            ),
        };
        check_name(&target)?;
        if policy.is_some() {
            self.used_variables.insert(source.clone());
        }
        let original = self.project.documents[&key.0].value()["variables"][&key.1].clone();
        let existing = self
            .project
            .root_document()
            .value()
            .get("variables")
            .and_then(|v| v.get(&target));
        let local = key.0 == self.project.root && target == key.1 && policy.is_none();
        let reuse = matches!(policy, Some(NameResolution::Reuse { .. })) || local;
        if reuse {
            if existing.is_none_or(|value| value["type"] != original["type"]) {
                return Err(copy_error(
                    "A reused variable must exist with the source variable's type",
                ));
            }
        } else if existing.is_some() || self.vars.values().any(|name| name == &target) {
            return Err(conflict("variable", &source, &target));
        }
        self.vars.insert(key.clone(), target);
        if !reuse {
            self.var_values.insert(key.clone(), original.clone());
            self.source_axes(&key.0)?;
            self.scan_value(&key.0, &original)?;
        }
        Ok(())
    }
    fn scan_value(&mut self, origin: &Path, value: &Value) -> Result<()> {
        self.project.check_cancelled()?;
        match value {
            Value::String(value) if value.starts_with('$') && value.len() > 1 => {
                self.variable(origin, &value[1..])?
            }
            Value::Array(values) => {
                for value in values {
                    self.scan_value(origin, value)?;
                }
            }
            Value::Object(object) => {
                for (key, value) in object {
                    if key == "uniforms" && value.is_object() {
                        for value in value.as_object().expect("uniforms").values() {
                            self.scan_value(origin, value)?;
                        }
                    } else if !literal_property(key) {
                        self.scan_value(origin, value)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn scan_node(&mut self, origin: &Path, node: &Value) -> Result<()> {
        self.project.check_cancelled()?;
        let id = node["id"]
            .as_str()
            .ok_or_else(|| copy_error("A copied node needs an ID"))?;
        let key = (origin.into(), id.into());
        if !self.owned.insert(key.clone()) {
            return Ok(());
        }
        if self.owned.len() > MAX_NODES {
            return Err(copy_error(
                "Copied components exceed the document node limit",
            ));
        }
        let kind = node["type"]
            .as_str()
            .ok_or_else(|| copy_error("A copied node needs a type"))?;
        if !crate::document::NODE_TYPES.contains(&kind) {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "An unknown node cannot be copied structurally",
            ));
        }
        if node["reusable"] == true {
            let source = self.key(&key)?;
            let policy = self.options.components.get(&source);
            if policy.is_some() {
                self.used_components.insert(source.clone());
            }
            if let Some(NameResolution::Reuse { target }) = policy {
                let target = target.clone();
                self.remember_reuse(&key, &target)?;
                let placement = reuse::placement(node);
                self.source_axes(origin)?;
                return self.scan_value(origin, &Value::Object(placement));
            }
            let name = match policy {
                Some(NameResolution::Rename { name }) => Some(name.clone()),
                _ => node.get("name").and_then(Value::as_str).map(|name| {
                    format!(
                        "{}{name}",
                        self.options.component_prefix.as_deref().unwrap_or("")
                    )
                }),
            };
            if let Some(name) = name {
                if name.is_empty() || name.chars().any(char::is_control) {
                    return Err(copy_error(
                        "Copied component names must be nonempty and contain no control characters",
                    ));
                }
                let existing = self.project.root_document().locations().keys().any(|id| {
                    self.project
                        .root_document()
                        .node(id)
                        .is_ok_and(|node| node["reusable"] == true && node["name"] == name)
                });
                if existing || self.names.values().any(|other| other == &name) {
                    return Err(conflict("component", &source, &name));
                }
                self.names.insert(key, name);
            }
        }
        self.source_axes(origin)?;
        self.scan_properties(origin, node)
    }
    fn scan_properties(&mut self, origin: &Path, node: &Value) -> Result<()> {
        let info = self.dynamic_info(origin, node)?;
        self.scan_properties_info(origin, node, info)
    }
    fn scan_properties_info(
        &mut self,
        origin: &Path,
        node: &Value,
        info: Option<DynamicInfo>,
    ) -> Result<()> {
        let script = info.as_ref().and_then(DynamicInfo::script);
        if let Some(info) = script {
            self.scan_script_info(info)?;
        }
        self.scan_sampler_values(origin, node)?;
        let object = node
            .as_object()
            .ok_or_else(|| copy_error("A copied node must be an object"))?;
        for (key, value) in object {
            if key == "inputs"
                && let Some(script) = script
            {
                let header = &script.header;
                for input in &header.inputs {
                    if let Some(value) = value.get(&input.name) {
                        let (source, value) =
                            self.script_input_expression(&script.resolved, &input.name, value)?;
                        self.scan_value(&source, &value)?;
                    }
                }
            } else if !literal_property(key) && key != "children" {
                self.scan_value(origin, value)?;
            }
        }
        if node["type"] == "ref"
            && let Some(reference) = node["ref"].as_str()
        {
            let (source, definition) = self.project.lookup_node(origin, reference)?;
            self.queue.push_back((
                source,
                definition["id"].as_str().expect("definition ID").into(),
            ));
        }
        for reference in node
            .get("slot")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Ok((source, definition)) = self.project.lookup_node(origin, reference)
                && definition["reusable"] == true
            {
                self.queue.push_back((
                    source,
                    definition["id"].as_str().expect("definition ID").into(),
                ));
            }
        }
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.scan_node(origin, child)?;
        }
        if node["type"] == "ref"
            && let Some(overrides) = node.get("descendants").and_then(Value::as_object)
        {
            let mut expanded =
                self.project
                    .expand(origin, node, None, &[], &mut Vec::new(), &mut 0)?;
            for (path, changes) in overrides {
                if changes.get("type").is_none() {
                    let target =
                        find_descendant_mut(&mut expanded, &path.split('/').collect::<Vec<_>>())
                            .ok_or_else(|| copy_error("A copied override target is missing"))?;
                    if matches!(target.kind(), "script" | "browser") {
                        let info = self.dynamic_info_resolved(
                            target.clone(),
                            (
                                origin.into(),
                                format!("{}/{path}", node["id"].as_str().expect("instance ID")),
                            ),
                        )?;
                        self.scan_properties_info(origin, changes, info)?;
                    } else {
                        self.scan_properties(origin, changes)?;
                    }
                } else {
                    self.scan_node(origin, changes)?;
                }
            }
        }
        Ok(())
    }
    fn rewrite_value(&self, origin: &Path, value: &Value) -> Result<Value> {
        self.project.check_cancelled()?;
        Ok(match value {
            Value::String(value) if value.starts_with('$') && value.len() > 1 => {
                let key = self.project.variable_key(origin, &value[1..])?;
                json!(format!(
                    "${}",
                    self.vars
                        .get(&key)
                        .ok_or_else(|| copy_error("Missing copied variable dependency"))?
                ))
            }
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|v| self.rewrite_value(origin, v))
                    .collect::<Result<_>>()?,
            ),
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| {
                        let value = if key == "theme" {
                            let values = value
                                .as_object()
                                .ok_or_else(|| copy_error("Theme values must be an object"))?;
                            Value::Object(
                                values
                                    .iter()
                                    .map(|(axis, value)| {
                                        Ok((self.axis_target(origin, axis)?, value.clone()))
                                    })
                                    .collect::<Result<_>>()?,
                            )
                        } else if key == "uniforms" && value.is_object() {
                            Value::Object(
                                value
                                    .as_object()
                                    .expect("uniforms")
                                    .iter()
                                    .map(|(key, value)| {
                                        Ok((key.clone(), self.rewrite_value(origin, value)?))
                                    })
                                    .collect::<Result<_>>()?,
                            )
                        } else if literal_property(key) {
                            value.clone()
                        } else {
                            self.rewrite_value(origin, value)?
                        };
                        Ok((key.clone(), value))
                    })
                    .collect::<Result<_>>()?,
            ),
            _ => value.clone(),
        })
    }
    fn asset(&mut self, origin: &Path, uri: &str, kind: &str) -> Result<String> {
        let path = self.project.resolve_file(origin, uri)?;
        self.project.capture_asset(&path, kind)?;
        let bytes = self.project.asset(&path)?.bytes.clone();
        let extension = match kind {
            "shader" => "glsl",
            "script" => "js",
            "image" => image_extension(&bytes)?,
            _ => {
                return Err(copy_error(
                    "Copied images must contain PNG, JPEG, or WebP data",
                ));
            }
        };
        let file = crate::managed::File::new(
            self.path
                .ok_or_else(|| copy_error("Copying assets requires a bound document path"))?,
            bytes,
            extension,
        )?;
        let uri = file.uri.clone();
        self.files.insert(file.path.clone(), file);
        Ok(uri)
    }
    fn rewrite_node(&mut self, origin: &Path, node: &Value) -> Result<Value> {
        self.rewrite_properties(origin, node, true)
    }
    fn reused_placement(&self, origin: &Path, node: &Value) -> Result<Option<Value>> {
        if self.reuse.is_empty() {
            return Ok(None);
        }
        let key = (origin.into(), node["id"].as_str().expect("node ID").into());
        if let Some(target) = self.reuse.get(&key).cloned() {
            let mut placement = reuse::placement(node);
            // Only placement expressions belong to the source namespace. The
            // selected definition and its code/assets belong to the consumer.
            for (key, value) in &mut placement {
                if key == "theme" || !literal_property(key) {
                    *value = self.rewrite_value(origin, &json!({key.clone():value.clone()}))?[key]
                        .clone();
                }
            }
            placement.insert("id".into(), json!(self.placement_target(&key)?));
            placement.insert("type".into(), json!("ref"));
            placement.insert("ref".into(), json!(target));
            return Ok(Some(Value::Object(placement)));
        }
        Ok(None)
    }
    fn rewrite_properties(&mut self, origin: &Path, node: &Value, is_node: bool) -> Result<Value> {
        self.project.check_cancelled()?;
        if is_node && let Some(placement) = self.reused_placement(origin, node)? {
            return Ok(placement);
        }
        let info = self.dynamic_info(origin, node)?;
        self.rewrite_properties_info(origin, node, is_node, info)
    }
    fn rewrite_properties_info(
        &mut self,
        origin: &Path,
        node: &Value,
        is_node: bool,
        info: Option<DynamicInfo>,
    ) -> Result<Value> {
        let script = info.as_ref().and_then(DynamicInfo::script);
        let mut result = node
            .as_object()
            .ok_or_else(|| copy_error("A copied override must be an object"))?
            .clone();
        if is_node {
            let key = (origin.into(), node["id"].as_str().expect("node ID").into());
            result.insert(
                "id".into(),
                json!(
                    self.ids
                        .get(&key)
                        .ok_or_else(|| copy_error("Missing copied node ID"))?
                ),
            );
            if let Some(name) = self.names.get(&key) {
                result.insert("name".into(), json!(name));
            }
        }
        for (key, value) in &mut result {
            if key == "inputs"
                && let Some(script) = script
            {
                let header = &script.header;
                if let Some(values) = value.as_object_mut() {
                    for input in &header.inputs {
                        if let Some(value) = values.get_mut(&input.name) {
                            *value = self.copied_input_value(script, &input.name, value)?;
                        }
                    }
                }
            } else if key == "theme" || (!literal_property(key) && key != "children") {
                let wrapper = json!({key.clone():value.clone()});
                *value = self.rewrite_value(origin, &wrapper)?[key].clone();
            }
        }
        if node["type"] == "ref" {
            let reference = node["ref"]
                .as_str()
                .ok_or_else(|| copy_error("A copied instance needs a ref"))?;
            let (source, definition) = self.project.lookup_node(origin, reference)?;
            result.insert(
                "ref".into(),
                json!(self.node_target(&(
                    source,
                    definition["id"].as_str().expect("definition ID").into()
                ))?),
            );
        }
        if let Some(slot) = result.get_mut("slot").and_then(Value::as_array_mut) {
            for reference in slot {
                if let Some(name) = reference.as_str()
                    && let Ok((source, definition)) = self.project.lookup_node(origin, name)
                {
                    let key = (
                        source,
                        definition["id"].as_str().expect("definition ID").into(),
                    );
                    if let Ok(target) = self.node_target(&key) {
                        *reference = json!(target);
                    }
                }
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            result.insert(
                "children".into(),
                Value::Array(
                    children
                        .iter()
                        .map(|child| self.rewrite_node(origin, child))
                        .collect::<Result<_>>()?,
                ),
            );
        }
        if let Some(overrides) = node.get("descendants").and_then(Value::as_object) {
            let mut expanded =
                self.project
                    .expand(origin, node, None, &[], &mut Vec::new(), &mut 0)?;
            let mut remapped = Map::new();
            let mapped_paths =
                self.copied_override_paths(&expanded, overrides.keys().map(String::as_str))?;
            let mut entries = overrides.iter().collect::<Vec<_>>();
            // Keep the source's application order when an instance boundary
            // disappears and previously different depths become equal.
            entries.sort_by_key(|(path, _)| path.split('/').count());
            for (path, changes) in entries {
                let parts = path.split('/').collect::<Vec<_>>();
                let mapped_path = mapped_paths[path].clone();
                let value = if changes.get("type").is_some() {
                    self.rewrite_node(origin, changes)?
                } else {
                    let target =
                        find_descendant_mut(&mut expanded, &parts).expect("validated override");
                    let info = if matches!(target.kind(), "script" | "browser") {
                        self.dynamic_info_resolved(
                            target.clone(),
                            (
                                origin.into(),
                                format!("{}/{path}", node["id"].as_str().expect("instance ID")),
                            ),
                        )?
                    } else {
                        None
                    };
                    self.rewrite_properties_info(origin, changes, false, info)?
                };
                if remapped.insert(mapped_path, value).is_some() {
                    return Err(copy_error(
                        "Copied override paths collide after component reuse",
                    ));
                }
            }
            result.insert("descendants".into(), Value::Object(remapped));
        }
        for key in ["fill", "stroke", "effect"] {
            if let Some(payload) = result.get_mut(key) {
                let fills = match payload {
                    Value::Array(values) => values.iter_mut().collect::<Vec<_>>(),
                    value => vec![value],
                };
                for fill in fills {
                    let shader = fill["type"] == "shader";
                    if fill["type"] == "image" || shader {
                        let kind = if shader { "shader" } else { "image" };
                        if let Some(uri) = fill
                            .get("url")
                            .and_then(Value::as_str)
                            .filter(|uri| !uri.is_empty())
                        {
                            let uri = self.asset(origin, uri, kind)?;
                            fill["url"] = json!(uri);
                        }
                        if shader
                            && let Some(uniforms) =
                                fill.get_mut("uniforms").and_then(Value::as_object_mut)
                        {
                            for value in uniforms.values_mut().flat_map(shader_values_mut) {
                                if let Some(uri) = value.as_str() {
                                    if uri.starts_with('$') {
                                        continue;
                                    }
                                    if !uri.is_empty()
                                        && !uri.starts_with('#')
                                        && !uri.starts_with("pencil:")
                                    {
                                        let uri = self.asset(origin, uri, "image")?;
                                        *value = json!(uri);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.rewrite_script(node, &mut result, script)?;
        self.rewrite_browser(
            node,
            &mut result,
            info.as_ref().and_then(DynamicInfo::browser),
        )?;
        Ok(Value::Object(result))
    }
}

fn image_extension(bytes: &[u8]) -> Result<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok("jpg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Ok("webp")
    } else {
        Err(copy_error(
            "Copied images must contain PNG, JPEG, or WebP data",
        ))
    }
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains(':')
        || name.starts_with('$')
        || name.chars().any(char::is_control)
    {
        return Err(copy_error(
            "Imported variable and theme names must be nonempty, unprefixed, and contain no ':' or control characters",
        ));
    }
    Ok(())
}
fn copy_error(message: impl Into<String>) -> Error {
    Error::new("INVALID_OPERATION", message)
}
fn conflict(kind: &str, source: &str, target: &str) -> Error {
    copy_error("An imported name conflicts; explicitly rename or reuse it")
        .details(json!({"kind":kind,"source_key":source,"target_name":target}))
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[test]
    fn cancelling_a_loaded_project_stops_component_expansion_copy_and_source_checks() {
        let document = Document::parse(
            json!({"version":"2.21","children":[
                {"id":"button","type":"frame","reusable":true,"children":[
                    {"id":"label","type":"text","content":"Save"}
                ]}, {"id":"instance","type":"ref","ref":"button"}
            ]})
            .to_string(),
        )
        .unwrap();
        let mut project = Project::local(document).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        project.set_cancellation(Some(&cancel));
        project.resolve().unwrap();
        cancel.cancel();
        let address = NodeAddress {
            node_id: "instance".into(),
            instance_path: vec![],
        };
        assert_eq!(
            project.expanded_node(&address).unwrap_err().code,
            "CANCELLED"
        );
        assert_eq!(
            project.detached_template(&address).unwrap_err().code,
            "CANCELLED"
        );
        assert_eq!(
            project
                .copy_component(None, "button", &ImportOptions::default(), None)
                .err()
                .unwrap()
                .code,
            "CANCELLED"
        );
        assert_eq!(project.verify_unchanged().unwrap_err().code, "CANCELLED");
        project.set_cancellation(None);
        project.resolve().unwrap();
    }
}
