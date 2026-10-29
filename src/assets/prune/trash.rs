//! The journal stays outside the payload throughout partial bundle cleanup.

use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod native;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    version: u32,
    operation: String,
    pub root: PathBuf,
    pub document: PathBuf,
    pub asset: Candidate,
}

impl Journal {
    pub fn directory(&self) -> PathBuf {
        self.asset
            .path
            .parent()
            .expect("validated asset parent")
            .join(format!("{TRASH}{}", self.operation))
    }
    fn payload(&self) -> Candidate {
        let mut asset = self.asset.clone();
        asset.path = self.directory().join("payload");
        asset
    }
    fn validate(&self, directory: &Path) -> Result<()> {
        if self.version != 1
            || uuid::Uuid::parse_str(&self.operation).is_err()
            || self.operation
                != uuid::Uuid::parse_str(&self.operation)
                    .map_err(|_| changed())?
                    .to_string()
            || !self.root.is_absolute()
            || !self.document.starts_with(&self.root)
            || !self.asset.path.starts_with(&self.root)
            || self
                .asset
                .path
                .parent()
                .is_none_or(|parent| !parent.ends_with("assets/lazypencil"))
            || self
                .asset
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(digest_name)
                != Some(&self.asset.digest)
            || crate::managed::normalize(&self.asset.path)? != self.asset.path
            || self.asset.bundle != self.asset.path.to_string_lossy().ends_with(".bundle")
            || self.directory() != directory
        {
            return Err(Error::new(
                "RECOVERY_REQUIRED",
                "Invalid asset deletion journal identity",
            ));
        }
        if self.asset.bundle {
            let mut files = self.asset.files.clone();
            let index = files.remove("bundle.json").ok_or_else(changed)?;
            if files.is_empty()
                || files.len() > 4097
                || files.values().any(|file| file.bytes > MAX_FILE as u64)
                || files.values().map(|file| file.bytes).sum::<u64>() > 128 * 1024 * 1024
                || files.keys().map(String::len).sum::<usize>() > 1024 * 1024
            {
                return Err(changed());
            }
            crate::managed::validate_bundle_names(files.keys().map(String::as_str), None)?;
            let bytes =
                serde_json::to_vec(&json!({"version":1,"files":files})).map_err(|_| changed())?;
            if index != Content::of(&bytes) || index.sha256 != self.asset.digest {
                return Err(changed());
            }
        } else if self.asset.files.len() != 1
            || self.asset.files.get("").is_none_or(|file| {
                file.sha256 != self.asset.digest || file.bytes == 0 || file.bytes > MAX_FILE as u64
            })
        {
            return Err(changed());
        }
        Ok(())
    }
}

pub(super) fn load(directory: &Path, budget: &mut Budget<'_>) -> Result<Journal> {
    let entries = filesystem::entries(directory, 3, budget.cancel)?;
    budget.entries(entries.len())?;
    if entries.iter().any(|entry| {
        !matches!(entry.name.as_str(), "journal.json" | "payload")
            || entry.kind == filesystem::Kind::Link
            || entry.name == "journal.json" && entry.kind != filesystem::Kind::File
    }) {
        return Err(Error::new(
            "RECOVERY_REQUIRED",
            "Unknown files in an asset deletion journal",
        ));
    }
    let journal: Journal =
        serde_json::from_slice(&budget.read(&directory.join("journal.json"), 4 * 1024 * 1024)?)
            .map_err(|_| Error::new("RECOVERY_REQUIRED", "Cannot read asset deletion journal"))?;
    journal.validate(directory)?;
    Ok(journal)
}

pub(super) fn quarantine(
    asset: &Candidate,
    root: &Path,
    document: &Path,
    budget: &mut Budget<'_>,
) -> Result<Journal> {
    let journal = Journal {
        version: 1,
        operation: uuid::Uuid::new_v4().to_string(),
        root: root.into(),
        document: document.into(),
        asset: asset.clone(),
    };
    journal.validate(&journal.directory())?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        native::quarantine(&journal, budget)?;
        Ok(journal)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = budget;
        Err(unavailable())
    }
}

pub(super) fn finish(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        native::finish(journal, budget)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (journal, budget);
        Err(unavailable())
    }
}

pub(super) fn restore(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        native::restore(journal, budget)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (journal, budget);
        Err(unavailable())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unavailable() -> Error {
    Error::new(
        "RUNTIME_UNAVAILABLE",
        "Guarded asset maintenance is unavailable on this platform",
    )
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;
