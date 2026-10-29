//! Captured aliases select views of shared binding tables without duplicating paths.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::project) struct CapturedScope {
    pub components: BTreeMap<String, String>,
    pub variables: BTreeMap<String, String>,
    pub themes: BTreeMap<String, String>,
    pub assets: BTreeMap<String, String>,
    pub libraries: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_library: Option<String>,
    pub browser_libraries: BTreeMap<String, String>,
}

pub(super) fn validate(values: &Bindings) -> Result<()> {
    if values.version == 1 && !values.script_scopes.is_empty()
        || values.version == 2 && values.script_scopes.is_empty()
    {
        return Err(invalid(
            "Captured execution namespaces require script bindings version 2",
        ));
    }
    if values.script_scopes.len() > 4096
        || values
            .script_namespace
            .as_ref()
            .is_some_and(|name| !values.script_scopes.contains_key(name))
        || !values.script_scopes.is_empty() && !values.script_scopes.contains_key("")
    {
        return Err(invalid("Invalid captured script namespace"));
    }
    for (name, scope) in &values.script_scopes {
        if values.assets.is_none() || scope.libraries.get("") != Some(name) {
            return Err(invalid(
                "A captured namespace needs its own identity and a closed asset manifest",
            ));
        }
        if !name.is_empty() && !valid_name(name) {
            return Err(invalid("Invalid captured script scope name"));
        }
        for (members, table) in [
            (&scope.components, &values.components),
            (&scope.variables, &values.variables),
            (&scope.themes, &values.themes),
        ] {
            for (name, key) in members {
                if !valid_name(name) || !table.contains_key(key) {
                    return Err(invalid(
                        "A captured scope member is absent from its binding table",
                    ));
                }
            }
        }
        for (alias, target) in &scope.libraries {
            if !alias.is_empty() && !valid_name(alias) || !values.script_scopes.contains_key(target)
            {
                return Err(invalid("A captured script library is missing"));
            }
        }
        for (key, uri) in &scope.assets {
            let (kind, source) = key
                .split_once(':')
                .ok_or_else(|| invalid("A scoped asset needs a kind"))?;
            if asset_key(kind, source)? != *key
                || !values
                    .assets
                    .as_ref()
                    .is_some_and(|assets| assets.values().any(|target| target == uri))
            {
                return Err(invalid("A scoped asset is outside the captured manifest"));
            }
        }
        for (alias, target) in &scope.browser_libraries {
            if !alias.is_empty() && !valid_name(alias) || !valid_name(target) {
                return Err(invalid("Invalid captured script browser namespace"));
            }
            if !values
                .browser_libraries
                .as_ref()
                .is_some_and(|libraries| libraries.values().any(|value| value == target))
            {
                return Err(invalid(
                    "A scoped browser library is outside the captured namespaces",
                ));
            }
        }
        if scope
            .browser_library
            .as_ref()
            .is_some_and(|alias| !valid_name(alias))
        {
            return Err(invalid("Invalid captured script browser scope"));
        }
        if scope.browser_library.as_ref().is_some_and(|target| {
            !values
                .browser_libraries
                .as_ref()
                .is_some_and(|libraries| libraries.values().any(|value| value == target))
        }) {
            return Err(invalid(
                "A scoped browser owner is outside the captured namespaces",
            ));
        }
    }
    Ok(())
}

impl Bound {
    pub(in crate::project) fn original(&self) -> &Bindings {
        self.captured.as_deref().unwrap_or(&self.values)
    }

    pub(super) fn from_values(
        origin: PathBuf,
        inputs_bound: bool,
        values: Bindings,
    ) -> Result<Self> {
        if values.script_scopes.is_empty() {
            return Ok(Self {
                origin,
                inputs_bound,
                values,
                captured: None,
            });
        }
        let scope = &values.script_scopes[values.script_namespace.as_deref().unwrap_or("")];
        let select = |members: &BTreeMap<String, String>, table: &BTreeMap<String, String>| {
            members
                .iter()
                .map(|(name, key)| (name.clone(), table[key].clone()))
                .collect()
        };
        let effective = Bindings {
            version: values.version,
            script_uri: values.script_uri.clone(),
            library: values.library.clone(),
            browser_library: scope.browser_library.clone(),
            browser_libraries: Some(scope.browser_libraries.clone()),
            inputs_hash: values.inputs_hash.clone(),
            input_components: values.input_components.clone(),
            components: select(&scope.components, &values.components),
            variables: select(&scope.variables, &values.variables),
            themes: select(&scope.themes, &values.themes),
            assets: Some(scope.assets.clone()),
            descendants: scope
                .components
                .iter()
                .filter_map(|(name, key)| {
                    values
                        .descendants
                        .get(key)
                        .map(|paths| (name.clone(), paths.clone()))
                })
                .collect(),
            descendant_contexts: scope
                .components
                .iter()
                .filter_map(|(name, key)| {
                    values
                        .descendant_contexts
                        .get(key)
                        .map(|paths| (name.clone(), paths.clone()))
                })
                .collect(),
            script_scopes: BTreeMap::new(),
            script_namespace: None,
        };
        Ok(Self {
            origin,
            inputs_bound,
            values: effective,
            captured: Some(Box::new(values)),
        })
    }

    #[cfg(feature = "rendering")]
    pub(in crate::project) fn selected(&self, alias: &str) -> Result<Self> {
        let mut values = self.original().clone();
        if values.script_scopes.is_empty() {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "This script lacks captured execution namespaces; copy it again from its source",
            ));
        }
        let active = values.script_namespace.as_deref().unwrap_or("");
        let target = values
            .script_scopes
            .get(active)
            .and_then(|scope| scope.libraries.get(alias))
            .ok_or_else(|| {
                invalid("A generated script scope is outside the captured library namespaces")
            })?
            .clone();
        values.script_namespace = Some(target);
        values.inputs_hash = None;
        values.input_components = None;
        Self::from_values(self.origin.clone(), false, values)
    }
}
