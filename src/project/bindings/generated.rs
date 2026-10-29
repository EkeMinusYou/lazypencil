//! Compose a generated explicit scope with the parent's captured library graph.
use super::*;

pub(super) struct Selection {
    pub output: Bound,
    input: Option<Bound>,
}

pub(super) fn selection(parent: &Bound, node: &Value) -> Result<Option<Selection>> {
    if node["enabled"] == false
        || node
            .get("type")
            .is_some_and(|kind| !matches!(kind.as_str(), Some("script" | "ref")))
    {
        return Ok(None);
    }
    let Some(namespace) = node.pointer("/metadata/lazypencil") else {
        return Ok(None);
    };
    let Some(scope) = namespace.get("script_scope") else {
        return Ok(None);
    };
    let scope = super::super::scope::parse_scope(scope)?;
    if ["script_bindings", "script_input_context", "script_override"]
        .iter()
        .any(|key| namespace.get(*key).is_some())
    {
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "Combined generated explicit script contexts require composition",
        ));
    }
    let Some(uri) = node.get("scriptUri").and_then(Value::as_str) else {
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "A generated inherited script scope requires source provenance",
        ));
    };
    let output = if scope.script_uri == uri {
        parent.selected(&scope.library)?
    } else {
        parent.clone()
    };
    let input = if let Some(inputs) = node.get("inputs") {
        Some(
            if let Some(scope) = scope
                .inputs
                .filter(|scope| super::super::scope::value_hash(inputs) == scope.hash)
            {
                parent.selected(&scope.library)?
            } else {
                parent.clone()
            },
        )
    } else {
        if scope.inputs.is_some() && node["type"] != "script" {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "A generated inherited input scope requires source provenance",
            ));
        }
        None
    };
    Ok(Some(Selection { output, input }))
}

fn known_variables(bound: &Bound, value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(name) = text.strip_prefix('$')
                && let Some(target) = bound.values.variables.get(name).or_else(|| {
                    bound
                        .values
                        .variables
                        .values()
                        .find(|target| target.as_str() == name)
                })
            {
                *text = format!("${target}");
            }
        }
        Value::Array(values) => {
            for value in values {
                known_variables(bound, value);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                known_variables(bound, value);
            }
        }
        _ => {}
    }
}

impl Selection {
    pub(super) fn apply(&self, node: &mut Value, budget: &mut usize) -> Result<()> {
        let output = if node["type"] != "script" {
            let uri = node["scriptUri"]
                .as_str()
                .ok_or_else(|| invalid("A generated script needs scriptUri"))?;
            Some(self.output.inherited_output(uri, None, budget)?)
        } else {
            None
        };
        let input = if let Some(bound) = &self.input {
            let mut values = node.get("inputs").cloned().expect("explicit inputs");
            known_variables(bound, &mut values);
            let context = json!({"version":1,"values":values,"variables":bound.values.variables,"components":bound.values.components});
            *budget = budget.saturating_add(context.to_string().len());
            if *budget > crate::script::MAX_OUTPUT_BYTES {
                return Err(invalid(
                    "Inherited script contexts exceed the generated output budget",
                ));
            }
            node["inputs"] = values;
            Some(context)
        } else {
            None
        };
        let namespace = super::super::scope::metadata_namespace(node)?;
        namespace.remove("script_scope");
        if let Some(output) = output {
            namespace.insert("script_bindings".into(), output);
        }
        if let Some(input) = input {
            namespace.insert("script_input_context".into(), input);
        }
        Ok(())
    }
}

impl Bound {
    fn reachable(&self, namespace: &str) -> bool {
        let source = self.original();
        if source.script_scopes.is_empty() {
            return namespace.is_empty();
        }
        source.script_scopes[source.script_namespace.as_deref().unwrap_or("")]
            .libraries
            .values()
            .any(|target| target == namespace)
    }

    fn captured_target(&self, target: &str, component: bool) -> bool {
        let source = self.original();
        let table = if component {
            &source.components
        } else {
            &source.variables
        };
        if source.script_scopes.is_empty() {
            return table.values().any(|value| value == target);
        }
        source
            .script_scopes
            .iter()
            .filter(|(name, _)| self.reachable(name))
            .any(|(_, scope)| {
                let members = if component {
                    &scope.components
                } else {
                    &scope.variables
                };
                members
                    .values()
                    .any(|key| table.get(key).is_some_and(|value| value == target))
            })
    }
    pub(in crate::project) fn generated_output(&self, value: &Value) -> Result<bool> {
        let output = parse(value)?;
        let source = self.original();
        Ok(
            self.reachable(output.script_namespace.as_deref().unwrap_or(""))
                && output.library.is_none()
                && output.inputs_hash.is_none()
                && output.input_components.is_none()
                && output.components == source.components
                && output.variables == source.variables
                && output.themes == source.themes
                && output.descendants == source.descendants
                && output.descendant_contexts == source.descendant_contexts
                && output.assets == source.assets
                && output.script_scopes == source.script_scopes
                && output.browser_library == source.browser_library
                && output.browser_libraries == source.browser_libraries,
        )
    }

    pub(in crate::project) fn generated_input(&self, value: &Value) -> Result<bool> {
        let values: super::super::script_context::Inputs = serde_json::from_value(value.clone())
            .map_err(|_| invalid("Invalid generated input context"))?;
        Ok(values.library.is_none()
            && values
                .variables
                .values()
                .all(|target| self.captured_target(target, false))
            && values
                .components
                .values()
                .all(|target| self.captured_target(target, true)))
    }
}
