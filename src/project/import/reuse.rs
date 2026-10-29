//! Keep placed node identities distinct from explicitly substituted definitions.
use super::*;

mod paths;

#[derive(Default)]
pub(super) struct Source {
    positions: BTreeMap<Key, BTreeSet<Key>>,
}

pub(super) struct Mapping<'a> {
    pub source: &'a str,
    pub target: Option<&'a str>,
    pub reused: Option<&'a Key>,
}

fn position(node: &ResolvedNode) -> Key {
    (node.component_source.clone(), node.component_key.clone())
}

fn authored(node: &ResolvedNode) -> Key {
    node.instance_identity
        .clone()
        .unwrap_or_else(|| (node.source_path.clone(), node.source_id.clone()))
}

fn index_source(
    project: &Project,
    node: &ResolvedNode,
    source: &mut Source,
    bytes: &mut usize,
) -> Result<()> {
    project.check_cancelled()?;
    let key = position(node);
    let owner = authored(node);
    let size = key
        .0
        .as_os_str()
        .len()
        .saturating_add(key.1.len())
        .saturating_add(owner.0.as_os_str().len())
        .saturating_add(owner.1.len())
        .saturating_add(64);
    if source.positions.entry(key).or_default().insert(owner) {
        *bytes = bytes.saturating_add(size);
        if *bytes > crate::document::MAX_DOCUMENT_BYTES {
            return Err(copy_error(
                "Reused component indexes exceed the aggregate byte limit",
            ));
        }
    }
    for child in &node.children {
        index_source(project, child, source, bytes)?;
    }
    Ok(())
}

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

fn index_paths(
    project: &Project,
    root: &ResolvedNode,
    prefix: &str,
    paths: &mut BTreeMap<String, bool>,
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
        let fresh = !paths.contains_key(&path);
        paths.entry(path.clone()).or_insert(child.is_instance);
        if child.is_instance {
            if fresh {
                index_paths(project, child, &format!("{path}/"), paths, bytes)?;
            }
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
            let mut paths = BTreeMap::new();
            index_paths(
                self.project,
                &expanded,
                "",
                &mut paths,
                &mut self.reused_path_bytes,
            )?;
            self.reused_targets.insert(target.into(), paths);
        }
        let original = self.project.expand(
            &key.0,
            self.project.documents[&key.0].node(&key.1)?,
            None,
            &[],
            &mut Vec::new(),
            &mut 0,
        )?;
        let mut source = Source::default();
        index_source(
            self.project,
            &original,
            &mut source,
            &mut self.reused_path_bytes,
        )?;
        if let Some(paths) = self.options.component_paths.get(&self.key(key)?) {
            let mut source_paths = BTreeMap::new();
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
                if !source_paths.contains_key(from) || !self.reused_targets[target].contains_key(to)
                {
                    return Err(copy_error(
                        "A reused component path must resolve in both the source and selected local definition",
                    ));
                }
            }
        }
        self.reused_sources.insert(key.clone(), source);
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

    pub(super) fn available_replacement_path(
        &self,
        key: &Key,
        path: &[&str],
    ) -> Result<Option<String>> {
        self.project.check_cancelled()?;
        let original = path.join("/");
        let replacement = self
            .options
            .component_paths
            .get(&self.key(key)?)
            .and_then(|paths| paths.get(&original))
            .unwrap_or(&original);
        Ok(self.reused_targets[&self.reuse[key]]
            .contains_key(replacement)
            .then(|| replacement.clone()))
    }

    pub(super) fn root_reuse(&self, node: &ResolvedNode) -> Result<Option<Key>> {
        if self.reuse.is_empty() {
            return Ok(None);
        }
        // A full descendant replacement retains the old address, but its
        // authored node owns the new body. Do not revive the old substitution.
        let key = authored(node);
        if self.reuse.contains_key(&key) {
            return Ok(Some(key));
        }
        if let Some((origin, id)) = &node.instance_identity {
            let mut source = origin.clone();
            let mut original = self.project.documents[origin].node(id)?.clone();
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
        Ok(None)
    }

    pub(super) fn walk_copied_paths(
        &self,
        root: &ResolvedNode,
        visitor: &mut dyn FnMut(Mapping<'_>) -> Result<bool>,
    ) -> Result<()> {
        paths::walk(self, root, visitor)
    }

    pub(super) fn copied_override_paths<'a>(
        &self,
        root: &ResolvedNode,
        requested: impl Iterator<Item = &'a str>,
    ) -> Result<BTreeMap<String, String>> {
        let mut wanted = requested.collect::<BTreeSet<_>>();
        let mut result = BTreeMap::new();
        if wanted.is_empty() {
            return Ok(result);
        }
        self.walk_copied_paths(root, &mut |path| {
            if wanted.remove(path.source) {
                let target = path.target.ok_or_else(|| {
                    copy_error("A descendant is absent from the reused component; supply an explicit component_paths mapping")
                        .details(json!({"source_path":path.source,"source_key":path.reused.and_then(|key|self.key(key).ok())}))
                })?;
                result.insert(path.source.to_owned(), target.to_owned());
            }
            Ok(!wanted.is_empty())
        })?;
        if !wanted.is_empty() {
            return Err(copy_error("Missing copied override target"));
        }
        Ok(result)
    }
}
