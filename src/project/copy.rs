//! Retarget typed inputs without executing or rewriting copied JavaScript.

use super::*;
use crate::script::header::{Header, InputKind};

impl Project {
    pub(crate) fn has_copied_script_inputs(node: &Value) -> bool {
        owns_script_inputs(node)
            || node
                .get("children")
                .and_then(Value::as_array)
                .is_some_and(|children| children.iter().any(Self::has_copied_script_inputs))
            || node
                .get("descendants")
                .and_then(Value::as_object)
                .is_some_and(|overrides| overrides.values().any(Self::has_copied_script_inputs))
    }

    pub(crate) fn remap_copied_script_inputs(
        &mut self,
        original: &Value,
        copy: &mut Value,
        ids: &BTreeMap<String, String>,
    ) -> Result<()> {
        let address = NodeAddress {
            node_id: original["id"].as_str().expect("stored node").into(),
            instance_path: Vec::new(),
        };
        self.remap_copy(original, copy, &address, ids)
    }

    fn remap_copy(
        &mut self,
        original: &Value,
        copy: &mut Value,
        address: &NodeAddress,
        ids: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.check_cancelled()?;
        if owns_script_inputs(original) {
            let node = self.expanded_node(address)?;
            if node.kind() == "script" {
                self.remap_script_inputs(&node, copy, ids)?;
            }
        }
        if let (Some(before), Some(after)) = (
            original.get("children").and_then(Value::as_array),
            copy.get_mut("children").and_then(Value::as_array_mut),
        ) {
            for (before, after) in before.iter().zip(after) {
                let address = NodeAddress {
                    node_id: before["id"].as_str().expect("stored child").into(),
                    instance_path: Vec::new(),
                };
                self.remap_copy(before, after, &address, ids)?;
            }
        }
        if original.get("type").is_none_or(|kind| kind == "ref")
            && let (Some(before), Some(after)) = (
                original.get("descendants").and_then(Value::as_object),
                copy.get_mut("descendants").and_then(Value::as_object_mut),
            )
        {
            for (path, before) in before {
                let mut address = address.clone();
                address
                    .instance_path
                    .extend(path.split('/').map(str::to_owned));
                self.remap_copy(
                    before,
                    after.get_mut(path).expect("copied override"),
                    &address,
                    ids,
                )?;
            }
        }
        Ok(())
    }

    fn remap_script_inputs(
        &mut self,
        node: &ResolvedNode,
        copy: &mut Value,
        ids: &BTreeMap<String, String>,
    ) -> Result<()> {
        // Captured inputs keep their spellings and provenance hashes. Only their
        // component targets move; external namespaces keep their original IDs.
        if let Some(bindings) = &node.script_bindings
            && bindings.origin == self.root
        {
            let expected = json!(bindings.original());
            for pointer in [
                "/metadata/lazypencil/script_bindings",
                "/metadata/lazypencil/script_override/output",
            ] {
                if let Some(value) = copy.pointer_mut(pointer)
                    && serde_json::from_value::<bindings::Bindings>(value.clone())
                        .ok()
                        .and_then(|value| serde_json::to_value(value).ok())
                        == Some(expected.clone())
                {
                    for key in ["components", "input_components"] {
                        if let Some(map) = value.get_mut(key) {
                            remap_component_map(map, ids);
                        }
                    }
                    if let Some(scopes) = value
                        .get_mut("script_scopes")
                        .and_then(Value::as_object_mut)
                    {
                        for scope in scopes.values_mut() {
                            remap_component_map(&mut scope["components"], ids);
                        }
                    }
                    if let Some(paths) = value.get_mut("descendants").and_then(Value::as_object_mut)
                    {
                        for path in paths.values_mut() {
                            remap_component_map(path, ids);
                        }
                    }
                }
            }
        }
        if let Some(context) = &node.input_context {
            if context.origin == self.root {
                let expected = json!(&context.values);
                for pointer in [
                    "/metadata/lazypencil/script_input_context",
                    "/metadata/lazypencil/script_override/input",
                ] {
                    if let Some(value) = copy.pointer_mut(pointer)
                        && serde_json::from_value::<script_context::Inputs>(value.clone())
                            .ok()
                            .and_then(|value| serde_json::to_value(value).ok())
                            == Some(expected.clone())
                    {
                        remap_input_context(value, ids);
                    }
                }
            }
            return Ok(());
        }
        if node
            .script_bindings
            .as_ref()
            .is_some_and(|bindings| bindings.inputs_bound)
        {
            return Ok(());
        }
        let Some(inputs) = copy.get_mut("inputs").and_then(Value::as_object_mut) else {
            return Ok(());
        };
        let Some(header) = self.copy_script_header(node)? else {
            // Unavailable or malformed scripts remain copyable for repair.
            return Ok(());
        };
        for input in header.inputs {
            self.check_cancelled()?;
            if !matches!(input.kind, InputKind::Ref) {
                continue;
            }
            let Some(value) = inputs.get_mut(&input.name) else {
                continue;
            };
            let Some(reference) = value
                .as_str()
                .filter(|v| !v.is_empty() && !v.starts_with('$'))
            else {
                continue;
            };
            if let Ok((origin, component)) =
                self.lookup_node_ref(node.script_input_origin(), reference)
                && origin == self.root
                && let Some(new) = component["id"].as_str().and_then(|id| ids.get(id))
            {
                *value = json!(new);
            }
        }
        Ok(())
    }

    fn copy_script_header(&mut self, node: &ResolvedNode) -> Result<Option<Header>> {
        let Some(uri) = node.properties.get("scriptUri").and_then(Value::as_str) else {
            return Ok(None);
        };
        let path = match self.resolve_file(node.property_source("scriptUri"), uri) {
            Ok(path) => path,
            Err(error) if error.code == "REVISION_CONFLICT" => return Err(error),
            Err(_) => return Ok(None),
        };
        self.capture_asset(&path, "script")?;
        Ok(std::str::from_utf8(&self.asset(&path)?.bytes)
            .ok()
            .and_then(|code| Header::parse(code).ok()))
    }
}

fn owns_script_inputs(node: &Value) -> bool {
    node.get("inputs").is_some()
        || ["script_input_context", "script_bindings", "script_override"]
            .iter()
            .any(|key| {
                node.pointer("/metadata/lazypencil")
                    .and_then(|namespace| namespace.get(*key))
                    .is_some()
            })
}

fn remap_component_map(value: &mut Value, ids: &BTreeMap<String, String>) {
    if let Some(map) = value.as_object_mut() {
        for target in map.values_mut() {
            if let Some(path) = target.as_str() {
                let remapped = path
                    .split('/')
                    .map(|part| ids.get(part).map_or(part, String::as_str))
                    .collect::<Vec<_>>()
                    .join("/");
                if remapped != path {
                    *target = json!(remapped);
                }
            }
        }
    }
}

fn remap_input_context(value: &mut Value, ids: &BTreeMap<String, String>) {
    remap_component_map(&mut value["components"], ids);
    if let Some(generations) = value.get_mut("generations").and_then(Value::as_object_mut) {
        for generation in generations.values_mut() {
            for key in ["components", "scope_components", "binding_components"] {
                if let Some(map) = generation.get_mut(key) {
                    remap_component_map(map, ids);
                }
            }
        }
    }
}
