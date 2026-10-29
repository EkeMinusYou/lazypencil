//! Keep placed node identities distinct from explicitly substituted definitions.
use super::*;

pub(super) fn placement(node: &Value) -> Map<String, Value> {
    // The requested substitution selects the target's body, appearance, code,
    // metadata and descendants. Only the source's placement stays in its parent.
    [
        "name",
        "x",
        "y",
        "width",
        "height",
        "rotation",
        "flipX",
        "flipY",
        "enabled",
        "opacity",
        "blendMode",
        "layoutPosition",
        "layoutIncludeStroke",
    ]
    .into_iter()
    .filter_map(|key| node.get(key).map(|value| (key.into(), value.clone())))
    .collect()
}

fn descendant<'a>(root: &'a ResolvedNode, path: &[&str]) -> Option<&'a ResolvedNode> {
    let (first, rest) = path.split_first()?;
    for child in &root.children {
        if child.component_key == *first {
            return if rest.is_empty() {
                Some(child)
            } else if child.is_instance {
                descendant(child, rest)
            } else {
                None
            };
        }
        if !child.is_instance
            && let Some(found) = descendant(child, path)
        {
            return Some(found);
        }
    }
    None
}

fn index_paths(
    project: &Project,
    root: &ResolvedNode,
    prefix: &str,
    paths: &mut BTreeSet<String>,
    bytes: &mut usize,
) -> Result<()> {
    for child in &root.children {
        project.check_cancelled()?;
        let path = format!("{prefix}{}", child.component_key);
        *bytes = bytes.saturating_add(path.len());
        if path.len() > 4096 || *bytes > crate::document::MAX_DOCUMENT_BYTES {
            return Err(copy_error(
                "Reused component paths exceed the path or aggregate byte limit",
            ));
        }
        paths.insert(path.clone());
        if child.is_instance {
            index_paths(project, child, &format!("{path}/"), paths, bytes)?;
        } else {
            index_paths(project, child, prefix, paths, bytes)?;
        }
    }
    Ok(())
}

impl Copier<'_> {
    pub(super) fn remember_reuse(&mut self, key: &Key, target: &str) -> Result<()> {
        if self.reuse.contains_key(key) {
            return Ok(());
        }
        self.existing_component(target)?;
        if !self.reused_targets.contains_key(target) {
            let definition = self.project.root_document().node(target)?;
            let mut count = self.reused_target_nodes;
            let expanded = self.project.expand(
                &self.project.root,
                definition,
                None,
                &[],
                &mut Vec::new(),
                &mut count,
            )?;
            self.reused_target_nodes = count;
            let mut paths = BTreeSet::new();
            index_paths(
                self.project,
                &expanded,
                "",
                &mut paths,
                &mut self.reused_path_bytes,
            )?;
            self.reused_targets.insert(target.into(), paths);
        }
        if let Some(paths) = self.options.component_paths.get(&self.key(key)?) {
            let original = self.project.expand(
                &key.0,
                self.project.documents[&key.0].node(&key.1)?,
                None,
                &[],
                &mut Vec::new(),
                &mut 0,
            )?;
            let mut source_paths = BTreeSet::new();
            index_paths(self.project, &original, "", &mut source_paths, &mut 0)?;
            for (from, to) in paths {
                self.project.check_cancelled()?;
                for path in [from, to] {
                    if path.is_empty() || path.len() > 4096 || path.split('/').any(str::is_empty) {
                        return Err(copy_error(
                            "A reused component path needs 1..4096 bytes and nonempty segments",
                        ));
                    }
                }
                if !source_paths.contains(from) || !self.reused_targets[target].contains(to) {
                    return Err(copy_error(
                        "A reused component path must resolve in both the source and selected local definition",
                    ));
                }
            }
        }
        self.reuse.insert(key.clone(), target.into());
        Ok(())
    }

    pub(super) fn placement_target(&self, key: &Key) -> Result<String> {
        self.ids
            .get(key)
            .cloned()
            .or_else(|| self.reuse.get(key).cloned())
            .ok_or_else(|| copy_error("Missing copied placement ID"))
    }

    pub(super) fn replacement_path(&self, key: &Key, path: &[&str]) -> Result<String> {
        self.project.check_cancelled()?;
        let original = path.join("/");
        let replacement = self
            .options
            .component_paths
            .get(&self.key(key)?)
            .and_then(|paths| paths.get(&original))
            .unwrap_or(&original);
        if !self.reused_targets[&self.reuse[key]].contains(replacement) {
            return Err(copy_error("A descendant is absent from the reused component; supply an explicit component_paths mapping")
                .details(json!({"source_key":self.key(key)?,"source_path":original,"target":self.reuse[key]})));
        }
        Ok(replacement.clone())
    }

    pub(super) fn root_reuse(&self, node: &ResolvedNode) -> Result<Option<Key>> {
        if self.reuse.is_empty() {
            return Ok(None);
        }
        // An instance's component identity names the placement. Its source
        // identity names the reusable definition, including ref-to-ref chains.
        let key = (node.component_source.clone(), node.component_key.clone());
        if self.reuse.contains_key(&key) {
            return Ok(Some(key));
        }
        if let Some((origin, id)) = &node.instance_identity {
            let mut source = origin.clone();
            let mut original = self.project.documents[origin].node(id)?;
            for _ in 0..128 {
                self.project.check_cancelled()?;
                let Some(reference) = original.get("ref").and_then(Value::as_str) else {
                    break;
                };
                let (path, definition) = self.project.lookup_node(&source, reference)?;
                let key = (
                    path.clone(),
                    definition["id"].as_str().expect("definition ID").into(),
                );
                if self.reuse.contains_key(&key) {
                    return Ok(Some(key));
                }
                source = path;
                original = definition;
            }
        }
        let key = (node.source_path.clone(), node.source_id.clone());
        if self.reuse.contains_key(&key) {
            return Ok(Some(key));
        }
        Ok(None)
    }

    pub(super) fn copied_override_path(&self, root: &ResolvedNode, path: &str) -> Result<String> {
        let parts = path.split('/').collect::<Vec<_>>();
        if descendant(root, &parts).is_none() {
            return Err(copy_error("Missing copied override target"));
        }
        self.map_descendant(root, &parts)?
            .ok_or_else(|| copy_error("Missing copied override target"))
    }

    fn map_descendant(&self, root: &ResolvedNode, path: &[&str]) -> Result<Option<String>> {
        self.project.check_cancelled()?;
        if let Some(key) = self.root_reuse(root)? {
            return self.replacement_path(&key, path).map(Some);
        }
        let Some((first, rest)) = path.split_first() else {
            return Ok(None);
        };
        for child in &root.children {
            let key = (child.component_source.clone(), child.component_key.clone());
            if child.component_key == *first {
                let placed = self.placement_target(&key)?;
                return if rest.is_empty() {
                    Ok(Some(placed))
                } else if child.is_instance {
                    Ok(self
                        .map_descendant(child, rest)?
                        .map(|path| format!("{placed}/{path}")))
                } else {
                    Ok(None)
                };
            }
            if !child.is_instance && descendant(child, path).is_some() {
                let mapped = self.map_descendant(child, path)?;
                // A contained definition was an ordinary grouping boundary.
                // Its replacement is an instance and needs an explicit prefix.
                return if self.reuse.contains_key(&key) {
                    Ok(mapped.map(|path| format!("{}/{path}", self.ids[&key])))
                } else {
                    Ok(mapped)
                };
            }
        }
        Ok(None)
    }
}
