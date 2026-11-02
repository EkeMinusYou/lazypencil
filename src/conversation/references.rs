//! Explicit, bounded node references captured by a frontend and checked before a turn.
use crate::{
    Error, Result,
    project::{NodeAddress, Project, ResolvedNode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) const LIMIT: usize = 16;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Reference {
    pub target: Value,
    pub name: String,
    pub kind: String,
    pub text_excerpt: String,
    pub revision: String,
    pub dependency_revision: String,
}

pub(crate) fn target(node: &ResolvedNode) -> Value {
    json!({"id":node.id(),"address":node.address,"source_id":node.source_id,
        "source_path":node.source_path,"generated_by":node.generated_by})
}

#[cfg(feature = "tui")]
pub(crate) fn selection(references: &[Reference]) -> Option<Value> {
    (!references.is_empty()).then(|| json!({"node_references":references}))
}

pub(crate) fn labels(references: &[Reference]) -> String {
    let clipped = |value: &str, limit: usize| {
        let value = single_line(value);
        let mut label = value.chars().take(limit).collect::<String>();
        if value.chars().count() > limit {
            label.push('…');
        }
        label
    };
    references
        .iter()
        .map(|reference| {
            format!(
                "{} ({}) [{}]",
                clipped(&reference.name, 96),
                clipped(&reference.kind, 32),
                clipped(reference.target["id"].as_str().unwrap_or(""), 96)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn single_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(256).collect()
}

pub(crate) fn decode(selection: Option<&Value>) -> Result<Vec<Reference>> {
    let Some(value) = selection.and_then(|selection| selection.get("node_references")) else {
        return Ok(Vec::new());
    };
    if value.to_string().len() > 64 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Node references exceed 64 KiB",
        ));
    }
    let references: Vec<Reference> = serde_json::from_value(value.clone())
        .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid node references"))?;
    if references.len() > LIMIT {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "At most 16 node references are allowed",
        ));
    }
    Ok(references)
}

pub(crate) fn validate(
    project: &mut Project,
    selection: Option<&Value>,
    worker: &Path,
    font_files: &[std::path::PathBuf],
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    let references = decode(selection)?;
    if references.is_empty() {
        return Ok(());
    }
    project.add_font_files(font_files)?;
    let unavailable = |reference: &Reference| {
        Error::new(
            "NODE_NOT_FOUND",
            format!(
                "Referenced node is unavailable: {}. Remove it or select it again.",
                single_line(&reference.name)
            ),
        )
    };
    #[cfg(feature = "rendering")]
    let calculated = if references
        .iter()
        .any(|reference| !reference.target["generated_by"].is_null())
    {
        let mut runtime = crate::script::ScriptRuntime::new(worker);
        runtime.set_cancellation(cancel.cloned());
        let mut fonts = project.load_fonts()?;
        Some(crate::script::compute(project, &mut fonts, &mut runtime)?)
    } else {
        None
    };
    #[cfg(not(feature = "rendering"))]
    let _ = worker;
    for reference in &references {
        crate::cancel::check(cancel)?;
        let address = &reference.target["address"];
        let address = NodeAddress {
            node_id: address["node_id"]
                .as_str()
                .ok_or_else(|| unavailable(reference))?
                .into(),
            instance_path: serde_json::from_value(address["instance_path"].clone())
                .map_err(|_| unavailable(reference))?,
        };
        if reference.target["generated_by"].is_null() {
            let node = project.expanded_node(&address).map_err(|error| {
                if error.code == "CANCELLED" {
                    error
                } else {
                    unavailable(reference)
                }
            })?;
            if target(&node) != reference.target {
                return Err(unavailable(reference));
            }
        } else {
            #[cfg(feature = "rendering")]
            if !calculated.as_ref().is_some_and(|snapshot| {
                snapshot
                    .items
                    .iter()
                    .any(|item| target(&item.node) == reference.target)
            }) {
                return Err(unavailable(reference));
            }
            #[cfg(not(feature = "rendering"))]
            return Err(unavailable(reference));
        }
    }
    Ok(())
}
