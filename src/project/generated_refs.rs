//! Preserve code and input ownership independently for generated references.

use super::*;

impl Project {
    pub(super) fn bind_generated_ref_contexts(
        &mut self,
        node: &mut Value,
        bindings: &bindings::Bound,
        inherited: &BTreeMap<String, String>,
        inherited_bytes: &mut usize,
        context: &bindings::GeneratedState,
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
                || browser_context::has_context(node)
                || bindings::has_generated_scope(node)
                || node.get("children").is_some())
        {
            let preview = if bindings.values.assets.is_some()
                && (browser_context::has_preview_contexts(node)
                    || bindings::has_pending_scopes(node))
            {
                let mut preview = node.clone();
                browser_context::clear_preview_contexts(&mut preview);
                bindings::clear_pending_scopes(&mut preview);
                std::borrow::Cow::Owned(preview)
            } else {
                std::borrow::Cow::Borrowed(&*node)
            };
            let mut resolved = self.expand(
                &bindings.origin,
                &preview,
                None,
                &[],
                &mut Vec::new(),
                &mut 0,
            )?;
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
            for (index, child) in children.iter_mut().enumerate() {
                self.bind_generated_ref_contexts(
                    child,
                    bindings,
                    &theme,
                    inherited_bytes,
                    context
                        .children
                        .get(index)
                        .ok_or_else(|| invalid("Generated context child is missing"))?,
                )?;
            }
        }
        if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
            for (path, changes) in changes {
                let context = context
                    .descendants
                    .get(path)
                    .ok_or_else(|| invalid("Generated descendant context is missing"))?;
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
                        context,
                    )?;
                } else if let Some(children) =
                    changes.get_mut("children").and_then(Value::as_array_mut)
                {
                    for (index, child) in children.iter_mut().enumerate() {
                        self.bind_generated_ref_contexts(
                            child,
                            bindings,
                            &target_theme,
                            inherited_bytes,
                            context.children.get(index).ok_or_else(|| {
                                invalid("Generated replacement context is missing")
                            })?,
                        )?;
                    }
                }
            }
        }
        if let Some(mut resolved) = reference {
            self.bind_generated_source(node, &mut resolved, bindings, inherited_bytes, context)?;
            if resolved.kind() == "browser" {
                let owner = bindings.document_scope()?;
                let library = self.generated_browser_url_library(&resolved, &owner)?;
                owner.generated_browser_context_at(
                    node,
                    false,
                    resolved.properties.get("url").and_then(Value::as_str),
                    library.as_deref(),
                    inherited_bytes,
                )?;
            }
            if let Some(changes) = node.get_mut("descendants").and_then(Value::as_object_mut) {
                for (path, changes) in changes {
                    if changes.get("type").is_none()
                        && (changes.get("inputs").is_some()
                            || changes.get("scriptUri").is_some()
                            || changes.get("url").is_some()
                            || browser_context::has_context(changes)
                            || bindings::has_generated_scope(changes))
                    {
                        let target = find_descendant_mut(
                            &mut resolved,
                            &path.split('/').collect::<Vec<_>>(),
                        )
                        .ok_or_else(|| invalid("A generated script override target is missing"))?;
                        self.bind_generated_source(
                            changes,
                            target,
                            bindings,
                            inherited_bytes,
                            context
                                .descendants
                                .get(path)
                                .ok_or_else(|| invalid("Generated override context is missing"))?,
                        )?;
                        if target.kind() == "browser" {
                            let owner = bindings.document_scope()?;
                            let library = self.generated_browser_url_library(target, &owner)?;
                            owner.generated_browser_context_at(
                                changes,
                                false,
                                target.properties.get("url").and_then(Value::as_str),
                                library.as_deref(),
                                inherited_bytes,
                            )?;
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
        if node.kind() != "script" || node.input_context.is_some() {
            return Ok(());
        }
        // Native child scopes already own saved ref choices. Turning them into
        // parent handles here would bypass their independent input guards.
        if bindings.values.assets.is_none()
            && (node.script_input_scope.is_some() || node.script_bindings.is_some())
        {
            return Ok(());
        }
        let uri = node
            .properties
            .get("scriptUri")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("A generated script reference needs scriptUri"))?;
        let path = self.resolve_file(node.property_source("scriptUri"), uri)?;
        if !self.assets.contains_key(&path) {
            self.capture_asset(&path, "script")?;
            self.runtime_assets.insert(path.clone());
        }
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
        context: &bindings::GeneratedState,
    ) -> Result<()> {
        let provided_source = changes.get("scriptUri").is_some();
        if !provided_source && !bindings::has_generated_scope(changes)
            || changes["enabled"] == false
            || target.kind() != "script"
            || bindings.values.assets.is_none()
        {
            return Ok(());
        }
        let uri = changes
            .get("scriptUri")
            .or_else(|| target.properties.get("scriptUri"))
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("A generated script override needs a source URI"))?
            .to_owned();
        let mut metadata = changes.get("metadata").cloned();
        let mut explicit_output = None;
        let mut explicit_input = None;
        let mut scoped = false;
        if let Some(namespace) = metadata
            .as_ref()
            .and_then(|metadata| metadata.get("lazypencil"))
            && (namespace.get("script_scope").is_some()
                || namespace
                    .get("script_bindings")
                    .map(|value| bindings.generated_output(value).map(|canonical| !canonical))
                    .transpose()?
                    .unwrap_or(false))
        {
            let selected = bindings.compose_generated_scopes(
                namespace,
                changes,
                target,
                context,
                &uri,
                inherited_bytes,
            )?;
            explicit_output = selected.0;
            explicit_input = selected.1;
            let namespace = metadata
                .as_mut()
                .and_then(|metadata| metadata.pointer_mut("/lazypencil"))
                .and_then(Value::as_object_mut)
                .expect("validated scope");
            namespace.remove("script_scope");
            namespace.remove("script_bindings");
            scoped = true;
        }
        if let Some(metadata) = &metadata {
            if !metadata.is_object() {
                return Err(invalid("Generated script metadata must be an object"));
            }
            let (output, input) = script_context::metadata_contexts(Some(metadata));
            if !scoped
                && (output || input || metadata.pointer("/lazypencil/script_override").is_some())
            {
                let candidate = metadata.pointer("/lazypencil/script_bindings");
                if !candidate
                    .map(|value| bindings.generated_output(value))
                    .transpose()?
                    .unwrap_or(false)
                    || candidate
                        .and_then(|value| value.get("script_uri"))
                        .and_then(Value::as_str)
                        != Some(uri.as_str())
                    || metadata.pointer("/lazypencil/script_scope").is_some()
                    || metadata.pointer("/lazypencil/script_override").is_some()
                {
                    return Err(Error::new(
                        "UNSUPPORTED_FEATURE",
                        "Generated explicit script contexts require independent scope composition",
                    ));
                }
                explicit_output = candidate.cloned();
                if let Some(context) = metadata.pointer("/lazypencil/script_input_context") {
                    if !bindings.generated_input(context)? {
                        return Err(invalid(
                            "Generated inputs are outside their captured namespace",
                        ));
                    }
                    explicit_input = Some(context.clone());
                }
            }
        }
        // The URI was already checked and remapped against the parent's closed
        // manifest. Capture it before reading the replacement header for ref inputs.
        let owner = if provided_source {
            bindings.origin.as_path()
        } else {
            target.property_source("scriptUri")
        };
        let path = self.resolve_file(owner, &uri)?;
        if !self.assets.contains_key(&path) {
            self.capture_asset(&path, "script")?;
            self.runtime_assets.insert(path.clone());
        }
        if let Some(inputs) = changes.get_mut("inputs") {
            target.input_context = None;
            target.script_input_scope = None;
            target.properties.insert("inputs".into(), inputs.clone());
            if let Some(context) = &explicit_input {
                target.input_context =
                    script_context::bind(self, target, context, &bindings.origin)?;
            }
            let code = std::str::from_utf8(&self.asset(&path)?.bytes)
                .map_err(|_| invalid("A generated script reference needs UTF-8 source"))?;
            let header = crate::script::header::Header::parse(code)?;
            let values = inputs
                .as_object_mut()
                .ok_or_else(|| invalid("Script inputs must be an object"))?;
            for input in header.inputs {
                if let Some(value) = values.get_mut(&input.name) {
                    if let Some(context) = &target.input_context {
                        *value = context.expression(&input.name, value)?;
                    } else {
                        bindings.declared_input(value)?;
                    }
                }
            }
            target.properties.insert("inputs".into(), inputs.clone());
            target.script_inputs_resolved = false;
        }
        let output = if explicit_output.is_some() || !provided_source {
            explicit_output
        } else {
            Some(bindings.document_scope()?.inherited_output_with_hash(
                &uri,
                None,
                context.uri_hash.clone(),
                inherited_bytes,
            )?)
        };
        let mut metadata = metadata;
        if let Some(namespace) = metadata
            .as_mut()
            .and_then(|value| value.get_mut("lazypencil"))
            .and_then(Value::as_object_mut)
        {
            namespace.remove("script_bindings");
            namespace.remove("script_input_context");
        }
        // Only code ownership changes here. Unoverridden saved inputs and user
        // metadata continue to come from the referenced component definition.
        changes.as_object_mut().expect("generated properties").insert("metadata".into(),
            json!({"type":"lazypencil","lazypencil":{"script_override":{"version":1,"metadata":metadata,"output":output,"input":explicit_input}}}));
        Ok(())
    }
}
