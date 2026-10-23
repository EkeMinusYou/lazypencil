//! Bounded code retained after a failed calculation, without its fonts or scene.

use super::{Asset, Dependency, MAX_ASSET_BYTES, Project};
use crate::{Error, Result, assets::ReadAssetRequest, document::hash, script::MAX_CODE_BYTES};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};

const MAX_GUARDS: usize = 4096;
const MAX_GUARD_PATH_BYTES: usize = 1024 * 1024;

pub(crate) struct FailedCodeCapture {
    revision: String,
    dependency_revision: String,
    asset: Asset,
    dependencies: Vec<Dependency>,
    bindings: Vec<(PathBuf, PathBuf)>,
    id: String,
}

impl Project {
    /// The diagnostic identifies an asset captured by execution, never a caller's path.
    pub(crate) fn capture_failed_code(
        &self,
        declared: &[Dependency],
        error: &Error,
    ) -> Option<FailedCodeCapture> {
        if error.code != "SCRIPT_EXECUTION_FAILED" || declared.len() > MAX_GUARDS {
            return None;
        }
        let details = error.details.as_ref()?;
        let path = PathBuf::from(details["script_path"].as_str()?);
        let source_hash = details["source_hash"].as_str()?;
        if !self.runtime_assets.contains(&path) {
            return None;
        }
        let asset = self.assets.get(&path)?;
        if asset.kind != "script"
            || asset.revision != source_hash
            || asset.bytes.len() > MAX_CODE_BYTES
        {
            return None;
        }
        let original: BTreeSet<_> = declared
            .iter()
            .map(|d| (&d.path, &d.kind, &d.revision))
            .collect();
        let dependencies = bounded_dependencies(
            self.dependencies()
                .into_iter()
                .filter(|d| !original.contains(&(&d.path, &d.kind, &d.revision)))
                .chain(self.runtime_dependencies()),
        )?;
        if dependencies
            .iter()
            .any(|d| !matches!(d.kind.as_str(), "document" | "script" | "shader" | "image"))
        {
            return None;
        }
        let bindings = {
            let bindings = self.file_bindings.lock().ok()?;
            if bindings.len() > MAX_GUARDS {
                return None;
            }
            let path_bytes = bindings
                .iter()
                .map(|(a, b)| a.as_os_str().len() + b.as_os_str().len())
                .sum::<usize>()
                + dependencies
                    .iter()
                    .map(|d| d.path.as_os_str().len())
                    .sum::<usize>();
            if path_bytes > MAX_GUARD_PATH_BYTES {
                return None;
            }
            bindings
                .iter()
                .map(|(a, b)| (a.clone(), b.clone()))
                .collect::<Vec<_>>()
        };
        if self.verify_unchanged().is_err() {
            return None;
        }
        let revision = self.root_document().revision();
        let dependency_revision = hash(&serde_json::to_vec(declared).ok()?);
        let id = hash(&serde_json::to_vec(&json!({"revision":revision,"dependency_revision":dependency_revision,"runtime":dependencies,"bindings":bindings,"source_hash":source_hash})).ok()?);
        Some(FailedCodeCapture {
            revision,
            dependency_revision,
            asset: Asset {
                bytes: asset.bytes.clone(),
                kind: asset.kind.clone(),
                revision: asset.revision.clone(),
            },
            dependencies,
            bindings,
            id,
        })
    }
}

fn bounded_dependencies(dependencies: impl Iterator<Item = Dependency>) -> Option<Vec<Dependency>> {
    let mut result = dependencies.take(MAX_GUARDS + 1).collect::<Vec<_>>();
    // Check before deduplicating: duplicates must never hide a truncated tail.
    if result.len() > MAX_GUARDS {
        return None;
    }
    result.sort_by(|a, b| (&a.path, &a.kind).cmp(&(&b.path, &b.kind)));
    result.dedup_by(|a, b| a.path == b.path && a.kind == b.kind && a.revision == b.revision);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_cannot_hide_an_over_budget_guard_tail() {
        let input = || {
            (0..MAX_GUARDS + 2).map(|index| Dependency {
                path: PathBuf::from(if index == MAX_GUARDS + 1 {
                    "unseen-tail.js"
                } else {
                    "repeated.js"
                }),
                revision: "same captured revision".into(),
                kind: "script".into(),
                version: None,
                availability: None,
            })
        };
        assert!(bounded_dependencies(input()).is_none());
        let small = bounded_dependencies(input().take(2)).unwrap();
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].path, PathBuf::from("repeated.js"));
    }
}

impl FailedCodeCapture {
    pub(crate) fn hash(&self) -> &str {
        &self.asset.revision
    }
    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn read(&self, project: &Project, request: &ReadAssetRequest) -> Result<Value> {
        let conflict = || {
            Error::new(
                "REVISION_CONFLICT",
                "Dependencies of the failed calculation changed; recalculate before reading its code",
            )
        };
        if project.root_document().revision() != self.revision
            || project.dependency_revision() != self.dependency_revision
        {
            return Err(conflict());
        }
        for (requested, expected) in &self.bindings {
            project.check_cancelled()?;
            if !crate::paths::canonical(requested).is_ok_and(|actual| actual == *expected) {
                return Err(conflict());
            }
        }
        for dependency in &self.dependencies {
            project.check_cancelled()?;
            let actual = crate::store::content::file_hash(
                &dependency.path,
                MAX_ASSET_BYTES,
                project.cancel.as_ref(),
            )
            .map_err(|error| {
                if error.code == "CANCELLED" {
                    error
                } else {
                    conflict()
                }
            })?;
            if actual != dependency.revision {
                return Err(conflict());
            }
        }
        project.verify_unchanged()?;
        let mut value = crate::assets::read_code(
            &self.asset,
            "script",
            request,
            &self.revision,
            &self.dependency_revision,
        )?;
        value["failed_calculation"] = json!(true);
        value["failure_capture_id"] = json!(self.id);
        Ok(value)
    }
}
