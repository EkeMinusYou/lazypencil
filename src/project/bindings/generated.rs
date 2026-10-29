//! Compose a generated explicit scope with the parent's captured library graph.
use super::*;

pub(super) struct Selection {
    pub output: Bound,
    input: Option<Bound>,
}

pub(in crate::project) fn has_scope(node: &Value) -> bool {
    ["script_scope", "script_bindings"]
        .into_iter()
        .any(|field| {
            node.pointer("/metadata/lazypencil")
                .and_then(|namespace| namespace.get(field))
                .is_some()
        })
}

pub(in crate::project) fn has_pending_scopes(node: &Value) -> bool {
    has_scope(node)
        || node
            .get("children")
            .and_then(Value::as_array)
            .is_some_and(|children| children.iter().any(has_pending_scopes))
        || node
            .get("descendants")
            .and_then(Value::as_object)
            .is_some_and(|changes| changes.values().any(has_pending_scopes))
}

pub(in crate::project) fn clear_pending_scopes(node: &mut Value) {
    if has_scope(node)
        && let Some(namespace) = node
            .pointer_mut("/metadata/lazypencil")
            .and_then(Value::as_object_mut)
    {
        for key in [
            "script_scope",
            "script_bindings",
            "script_input_context",
            "script_override",
        ] {
            namespace.remove(key);
        }
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            clear_pending_scopes(child);
        }
    }
    if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
        for changes in changes.values_mut() {
            clear_pending_scopes(changes);
        }
    }
}

pub(super) fn selection(parent: &Bound, node: &Value) -> Result<Option<Selection>> {
    if node["enabled"] == false
        || node
            .get("type")
            .is_some_and(|kind| !matches!(kind.as_str(), Some("script" | "ref")))
    {
        return Ok(None);
    }
    // A reference's effective URI, raw inherited controls and script kind are
    // available only after expansion. The parallel GeneratedState retains
    // original URI guards without inserting temporary fields into user JSON.
    if node["type"] != "script" {
        return Ok(None);
    }
    let Some(namespace) = node.pointer("/metadata/lazypencil") else {
        return Ok(None);
    };
    if let Some(value) = namespace.get("script_bindings") {
        if ["script_scope", "script_input_context", "script_override"]
            .iter()
            .any(|key| namespace.get(*key).is_some())
        {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "Combined generated explicit script contexts require composition",
            ));
        }
        let uri = node
            .get("scriptUri")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("A generated script needs scriptUri"))?;
        let output = parent.custom_output(value, hash(uri.as_bytes()))?;
        let input = node.get("inputs").map(|inputs| {
            let mut owner = parent.clone();
            if output.values.inputs_hash.as_ref() == Some(&super::super::scope::value_hash(inputs))
            {
                owner.values.components = output
                    .values
                    .input_components
                    .as_ref()
                    .unwrap_or(&output.values.components)
                    .clone();
            }
            owner
        });
        return Ok(Some(Selection { output, input }));
    }
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
        return Err(invalid("A generated script needs scriptUri"));
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
        let input = if let Some(bound) = &self.input {
            let original = node.get("inputs").cloned().expect("explicit inputs");
            let mut values = original.clone();
            known_variables(bound, &mut values);
            let context = super::super::script_context::capture(
                Some(&original),
                json!({"version":1,"values":values,"variables":bound.values.variables,"components":bound.values.components}),
            )?;
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
        namespace.remove("script_bindings");
        if let Some(input) = input {
            namespace.insert("script_input_context".into(), input);
        }
        Ok(())
    }
}

impl Bound {
    fn generated_source_hash(target: &ResolvedNode, state: &GeneratedState) -> Result<String> {
        state.uri_hash.clone().or_else(|| {
            if let Some(bound) = &target.script_bindings && bound.values.assets.is_some() {
                bound.original().source_uri_hash.clone()
            } else {
                target.properties.get("scriptUri").and_then(Value::as_str).map(|uri| hash(uri.as_bytes()))
            }
        }).ok_or_else(|| Error::new("UNSUPPORTED_FEATURE", "The inherited source lacks original URI provenance; copy it again from its source"))
    }

    fn inherited_raw_inputs(target: &ResolvedNode) -> Result<Option<Value>> {
        if let Some(context) = &target.input_context {
            return context.values.original_values().map(Some).ok_or_else(|| Error::new("UNSUPPORTED_FEATURE", "The inherited inputs lack original expression provenance; copy them again from their source"));
        }
        if target
            .script_bindings
            .as_ref()
            .is_some_and(|bound| bound.inputs_bound && bound.values.assets.is_some())
        {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "The inherited inputs lack original expression provenance; copy them again from their source",
            ));
        }
        Ok(target.properties.get("inputs").cloned())
    }

    pub(in crate::project) fn compose_generated_bindings(
        &self,
        value: &Value,
        changes: &mut Value,
        target: &ResolvedNode,
        state: &GeneratedState,
        uri: &str,
        budget: &mut usize,
    ) -> Result<(Option<Value>, Option<Value>)> {
        let source = parse(value)?;
        let uri_hash = Self::generated_source_hash(target, state)?;
        if hash(source.script_uri.as_bytes()) != uri_hash {
            return Ok((None, None));
        }
        let output = self.custom_output(value, uri_hash.clone())?;
        let provided = changes.get("inputs").cloned();
        let inherited = if provided.is_none() && source.inputs_hash.is_some() {
            Self::inherited_raw_inputs(target)?
        } else {
            None
        };
        let raw = provided.as_ref().or(inherited.as_ref());
        let active = raw.is_some_and(|raw| {
            source.inputs_hash.as_ref() == Some(&super::super::scope::value_hash(raw))
        });
        let input = if provided.is_some() || active {
            let raw = raw.expect("provided or guarded inputs");
            let mut owner = self.clone();
            if provided.is_none()
                && let Some(context) = &target.input_context
            {
                owner.values.variables = context.values.variables.clone();
                owner.values.components = context.values.components.clone();
            }
            if active {
                owner.values.components = output
                    .values
                    .input_components
                    .as_ref()
                    .unwrap_or(&output.values.components)
                    .clone();
            }
            changes["inputs"] = raw.clone();
            let selection = Selection {
                output: output.clone(),
                input: Some(owner),
            };
            selection.apply(changes, budget)?;
            changes
                .pointer("/metadata/lazypencil/script_input_context")
                .cloned()
        } else {
            None
        };
        Ok((
            Some(output.inherited_output_with_hash(uri, None, Some(uri_hash), budget)?),
            input,
        ))
    }

    pub(in crate::project) fn compose_generated_scope(
        &self,
        scope: &Value,
        changes: &mut Value,
        target: &ResolvedNode,
        state: &GeneratedState,
        uri: &str,
        budget: &mut usize,
    ) -> Result<(Option<Value>, Option<Value>)> {
        let namespace = changes
            .pointer("/metadata/lazypencil")
            .expect("scope owner");
        if ["script_bindings", "script_input_context", "script_override"]
            .iter()
            .any(|key| namespace.get(*key).is_some())
        {
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "Combined generated explicit script contexts require composition",
            ));
        }
        let scope = super::super::scope::parse_scope(scope)?;
        let uri_hash = Self::generated_source_hash(target, state)?;
        let output = if uri_hash == hash(scope.script_uri.as_bytes()) {
            Some(self.selected(&scope.library)?.inherited_output_with_hash(
                uri,
                None,
                Some(uri_hash),
                budget,
            )?)
        } else if changes.get("scriptUri").is_some() {
            Some(self.inherited_output_with_hash(uri, None, Some(uri_hash), budget)?)
        } else {
            None
        };
        let provided = changes.get("inputs").cloned();
        let inherited = if provided.is_none() && scope.inputs.is_some() {
            Self::inherited_raw_inputs(target)?
        } else {
            None
        };
        let raw = provided.as_ref().or(inherited.as_ref());
        let active = scope.inputs.as_ref().filter(|scope| {
            raw.is_some_and(|raw| super::super::scope::value_hash(raw) == scope.hash)
        });
        let input = if provided.is_some() || active.is_some() {
            let raw = raw.expect("provided or guarded inputs");
            let owner = if let Some(scope) = active {
                self.selected(&scope.library)?
            } else {
                self.clone()
            };
            let mut values = raw.clone();
            known_variables(&owner, &mut values);
            let context = super::super::script_context::capture(
                Some(raw),
                json!({"version":1,"values":values,"variables":owner.values.variables,"components":owner.values.components}),
            )?;
            *budget = budget.saturating_add(context.to_string().len());
            if *budget > crate::script::MAX_OUTPUT_BYTES {
                return Err(invalid(
                    "Inherited script contexts exceed the generated output budget",
                ));
            }
            changes["inputs"] = values;
            Some(context)
        } else {
            None
        };
        Ok((output, input))
    }

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
