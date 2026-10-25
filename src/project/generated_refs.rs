//! Preserve code and input ownership independently for generated references.

use super::*;

impl Project {
    pub(super) fn bind_generated_ref_contexts(
        &mut self,
        node: &mut Value,
        bindings: &bindings::Bound,
        inherited: &BTreeMap<String, String>,
        inherited_bytes: &mut usize,
    ) -> Result<()> {
        self.check_cancelled()?;
        if node["enabled"] == false {
            return Ok(());
        }
        let mut theme = inherited.clone();
        if let Some(overrides) = node.get("theme").and_then(Value::as_object) {
            for (axis, value) in overrides {
                if let Some(value) = value.as_str() {
                    theme.insert(axis.clone(), value.into());
                }
            }
        }
        let mut reference = if node["type"] == "ref"
            && (node.get("inputs").is_some()
                || node.get("descendants").is_some()
                || node.get("scriptUri").is_some()
                || node.get("url").is_some()
                || node.get("children").is_some())
        {
            let mut resolved =
                self.expand(&bindings.origin, node, None, &[], &mut Vec::new(), &mut 0)?;
            // Input handles depend on the effective theme at their target,
            // including selections inside the referenced component. Derive
            // only themes here; raw controls still await their own header.
            self.generated_reference_themes(&mut resolved, &theme)?;
            theme = resolved.theme.clone();
            Some(resolved)
        } else {
            None
        };
        // Prepare replacement contexts before any input/property resolution.
        if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
            for child in children {
                self.bind_generated_ref_contexts(child, bindings, &theme, inherited_bytes)?;
            }
        }
        if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
            for (path, changes) in changes {
                let target_theme = if let Some(reference) = reference.as_mut() {
                    find_descendant_mut(reference, &path.split('/').collect::<Vec<_>>())
                        .ok_or_else(|| invalid("A generated override target is missing"))?
                        .theme
                        .clone()
                } else {
                    theme.clone()
                };
                if changes.get("type").is_some() {
                    self.bind_generated_ref_contexts(
                        changes,
                        bindings,
                        &target_theme,
                        inherited_bytes,
                    )?;
                } else if let Some(children) =
                    changes.get_mut("children").and_then(Value::as_array_mut)
                {
                    for child in children {
                        self.bind_generated_ref_contexts(
                            child,
                            bindings,
                            &target_theme,
                            inherited_bytes,
                        )?;
                    }
                }
            }
        }
        if let Some(mut resolved) = reference {
            self.bind_generated_source(node, &mut resolved, bindings, inherited_bytes)?;
            if resolved.kind() == "browser" {
                bindings.generated_browser_context(node, false, inherited_bytes)?;
            }
            if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
                for (path, changes) in changes {
                    if changes.get("type").is_none()
                        && (changes.get("inputs").is_some()
                            || changes.get("scriptUri").is_some()
                            || changes.get("url").is_some())
                    {
                        let target = find_descendant_mut(
                            &mut resolved,
                            &path.split('/').collect::<Vec<_>>(),
                        )
                        .ok_or_else(|| invalid("A generated script override target is missing"))?;
                        self.bind_generated_source(changes, target, bindings, inherited_bytes)?;
                        if target.kind() == "browser" {
                            bindings.generated_browser_context(changes, false, inherited_bytes)?;
                        }
                    }
                }
            }
            if let Some(inputs) = node.get_mut("inputs") {
                self.bind_generated_inputs(inputs, &resolved, bindings)?;
            }
            if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
                for (path, changes) in changes {
                    if changes.get("type").is_none()
                        && let Some(inputs) = changes.get_mut("inputs")
                    {
                        let target = find_descendant_mut(
                            &mut resolved,
                            &path.split('/').collect::<Vec<_>>(),
                        )
                        .ok_or_else(|| {
                            invalid("A generated script input override target is missing")
                        })?;
                        self.bind_generated_inputs(inputs, target, bindings)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn generated_reference_themes(
        &self,
        node: &mut ResolvedNode,
        inherited: &BTreeMap<String, String>,
    ) -> Result<()> {
        let theme = self.effective_theme(node, inherited)?;
        node.theme = theme.clone();
        for child in &mut node.children {
            self.generated_reference_themes(child, &theme)?;
        }
        Ok(())
    }

    fn bind_generated_inputs(
        &mut self,
        inputs: &mut Value,
        node: &ResolvedNode,
        bindings: &bindings::Bound,
    ) -> Result<()> {
        if node.kind() != "script" {
            return Ok(());
        }
        let uri = node
            .properties
            .get("scriptUri")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("A generated script reference needs scriptUri"))?;
        let path = self.resolve_file(node.property_source("scriptUri"), uri)?;
        let code = std::str::from_utf8(&self.asset(&path)?.bytes)
            .map_err(|_| invalid("A generated script reference needs UTF-8 source"))?;
        let header = crate::script::header::Header::parse(code)?;
        for input in header.inputs {
            if !matches!(input.kind, crate::script::header::InputKind::Ref) {
                continue;
            }
            let Some(value) = inputs.get_mut(&input.name).filter(|value| !value.is_null()) else {
                continue;
            };
            let resolved =
                self.resolve_value(&bindings.origin, value, &node.theme, &mut Vec::new())?;
            let name = resolved
                .as_str()
                .ok_or_else(|| invalid("A generated ref input must resolve to a component name"))?;
            if name.is_empty() || self.runtime_refs.contains_key(name) {
                continue;
            }
            let target = bindings.component(name)?;
            let (source, definition) = self.lookup_node(&bindings.origin, target)?;
            if definition["reusable"] != true {
                return Err(invalid("A generated ref input target must be reusable"));
            }
            let id = definition["id"].as_str().expect("definition ID").to_owned();
            let handle = format!(
                "@runtime/component/{}",
                hash(json!([source, id]).to_string().as_bytes())
            );
            self.runtime_refs.insert(handle.clone(), (source, id));
            *value = json!(handle);
        }
        Ok(())
    }

    fn bind_generated_source(
        &mut self,
        changes: &mut Value,
        target: &mut ResolvedNode,
        bindings: &bindings::Bound,
        inherited_bytes: &mut usize,
    ) -> Result<()> {
        if changes.get("scriptUri").is_none()
            || changes["enabled"] == false
            || target.kind() != "script"
            || bindings.values.assets.is_none()
        {
            return Ok(());
        }
        let uri = changes["scriptUri"]
            .as_str()
            .ok_or_else(|| invalid("A generated script override needs a source URI"))?
            .to_owned();
        let metadata = changes.get("metadata").cloned();
        if let Some(metadata) = &metadata {
            if !metadata.is_object() {
                return Err(invalid("Generated script metadata must be an object"));
            }
            let (output, input) = script_context::metadata_contexts(Some(metadata));
            if output || input || metadata.pointer("/lazypencil/script_override").is_some() {
                return Err(Error::new(
                    "UNSUPPORTED_FEATURE",
                    "Generated explicit script contexts require independent scope composition",
                ));
            }
        }
        // The URI was already checked and remapped against the parent's closed
        // manifest. Capture it before reading the replacement header for ref inputs.
        let path = self.resolve_file(&bindings.origin, &uri)?;
        if !self.assets.contains_key(&path) {
            self.capture_asset(&path, "script")?;
            self.runtime_assets.insert(path.clone());
        }
        if let Some(inputs) = changes.get_mut("inputs") {
            let code = std::str::from_utf8(&self.asset(&path)?.bytes)
                .map_err(|_| invalid("A generated script reference needs UTF-8 source"))?;
            let header = crate::script::header::Header::parse(code)?;
            let values = inputs
                .as_object_mut()
                .ok_or_else(|| invalid("Script inputs must be an object"))?;
            for input in header.inputs {
                if let Some(value) = values.get_mut(&input.name) {
                    bindings.declared_input(value)?;
                }
            }
            target.properties.insert("inputs".into(), inputs.clone());
            target.script_inputs_resolved = false;
        }
        let output = bindings.inherited_output(&uri, None, inherited_bytes)?;
        // Only code ownership changes here. Unoverridden saved inputs and user
        // metadata continue to come from the referenced component definition.
        changes.as_object_mut().expect("generated properties").insert("metadata".into(),
            json!({"type":"lazypencil","lazypencil":{"script_override":{"version":1,"metadata":metadata,"output":output}}}));
        Ok(())
    }
}
