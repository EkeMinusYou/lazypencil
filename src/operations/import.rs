use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ComponentSource {
    Style {
        style_id: String,
        node_id: String,
        #[serde(default)]
        import_alias: Option<String>,
    },
    File {
        uri: String,
        node_id: String,
        import_alias: String,
    },
    Library {
        node_id: String,
        #[serde(default)]
        library_alias: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    Link,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum NameResolution {
    Rename { name: String },
    Reuse { target: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScriptAssetKind {
    Image,
    Shader,
    Script,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptAsset {
    pub uri: String,
    pub kind: ScriptAssetKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ImportOptions {
    pub variable_prefix: Option<String>,
    pub component_prefix: Option<String>,
    pub variables: BTreeMap<String, NameResolution>,
    pub components: BTreeMap<String, NameResolution>,
    pub themes: BTreeMap<String, NameResolution>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub script_assets: BTreeMap<String, Vec<ScriptAsset>>,
}

impl Context<'_> {
    fn record_source(&mut self, source: Project) -> Result<()> {
        crate::cancel::check(self.cancel)?;
        let identity = source.read_identity()?;
        for existing in &self.sources {
            if existing.read_identity()? == identity {
                return Ok(());
            }
        }
        if self
            .sources
            .iter()
            .map(Project::captured_bytes)
            .sum::<usize>()
            .saturating_add(source.captured_bytes())
            > 128 * 1024 * 1024
        {
            return Err(invalid(
                "Captured import sources exceed the 128 MiB batch limit",
            ));
        }
        self.sources.push(source);
        Ok(())
    }
    fn stage(&mut self, file: crate::managed::File, publish: bool) -> Result<()> {
        crate::cancel::check(self.cancel)?;
        self.overlay.insert(file.path.clone(), file.clone());
        if self
            .overlay
            .values()
            .map(|file| file.bytes.len())
            .sum::<usize>()
            > 128 * 1024 * 1024
        {
            return Err(invalid("Managed imports exceed the 128 MiB batch limit"));
        }
        if publish && !self.files.iter().any(|other| other.path == file.path) {
            self.files.push(file);
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_import(
    document: &mut Document,
    context: &mut Context<'_>,
    source: &ComponentSource,
    mode: ImportMode,
    options: &ImportOptions,
    expected: Option<&str>,
    parent: Option<&Target>,
    index: Option<usize>,
    bindings: &BTreeMap<String, String>,
) -> Result<(String, Value)> {
    crate::cancel::check(context.cancel)?;
    let mut source_document = document.clone();
    let mut file_source = None;
    let mut template_revision = None;
    let (alias, node_id) = match source {
        ComponentSource::Library {
            library_alias,
            node_id,
        } => (library_alias.clone(), node_id.as_str()),
        ComponentSource::File {
            uri,
            node_id,
            import_alias: alias,
        } => {
            import_alias(alias)?;
            file_source = Some((alias.clone(), uri.clone()));
            (Some(alias.clone()), node_id.as_str())
        }
        ComponentSource::Style {
            style_id,
            node_id,
            import_alias: requested,
        } => {
            let path = context
                .path
                .ok_or_else(|| invalid("Style imports require a bound document path"))?;
            if style_id.starts_with("project:") {
                let project = project_for(&source_document, context)?;
                let data = context
                    .data
                    .map(Path::to_path_buf)
                    .map(Ok)
                    .unwrap_or_else(crate::store::default_data_dir)?;
                let example = project
                    .design_catalog()?
                    .example_path(style_id, &data)?
                    .to_path_buf();
                let alias = requested
                    .clone()
                    .unwrap_or_else(|| format!("style-{}", style_id.replace(':', "-")));
                import_alias(&alias)?;
                file_source = Some((alias.clone(), relative_style_uri(path, &example)?));
                context.record_source(project)?;
                (Some(alias), node_id.as_str())
            } else {
                let style = crate::guidance::style_document(style_id)?;
                template_revision = Some(style.revision());
                let codec = document
                    .codec()
                    .ok_or_else(|| invalid("Style imports need a writable document schema"))?;
                let style = codec.encode_owned_template(&style)?;
                let file =
                    crate::managed::File::new(path, style.text().as_bytes().into(), "lib.pen")?;
                let alias = requested
                    .clone()
                    .unwrap_or_else(|| format!("style-{style_id}"));
                import_alias(&alias)?;
                if source_document.value().get("imports").is_none() {
                    source_document.set_property("", "imports", &json!({}))?;
                }
                if source_document.value()["imports"]
                    .get(&alias)
                    .is_some_and(|value| value != file.uri.as_str())
                {
                    return Err(invalid(
                        "The style import alias is already used; specify a different import_alias",
                    ));
                }
                source_document.set_property("/imports", &alias, &json!(file.uri))?;
                context.stage(file, matches!(mode, ImportMode::Link))?;
                (Some(alias), node_id.as_str())
            }
        }
    };
    let mut project = project_for(&source_document, context)?;
    if let Some((alias, uri)) = &file_source {
        project.attach_import_source(alias, uri)?;
    }
    let source_revision = match template_revision {
        Some(revision) => revision,
        None => project.library_document(alias.as_deref())?.revision(),
    };
    if expected.is_some_and(|expected| expected != source_revision) {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "The requested source component document changed",
        ));
    }
    let (component_id, mut result) = match mode {
        ImportMode::Link => {
            if options.variable_prefix.is_some()
                || options.component_prefix.is_some()
                || !options.variables.is_empty()
                || !options.components.is_empty()
                || !options.themes.is_empty()
                || !options.script_assets.is_empty()
            {
                return Err(invalid("Name resolutions are only valid for copy imports"));
            }
            let reference = project.component_reference(alias.as_deref(), node_id)?;
            if matches!(source, ComponentSource::Style { .. }) && file_source.is_none() {
                let alias = alias.as_deref().expect("style import alias");
                let uri = source_document.value()["imports"][alias].clone();
                if document.value().get("imports").is_none() {
                    document.set_property("", "imports", &json!({}))?;
                }
                if document.value()["imports"].get(alias) != Some(&uri) {
                    document.set_property("/imports", alias, &uri)?;
                }
            }
            if let Some((alias, uri)) = &file_source {
                if document.value().get("imports").is_none() {
                    document.set_property("", "imports", &json!({}))?;
                }
                document.set_property("/imports", alias, &json!(uri))?;
            }
            (reference, json!({"mode":"link"}))
        }
        ImportMode::Copy => {
            let mut plan =
                project.copy_component(alias.as_deref(), node_id, options, context.path)?;
            for file in plan.files.drain(..) {
                context.stage(file, true)?;
            }
            for (section, values) in [
                ("imports", &plan.imports),
                ("variables", &plan.variables),
                ("themes", &plan.themes),
            ] {
                if !values.is_empty() && document.value().get(section).is_none() {
                    document.set_property("", section, &json!({}))?;
                }
                for (name, value) in values {
                    document.set_property(&format!("/{section}"), name, value)?;
                }
            }
            if !plan.definitions.is_empty() {
                let container = json!({"id":format!("n{}",Uuid::new_v4().simple()),"type":"frame","name":"Imported components","enabled":false,"layout":"none","width":0,"height":0,"children":plan.definitions});
                let at = document.value()["children"]
                    .as_array()
                    .expect("children")
                    .len();
                document.insert_element("/children", at, &container.to_string())?;
            }
            (plan.component_id, plan.result)
        }
    };
    let id = format!("n{}", Uuid::new_v4().simple());
    let node = json!({"id":id,"type":"ref","ref":component_id});
    let (pointer, position) = destination(document, parent, index, bindings)?;
    document.insert_element(&pointer, position, &node.to_string())?;
    project.verify_unchanged()?;
    result["id"] = json!(id);
    result["component_id"] = json!(component_id);
    result["source_revision"] = json!(source_revision);
    context.record_source(project)?;
    Ok((id, result))
}

fn relative_style_uri(document: &Path, target: &Path) -> Result<String> {
    let parent = document
        .parent()
        .ok_or_else(|| invalid("Document has no style import directory"))?;
    for ancestor in parent.ancestors() {
        if let Ok(tail) = target.strip_prefix(ancestor) {
            let mut parts = parent
                .strip_prefix(ancestor)
                .expect("ancestor")
                .components()
                .map(|_| "..".to_string())
                .collect::<Vec<_>>();
            for part in tail.components() {
                parts.push(
                    part.as_os_str()
                        .to_str()
                        .ok_or_else(|| invalid("Style paths must be UTF-8"))?
                        .to_string(),
                );
            }
            return Ok(parts.join("/"));
        }
    }
    Err(invalid(
        "Project style and document must have a common project root",
    ))
}
