//! Translate authored binding tables through the captured, closed namespace.
use super::*;

struct Composition {
    values: Bindings,
    bytes: usize,
}

impl Composition {
    fn charge(&mut self, value: &impl Serialize) -> Result<()> {
        self.bytes = self
            .bytes
            .saturating_add(serde_json::to_vec(value).expect("binding data").len());
        if self.bytes > crate::script::MAX_OUTPUT_BYTES {
            return Err(invalid(
                "Custom script bindings exceed the generated output budget",
            ));
        }
        Ok(())
    }

    fn component(
        &mut self,
        owner: &Bound,
        namespace: &str,
        name: &str,
        target: &str,
        paths: Option<&BTreeMap<String, String>>,
        contexts: Option<&BTreeMap<String, DescendantContext>>,
    ) -> Result<String> {
        let id = owner.component(target)?.to_owned();
        let key = private_key(namespace, "components", name);
        if self.values.components.get(&key) == Some(&id) {
            return Ok(key);
        }
        self.charge(&(&key, &id))?;
        self.values.components.insert(key.clone(), id);
        let source = owner
            .values
            .components
            .contains_key(target)
            .then_some(target)
            .or_else(|| {
                owner
                    .values
                    .components
                    .iter()
                    .find(|(_, value)| value.as_str() == target)
                    .map(|(name, _)| name.as_str())
            })
            .expect("captured component");
        let mut mapped = BTreeMap::new();
        for (from, to) in paths.into_iter().flat_map(BTreeMap::iter) {
            let target = owner
                .values
                .descendants
                .get(source)
                .and_then(|paths| paths.get(to))
                .ok_or_else(|| {
                    invalid("A custom descendant path is outside its captured component")
                })?;
            self.charge(&(from, target))?;
            mapped.insert(from.clone(), target.clone());
        }
        self.values.descendants.insert(key.clone(), mapped);
        if let Some(contexts) = contexts {
            let mut mapped = BTreeMap::new();
            for (path, context) in contexts {
                let target_path = paths.and_then(|paths| paths.get(path)).ok_or_else(|| {
                    invalid("A custom descendant context needs its declared path")
                })?;
                let target = owner
                    .values
                    .descendant_contexts
                    .get(source)
                    .and_then(|contexts| contexts.get(target_path))
                    .ok_or_else(|| {
                        invalid("A custom descendant context lacks captured ancestry")
                    })?;
                let mut context = context.clone();
                context.target_instance = target.target_instance;
                self.charge(&(path, &context))?;
                mapped.insert(path.clone(), context);
            }
            self.values.descendant_contexts.insert(key.clone(), mapped);
        }
        Ok(key)
    }

    fn identity_component(
        &mut self,
        owner: &Bound,
        namespace: &str,
        target: &str,
    ) -> Result<String> {
        let id = owner.component(target)?.to_owned();
        let key = private_key(namespace, "identity", target);
        if self.values.components.get(&key) == Some(&id) {
            return Ok(key);
        }
        let source = owner
            .values
            .components
            .contains_key(target)
            .then_some(target)
            .or_else(|| {
                owner
                    .values
                    .components
                    .iter()
                    .find(|(_, value)| value.as_str() == target)
                    .map(|(name, _)| name.as_str())
            })
            .expect("captured component");
        self.charge(&(&key, &id))?;
        self.values.components.insert(key.clone(), id);
        if let Some(paths) = owner.values.descendants.get(source) {
            self.charge(paths)?;
            self.values.descendants.insert(key.clone(), paths.clone());
        }
        if let Some(contexts) = owner.values.descendant_contexts.get(source) {
            self.charge(contexts)?;
            self.values
                .descendant_contexts
                .insert(key.clone(), contexts.clone());
        }
        Ok(key)
    }
}

fn private_key(namespace: &str, section: &str, name: &str) -> String {
    format!(
        "@lazypencil/generated/{}",
        hash(json!([namespace, section, name]).to_string().as_bytes())
    )
}

fn table(
    values: &BTreeMap<String, String>,
    owner: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    let mut bytes = 0usize;
    for (name, target) in values
        .iter()
        .chain(values.values().map(|target| (target, target)))
    {
        if result.contains_key(name) {
            continue;
        }
        let mapped = owner
            .get(target)
            .or_else(|| owner.values().find(|value| *value == target))
            .ok_or_else(|| invalid("A custom binding target is outside its captured namespace"))?;
        bytes = bytes.saturating_add(
            serde_json::to_vec(&(name, mapped))
                .expect("binding pair")
                .len(),
        );
        if bytes > crate::script::MAX_OUTPUT_BYTES {
            return Err(invalid(
                "Custom script bindings exceed the generated output budget",
            ));
        }
        result.insert(name.clone(), mapped.clone());
    }
    Ok(result)
}

fn assets(
    values: Option<&BTreeMap<String, String>>,
    owner: &Bound,
) -> Result<BTreeMap<String, String>> {
    let Some(values) = values else {
        return Ok(owner.values.assets.clone().unwrap_or_default());
    };
    let mut result = BTreeMap::new();
    let mut bytes = 0usize;
    for (key, uri) in values {
        let (kind, source) = key
            .split_once(':')
            .ok_or_else(|| invalid("A custom asset needs a kind and URI"))?;
        if asset_key(kind, source)? != *key {
            return Err(invalid("Custom asset keys must be normalized"));
        }
        let target = owner.asset(kind, uri)?;
        bytes = bytes.saturating_add(
            serde_json::to_vec(&(key, &target))
                .expect("asset pair")
                .len(),
        );
        if bytes > crate::script::MAX_OUTPUT_BYTES {
            return Err(invalid(
                "Custom script bindings exceed the generated output budget",
            ));
        }
        result.insert(key.clone(), target);
    }
    for (key, uri) in values {
        let (kind, _) = key.split_once(':').expect("asset key");
        let key = asset_key(kind, uri)?;
        if let std::collections::btree_map::Entry::Vacant(entry) = result.entry(key) {
            let target = owner.asset(kind, uri)?;
            bytes = bytes.saturating_add(
                serde_json::to_vec(&(entry.key(), &target))
                    .expect("asset pair")
                    .len(),
            );
            if bytes > crate::script::MAX_OUTPUT_BYTES {
                return Err(invalid(
                    "Custom script bindings exceed the generated output budget",
                ));
            }
            entry.insert(target);
        }
    }
    Ok(result)
}

impl Bound {
    pub(super) fn custom_output(&self, value: &Value, uri_hash: String) -> Result<Self> {
        let source = parse(value)?;
        if source.assets.is_none() && self.original().script_scopes.is_empty() {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "This script lacks its original document namespace; copy it again from its source",
            ));
        }
        let owner = self
            .document_scope()?
            .selected(source.library.as_deref().unwrap_or(""))?;
        for target in source.components.values() {
            owner.component(target)?;
        }
        table(&source.variables, &owner.values.variables)?;
        table(&source.themes, &owner.values.themes)?;
        assets(source.assets.as_ref(), &owner)?;
        let browsers = owner.values.browser_libraries.clone().unwrap_or_default();
        if let Some(alias) = &source.browser_library {
            table(&BTreeMap::from([(alias.clone(), alias.clone())]), &browsers)?;
        }
        if let Some(libraries) = &source.browser_libraries {
            table(libraries, &browsers)?;
        }
        let base = owner.original();
        let mut composition = Composition {
            values: self.original().clone(),
            bytes: serde_json::to_vec(self.original()).expect("bindings").len(),
        };
        // Legacy parents retain their closed manifest but cannot manufacture
        // declared aliases which were never captured.
        if composition.values.script_scopes.is_empty() {
            composition.values.script_scopes.insert(
                String::new(),
                CapturedScope {
                    document_namespace: None,
                    components: owner
                        .values
                        .components
                        .keys()
                        .map(|key| (key.clone(), key.clone()))
                        .collect(),
                    variables: owner
                        .values
                        .variables
                        .keys()
                        .map(|key| (key.clone(), key.clone()))
                        .collect(),
                    themes: owner
                        .values
                        .themes
                        .keys()
                        .map(|key| (key.clone(), key.clone()))
                        .collect(),
                    assets: owner.values.assets.clone().unwrap_or_default(),
                    libraries: BTreeMap::from([(String::new(), String::new())]),
                    browser_library: owner.values.browser_library.clone(),
                    browser_libraries: owner.values.browser_libraries.clone().unwrap_or_default(),
                },
            );
        }
        let prefix = private_key(
            base.script_namespace.as_deref().unwrap_or(""),
            "scope",
            &value.to_string(),
        );
        let identity = |name: &str| private_key(&prefix, "namespace", name);
        let original_scopes = if source.script_scopes.is_empty() {
            BTreeMap::from([(
                String::new(),
                CapturedScope {
                    document_namespace: None,
                    components: source
                        .components
                        .keys()
                        .map(|key| (key.clone(), key.clone()))
                        .collect(),
                    variables: source
                        .variables
                        .keys()
                        .map(|key| (key.clone(), key.clone()))
                        .collect(),
                    themes: source
                        .themes
                        .keys()
                        .map(|key| (key.clone(), key.clone()))
                        .collect(),
                    assets: BTreeMap::new(),
                    libraries: BTreeMap::new(),
                    browser_library: None,
                    browser_libraries: BTreeMap::new(),
                },
            )])
        } else {
            source.script_scopes.clone()
        };
        for (name, raw) in &original_scopes {
            let namespace = identity(name);
            let mut scope = raw.clone();
            scope.components.clear();
            for (logical, key) in &raw.components {
                let target = &source.components[key];
                let mapped = composition.component(
                    &owner,
                    &prefix,
                    key,
                    target,
                    source.descendants.get(key),
                    source.descendant_contexts.get(key),
                )?;
                scope.components.insert(logical.clone(), mapped);
            }
            for key in raw.components.values() {
                let target = &source.components[key];
                if !scope.components.contains_key(target) {
                    let mapped = composition.identity_component(&owner, &prefix, target)?;
                    scope.components.insert(target.clone(), mapped);
                }
            }
            for (section, members, authored, captured) in [
                (
                    "variables",
                    &mut scope.variables,
                    &source.variables,
                    &owner.values.variables,
                ),
                (
                    "themes",
                    &mut scope.themes,
                    &source.themes,
                    &owner.values.themes,
                ),
            ] {
                let originals = members.clone();
                let names = originals
                    .iter()
                    .map(|(name, key)| (name.clone(), authored[key].clone()))
                    .collect();
                let targets = table(&names, captured)?;
                members.clear();
                for (name, target) in targets {
                    let key = private_key(&namespace, section, &name);
                    composition.charge(&(&key, &target))?;
                    if section == "variables" {
                        composition.values.variables.insert(key.clone(), target);
                    } else {
                        composition.values.themes.insert(key.clone(), target);
                    }
                    members.insert(name, key);
                }
            }
            if source.script_scopes.is_empty() {
                scope.document_namespace = source
                    .assets
                    .is_none()
                    .then(|| base.script_namespace.clone().unwrap_or_default());
                scope.assets = assets(source.assets.as_ref(), &owner)?;
                scope.libraries = base
                    .script_scopes
                    .get(base.script_namespace.as_deref().unwrap_or(""))
                    .map(|scope| scope.libraries.clone())
                    .unwrap_or_default();
                scope.libraries.insert(String::new(), namespace.clone());
                scope.browser_library = match &source.browser_library {
                    Some(alias) => Some(
                        table(
                            &BTreeMap::from([(alias.clone(), alias.clone())]),
                            owner
                                .values
                                .browser_libraries
                                .as_ref()
                                .unwrap_or(&BTreeMap::new()),
                        )?[alias]
                            .clone(),
                    ),
                    None if source.assets.is_none() => owner.values.browser_library.clone(),
                    None => None,
                };
                scope.browser_libraries = match &source.browser_libraries {
                    Some(libraries) => table(
                        libraries,
                        owner
                            .values
                            .browser_libraries
                            .as_ref()
                            .unwrap_or(&BTreeMap::new()),
                    )?,
                    None if source.assets.is_none() => {
                        owner.values.browser_libraries.clone().unwrap_or_default()
                    }
                    None => BTreeMap::new(),
                };
            } else {
                scope.document_namespace = raw.document_namespace.as_deref().map(identity);
                scope.assets = assets(Some(&raw.assets), &owner)?;
                scope.libraries = raw
                    .libraries
                    .iter()
                    .map(|(alias, target)| (alias.clone(), identity(target)))
                    .collect();
                scope.browser_library = raw
                    .browser_library
                    .as_ref()
                    .map(|alias| {
                        table(
                            &BTreeMap::from([(alias.clone(), alias.clone())]),
                            owner
                                .values
                                .browser_libraries
                                .as_ref()
                                .unwrap_or(&BTreeMap::new()),
                        )
                        .map(|targets| targets[alias].clone())
                    })
                    .transpose()?;
                scope.browser_libraries = table(
                    &raw.browser_libraries,
                    owner
                        .values
                        .browser_libraries
                        .as_ref()
                        .unwrap_or(&BTreeMap::new()),
                )?;
            }
            composition.charge(&(&namespace, &scope))?;
            composition.values.script_scopes.insert(namespace, scope);
        }
        composition.values.script_namespace =
            Some(identity(source.script_namespace.as_deref().unwrap_or("")));
        composition.values.input_components = source
            .input_components
            .as_ref()
            .map(|values| table(values, &owner.values.components))
            .transpose()?;
        composition.values.inputs_hash = source.inputs_hash;
        composition.values.library = None;
        composition.values.script_uri = source.script_uri;
        composition.values.source_uri_hash = Some(uri_hash);
        composition.values.version = if composition
            .values
            .script_scopes
            .values()
            .any(|scope| scope.document_namespace.is_some())
        {
            4
        } else {
            3
        };
        let value = serde_json::to_value(&composition.values).expect("bindings");
        // Validate the complete graph and existing per-script count/byte limits.
        let values = parse(&value)?;
        Self::from_values(self.origin.clone(), false, values)
    }
}
