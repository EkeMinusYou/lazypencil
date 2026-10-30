//! Remap a script's generated references without rewriting its JavaScript.

use super::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "rendering")]
mod code;
#[cfg(feature = "rendering")]
mod custom;
#[cfg(feature = "rendering")]
mod generated;
#[cfg(feature = "rendering")]
pub(super) use generated::{
    clear_pending_scopes, has_pending_scopes, has_scope as has_generated_scope,
};
#[cfg(feature = "rendering")]
mod paths;
mod scopes;
pub(super) use scopes::CapturedScope;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DescendantContext {
    pub parent: String,
    pub instance: bool,
    pub target_instance: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bindings {
    pub(super) version: u32,
    pub(super) script_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) source_uri_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) input_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) code_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) library: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) browser_library: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) browser_libraries: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) inputs_hash: Option<String>,
    #[serde(default)]
    pub(super) components: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) input_components: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) variables: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) themes: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) descendants: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) descendant_contexts: BTreeMap<String, BTreeMap<String, DescendantContext>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) assets: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) script_scopes: BTreeMap<String, CapturedScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) script_namespace: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct Bound {
    pub(super) origin: PathBuf,
    pub(super) inputs_bound: bool,
    pub(super) values: Bindings,
    pub(super) captured: Option<Box<Bindings>>,
}

#[cfg(feature = "rendering")]
#[derive(Debug, Default)]
pub(super) struct GeneratedState {
    pub uri_hash: Option<String>,
    pub bound_script: bool,
    pub children: Vec<GeneratedState>,
    pub descendants: BTreeMap<String, GeneratedState>,
}

impl Serialize for Bound {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct State<'a> {
            origin: &'a Path,
            inputs_bound: bool,
            values: &'a Bindings,
        }
        State {
            origin: &self.origin,
            inputs_bound: self.inputs_bound,
            values: self.original(),
        }
        .serialize(serializer)
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

pub(super) fn valid_epoch(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn bind(project: &Project, node: &ResolvedNode) -> Result<Option<Bound>> {
    if node.kind() != "script" {
        return Ok(None);
    }
    let Some(value) = node
        .properties
        .get("metadata")
        .and_then(|value| value.get("lazypencil"))
        .and_then(|value| value.get("script_bindings"))
    else {
        return Ok(None);
    };
    bind_value(project, node, value, node.property_source("metadata"))
}

fn parse(value: &Value) -> Result<Bindings> {
    if value.to_string().len() > 4 * 1024 * 1024 {
        return Err(invalid("Script bindings exceed 4 MiB"));
    }
    let values: Bindings = serde_json::from_value(value.clone())
        .map_err(|_| invalid("Invalid lazypencil script bindings"))?;
    if ![1, 2, 3, 4].contains(&values.version) {
        return Err(Error::new(
            "UNSUPPORTED_VERSION",
            "Unsupported script bindings version",
        ));
    }
    let mut maps = vec![&values.components, &values.variables, &values.themes];
    if let Some(inputs) = &values.input_components {
        maps.push(inputs);
    }
    if let Some(assets) = &values.assets {
        maps.push(assets);
    }
    let count = maps.iter().map(|map| map.len()).sum::<usize>()
        + values
            .descendants
            .values()
            .map(BTreeMap::len)
            .sum::<usize>();
    if count + values.browser_libraries.as_ref().map_or(0, BTreeMap::len) > MAX_NODES
        || !valid_name(&values.script_uri)
        || values
            .library
            .as_ref()
            .is_some_and(|alias| alias.len() > 4096 || alias.chars().any(char::is_control))
        || values
            .browser_library
            .as_ref()
            .is_some_and(|alias| !valid_name(alias))
        || values.browser_libraries.as_ref().is_some_and(|map| {
            map.iter().any(|(name, target)| {
                (!name.is_empty() && !valid_name(name)) || !valid_name(target)
            })
        })
        || maps
            .iter()
            .flat_map(|map| map.iter())
            .any(|(key, value)| !valid_name(key) || !valid_name(value))
        || values.descendants.iter().any(|(key, paths)| {
            !values.components.contains_key(key)
                || paths
                    .iter()
                    .any(|(from, to)| !valid_name(from) || !valid_name(to))
        })
        || values.inputs_hash.as_ref().is_some_and(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(invalid("Invalid or oversized script binding names"));
    }
    if values
        .input_epoch
        .as_ref()
        .is_some_and(|value| values.version < 3 || !valid_epoch(value))
    {
        return Err(invalid("Invalid script input capture generation"));
    }
    if values.source_uri_hash.as_ref().is_some_and(|value| {
        value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) || values.source_uri_hash.is_some() != (values.version >= 3)
    {
        return Err(invalid("Invalid script source provenance"));
    }
    for (name, contexts) in &values.descendant_contexts {
        if !values.components.contains_key(name) || contexts.len() > MAX_NODES {
            return Err(invalid("Invalid script descendant contexts"));
        }
        for (path, context) in contexts {
            if !valid_name(path) || (!context.parent.is_empty() && !valid_name(&context.parent)) {
                return Err(invalid("Invalid script descendant context path"));
            }
            let mut parent = context.parent.as_str();
            let mut depth = 0;
            while !parent.is_empty() {
                depth += 1;
                if parent == path || depth > 256 {
                    return Err(invalid("Cyclic or oversized script descendant ancestry"));
                }
                parent = &contexts
                    .get(parent)
                    .ok_or_else(|| invalid("Missing script descendant parent"))?
                    .parent;
            }
        }
    }
    scopes::validate(&values)?;
    Ok(values)
}

/// Inspect the original URI before asset relocation can make stale metadata
/// match again. Valid explicit bindings still need independent composition.
#[cfg(feature = "rendering")]
pub(super) fn discard_inactive_generated_bindings(node: &mut Value) -> Result<()> {
    let Some(value) = node.pointer("/metadata/lazypencil/script_bindings") else {
        return Ok(());
    };
    let bindings = parse(value)?;
    if node.get("scriptUri").and_then(Value::as_str) != Some(bindings.script_uri.as_str()) {
        node.pointer_mut("/metadata/lazypencil")
            .and_then(Value::as_object_mut)
            .expect("validated script bindings namespace")
            .remove("script_bindings");
    }
    Ok(())
}

pub(super) fn bind_value(
    project: &Project,
    node: &ResolvedNode,
    value: &Value,
    owner: &Path,
) -> Result<Option<Bound>> {
    let values = parse(value)?;
    // An explicit source override must not revive a stale binding, even when
    // another document uses an identical URI. Same-value instance overrides
    // are separately revoked by apply_properties.
    if node.properties.get("scriptUri").and_then(Value::as_str) != Some(values.script_uri.as_str())
    {
        return Ok(None);
    }
    let origin = match &values.library {
        Some(alias) => project.scope_document(owner, alias)?,
        None => owner.to_path_buf(),
    };
    let document = &project.documents[&origin];
    if let Some(alias) = &values.browser_library {
        let browser_origin = project.scope_document(&origin, alias)?;
        if !project.browser_resources.contains_key(&browser_origin) {
            return Err(invalid(
                "A script browser binding needs a captured resource namespace",
            ));
        }
    }
    for alias in values.browser_libraries.iter().flat_map(BTreeMap::values) {
        project.check_cancelled()?;
        let browser_origin = project.scope_document(&origin, alias)?;
        if !project.browser_resources.contains_key(&browser_origin) {
            return Err(invalid(
                "A script browser scope map needs captured resource namespaces",
            ));
        }
    }
    for target in values.components.values() {
        project.check_cancelled()?;
        if document.node(target)?["reusable"] != true {
            return Err(invalid(
                "A script binding must target a local reusable component",
            ));
        }
    }
    if let Some(inputs) = &values.input_components {
        for target in inputs.values() {
            if document.node(target)?["reusable"] != true {
                return Err(invalid(
                    "A saved input binding must target a local reusable component",
                ));
            }
        }
    }
    if let Some(assets) = &values.assets {
        for key in assets.keys() {
            let (kind, uri) = key
                .split_once(':')
                .ok_or_else(|| invalid("A script asset binding needs a kind and URI"))?;
            if asset_key(kind, uri)? != *key {
                return Err(invalid("Script asset binding keys must be normalized"));
            }
        }
    }
    for (section, names) in [("variables", &values.variables), ("themes", &values.themes)] {
        for target in names.values() {
            project.check_cancelled()?;
            if document.value()[section].get(target).is_none() {
                return Err(invalid(
                    "A script binding target is missing from its document",
                ));
            }
        }
    }
    let inputs_bound = values.inputs_hash.as_ref().is_some_and(|expected| {
        node.properties.get("inputs").is_some_and(|inputs| {
            let mut inputs = inputs.clone();
            inputs.sort_all_objects();
            hash(inputs.to_string().as_bytes()) == *expected
        })
    });
    Ok(Some(Bound::from_values(origin, inputs_bound, values)?))
}

impl Bound {
    pub(super) fn equivalent(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.inputs_bound == other.inputs_bound
            && self.values.components == other.values.components
            && self.values.input_components == other.values.input_components
            && self.values.variables == other.values.variables
            && self.values.themes == other.values.themes
            && self.values.descendants == other.values.descendants
            && self.values.descendant_contexts == other.values.descendant_contexts
            && self.values.assets == other.values.assets
            && self.values.browser_library == other.values.browser_library
            && self.values.browser_libraries == other.values.browser_libraries
            && self.original().script_scopes == other.original().script_scopes
            && self.original().script_namespace == other.original().script_namespace
            && self.original().source_uri_hash == other.original().source_uri_hash
            && self.original().input_epoch == other.original().input_epoch
            && self.original().code_namespace == other.original().code_namespace
    }
}

#[cfg(feature = "rendering")]
impl Bound {
    pub(super) fn input_component<'a>(&'a self, name: &'a str, saved: bool) -> Result<&'a str> {
        if saved && let Some(map) = &self.values.input_components {
            return map
                .get(name)
                .map(String::as_str)
                .or_else(|| map.values().any(|value| value == name).then_some(name))
                .ok_or_else(|| {
                    invalid("A saved input reference is absent from the script bindings")
                });
        }
        self.component(name)
    }

    fn asset(&self, kind: &str, uri: &str) -> Result<String> {
        if kind == "image" && crate::inline_images::is_data(uri) {
            crate::inline_images::validate(uri)?;
            return Ok(uri.into());
        }
        let Some(assets) = &self.values.assets else {
            return Ok(uri.into());
        };
        if uri.is_empty() {
            return Ok(uri.into());
        }
        assets.get(&asset_key(kind,uri)?).cloned()
            .or_else(||assets.iter().any(|(key,target)|key.starts_with(&format!("{kind}:")) && target==uri).then(||uri.to_owned()))
            .ok_or_else(||Error::new("ASSET_NOT_FOUND","A generated file is outside the copied script asset manifest; include it in script_assets when copying"))
    }

    fn assets(&self, node: &mut Value, inherited_bytes: &mut usize) -> Result<()> {
        if self.values.assets.is_none() || node["enabled"] == false {
            return Ok(());
        }
        if node["type"] == "browser" {
            self.document_scope()?
                .generated_browser_context(node, true, inherited_bytes)?;
        }
        if let Some(uri) = node.get_mut("scriptUri")
            && let Some(value) = uri.as_str()
        {
            *uri = json!(self.asset("script", value)?);
        }
        for property in ["fill", "stroke", "effect"] {
            if let Some(value) = node.get_mut(property) {
                for fill in shader_values_mut(value) {
                    if fill["enabled"] == false {
                        continue;
                    }
                    let kind = match fill["type"].as_str() {
                        Some("image") => "image",
                        Some("shader") => "shader",
                        _ => continue,
                    };
                    if let Some(uri) = fill.get_mut("url")
                        && let Some(value) = uri.as_str()
                    {
                        *uri = json!(self.asset(kind, value)?);
                    }
                    if kind == "shader"
                        && let Some(uniforms) =
                            fill.get_mut("uniforms").and_then(Value::as_object_mut)
                    {
                        for value in uniforms.values_mut().flat_map(shader_values_mut) {
                            if let Some(uri) = value.as_str().filter(|uri| {
                                !uri.is_empty()
                                    && !uri.starts_with(['$', '#'])
                                    && !uri.starts_with("pencil:")
                            }) {
                                *value = json!(self.asset("image", uri)?);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn component<'a>(&'a self, name: &'a str) -> Result<&'a str> {
        self.values
            .components
            .get(name)
            .map(String::as_str)
            .or_else(|| {
                self.values
                    .components
                    .values()
                    .any(|target| target == name)
                    .then_some(name)
            })
            .ok_or_else(|| {
                invalid("A generated component reference is absent from the script bindings")
            })
    }

    fn value(&self, value: &mut Value) -> Result<()> {
        rewrite_property_variables(value, Some(&self.values.variables), true)
    }

    pub(super) fn declared_input(&self, value: &mut Value) -> Result<()> {
        self.value(value)
    }

    #[cfg(test)]
    pub(super) fn inherited_output_with_hash(
        &self,
        uri: &str,
        inputs: Option<&Value>,
        uri_hash: Option<String>,
        inherited_bytes: &mut usize,
    ) -> Result<Value> {
        Ok(serde_json::to_value(self.inherited_output_values(
            uri,
            inputs,
            uri_hash,
            inherited_bytes,
        )?)
        .expect("bindings"))
    }

    pub(super) fn inherited_output_values(
        &self,
        uri: &str,
        inputs: Option<&Value>,
        uri_hash: Option<String>,
        inherited_bytes: &mut usize,
    ) -> Result<Bindings> {
        *inherited_bytes = inherited_bytes
            .saturating_add(serde_json::to_vec(self.original()).expect("bindings").len())
            .saturating_add(8192);
        if *inherited_bytes > crate::script::MAX_OUTPUT_BYTES {
            return Err(invalid(
                "Inherited script bindings exceed the generated output budget",
            ));
        }
        let mut values = self.original().clone();
        // Generated nodes already belong to the resolved namespace owner.
        values.library = None;
        values.script_uri = uri.into();
        values.source_uri_hash = uri_hash;
        // An emitted node's source can differ from this generator's source.
        // Its code ownership must be captured separately, never inherited by guess.
        values.code_namespace = None;
        if values.source_uri_hash.is_none()
            && values
                .script_scopes
                .values()
                .any(|scope| scope.document_namespace.is_some())
        {
            return Err(invalid(
                "A document execution namespace requires original source URI provenance",
            ));
        }
        values.version = if values.source_uri_hash.is_some() {
            if values
                .script_scopes
                .values()
                .any(|scope| scope.document_namespace.is_some())
            {
                4
            } else {
                3
            }
        } else if values.script_scopes.is_empty() {
            1
        } else {
            2
        };
        if values.version < 3 {
            values.input_epoch = None;
        }
        values.input_components = None;
        values.inputs_hash = inputs.map(|value| {
            let mut value = value.clone();
            value.sort_all_objects();
            hash(value.to_string().as_bytes())
        });
        Ok(values)
    }

    pub(super) fn rewrite(
        &self,
        node: &mut Value,
        handles: &BTreeMap<String, (PathBuf, String)>,
        inherited_bytes: &mut usize,
        project: &Project,
        ids: &BTreeMap<String, String>,
    ) -> Result<GeneratedState> {
        let mut state = GeneratedState {
            uri_hash: node
                .get("scriptUri")
                .and_then(Value::as_str)
                .map(|uri| hash(uri.as_bytes())),
            ..GeneratedState::default()
        };
        if self.values.assets.is_some()
            && node.get("scriptUri").is_some()
            && node["enabled"] != false
            && node
                .get("type")
                .is_none_or(|kind| matches!(kind.as_str(), Some("script" | "ref")))
        {
            if node["type"] == "script" {
                let inputs = if node.pointer("/metadata/lazypencil/script_scope").is_some() {
                    node.get("inputs")
                        .map(|raw| self.generated_inputs(raw).map(|(_, raw)| raw))
                        .transpose()?
                } else {
                    None
                };
                super::scope::discard_inactive_generated_scope(node, inputs.as_ref())?;
            }
            if node["type"] == "script" {
                discard_inactive_generated_bindings(node)?;
            }
        }
        let selected = if self.values.assets.is_some() {
            generated::selection(self, node)?
        } else {
            None
        };
        self.assets(node, inherited_bytes)?;
        if let Some(selection) = &selected {
            selection.apply(node, inherited_bytes)?;
        }
        let original_reference = node.get("ref").and_then(Value::as_str).map(str::to_owned);
        let remapped = if let Some(changes) = node
            .as_object_mut()
            .and_then(|node| node.remove("descendants"))
        {
            let Value::Object(changes) = changes else {
                return Err(invalid("Generated overrides must be objects"));
            };
            Some(paths::remap(
                Some(self),
                &self.origin,
                node,
                changes,
                handles,
                project,
                ids,
            )?)
        } else {
            None
        };
        if node["type"] == "ref" {
            let name = original_reference
                .as_deref()
                .ok_or_else(|| invalid("A generated ref needs its target"))?;
            if !handles.contains_key(name) {
                node["ref"] = json!(self.component(name)?);
            }
        }
        if let Some(theme) = node.get_mut("theme").and_then(Value::as_object_mut) {
            let mut remapped = Map::new();
            for (axis, value) in std::mem::take(theme) {
                let target = self.values.themes.get(&axis).cloned().unwrap_or(axis);
                if remapped.insert(target, value).is_some() {
                    return Err(invalid("Generated theme axes collide after script binding"));
                }
            }
            *theme = remapped;
        }
        // A replacement header determines which controls are active. Keep raw
        // inputs until that captured header is available; stale controls must
        // not bind to either the copied namespace or a same-named caller value.
        let deferred_inputs = if self.values.assets.is_some()
            && (selected.is_some() && node["type"] == "script"
                || generated::has_scope(node)
                || node.get("scriptUri").is_some()
                    && (node["type"] == "ref" || node.get("type").is_none()))
        {
            node.as_object_mut().and_then(|node| node.remove("inputs"))
        } else {
            None
        };
        let rewritten = self.value(node);
        if let Some(inputs) = deferred_inputs {
            node.as_object_mut()
                .expect("generated properties")
                .insert("inputs".into(), inputs);
        }
        rewritten?;
        if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
            for child in children {
                state
                    .children
                    .push(self.rewrite(child, handles, inherited_bytes, project, ids)?);
            }
        }
        if let Some(slot) = node.get_mut("slot").and_then(Value::as_array_mut) {
            for value in slot {
                if let Some(target) = value
                    .as_str()
                    .and_then(|name| self.values.components.get(name))
                {
                    *value = json!(target);
                }
            }
        }
        if let Some(entries) = remapped {
            let mut rewritten = Map::new();
            for (path, mut properties) in entries {
                let context =
                    self.rewrite(&mut properties, handles, inherited_bytes, project, ids)?;
                state.descendants.insert(path.clone(), context);
                if rewritten.insert(path, properties).is_some() {
                    return Err(invalid(
                        "Generated override paths collide after script binding",
                    ));
                }
            }
            node["descendants"] = Value::Object(rewritten);
        }
        if node["type"] == "script" && self.values.assets.is_some() && node["enabled"] != false {
            let uri = node["scriptUri"]
                .as_str()
                .ok_or_else(|| invalid("A generated script needs scriptUri"))?;
            let values = if let Some(selection) = &selected {
                selection.output.output_with_code_owner(
                    uri,
                    None,
                    state.uri_hash.clone(),
                    Some(self),
                    inherited_bytes,
                )?
            } else {
                self.document_scope()?.output_with_code_owner(
                    uri,
                    node.get("inputs"),
                    state.uri_hash.clone(),
                    Some(self),
                    inherited_bytes,
                )?
            };
            let metadata = node
                .as_object_mut()
                .expect("script")
                .entry("metadata")
                .or_insert_with(|| json!({"type":"lazypencil"}))
                .as_object_mut()
                .ok_or_else(|| invalid("Generated script metadata must be an object"))?;
            let namespace = metadata
                .entry("lazypencil")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| invalid("Generated script namespace must be an object"))?;
            if selected.is_none()
                && (namespace.contains_key("script_scope")
                    || namespace.contains_key("script_bindings"))
            {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Generated explicit script scopes need independent scope composition",
                ));
            }
            namespace.insert("script_bindings".into(), values);
            state.bound_script = true;
        }
        Ok(state)
    }
}

#[cfg(feature = "rendering")]
fn rewrite_property_variables(
    value: &mut Value,
    variables: Option<&BTreeMap<String, String>>,
    strict: bool,
) -> Result<()> {
    match value {
        Value::String(text) if text.starts_with('$') && text.len() > 1 => {
            let variables = variables.ok_or_else(|| Error::new(
                "UNSUPPORTED_FEATURE",
                "The native inputs lack original variable provenance; copy them again from their source",
            ))?;
            let name = &text[1..];
            let target = variables
                .get(name)
                .or_else(|| variables.values().find(|target| target.as_str() == name));
            if let Some(target) = target {
                *text = format!("${target}");
            } else if strict {
                return Err(invalid(
                    "A generated variable reference is absent from the script bindings",
                ));
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_property_variables(value, variables, strict)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key == "uniforms" || key == "inputs" {
                    if let Some(values) = value.as_object_mut() {
                        for value in values.values_mut() {
                            rewrite_property_variables(value, variables, strict)?;
                        }
                    } else {
                        rewrite_property_variables(value, variables, strict)?;
                    }
                } else if key != "children" && !literal_property(key) {
                    rewrite_property_variables(value, variables, strict)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(feature = "rendering")]
pub(super) fn rewrite_generated_paths(
    project: &Project,
    node: &mut Value,
    owner: &Path,
    handles: &BTreeMap<String, (PathBuf, String)>,
    ids: &BTreeMap<String, String>,
) -> Result<()> {
    project.check_cancelled()?;
    let changes = node
        .as_object_mut()
        .and_then(|node| node.remove("descendants"));
    if let Some(changes) = changes {
        let Value::Object(changes) = changes else {
            return Err(invalid("Generated overrides must be objects"));
        };
        let mut mapped = Map::new();
        for (path, mut properties) in
            paths::remap(None, owner, node, changes, handles, project, ids)?
        {
            rewrite_generated_paths(project, &mut properties, owner, handles, ids)?;
            if mapped.insert(path, properties).is_some() {
                return Err(invalid(
                    "Generated override paths collide after script binding",
                ));
            }
        }
        node["descendants"] = Value::Object(mapped);
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            rewrite_generated_paths(project, child, owner, handles, ids)?;
        }
    }
    Ok(())
}

pub(super) fn asset_key(kind: &str, uri: &str) -> Result<String> {
    if !matches!(kind, "image" | "shader" | "script")
        || !valid_name(uri)
        || uri.contains("://")
        || uri.starts_with("file:")
    {
        return Err(invalid(
            "A copied script asset needs a local file URI and known kind",
        ));
    }
    let path = Path::new(uri);
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // a/.. may follow a symlink. Preserve that traversal spelling;
                // the copy planner separately records its verified target.
                parts.push("..".to_owned());
            }
            std::path::Component::RootDir => {}
            std::path::Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| invalid("Script asset paths must be UTF-8"))?
                    .to_owned(),
            ),
            _ => return Err(invalid("Unsupported script asset path prefix")),
        }
    }
    Ok(format!(
        "{kind}:{}{}",
        if path.is_absolute() { "/" } else { "" },
        parts.join("/")
    ))
}

#[cfg(test)]
mod asset_tests;

#[cfg(test)]
mod tests;
