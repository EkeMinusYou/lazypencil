use super::*;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ValidationScope {
    #[default]
    Structure,
    Layout,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateRequest {
    #[serde(default)]
    pub scope: ValidationScope,
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub expected_dependency_revision: Option<String>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default = "hundred")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}

fn hundred() -> usize {
    100
}

impl Default for ValidateRequest {
    fn default() -> Self {
        Self {
            scope: ValidationScope::Structure,
            expected_revision: None,
            expected_dependency_revision: None,
            snapshot_id: None,
            limit: 100,
            cursor: None,
        }
    }
}

impl ValidateRequest {
    pub fn validate(&self) -> Result<()> {
        if !(1..=500).contains(&self.limit)
            || (self.scope == ValidationScope::Structure && self.snapshot_id.is_some())
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Validation limit must be 1..500; snapshot_id requires layout scope",
            ));
        }
        Ok(())
    }
}

pub fn validate_document_with_options(
    store: &Store,
    fonts: &[PathBuf],
    request: &ValidateRequest,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Value> {
    crate::cancel::check(cancel)?;
    request.validate()?;
    let result = match request.scope {
        ValidationScope::Structure => {
            let project = project_with_overlay_and_cancellation(
                store,
                store.read()?,
                fonts,
                &Default::default(),
                cancel,
            )?;
            let scene = project.resolve()?;
            validation_preconditions(&project, request)?;
            let result = structural_report(&project, &scene, request)?;
            store::require_revision(&store.read()?, &scene.revision)?;
            result
        }
        ValidationScope::Layout => {
            #[cfg(not(feature = "rendering"))]
            return Err(Error::new(
                "UNSUPPORTED_FEATURE",
                "Layout validation requires the rendering feature",
            ));
            #[cfg(feature = "rendering")]
            {
                if request.snapshot_id.is_some() {
                    return Err(Error::new(
                        "SNAPSHOT_UNAVAILABLE",
                        "CLI validation does not retain calculation snapshots; omit snapshot_id or use MCP",
                    ));
                }
                let mut runtime = crate::script::ScriptRuntime::current()?;
                runtime.set_cancellation(cancel.cloned());
                let (project, snapshot) = layout_snapshot_with_runtime(
                    store,
                    fonts,
                    request.expected_revision.as_deref(),
                    request.expected_dependency_revision.as_deref(),
                    &mut runtime,
                )?;
                validation_from_snapshot(&project, &snapshot, request)?
            }
        }
    };
    crate::cancel::check(cancel)?;
    if let Some(revision) = result["revision"].as_str() {
        store::require_revision(&store.read()?, revision)?;
    }
    crate::query::response(result)
}

fn validation_preconditions(project: &Project, request: &ValidateRequest) -> Result<()> {
    if let Some(revision) = &request.expected_revision {
        store::require_revision(project.root_document(), revision)?;
    }
    if request.expected_dependency_revision.is_some() {
        project.require_dependencies(request.expected_dependency_revision.as_deref())?;
    }
    Ok(())
}

fn structural_report(
    project: &Project,
    scene: &crate::project::ResolvedScene,
    request: &ValidateRequest,
) -> Result<Value> {
    let diagnostics = project.diagnostics();
    let identity = crate::document::hash(json!({"scope":request.scope,"revision":scene.revision,"dependency_revision":scene.dependency_revision}).to_string().as_bytes());
    let offset = crate::query::page_offset(
        &identity,
        request.cursor.as_deref(),
        request.limit,
        diagnostics.len(),
    )?;
    let page = diagnostics
        .iter()
        .skip(offset)
        .take(request.limit)
        .collect::<Vec<_>>();
    let mut result = json!({"valid":true,"complete":diagnostics.is_empty(),"revision":scene.revision,
        "dependency_revision":scene.dependency_revision,"dependencies":scene.dependencies,
        "node_count":project.root_document().node_count(),"diagnostics":page,"total_diagnostics":diagnostics.len(),
        "next_cursor":(offset + page.len() < diagnostics.len()).then(|| format!("{identity}:{}",offset + page.len())),
        "validated_scopes":["document_structure","references","variables","themes"]});
    asset_status(&mut result, project);
    project.verify_unchanged()?;
    let result = crate::query::response(result)?;
    if !diagnostics.is_empty() {
        return Err(Error::new(
            "UNSUPPORTED_FEATURE",
            "The document includes unrecognized node types",
        )
        .details(result));
    }
    Ok(result)
}

#[cfg(feature = "rendering")]
pub fn validation_from_snapshot(
    project: &Project,
    snapshot: &crate::layout::LayoutSnapshot,
    request: &ValidateRequest,
) -> Result<Value> {
    request.validate()?;
    if request.scope != ValidationScope::Layout {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "A calculation snapshot requires layout validation scope",
        ));
    }
    if request
        .snapshot_id
        .as_deref()
        .is_some_and(|id| id != snapshot.id)
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "The requested calculation does not match this snapshot",
        ));
    }
    validation_preconditions(project, request)?;
    if project.root_document().revision() != snapshot.revision
        || project.dependency_revision() != snapshot.dependency_revision
    {
        return Err(Error::new(
            "REVISION_CONFLICT",
            "The calculation does not belong to this project snapshot",
        ));
    }
    // Keep the structural scope honest even when a layout engine can measure an unknown node.
    let diagnostics = project.diagnostics();
    if !diagnostics.is_empty() {
        let scene = project.resolve()?;
        structural_report(project, &scene, request)?;
    }
    let diagnostics = snapshot
        .items
        .iter()
        .flat_map(|item| &item.record.diagnostics)
        .collect::<Vec<_>>();
    let identity = crate::document::hash(
        json!({"scope":request.scope,"snapshot_id":snapshot.id})
            .to_string()
            .as_bytes(),
    );
    let offset = crate::query::page_offset(
        &identity,
        request.cursor.as_deref(),
        request.limit,
        diagnostics.len(),
    )?;
    let page = diagnostics
        .iter()
        .skip(offset)
        .take(request.limit)
        .collect::<Vec<_>>();
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "warning")
        .count();
    let mut result = snapshot.metadata();
    result["valid"] = json!(errors == 0);
    result["complete"] = json!(true);
    result["validated_scopes"] = json!([
        "document_structure",
        "references",
        "variables",
        "themes",
        "layout"
    ]);
    result["diagnostics"] = json!(page);
    result["total_diagnostics"] = json!(diagnostics.len());
    result["error_count"] = json!(errors);
    result["warning_count"] = json!(warnings);
    result["next_cursor"] = json!(
        (offset + page.len() < diagnostics.len())
            .then(|| format!("{identity}:{}", offset + page.len()))
    );
    asset_status(&mut result, project);
    project.verify_unchanged()?;
    crate::query::response(result)
}
