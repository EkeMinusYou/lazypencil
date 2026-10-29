use crate::{
    Result,
    project::Project,
    store::{self, Store},
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

pub fn project_guidance(store: &Store, id: Option<&str>) -> Result<Value> {
    project_guidance_with_cancellation(store, id, None)
}
pub fn project_guidance_with_cancellation(
    store: &Store,
    id: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    read(store, cancel, |project| {
        project.design_catalog()?.guidance(id, store.data_dir())
    })
}
pub fn project_style(store: &Store, id: &str) -> Result<Value> {
    project_style_with_cancellation(store, id, None)
}
pub fn project_style_with_cancellation(
    store: &Store,
    id: &str,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    read(store, cancel, |project| {
        project.design_catalog()?.style(id, store.data_dir())
    })
}
pub fn project_styles(
    store: &Store,
    use_case: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    project_styles_with_cancellation(store, use_case, limit, cursor, None)
}
pub fn project_styles_with_cancellation(
    store: &Store,
    use_case: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<Value> {
    read(store, cancel, |project| {
        project
            .design_catalog()?
            .styles(use_case, limit, cursor, store.data_dir())
    })
}
pub fn project_design_start(store: &Store, style: Option<&str>) -> Result<(Value, Option<Value>)> {
    prepare_design_start(store, style, None, None)?.finish(store)
}

pub(crate) struct PreparedDesignStart {
    project: Project,
    guidance: Value,
    style: Option<Value>,
}

impl PreparedDesignStart {
    pub(crate) fn finish(self, store: &Store) -> Result<(Value, Option<Value>)> {
        self.project.verify_unchanged()?;
        store::require_revision(&store.read()?, &self.project.root_document().revision())?;
        Ok((self.guidance, self.style))
    }
}

pub(crate) fn prepare_design_start(
    store: &Store,
    style: Option<&str>,
    worker: Option<&std::path::Path>,
    cancel: Option<&CancellationToken>,
) -> Result<PreparedDesignStart> {
    let project = match worker {
        Some(worker) => Project::load_with_remote_worker(store, worker, cancel)?,
        None => Project::load_with_cancellation(store, cancel)?,
    };
    let catalog = project.design_catalog()?;
    let guidance = catalog.guidance(None, store.data_dir())?;
    let style = style
        .or_else(|| catalog.default_style())
        .map(|id| catalog.style(id, store.data_dir()))
        .transpose()?;
    project.verify_unchanged()?;
    store::require_revision(&store.read()?, &project.root_document().revision())?;
    crate::cancel::check(cancel)?;
    Ok(PreparedDesignStart {
        project,
        guidance,
        style,
    })
}
fn read(
    store: &Store,
    cancel: Option<&CancellationToken>,
    f: impl FnOnce(&Project) -> Result<Value>,
) -> Result<Value> {
    let project = Project::load_with_cancellation(store, cancel)?;
    let mut value = f(&project)?;
    project.verify_unchanged()?;
    store::require_revision(&store.read()?, &project.root_document().revision())?;
    if project.design_catalog()?.configured() {
        value["revision"] = json!(project.root_document().revision());
        value["dependency_revision"] = json!(project.dependency_revision());
    }
    crate::query::response(value)
}
