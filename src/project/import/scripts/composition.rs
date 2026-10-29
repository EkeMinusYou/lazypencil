use super::*;
use crate::project::bindings::Bound;

const LIMIT: usize = 4 * 1024 * 1024;

pub(super) struct Budget {
    bytes: usize,
    previous: usize,
}

impl Budget {
    pub(super) fn new(previous: usize) -> Self {
        Self {
            bytes: 8192,
            previous,
        }
    }
    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > LIMIT
            || self.previous.saturating_add(self.bytes) > crate::document::MAX_DOCUMENT_BYTES
        {
            return Err(copy_error(
                "Copied script bindings exceed the per-script or aggregate document byte budget",
            ));
        }
        Ok(())
    }
    pub(super) fn key(&mut self, key: &str) -> Result<()> {
        if key.len() > 4096 {
            return Err(copy_error(
                "A copied script binding name exceeds 4096 bytes",
            ));
        }
        self.charge(serde_json::to_string(key).expect("string").len() + 4)
    }
    pub(super) fn pair(&mut self, key: &str, value: &str) -> Result<()> {
        self.key(key)?;
        if value.len() > 4096 {
            return Err(copy_error(
                "A copied script binding target exceeds 4096 bytes",
            ));
        }
        self.charge(serde_json::to_string(value).expect("string").len() + 2)
    }
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
}

pub(super) fn mapped(
    source: &BTreeMap<String, String>,
    budget: &mut Budget,
    mut target: impl FnMut(&str) -> Result<String>,
) -> Result<Map<String, Value>> {
    let mut result = Map::new();
    // A literal key shadows the identity fallback in the original runtime.
    for (name, old) in source
        .iter()
        .chain(source.values().map(|value| (value, value)))
    {
        if result.contains_key(name) {
            continue;
        }
        let value = target(old)?;
        budget.pair(name, &value)?;
        result.insert(name.clone(), json!(value));
    }
    Ok(result)
}

impl Copier<'_> {
    pub(super) fn copy_browser_libraries(
        &mut self,
        origin: &Path,
        budget: &mut Budget,
    ) -> Result<Value> {
        let sources = self.script_scopes[origin].browsers.clone();
        let mut result = Map::new();
        for (name, source) in sources {
            self.project.check_cancelled()?;
            let target = self.copied_browser_scope(&source)?;
            budget.pair(&name, &target)?;
            result.insert(name, json!(target));
        }
        Ok(Value::Object(result))
    }

    pub(super) fn compose_bindings(
        &mut self,
        bound: &Bound,
        uri: &str,
        input: &Path,
        output: &Path,
        additional: &[crate::operations::ScriptAsset],
        budget: &mut Budget,
    ) -> Result<Value> {
        let components = mapped(&bound.values.components, budget, |id| {
            self.node_target(&(bound.origin.clone(), id.into()))
        })?;
        let input_components = if bound.inputs_bound {
            mapped(
                bound
                    .values
                    .input_components
                    .as_ref()
                    .unwrap_or(&bound.values.components),
                budget,
                |id| self.node_target(&(bound.origin.clone(), id.into())),
            )?
        } else {
            self.component_bindings(input, budget)?
        };
        let variables = mapped(&bound.values.variables, budget, |name| {
            self.vars
                .get(&(bound.origin.clone(), name.into()))
                .cloned()
                .ok_or_else(|| copy_error("Missing re-copied script variable"))
        })?;
        let themes = mapped(&bound.values.themes, budget, |name| {
            self.axis_target(&bound.origin, name)
        })?;
        let mut descendants = Map::new();
        let mut descendant_contexts = Map::new();
        for name in components.keys() {
            self.project.check_cancelled()?;
            let original = bound
                .values
                .components
                .get(name)
                .map(String::as_str)
                .unwrap_or(name);
            let definition = self.project.documents[&bound.origin].node(original)?;
            let expanded = self.project.expand(
                &bound.origin,
                definition,
                None,
                &[],
                &mut Vec::new(),
                &mut 0,
            )?;
            let mut paths = Map::new();
            let mut contexts = Map::new();
            budget.key(name)?;
            if bound.values.components.contains_key(name) {
                if let Some(old) = bound.values.descendants.get(name) {
                    let mapped =
                        self.copied_override_nodes(&expanded, old.values().map(String::as_str))?;
                    for (from, to) in old {
                        let target = &mapped[to].0;
                        budget.pair(from, target)?;
                        paths.insert(from.clone(), json!(target));
                    }
                    if let Some(previous) = bound.values.descendant_contexts.get(name) {
                        for (path, context) in previous {
                            let mut context = context.clone();
                            context.target_instance = old
                                .get(path)
                                .and_then(|target| mapped.get(target))
                                .is_some_and(|(_, instance)| *instance);
                            let context = serde_json::to_value(context).expect("context");
                            budget.key(path)?;
                            budget.charge(context.to_string().len() + 4)?;
                            contexts.insert(path.clone(), context);
                        }
                    }
                }
            } else {
                self.override_paths(&expanded, &mut paths, &mut contexts, budget)?;
            }
            descendants.insert(name.clone(), json!(paths));
            if !bound.values.components.contains_key(name)
                || bound.values.descendant_contexts.contains_key(name)
            {
                descendant_contexts.insert(name.clone(), json!(contexts));
            }
        }
        let mut assets = Map::new();
        if let Some(old) = &bound.values.assets {
            for (key, value) in old {
                let (kind, _) = key.split_once(':').expect("validated asset binding");
                let target = self.asset(&bound.origin, value, kind)?;
                budget.pair(key, &target)?;
                assets.insert(key.clone(), json!(target));
            }
            for (key, value) in old {
                let (kind, _) = key.split_once(':').expect("validated asset binding");
                let alias = crate::project::bindings::asset_key(kind, value)?;
                if !assets.contains_key(&alias) {
                    let target = self.asset(&bound.origin, value, kind)?;
                    budget.pair(&alias, &target)?;
                    assets.insert(alias, json!(target));
                }
            }
        } else {
            for (key, target) in &self.script_scopes[&bound.origin].assets {
                budget.pair(key, target)?;
                assets.insert(key.clone(), json!(target));
            }
        }
        for asset in additional {
            let kind = match asset.kind {
                crate::operations::ScriptAssetKind::Image => "image",
                crate::operations::ScriptAssetKind::Shader => "shader",
                crate::operations::ScriptAssetKind::Script => "script",
            };
            let key = crate::project::bindings::asset_key(kind, &asset.uri)?;
            let target = self.asset(output, &asset.uri, kind)?;
            if let Some(previous) = assets.get(&key) {
                if previous != &json!(target) {
                    return Err(Error::new(
                        "ASSET_CONFLICT",
                        "Additional assets conflict with a preserved script binding",
                    ));
                }
            } else {
                budget.pair(&key, &target)?;
                assets.insert(key, json!(target));
            }
        }
        let mut values = json!({"version":1,"script_uri":uri,"components":components,"input_components":input_components,
            "variables":variables,"themes":themes,"descendants":descendants,"descendant_contexts":descendant_contexts,"assets":assets});
        let browser_origin = if let Some(alias) = &bound.values.browser_library {
            Some(self.project.scope_document(&bound.origin, alias)?)
        } else if bound.values.assets.is_none()
            && self.project.browser_resources.contains_key(&bound.origin)
        {
            Some(bound.origin.clone())
        } else {
            None
        };
        if let Some(origin) = browser_origin {
            let alias = self.copied_browser_scope(&origin)?;
            budget.pair("browser_library", &alias)?;
            values["browser_library"] = json!(alias);
        }
        // Preserve original names and the previous copy's names, matching the
        // identity fallback used by the generated runtime. Never import an
        // unrelated destination namespace as a replacement for a missing name.
        if let Some(old) = &bound.values.browser_libraries {
            values["browser_libraries"] = Value::Object(mapped(old, budget, |alias| {
                let origin = self.project.scope_document(&bound.origin, alias)?;
                self.copied_browser_scope(&origin)
            })?);
        } else if bound.values.assets.is_none() {
            values["browser_libraries"] = self.copy_browser_libraries(&bound.origin, budget)?;
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_binding_budget_counts_escaped_names_and_all_preceding_scripts() {
        let mut normal = Budget::new(0);
        normal.pair("quoted\"name", "target").unwrap();
        assert!(normal.bytes() > 8192 + "quoted\"name".len() + "target".len());
        assert!(
            Budget::new(crate::document::MAX_DOCUMENT_BYTES)
                .pair("name", "target")
                .is_err()
        );
        assert!(Budget::new(0).pair(&"x".repeat(4097), "target").is_err());
        let mut full = Budget::new(0);
        while full.bytes() <= LIMIT {
            if full.pair(&"x".repeat(4096), &"y".repeat(4096)).is_err() {
                return;
            }
        }
        panic!("A binding budget must reject growth before allocating a larger map");
    }
}
