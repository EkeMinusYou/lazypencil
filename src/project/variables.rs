use super::*;

type Theme = BTreeMap<String, String>;
type VariableContext = (PathBuf, Theme);
type NodeContext<'a> = (&'a ResolvedNode, Theme);

#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariablesRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub library_alias: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub instance_path: Vec<String>,
    /// Select the namespace of a top-level node property, including instance overrides.
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub theme: BTreeMap<String, String>,
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub expected_dependency_revision: Option<String>,
    #[serde(default = "hundred")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}

fn hundred() -> usize {
    100
}

impl Project {
    pub fn variables(&self, request: &VariablesRequest) -> Result<Value> {
        self.check_cancelled()?;
        request.validate()?;
        if let Some(expected) = &request.expected_revision {
            crate::store::require_revision(self.root_document(), expected)?;
        }
        if request.expected_dependency_revision.is_some() {
            self.require_dependencies(request.expected_dependency_revision.as_deref())?;
        }
        let (scope, mut theme) = self.variable_context(request)?;
        for (axis, value) in &request.theme {
            self.check_cancelled()?;
            let declared = self.documents.values().any(|document| {
                document
                    .value()
                    .get("themes")
                    .and_then(|themes| themes.get(axis))
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.iter().any(|entry| entry.as_str() == Some(value)))
            });
            if !declared {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "A theme override must select a declared axis and value",
                )
                .details(json!({"axis":axis,"value":value})));
            }
            theme.insert(axis.clone(), value.clone());
        }
        let definitions = self.documents[&scope]
            .value()
            .get("variables")
            .map(|value| {
                value
                    .as_object()
                    .ok_or_else(|| invalid("variables must be an object"))
            })
            .transpose()?;
        let names = if let Some(name) = &request.name {
            vec![name.clone()]
        } else {
            let mut names = definitions
                .into_iter()
                .flat_map(|values| values.keys())
                .cloned()
                .collect::<Vec<_>>();
            names.sort();
            names
        };
        let mut query = serde_json::to_value(request).expect("serializable query");
        for key in [
            "cursor",
            "limit",
            "expected_revision",
            "expected_dependency_revision",
        ] {
            query.as_object_mut().expect("object").remove(key);
        }
        let identity = hash(json!({"revision":self.root_document().revision(),"dependency_revision":self.dependency_revision(),"scope":scope,"theme":theme,"query":query}).to_string().as_bytes());
        let offset = crate::query::page_offset(
            &identity,
            request.cursor.as_deref(),
            request.limit,
            names.len(),
        )?;
        let mut variables = Map::new();
        let mut values = Map::new();
        let mut sources = Map::new();
        let mut diagnostics = Vec::new();
        for name in names.iter().skip(offset).take(request.limit) {
            self.check_cancelled()?;
            let (source, definition) = self.lookup_variable(&scope, name)?;
            variables.insert(name.clone(), definition);
            let mut value_source = source.clone();
            match self.variable_with_file_origin(
                &scope,
                name,
                &theme,
                &mut Vec::new(),
                Some(&mut value_source),
            ) {
                Ok(value) => {
                    values.insert(name.clone(), value);
                    sources.insert(
                        name.clone(),
                        json!({"definition":source,"value":value_source}),
                    );
                }
                Err(error) if matches!(error.code, "CANCELLED" | "REVISION_CONFLICT") => {
                    return Err(error);
                }
                Err(error) => {
                    sources.insert(name.clone(), json!({"definition":source}));
                    diagnostics.push(json!({"variable":name,"severity":"error","code":error.code,"message":error.message,"details":error.details}));
                }
            }
        }
        let end = offset + variables.len();
        let result = json!({"revision":self.root_document().revision(),"dependency_revision":self.dependency_revision(),
            "scope_document":scope,"library_alias":request.library_alias,"node_id":request.node_id,"instance_path":request.instance_path,"property":request.property,
            "variables":variables,"resolved_values":values,"sources":sources,"diagnostics":diagnostics,"resolution_complete":diagnostics.is_empty(),
            "themes":self.documents[&scope].value().get("themes").cloned().unwrap_or(json!({})),"effective_theme":theme,
            "total_matches":names.len(),"next_cursor":(end < names.len()).then(|| format!("{identity}:{end}"))});
        self.verify_unchanged()?;
        crate::query::response(result)
    }

    fn variable_context(&self, request: &VariablesRequest) -> Result<VariableContext> {
        let scope = self.library_path(request.library_alias.as_deref())?;
        let default = self.default_theme(scope)?;
        let Some(id) = &request.node_id else {
            return Ok((scope.clone(), default));
        };
        let document = &self.documents[scope];
        let mut root_id = id.as_str();
        while let Some(parent) = &document
            .location(root_id)
            .map_err(|_| {
                Error::new(
                    "INVALID_ARGUMENT",
                    "The selected stored node does not exist",
                )
            })?
            .parent_id
        {
            self.check_cancelled()?;
            root_id = parent;
        }
        // Only expand the selected stored ancestor, so an unrelated broken root does
        // not prevent token inspection. Preserve ancestor overrides and theme context.
        let mut root = self.expand(
            scope,
            document.node(root_id)?,
            None,
            &[],
            &mut Vec::new(),
            &mut 0,
        )?;
        assign_addresses(&mut root, None, &[]);
        fn find<'a>(
            project: &Project,
            node: &'a ResolvedNode,
            inherited: &Theme,
            matches: &impl Fn(&ResolvedNode) -> bool,
        ) -> Result<Option<NodeContext<'a>>> {
            let theme = project.effective_theme(node, inherited)?;
            if matches(node) {
                return Ok(Some((node, theme)));
            }
            for child in &node.children {
                if let Some(found) = find(project, child, &theme, matches)? {
                    return Ok(Some(found));
                }
            }
            Ok(None)
        }
        let matches = |node: &ResolvedNode| {
            (node.address.node_id == *id && node.address.instance_path.is_empty())
                || (node.source_path == *scope && node.source_id == *id)
                || node
                    .instance_identity
                    .as_ref()
                    .is_some_and(|(origin, name)| origin == scope && name == id)
        };
        let (mut node, mut theme) = find(self, &root, &default, &matches)?.ok_or_else(|| {
            Error::new(
                "INVALID_ARGUMENT",
                "The selected node is not present in the effective tree",
            )
        })?;
        if !request.instance_path.is_empty() {
            if !node.is_instance {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "An inherited path requires an instance",
                ));
            }
            let mut path = node.address.instance_path.clone();
            path.extend(request.instance_path.iter().cloned());
            let matches = |child: &ResolvedNode| {
                child.address.node_id == node.address.node_id && child.address.instance_path == path
            };
            let mut found = None;
            for child in &node.children {
                if let Some(context) = find(self, child, &theme, &matches)? {
                    found = Some(context);
                    break;
                }
            }
            let found = found.ok_or_else(|| {
                Error::new(
                    "INVALID_ARGUMENT",
                    "The selected inherited node does not exist",
                )
            })?;
            (node, theme) = found;
        }
        let scope = if let Some(property) = &request.property {
            if !node.properties.contains_key(property) {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "The selected node does not have this property",
                ));
            }
            node.property_source(property)
        } else {
            &node.source_path
        };
        Ok((scope.to_path_buf(), theme))
    }
}

impl VariablesRequest {
    pub(crate) fn validate(&self) -> Result<()> {
        if !(1..=500).contains(&self.limit)
            || self.instance_path.len() > 128
            || self
                .instance_path
                .iter()
                .any(|part| part.is_empty() || part.contains('/') || part.len() > 4096)
            || (self.node_id.is_none()
                && (!self.instance_path.is_empty() || self.property.is_some()))
            || [
                &self.name,
                &self.node_id,
                &self.library_alias,
                &self.property,
            ]
            .iter()
            .filter_map(|value| value.as_ref())
            .any(|value| value.is_empty() || value.len() > 4096)
            || self.theme.len() > 128
            || self.theme.iter().any(|(axis, value)| {
                axis.is_empty() || value.is_empty() || axis.len() > 4096 || value.len() > 4096
            })
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Invalid variable query, node address, theme, or page limit",
            ));
        }
        Ok(())
    }
}
