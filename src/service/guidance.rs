use crate::{
    Result,
    project::Project,
    store::{self, Store},
};
use serde_json::{Value, json};

pub fn project_guidance(store: &Store, id: Option<&str>) -> Result<Value> {
    read(store, |project| {
        project.design_catalog()?.guidance(id, store.data_dir())
    })
}
pub fn project_style(store: &Store, id: &str) -> Result<Value> {
    read(store, |project| {
        project.design_catalog()?.style(id, store.data_dir())
    })
}
pub fn project_styles(
    store: &Store,
    use_case: Option<&str>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<Value> {
    read(store, |project| {
        project
            .design_catalog()?
            .styles(use_case, limit, cursor, store.data_dir())
    })
}
pub fn project_design_start(store: &Store, style: Option<&str>) -> Result<(Value, Option<Value>)> {
    let project = Project::load(store)?;
    let catalog = project.design_catalog()?;
    let guidance = catalog.guidance(None, store.data_dir())?;
    let style = style
        .or_else(|| catalog.default_style())
        .map(|id| catalog.style(id, store.data_dir()))
        .transpose()?;
    project.verify_unchanged()?;
    store::require_revision(&store.read()?, &project.root_document().revision())?;
    Ok((guidance, style))
}
fn read(store: &Store, f: impl FnOnce(&Project) -> Result<Value>) -> Result<Value> {
    let project = Project::load(store)?;
    let mut value = f(&project)?;
    project.verify_unchanged()?;
    store::require_revision(&store.read()?, &project.root_document().revision())?;
    if project.design_catalog()?.configured() {
        value["revision"] = json!(project.root_document().revision());
        value["dependency_revision"] = json!(project.dependency_revision());
    }
    crate::query::response(value)
}
