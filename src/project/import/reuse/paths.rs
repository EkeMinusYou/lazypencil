//! Translate source addresses while retaining separately authored replacement bodies.
use super::*;

#[derive(Clone)]
struct Substitution {
    key: Key,
    from: String,
    to: Option<String>,
}

struct Prefix {
    parent: String,
    from: String,
    to: Option<String>,
    substitution: Option<Substitution>,
}

#[derive(Clone, Copy, PartialEq)]
enum Provenance {
    Ordinary,
    Body,
    Replacement,
    NewChild,
}

struct Walker<'a, 'project, 'b> {
    copier: &'a Copier<'project>,
    visitor: &'b mut dyn FnMut(Mapping<'_>) -> Result<bool>,
    seen: BTreeSet<String>,
    bytes: usize,
}

impl Walker<'_, '_, '_> {
    fn provenance(&self, prefix: &Prefix, child: &ResolvedNode) -> Provenance {
        let Some(scope) = &prefix.substitution else {
            return Provenance::Ordinary;
        };
        match self.copier.reused_sources[&scope.key]
            .positions
            .get(&position(child))
        {
            Some(owners) if owners.contains(&authored(child)) => Provenance::Body,
            Some(_) => Provenance::Replacement,
            None => Provenance::NewChild,
        }
    }

    fn children(&mut self, node: &ResolvedNode, prefix: Prefix) -> Result<bool> {
        for child in &node.children {
            self.copier.project.check_cancelled()?;
            let source = format!("{}{}", prefix.from, child.component_key);
            let fresh = self.seen.insert(source.clone());
            if fresh {
                self.bytes = self.bytes.saturating_add(source.len());
                if source.len() > 4096 || self.bytes > crate::document::MAX_DOCUMENT_BYTES {
                    return Err(copy_error(
                        "Copied override paths exceed the path or aggregate byte limit",
                    ));
                }
            } else if child.is_instance {
                // find_descendant uses the first matching address. An ordinary
                // duplicate may still contain independently reachable children.
                continue;
            }
            let provenance = self.provenance(&prefix, child);
            let relative = if matches!(provenance, Provenance::Body | Provenance::Replacement) {
                let scope = prefix.substitution.as_ref().expect("substitution");
                let old = source.strip_prefix(&scope.from).expect("source prefix");
                self.copier
                    .available_replacement_path(&scope.key, &old.split('/').collect::<Vec<_>>())?
            } else {
                None
            };
            let target = match provenance {
                Provenance::Body | Provenance::Replacement => {
                    let scope = prefix.substitution.as_ref().expect("substitution");
                    scope
                        .to
                        .as_ref()
                        .zip(relative.as_ref())
                        .map(|(prefix, path)| format!("{prefix}{path}"))
                }
                Provenance::Ordinary | Provenance::NewChild => prefix
                    .to
                    .as_ref()
                    .map(|prefix| {
                        self.copier
                            .placement_target(&position(child))
                            .map(|id| format!("{prefix}{id}"))
                    })
                    .transpose()?,
            };
            let instance = if provenance == Provenance::Body {
                prefix
                    .substitution
                    .as_ref()
                    .zip(relative.as_ref())
                    .is_some_and(|(scope, path)| {
                        self.copier.reused_targets[&self.copier.reuse[&scope.key]][path]
                    })
            } else {
                child.is_instance || self.copier.reuse.contains_key(&authored(child))
            };
            if fresh
                && !(self.visitor)(Mapping {
                    source: &source,
                    target: target.as_deref(),
                    reused: prefix.substitution.as_ref().map(|scope| &scope.key),
                    parent: &prefix.parent,
                    instance: child.is_instance,
                    target_instance: instance,
                })?
            {
                return Ok(false);
            }
            let from = if child.is_instance {
                format!("{source}/")
            } else {
                prefix.from.clone()
            };
            let to = target.as_ref().map(|target| {
                if instance {
                    format!("{target}/")
                } else {
                    target
                        .rsplit_once('/')
                        .map_or_else(String::new, |(prefix, _)| format!("{prefix}/"))
                }
            });
            let substitution = if provenance == Provenance::Body {
                prefix.substitution.clone()
            } else {
                self.copier.root_reuse(child)?.map(|key| Substitution {
                    key,
                    from: from.clone(),
                    to: target.as_ref().map(|target| format!("{target}/")),
                })
            };
            if !self.children(
                child,
                Prefix {
                    parent: source,
                    from,
                    to,
                    substitution,
                },
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(super) fn walk(
    copier: &Copier<'_>,
    root: &ResolvedNode,
    visitor: &mut dyn FnMut(Mapping<'_>) -> Result<bool>,
) -> Result<()> {
    let substitution = copier.root_reuse(root)?.map(|key| Substitution {
        key,
        from: String::new(),
        to: Some(String::new()),
    });
    Walker {
        copier,
        visitor,
        seen: BTreeSet::new(),
        bytes: 0,
    }
    .children(
        root,
        Prefix {
            parent: String::new(),
            from: String::new(),
            to: Some(String::new()),
            substitution,
        },
    )?;
    Ok(())
}
