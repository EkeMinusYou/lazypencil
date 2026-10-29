//! Compose a generated explicit scope with the parent's captured library graph.
use super::*;

pub(super) struct Selection {
    pub output: Bound,
    input: Option<(Bound, Value)>,
    input_values: Option<Value>,
    output_active: bool,
}

fn native_inputs(parent: &Bound, node: &Value) -> Result<Option<Selection>> {
    if parent.native_scope().is_none() {
        return Ok(None);
    }
    node.get("inputs")
        .map(|raw| -> Result<Selection> {
            Ok(Selection {
                output: parent.document_scope()?.into_owned(),
                input: Some(parent.generated_inputs(raw)?),
                input_values: None,
                output_active: false,
            })
        })
        .transpose()
}

pub(in crate::project) fn has_scope(node: &Value) -> bool {
    [
        "script_scope",
        "script_bindings",
        "script_input_context",
        "script_override",
    ]
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
        return native_inputs(parent, node);
    };
    if namespace.get("script_override").is_some() {
        return Err(invalid(
            "Copied script override envelopes require a ref node",
        ));
    }
    if !has_scope(node) {
        return native_inputs(parent, node);
    }
    let scopes = Scopes::parse(namespace)?;
    let uri = node
        .get("scriptUri")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("A generated script needs scriptUri"))?;
    let input = node
        .get("inputs")
        .map(|raw| parent.generated_inputs(raw))
        .transpose()?;
    Ok(Some(scopes.select(
        parent,
        &hash(uri.as_bytes()),
        Some(uri),
        input,
        true,
    )?))
}

/// Scope guards are independent; bindings override only their active side.
struct Scopes<'a> {
    scope: Option<super::super::scope::ScriptScope>,
    bindings: Option<Bindings>,
    binding_value: Option<&'a Value>,
    input_context: Option<super::super::script_context::Inputs>,
    input_value: Option<&'a Value>,
    override_output: Option<(Bindings, &'a Value)>,
    override_input: Option<(super::super::script_context::Inputs, &'a Value)>,
}

impl<'a> Scopes<'a> {
    fn parse(namespace: &'a Value) -> Result<Self> {
        let binding_value = namespace.get("script_bindings");
        Ok(Self {
            scope: namespace
                .get("script_scope")
                .map(super::super::scope::parse_scope)
                .transpose()?,
            bindings: binding_value.map(parse).transpose()?,
            binding_value,
            input_context: namespace
                .get("script_input_context")
                .map(super::super::script_context::parse)
                .transpose()?,
            input_value: namespace.get("script_input_context"),
            override_output: None,
            override_input: None,
        })
    }

    fn with_override(
        mut self,
        carrier: Option<&'a super::super::script_context::Override>,
    ) -> Result<Self> {
        self.override_output = carrier
            .and_then(|value| value.output.as_ref())
            .map(|value| parse(value).map(|parsed| (parsed, value)))
            .transpose()?;
        self.override_input = carrier
            .and_then(|value| value.input.as_ref())
            .map(|value| super::super::script_context::parse(value).map(|parsed| (parsed, value)))
            .transpose()?;
        Ok(self)
    }

    fn select(
        &self,
        parent: &Bound,
        uri_hash: &str,
        canonical_uri: Option<&str>,
        mut input: Option<(Bound, Value)>,
        provided: bool,
    ) -> Result<Selection> {
        let mut output = parent.document_scope()?.into_owned();
        let mut output_active = false;
        let mut input_active = false;
        // Evaluate explicit context against the lexical controls, before an
        // independent scope or binding changes their component namespace.
        let mut explicit = self
            .input_context
            .as_ref()
            .zip(self.input_value)
            .zip(input.as_ref())
            .map(|((context, value), (owner, raw))| {
                parent.explicit_generated_input(context, value, owner, raw)
            })
            .transpose()?
            .flatten();
        if let Some((context, value)) = &self.override_input {
            explicit = input
                .as_ref()
                .map(|(owner, raw)| parent.explicit_generated_input(context, value, owner, raw))
                .transpose()?
                .flatten();
        }
        if let Some(scope) = &self.scope {
            if hash(scope.script_uri.as_bytes()) == uri_hash {
                output = parent.document_scope()?.selected(&scope.library)?;
                output_active = true;
            }
            if let Some((owner, raw)) = &mut input
                && let Some(scope) = &scope.inputs
                && super::super::scope::value_hash(raw) == scope.hash
            {
                owner.values.components = parent
                    .document_scope()?
                    .selected(&scope.library)?
                    .values
                    .components;
                input_active = true;
            }
        }
        let mut scope_components = self.override_output.as_ref().and_then(|_| {
            input
                .as_ref()
                .map(|(owner, _)| owner.values.components.clone())
        });
        let scope_input_active = input_active;
        for (source, value, overridden) in self
            .bindings
            .as_ref()
            .zip(self.binding_value)
            .map(|(source, value)| (source, value, false))
            .into_iter()
            .chain(
                self.override_output
                    .as_ref()
                    .map(|(source, value)| (source, *value, true)),
            )
        {
            if overridden {
                output = parent.document_scope()?.into_owned();
                output_active = true;
                input_active = scope_input_active;
                if let Some((owner, _)) = &mut input {
                    owner.values.components = scope_components.take().expect("scope inputs");
                }
            }
            let canonical = canonical_uri == Some(source.script_uri.as_str())
                && parent.generated_output(value)?;
            if canonical || hash(source.script_uri.as_bytes()) == uri_hash {
                output = if canonical {
                    Bound::from_values(parent.origin.clone(), false, source.clone())?
                } else {
                    parent.custom_output(value, uri_hash.into())?
                };
                output_active = true;
                if let Some((owner, raw)) = &mut input
                    && source.inputs_hash.as_ref() == Some(&super::super::scope::value_hash(raw))
                {
                    owner.values.components = output
                        .values
                        .input_components
                        .as_ref()
                        .unwrap_or(&output.values.components)
                        .clone();
                    input_active = true;
                }
            }
        }
        let input_values = explicit.map(|(owner, values)| {
            let raw = input.as_ref().expect("explicit controls").1.clone();
            input = Some((owner, raw));
            input_active = true;
            values
        });
        Ok(Selection {
            output,
            input: input.filter(|_| provided || input_active),
            input_values,
            output_active,
        })
    }
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
        let input = if let Some((bound, original)) = &self.input {
            let mut values = self.input_values.as_ref().unwrap_or(original).clone();
            if self.input_values.is_none() {
                known_variables(bound, &mut values);
            }
            let context = super::super::script_context::capture(
                Some(original),
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
    fn explicit_generated_input(
        &self,
        context: &super::super::script_context::Inputs,
        value: &Value,
        lexical: &Bound,
        raw: &Value,
    ) -> Result<Option<(Bound, Value)>> {
        let canonical = self.generated_input(value)?;
        let namespace = if canonical {
            Ok(self.clone())
        } else {
            self.document_scope()?
                .selected(context.library.as_deref().unwrap_or(""))
        };
        let lookup = |table: &BTreeMap<String, String>, name: &str| {
            table
                .get(name)
                .cloned()
                .or_else(|| table.values().find(|value| value.as_str() == name).cloned())
        };
        if !context.matches(Some(raw), |actual, expected, component| {
            let Ok(namespace) = &namespace else {
                return false;
            };
            let (first, second) = if component {
                (&lexical.values.components, &namespace.values.components)
            } else {
                (&lexical.values.variables, &namespace.values.variables)
            };
            lookup(first, actual)
                .zip(if canonical {
                    Some(expected.into())
                } else {
                    lookup(second, expected)
                })
                .is_some_and(|(first, second)| first == second)
        }) {
            return Ok(None);
        }
        let mut owner = namespace?;
        let mut values = json!(context.values);
        if canonical {
            owner.values.variables = context.variables.clone();
            owner.values.components = context.components.clone();
        } else {
            // Context values already name target variables, not logical map
            // keys. Map through the owning document before installing aliases.
            known_variables(&owner, &mut values);
            owner.values.variables =
                super::custom::table(&context.variables, &owner.values.variables)?;
            owner.values.components =
                super::custom::table(&context.components, &owner.values.components)?;
        }
        Ok(Some((owner, values)))
    }

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

    pub(in crate::project) fn compose_generated_scopes(
        &self,
        namespace: &Value,
        carrier: Option<&super::super::script_context::Override>,
        changes: &mut Value,
        target: &ResolvedNode,
        state: &GeneratedState,
        budget: &mut usize,
    ) -> Result<(Option<Value>, Option<Value>)> {
        let scopes = Scopes::parse(namespace)?.with_override(carrier)?;
        let uri = changes
            .get("scriptUri")
            .or_else(|| target.properties.get("scriptUri"))
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("A generated script needs scriptUri"))?
            .to_owned();
        let uri_hash = Self::generated_source_hash(target, state)?;
        let inactive_override = scopes
            .override_output
            .as_ref()
            .map(|(source, value)| {
                Ok::<_, Error>(
                    hash(source.script_uri.as_bytes()) != uri_hash
                        && !(source.script_uri == uri && self.generated_output(value)?),
                )
            })
            .transpose()?
            .unwrap_or(false);
        let provided = changes
            .get("inputs")
            .map(|raw| self.generated_inputs(raw))
            .transpose()?;
        let needs_inherited = scopes
            .scope
            .as_ref()
            .is_some_and(|scope| scope.inputs.is_some())
            || scopes.input_context.is_some()
            || scopes.override_input.is_some()
            || scopes
                .override_output
                .as_ref()
                .is_some_and(|(source, _)| source.inputs_hash.is_some())
            || scopes.bindings.as_ref().is_some_and(|bindings| {
                bindings.inputs_hash.is_some() && hash(bindings.script_uri.as_bytes()) == uri_hash
            });
        let inherited = if provided.is_none() && needs_inherited {
            if scopes.scope.is_some()
                && (scopes.bindings.is_some() || scopes.override_output.is_some())
                && (target.input_context.as_ref().is_some_and(|context| {
                    context.values.lexical_inputs != Some(true)
                        || self.original().input_epoch.is_none()
                        || context.values.input_epoch != self.original().input_epoch
                }) || target.script_input_scope.is_some()
                    || target.input_context.is_none()
                        && target
                            .script_bindings
                            .as_ref()
                            .is_some_and(|bindings| bindings.inputs_bound))
            {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Generated combined scopes require lexical inputs from the same captured generation; independent inherited input contexts need further composition",
                ));
            }
            Self::inherited_raw_inputs(target)?
        } else {
            None
        };
        let from_changes = provided.is_some();
        let input = if let Some(provided) = provided {
            Some(provided)
        } else if let Some(raw) = inherited {
            let mut owner = self.clone();
            if let Some(context) = &target.input_context {
                owner.values.variables = context.values.variables.clone();
                owner.values.components = context.values.components.clone();
            }
            Some((owner, raw))
        } else {
            None
        };
        let mut selected = scopes.select(self, &uri_hash, Some(&uri), input, from_changes)?;
        if changes.get("scriptUri").is_none()
            && (inactive_override
                || scopes.override_output.is_none()
                    && (scopes.scope.is_some() || scopes.bindings.is_some())
                    && !selected.output_active)
        {
            selected.output = target.script_bindings.as_ref().ok_or_else(|| Error::new(
                "UNSUPPORTED_FEATURE", "The inherited script lacks captured code ownership; copy it again from its source",
            ))?.code_scope()?;
            selected.output_active = true;
        }
        let output = if selected.output_active || changes.get("scriptUri").is_some() {
            Some(
                selected
                    .output
                    .inherited_output_with_hash(&uri, None, Some(uri_hash), budget)?,
            )
        } else {
            None
        };
        let has_input = selected.input.is_some();
        if has_input {
            selected.apply(changes, budget)?;
        }
        let input = has_input.then(|| {
            changes
                .pointer("/metadata/lazypencil/script_input_context")
                .expect("selected input")
                .clone()
        });
        Ok((output, input))
    }

    fn reachable(&self, namespace: &str) -> bool {
        let source = self.original();
        if source.script_scopes.is_empty() {
            return namespace.is_empty();
        }
        let scope = &source.script_scopes[source.script_namespace.as_deref().unwrap_or("")];
        scope.libraries.values().any(|target| target == namespace)
            || scope.document_namespace.as_ref().is_some_and(|document| {
                document == namespace
                    || source.script_scopes[document]
                        .libraries
                        .values()
                        .any(|target| target == namespace)
            })
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
        let values = super::super::script_context::parse(value)?;
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
