//! Resolve components and variables through the same declared import graph.

use super::*;

#[derive(Clone, Copy)]
enum Member {
    Component,
    Variable,
}

impl Member {
    fn get<'a>(self, document: &'a Document, name: &str) -> Option<&'a Value> {
        match self {
            Self::Component => document.node(name).ok(),
            Self::Variable => document.value().get("variables").and_then(|v| v.get(name)),
        }
    }

    fn missing(self, name: &str) -> Error {
        match self {
            Self::Component => Error::new(
                "ASSET_NOT_FOUND",
                format!("Component {name:?} does not exist in this document or its imports"),
            ),
            Self::Variable => invalid(format!("Missing variable ${name}")),
        }
    }

    fn ambiguous(self, name: &str) -> Error {
        match self {
            Self::Component => invalid(format!(
                "Ambiguous imported component {name:?}; specify its library alias"
            )),
            Self::Variable => invalid(format!(
                "Ambiguous imported variable ${name}; specify its library alias"
            )),
        }
    }
}

impl Project {
    pub(super) fn lookup_node(&self, origin: &Path, name: &str) -> Result<(PathBuf, Value)> {
        self.lookup_node_ref(origin, name)
            .map(|(path, node)| (path, node.clone()))
    }

    pub(super) fn lookup_node_ref(&self, origin: &Path, name: &str) -> Result<(PathBuf, &Value)> {
        self.lookup_member(origin, name, Member::Component)
    }

    pub(super) fn lookup_variable(&self, origin: &Path, name: &str) -> Result<(PathBuf, Value)> {
        self.lookup_variable_ref(origin, name)
            .map(|(path, value)| (path, value.clone()))
    }

    pub(super) fn lookup_variable_ref(
        &self,
        origin: &Path,
        name: &str,
    ) -> Result<(PathBuf, &Value)> {
        self.lookup_member(origin, name, Member::Variable)
    }

    fn lookup_member(
        &self,
        origin: &Path,
        name: &str,
        member: Member,
    ) -> Result<(PathBuf, &Value)> {
        if matches!(member, Member::Component)
            && let Some((path, id)) = self.runtime_refs.get(name)
        {
            return Ok((path.clone(), self.documents[path].node(id)?));
        }
        if let Some(value) = member.get(&self.documents[origin], name) {
            return Ok((origin.into(), value));
        }
        if let Some((alias, name)) = name.split_once(':')
            && let Some(path) = self.imported_path(origin, alias)?
        {
            return self.lookup_member(path, name, member);
        }
        let mut found = None;
        // Two aliases for one document do not introduce an ambiguity.
        for path in self
            .imports
            .get(origin)
            .into_iter()
            .flat_map(BTreeMap::values)
            .collect::<BTreeSet<_>>()
        {
            if let Some(value) = member.get(&self.documents[path], name) {
                if found.is_some() {
                    return Err(member.ambiguous(name));
                }
                found = Some((path.clone(), value));
            }
        }
        found.ok_or_else(|| member.missing(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(color: &str) -> Document {
        Document::parse(
            json!({"version":"2.21","variables":{"tone":{"type":"color","value":color}},
            "children":[{"id":"badge","type":"rectangle","fill":color}]})
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn aliases_share_document_identity_but_distinct_sources_remain_ambiguous() {
        let mut project = Project::local(Document::empty()).unwrap();
        let first = PathBuf::from("first.pen");
        let second = PathBuf::from("second.pen");
        project.documents.insert(first.clone(), document("#ff0000"));
        project
            .documents
            .insert(second.clone(), document("#0000ff"));
        project.imports.insert(first.clone(), BTreeMap::new());
        project.imports.insert(second.clone(), BTreeMap::new());
        project
            .imports
            .get_mut(&project.root)
            .unwrap()
            .extend([("a".into(), first.clone()), ("same".into(), first.clone())]);
        assert_eq!(
            project.lookup_node_ref(&project.root, "badge").unwrap().0,
            first
        );
        assert_eq!(
            project
                .lookup_variable_ref(&project.root, "tone")
                .unwrap()
                .0,
            first
        );
        project
            .imports
            .get_mut(&project.root)
            .unwrap()
            .insert("b".into(), second.clone());
        assert!(
            project
                .lookup_node_ref(&project.root, "badge")
                .unwrap_err()
                .message
                .contains("Ambiguous")
        );
        assert!(
            project
                .lookup_variable_ref(&project.root, "tone")
                .unwrap_err()
                .message
                .contains("Ambiguous")
        );
        assert_eq!(
            project.lookup_node_ref(&project.root, "b:badge").unwrap().0,
            second
        );
        assert_eq!(
            project
                .lookup_variable_ref(&project.root, "b:tone")
                .unwrap()
                .0,
            second
        );
    }

    #[test]
    fn local_qualified_names_and_runtime_components_keep_their_precedence() {
        let mut project = Project::local(
            Document::parse(
                json!({"version":"2.21",
            "variables":{"a:tone":{"type":"color","value":"#00ff00"}},
            "children":[{"id":"a:badge","type":"rectangle"}]})
                .to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        let source = PathBuf::from("source.pen");
        project
            .documents
            .insert(source.clone(), document("#ff0000"));
        project.imports.insert(source.clone(), BTreeMap::new());
        project
            .imports
            .get_mut(&project.root)
            .unwrap()
            .insert("a".into(), source.clone());
        assert_eq!(
            project.lookup_node_ref(&project.root, "a:badge").unwrap().0,
            project.root
        );
        assert_eq!(
            project
                .lookup_variable_ref(&project.root, "a:tone")
                .unwrap()
                .0,
            project.root
        );
        project
            .runtime_refs
            .insert("a:badge".into(), (source.clone(), "badge".into()));
        assert_eq!(
            project.lookup_node_ref(&project.root, "a:badge").unwrap().0,
            source
        );
        assert_eq!(
            project
                .lookup_variable_ref(&project.root, "a:tone")
                .unwrap()
                .0,
            project.root
        );
    }
}
