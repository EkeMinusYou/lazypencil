//! Map authored runtime children without reviving replaced component bodies.
use super::*;

#[derive(Clone)]
enum Namespace {
    Original(String),
    Current(PathBuf, String),
}

#[derive(Clone)]
struct Address {
    source: String,
    target: String,
    context: DescendantContext,
    namespace: Option<Namespace>,
    generation: usize,
}

struct Paths<'a> {
    bound: Option<&'a Bound>,
    owner: &'a Path,
    project: &'a Project,
    handles: &'a BTreeMap<String, (PathBuf, String)>,
    ids: &'a BTreeMap<String, String>,
    root: Namespace,
    authored: BTreeMap<String, Address>,
    removed: BTreeMap<String, usize>,
    current: BTreeMap<(PathBuf, String), BTreeMap<String, DescendantContext>>,
    generation: usize,
    bytes: usize,
}

fn prefix(path: &str, instance: bool) -> String {
    if instance {
        format!("{path}/")
    } else {
        path.rsplit_once('/')
            .map_or_else(String::new, |(parent, _)| format!("{parent}/"))
    }
}

impl Paths<'_> {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > crate::script::MAX_OUTPUT_BYTES {
            return Err(invalid(
                "Generated override addresses exceed their byte budget",
            ));
        }
        self.project.check_cancelled()
    }

    fn namespace(&self, node: &Value) -> Result<Option<Namespace>> {
        if node["type"] != "ref" || node.get("children").is_some() {
            return Ok(None);
        }
        let name = node["ref"]
            .as_str()
            .ok_or_else(|| invalid("A generated ref needs its target"))?;
        if self
            .bound
            .is_some_and(|bound| bound.values.components.contains_key(name))
        {
            Ok(Some(Namespace::Original(name.into())))
        } else if let Some((origin, id)) = self.handles.get(name) {
            Ok(Some(Namespace::Current(origin.clone(), id.clone())))
        } else {
            let target = self
                .bound
                .map(|bound| bound.component(name))
                .transpose()?
                .unwrap_or(name);
            let (origin, definition) = self.project.lookup_node(self.owner, target)?;
            Ok(Some(Namespace::Current(
                origin,
                definition["id"].as_str().expect("component ID").into(),
            )))
        }
    }

    fn in_namespace(&mut self, namespace: &Namespace, path: &str) -> Result<Option<Address>> {
        let (target, context) = match namespace {
            Namespace::Original(name) => {
                let bound = self.bound.expect("original namespace");
                let Some(target) = bound
                    .values
                    .descendants
                    .get(name)
                    .and_then(|paths| paths.get(path))
                else {
                    return Ok(None);
                };
                let context = bound
                    .values
                    .descendant_contexts
                    .get(name)
                    .and_then(|contexts| contexts.get(path))
                    .ok_or_else(|| {
                        invalid("Runtime child overrides need captured descendant contexts")
                    })?;
                (target.clone(), context.clone())
            }
            Namespace::Current(origin, id) => {
                let key = (origin.clone(), id.clone());
                if !self.current.contains_key(&key) {
                    let node = self
                        .project
                        .documents
                        .get(origin)
                        .ok_or_else(|| invalid("Missing generated component document"))?
                        .node(id)?;
                    let root = self.project.expand(
                        origin,
                        node,
                        None,
                        &[],
                        &mut Vec::new(),
                        &mut Expansion::default(),
                    )?;
                    let mut contexts = BTreeMap::new();
                    self.capture(&root, "", "", &mut contexts)?;
                    self.current.insert(key.clone(), contexts);
                }
                let Some(context) = self.current[&key].get(path) else {
                    return Ok(None);
                };
                (path.into(), context.clone())
            }
        };
        Ok(Some(Address {
            source: path.into(),
            target,
            context,
            namespace: None,
            generation: 0,
        }))
    }

    fn capture(
        &mut self,
        node: &ResolvedNode,
        from: &str,
        parent: &str,
        contexts: &mut BTreeMap<String, DescendantContext>,
    ) -> Result<()> {
        for child in &node.children {
            let path = format!("{from}{}", child.component_key);
            if contexts.contains_key(&path) {
                if child.is_instance {
                    continue;
                }
            } else {
                self.charge(path.len() + parent.len() + 64)?;
                contexts.insert(
                    path.clone(),
                    DescendantContext {
                        parent: parent.into(),
                        instance: child.is_instance,
                        target_instance: child.is_instance,
                    },
                );
            }
            self.capture(child, &prefix(&path, child.is_instance), &path, contexts)?;
        }
        Ok(())
    }

    fn raw(&mut self, path: &str) -> Result<Option<Address>> {
        if let Some(address) = self.authored.get(path) {
            return Ok(Some(address.clone()));
        }
        for (index, _) in path.match_indices('/').rev() {
            let Some(parent) = self.authored.get(&path[..index]).cloned() else {
                continue;
            };
            let Some(namespace) = &parent.namespace else {
                continue;
            };
            let Some(mut address) = self.in_namespace(namespace, &path[index + 1..])? else {
                return Ok(None);
            };
            address.source = path.into();
            address.target = format!("{}/{}", parent.target, address.target);
            address.context.parent = if address.context.parent.is_empty() {
                parent.source
            } else {
                format!("{}/{}", parent.source, address.context.parent)
            };
            address.generation = parent.generation;
            return Ok(Some(address));
        }
        self.in_namespace(&self.root.clone(), path)
    }

    fn ancestry(&self, path: &str) -> Option<String> {
        let bound = self.bound?;
        for (index, _) in path.match_indices('/').rev() {
            let parent = self.authored.get(&path[..index]);
            if let Some(Namespace::Original(name)) =
                parent.and_then(|parent| parent.namespace.as_ref())
            {
                let context = bound
                    .values
                    .descendant_contexts
                    .get(name)?
                    .get(&path[index + 1..])?;
                return Some(if context.parent.is_empty() {
                    path[..index].into()
                } else {
                    format!("{}/{}", &path[..index], context.parent)
                });
            }
        }
        if let Namespace::Original(name) = &self.root {
            return bound
                .values
                .descendant_contexts
                .get(name)?
                .get(path)
                .map(|context| context.parent.clone());
        }
        None
    }

    fn removed(&mut self, address: &Address) -> Result<bool> {
        let mut parent = address.context.parent.clone();
        let mut depth = 0;
        while !parent.is_empty() {
            depth += 1;
            if depth > 256
                || self
                    .removed
                    .get(&parent)
                    .is_some_and(|generation| address.generation < *generation)
            {
                return Ok(true);
            }
            parent = if let Some(address) = self.raw(&parent)? {
                address.context.parent
            } else {
                self.ancestry(&parent)
                    .ok_or_else(|| invalid("Missing generated override ancestry"))?
            };
        }
        if self
            .removed
            .get("")
            .is_some_and(|generation| address.generation < *generation)
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn address(&mut self, path: &str) -> Result<Address> {
        let address = self.raw(path)?.ok_or_else(|| {
            invalid("A generated override path is absent from the script bindings")
        })?;
        if self.removed(&address)? {
            return Err(invalid("A generated override addresses a replaced child"));
        }
        Ok(address)
    }

    fn insert(&mut self, address: Address) -> Result<()> {
        if address.source.len() > 4096 || address.target.len() > 4096 {
            return Err(invalid("Generated override paths exceed 4096 bytes"));
        }
        self.charge(
            address.source.len() + address.target.len() + address.context.parent.len() + 128,
        )?;
        self.authored.insert(address.source.clone(), address);
        Ok(())
    }

    fn children(&mut self, parent: &Address, children: &[Value]) -> Result<()> {
        for child in children {
            let id = child["id"]
                .as_str()
                .ok_or_else(|| invalid("A generated child needs an ID"))?;
            let source_id = self.ids.get(id).map(String::as_str).unwrap_or(id);
            let source = format!(
                "{}{source_id}",
                prefix(&parent.source, parent.context.instance)
            );
            if let Some(previous) = self.raw(&source)?
                && !self.removed(&previous)?
            {
                return Err(invalid(
                    "Generated child addresses collide with a retained component child",
                ));
            }
            let target = format!(
                "{}{id}",
                prefix(&parent.target, parent.context.target_instance)
            );
            let instance = child["type"] == "ref";
            let address = Address {
                source,
                target,
                context: DescendantContext {
                    parent: parent.source.clone(),
                    instance,
                    target_instance: instance,
                },
                namespace: self.namespace(child)?,
                generation: self.generation,
            };
            self.insert(address.clone())?;
            if let Some(children) = child.get("children").and_then(Value::as_array) {
                self.children(&address, children)?;
            }
            self.nested(&address, child)?;
        }
        Ok(())
    }

    fn nested(&mut self, parent: &Address, node: &Value) -> Result<()> {
        if node["type"] != "ref" {
            return Ok(());
        }
        if let Some(changes) = node.get("descendants").and_then(Value::as_object) {
            let mut entries = changes.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(path, _)| path.split('/').count());
            for (path, changes) in entries {
                let address = self.address(&format!("{}/{path}", parent.source))?;
                self.apply(address, changes)?;
            }
        }
        Ok(())
    }

    fn apply(&mut self, mut address: Address, changes: &Value) -> Result<()> {
        if changes.get("type").is_none() && changes.get("children").is_none() {
            return Ok(());
        }
        self.generation += 1;
        self.removed.insert(address.source.clone(), self.generation);
        if changes.get("type").is_some() {
            let instance = changes["type"] == "ref";
            address.context.instance = instance;
            address.context.target_instance = instance;
            address.namespace = self.namespace(changes)?;
            address.generation = self.generation;
        } else {
            address.namespace = None;
        }
        self.insert(address.clone())?;
        if let Some(children) = changes.get("children").and_then(Value::as_array) {
            self.children(&address, children)?;
        }
        self.nested(&address, changes)?;
        Ok(())
    }
}

pub(super) fn remap(
    bound: Option<&Bound>,
    owner: &Path,
    node: &Value,
    changes: Map<String, Value>,
    handles: &BTreeMap<String, (PathBuf, String)>,
    project: &Project,
    ids: &BTreeMap<String, String>,
) -> Result<Vec<(String, Value)>> {
    let name = node.get("ref").and_then(Value::as_str);
    let paths = bound.and_then(|bound| name.and_then(|name| bound.values.descendants.get(name)));
    let declared = bound
        .is_some_and(|bound| name.is_some_and(|name| bound.values.components.contains_key(name)));
    let structured = bound.is_some_and(|bound| {
        name.is_some_and(|name| bound.values.descendant_contexts.contains_key(name))
    });
    let mut entries = changes.into_iter().collect::<Vec<_>>();
    entries.sort_by_key(|(path, _)| path.split('/').count());
    let structural = node.get("children").is_some()
        || entries
            .iter()
            .any(|(_, changes)| changes.get("type").is_some() || changes.get("children").is_some());
    if node["type"] != "ref" || !structural || declared && !structured {
        return entries
            .into_iter()
            .map(|(path, changes)| {
                let target = if declared {
                    paths
                        .and_then(|paths| paths.get(&path))
                        .cloned()
                        .ok_or_else(|| {
                            invalid("A generated override path is absent from the script bindings")
                        })?
                } else {
                    path
                };
                Ok((target, changes))
            })
            .collect();
    }
    let mut resolver = Paths {
        bound,
        owner,
        project,
        handles,
        ids,
        root: Namespace::Original(String::new()),
        authored: BTreeMap::new(),
        removed: BTreeMap::new(),
        current: BTreeMap::new(),
        generation: 0,
        bytes: 0,
    };
    resolver.root = resolver.namespace(node)?.unwrap_or_else(|| {
        // Explicit children replace the body, but root address discovery still
        // needs the referenced definition for validating removed source paths.
        Namespace::Original(name.unwrap_or_default().into())
    });
    if node.get("children").is_some() {
        let mut reference = node.clone();
        reference.as_object_mut().expect("node").remove("children");
        resolver.root = resolver
            .namespace(&reference)?
            .expect("reference namespace");
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        resolver.generation = 1;
        let root = Address {
            source: String::new(),
            target: String::new(),
            context: DescendantContext {
                parent: String::new(),
                instance: false,
                target_instance: false,
            },
            namespace: None,
            generation: 1,
        };
        resolver.removed.insert(String::new(), 1);
        resolver.children(&root, children)?;
    }
    let mut result = Vec::new();
    for (path, changes) in entries {
        project.check_cancelled()?;
        let address = resolver.address(&path)?;
        let target = address.target.clone();
        resolver.apply(address, &changes)?;
        result.push((target, changes));
    }
    Ok(result)
}
