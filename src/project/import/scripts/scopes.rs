//! Capture each declared execution namespace using shared component path tables.
use super::*;
use crate::project::bindings::CapturedScope;

fn private_key(values: &Value, section: &str) -> String {
    let mut index = values[section].as_object().expect("binding table").len();
    loop {
        let key = format!("@lazypencil/scope/{section}/{index}");
        if values[section].get(&key).is_none() {
            return key;
        }
        index += 1;
    }
}

impl Copier<'_> {
    fn scope_component(
        &self,
        key: &Key,
        name: &str,
        values: &mut Value,
        budget: &mut Budget,
    ) -> Result<()> {
        let id = self.node_target(key)?;
        budget.pair(name, &id)?;
        values["components"][name] = json!(id);
        let definition = self.project.documents[&key.0].node(&key.1)?;
        let expanded =
            self.project
                .expand(&key.0, definition, None, &[], &mut Vec::new(), &mut 0)?;
        let mut paths = Map::new();
        let mut contexts = Map::new();
        self.override_paths(&expanded, &mut paths, &mut contexts, budget)?;
        values["descendants"][name] = json!(paths);
        values["descendant_contexts"][name] = json!(contexts);
        Ok(())
    }
    pub(super) fn capture_script_scopes(
        &mut self,
        origin: &Path,
        values: &mut Value,
        budget: &mut Budget,
    ) -> Result<()> {
        let mut identities = BTreeMap::new();
        for (alias, path) in &self.script_scopes[origin].browsers {
            identities
                .entry(path.clone())
                .or_insert_with(|| alias.clone());
        }
        let mut components = BTreeMap::new();
        for (name, key) in &self.script_scopes[origin].components {
            components
                .entry(key.clone())
                .or_insert_with(|| name.clone());
        }
        let mut variables = BTreeMap::new();
        for (name, key) in &self.script_scopes[origin].variables {
            variables.entry(key.clone()).or_insert_with(|| name.clone());
        }
        let root_assets: BTreeMap<String, String> =
            serde_json::from_value(values["assets"].clone()).expect("assets");
        let browser_libraries: BTreeMap<String, String> =
            serde_json::from_value(values["browser_libraries"].clone()).expect("browser libraries");
        let mut result = Map::new();
        for (path, identity) in &identities {
            self.project.check_cancelled()?;
            let source = &self.script_scopes[path];
            let mut scope = CapturedScope {
                components: BTreeMap::new(),
                variables: BTreeMap::new(),
                themes: BTreeMap::new(),
                assets: source
                    .assets
                    .iter()
                    .filter(|(_, target)| root_assets.values().any(|root| root == *target))
                    .map(|(name, target)| (name.clone(), target.clone()))
                    .collect(),
                libraries: source
                    .browsers
                    .iter()
                    .map(|(name, path)| (name.clone(), identities[path].clone()))
                    .collect(),
                browser_library: self
                    .project
                    .browser_resources
                    .contains_key(path)
                    .then(|| browser_libraries[identity].clone()),
                browser_libraries: source
                    .browsers
                    .iter()
                    .map(|(name, path)| {
                        (name.clone(), browser_libraries[&identities[path]].clone())
                    })
                    .collect(),
            };
            for (name, key) in source.components.clone() {
                let target = if let Some(target) = components.get(&key) {
                    target.clone()
                } else {
                    let target = private_key(values, "components");
                    self.scope_component(&key, &target, values, budget)?;
                    components.insert(key, target.clone());
                    target
                };
                scope.components.insert(name, target);
            }
            for (name, key) in self.script_scopes[path].variables.clone() {
                let target = if let Some(target) = variables.get(&key) {
                    target.clone()
                } else {
                    let target = private_key(values, "variables");
                    budget.pair(&target, &self.vars[&key])?;
                    values["variables"][&target] = json!(self.vars[&key]);
                    variables.insert(key, target.clone());
                    target
                };
                scope.variables.insert(name, target);
            }
            for document in &self.script_scopes[path].documents {
                if let Some(axes) = self.project.documents[document].value()["themes"].as_object() {
                    for name in axes.keys() {
                        scope.themes.insert(name.clone(), name.clone());
                    }
                }
            }
            // URI aliases are relative to this source document, while all
            // targets remain inside the parent's original closed manifest.
            for (key, target) in &root_assets {
                let (kind, uri) = key.split_once(':').expect("asset key");
                let source = if kind == "image" {
                    self.project.sampler_uri(origin, uri)
                } else {
                    uri
                };
                if let Ok(file) = self.project.resolve_file(origin, source) {
                    let relative = relative_uri(path, &file)?;
                    let key = crate::project::bindings::asset_key(kind, &relative)?;
                    if let Some(previous) = scope.assets.insert(key, target.clone())
                        && previous != *target
                    {
                        return Err(Error::new(
                            "ASSET_CONFLICT",
                            "Captured script scope assets conflict",
                        ));
                    }
                }
            }
            let scope = serde_json::to_value(scope).expect("captured scope");
            budget.key(identity)?;
            budget.charge(scope.to_string().len() + 4)?;
            result.insert(identity.clone(), scope);
        }
        values["script_scopes"] = Value::Object(result);
        values["version"] = json!(2);
        Ok(())
    }

    pub(super) fn compose_script_scopes(
        &mut self,
        bound: &crate::project::bindings::Bound,
        values: &mut Value,
        budget: &mut Budget,
    ) -> Result<()> {
        let original = bound.original();
        if original.script_scopes.is_empty() {
            return Ok(());
        }
        let mut result = BTreeMap::new();
        let mut identities: BTreeMap<String, String> = BTreeMap::new();
        for (name, scope) in &original.script_scopes {
            self.project.check_cancelled()?;
            let mut scope = scope.clone();
            for key in scope.components.clone().into_values() {
                let previous = &original.components[&key];
                if scope.components.contains_key(previous) {
                    continue;
                }
                let target = if let Some(key) = identities.get(previous) {
                    key.clone()
                } else {
                    let key = if !original.components.contains_key(previous) {
                        previous.clone()
                    } else {
                        let key = private_key(values, "components");
                        self.scope_component(
                            &(bound.origin.clone(), previous.clone()),
                            &key,
                            values,
                            budget,
                        )?;
                        key
                    };
                    identities.insert(previous.clone(), key.clone());
                    key
                };
                scope.components.insert(previous.clone(), target);
            }
            for (members, table) in [
                (&mut scope.variables, &original.variables),
                (&mut scope.themes, &original.themes),
            ] {
                for key in members.clone().into_values() {
                    members.entry(table[&key].clone()).or_insert(key);
                }
            }
            let mut assets = BTreeMap::new();
            for (key, uri) in &scope.assets {
                let (kind, _) = key.split_once(':').expect("asset key");
                let target = self.asset(&bound.origin, uri, kind)?;
                assets.insert(key.clone(), target.clone());
                assets
                    .entry(crate::project::bindings::asset_key(kind, uri)?)
                    .or_insert(target);
            }
            scope.assets = assets;
            if let Some(alias) = &scope.browser_library {
                let owner = self.project.scope_document(&bound.origin, alias)?;
                scope.browser_library = Some(self.copied_browser_scope(&owner)?);
            }
            let mut browsers = BTreeMap::new();
            for (name, alias) in scope
                .browser_libraries
                .iter()
                .chain(scope.browser_libraries.values().map(|alias| (alias, alias)))
            {
                if browsers.contains_key(name) {
                    continue;
                }
                let owner = self.project.scope_document(&bound.origin, alias)?;
                browsers.insert(name.clone(), self.copied_browser_scope(&owner)?);
            }
            scope.browser_libraries = browsers;
            budget.key(name)?;
            budget.charge(serde_json::to_string(&scope).expect("scope").len() + 4)?;
            result.insert(name.clone(), scope);
        }
        values["script_scopes"] = json!(result);
        values["version"] = json!(2);
        if let Some(active) = &original.script_namespace {
            values["script_namespace"] = json!(active);
        }
        Ok(())
    }
}
