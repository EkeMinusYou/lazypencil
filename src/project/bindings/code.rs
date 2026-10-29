//! Preserve a generated script's code document independently of its output view.
use super::*;

fn reachable(values: &Bindings, root: &str) -> BTreeSet<String> {
    let mut pending = vec![root.to_owned()];
    let mut found = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !found.insert(name.clone()) {
            continue;
        }
        let scope = &values.script_scopes[&name];
        pending.extend(scope.libraries.values().cloned());
        pending.extend(scope.document_namespace.iter().cloned());
    }
    found
}

fn member_key(section: &str, name: &str, prefix: &str, used: impl Fn(&str) -> bool) -> String {
    let key = format!("{prefix}/{section}/{}", hash(name.as_bytes()));
    let mut candidate = key.clone();
    let mut index = 0;
    while used(&candidate) {
        index += 1;
        candidate = format!("{key}/{index}");
    }
    candidate
}

fn merge_members(
    source: &BTreeMap<String, String>,
    target: &mut BTreeMap<String, String>,
    names: &BTreeSet<String>,
    section: &str,
    prefix: &str,
) -> BTreeMap<String, String> {
    names
        .iter()
        .map(|name| {
            let value = &source[name];
            let key = if target.get(name) == Some(value) {
                name.clone()
            } else {
                let key = member_key(section, name, prefix, |key| target.contains_key(key));
                target.insert(key.clone(), value.clone());
                key
            };
            (name.clone(), key)
        })
        .collect()
}

impl Bound {
    pub(in crate::project) fn output_with_code_owner(
        &self,
        uri: &str,
        inputs: Option<&Value>,
        uri_hash: Option<String>,
        code: Option<&Bound>,
        budget: &mut usize,
    ) -> Result<Value> {
        let mut result = self.inherited_output_values(uri, inputs, uri_hash, budget)?;
        let Some(code) = code else {
            return Ok(serde_json::to_value(result).expect("bindings"));
        };
        let code = code.document_scope()?;
        let source = code.original();
        if source.script_scopes.is_empty() {
            // A legacy capture has no document graph to preserve.
            return Ok(serde_json::to_value(result).expect("bindings"));
        }
        if self.origin != code.origin {
            return Err(invalid(
                "Generated code and output must share their captured document",
            ));
        }
        let root = source.script_namespace.as_deref().unwrap_or("");
        let names = reachable(source, root);
        let initial_bytes = serde_json::to_vec(&result).expect("bindings").len();
        let reusable = names.iter().all(|name| {
            let scope = &source.script_scopes[name];
            result.script_scopes.get(name) == Some(scope)
                && scope.components.values().all(|key| {
                    source.components.get(key) == result.components.get(key)
                        && source.descendants.get(key) == result.descendants.get(key)
                        && source.descendant_contexts.get(key)
                            == result.descendant_contexts.get(key)
                })
                && scope
                    .variables
                    .values()
                    .all(|key| source.variables.get(key) == result.variables.get(key))
                && scope
                    .themes
                    .values()
                    .all(|key| source.themes.get(key) == result.themes.get(key))
        });
        let namespace = if reusable {
            root.to_owned()
        } else {
            let mut index = result.script_scopes.len();
            let prefix = loop {
                let prefix = format!("@lazypencil/code/{index}");
                let children = format!("{prefix}/");
                if result
                    .script_scopes
                    .keys()
                    .all(|name| name != &prefix && !name.starts_with(&children))
                {
                    break prefix;
                }
                index += 1;
            };
            let namespaces: BTreeMap<_, _> = names
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        if name == root {
                            prefix.clone()
                        } else {
                            format!("{prefix}/{}", hash(name.as_bytes()))
                        },
                    )
                })
                .collect();
            let scopes: Vec<_> = names
                .iter()
                .map(|name| &source.script_scopes[name])
                .collect();
            let component_names = scopes
                .iter()
                .flat_map(|scope| scope.components.values().cloned())
                .collect::<BTreeSet<_>>();
            let mut components = BTreeMap::new();
            for name in component_names {
                let value = &source.components[&name];
                let key = if result.components.get(&name) == Some(value)
                    && source.descendants.get(&name) == result.descendants.get(&name)
                    && source.descendant_contexts.get(&name)
                        == result.descendant_contexts.get(&name)
                {
                    name.clone()
                } else {
                    let key = member_key("components", &name, &prefix, |key| {
                        result.components.contains_key(key)
                    });
                    result.components.insert(key.clone(), value.clone());
                    if let Some(paths) = source.descendants.get(&name) {
                        result.descendants.insert(key.clone(), paths.clone());
                    }
                    if let Some(paths) = source.descendant_contexts.get(&name) {
                        result
                            .descendant_contexts
                            .insert(key.clone(), paths.clone());
                    }
                    key
                };
                components.insert(name, key);
            }
            let variables = merge_members(
                &source.variables,
                &mut result.variables,
                &scopes
                    .iter()
                    .flat_map(|scope| scope.variables.values().cloned())
                    .collect(),
                "variables",
                &prefix,
            );
            let themes = merge_members(
                &source.themes,
                &mut result.themes,
                &scopes
                    .iter()
                    .flat_map(|scope| scope.themes.values().cloned())
                    .collect(),
                "themes",
                &prefix,
            );
            for (name, scope) in names.iter().zip(scopes) {
                let mut scope = (*scope).clone();
                for (members, mapping) in [
                    (&mut scope.components, &components),
                    (&mut scope.variables, &variables),
                    (&mut scope.themes, &themes),
                ] {
                    for key in members.values_mut() {
                        *key = mapping.get(key.as_str()).expect("captured member").clone();
                    }
                }
                for target in scope.libraries.values_mut() {
                    *target = namespaces
                        .get(target.as_str())
                        .expect("captured library")
                        .clone();
                }
                if let Some(target) = &mut scope.document_namespace {
                    *target = namespaces
                        .get(target.as_str())
                        .expect("captured document")
                        .clone();
                }
                let assets = result.assets.as_mut().expect("closed output manifest");
                for (key, value) in &scope.assets {
                    let kind = key.split_once(':').expect("asset key").0;
                    if !assets.iter().any(|(key, target)| {
                        key.starts_with(&format!("{kind}:")) && target == value
                    }) {
                        let key = member_key(kind, key, &prefix, |key| {
                            assets.contains_key(&format!("{kind}:{key}"))
                        });
                        assets.insert(format!("{kind}:{key}"), value.clone());
                    }
                }
                let browsers = result.browser_libraries.get_or_insert_with(BTreeMap::new);
                for target in scope
                    .browser_libraries
                    .values()
                    .chain(scope.browser_library.iter())
                {
                    if !browsers.values().any(|value| value == target) {
                        let key = member_key("browsers", target, &prefix, |key| {
                            browsers.contains_key(key)
                        });
                        browsers.insert(key, target.clone());
                    }
                }
                result.script_scopes.insert(namespaces[name].clone(), scope);
            }
            namespaces[root].clone()
        };
        result.code_namespace = Some(namespace);
        if result
            .script_scopes
            .values()
            .any(|scope| scope.document_namespace.is_some())
        {
            result.version = 4;
        }
        let value = serde_json::to_value(result).expect("code ownership");
        let growth = value.to_string().len().saturating_sub(initial_bytes);
        *budget = budget.saturating_add(growth);
        if *budget > crate::script::MAX_OUTPUT_BYTES {
            return Err(invalid(
                "Generated code ownership exceeds the output budget",
            ));
        }
        parse(&value)?;
        Ok(value)
    }
}
