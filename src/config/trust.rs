//! Purpose-specific local grants, never created through MCP.
use crate::{Error, Result, document::hash};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    DesignInstructions,
    AgentExecution,
}
impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::DesignInstructions => "design_instructions",
            Self::AgentExecution => "agent_execution",
        }
    }
    fn directory(self) -> &'static str {
        match self {
            Self::DesignInstructions => "trusted-projects",
            Self::AgentExecution => "trusted-agent-projects",
        }
    }
}

pub(crate) fn path(data: &Path, root: &Path, kind: Kind) -> PathBuf {
    data.join(kind.directory()).join(format!(
        "{}.json",
        hash(root.as_os_str().as_encoded_bytes())
    ))
}

pub(crate) fn trusted(data: &Path, root: &Path, kind: Kind, fingerprint: &str) -> Result<bool> {
    let path = path(data, root, kind);
    if !fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        return Ok(false);
    }
    let file = match crate::store::open_regular_file(&path) {
        Ok(file) => file,
        Err(_) => return Ok(false),
    };
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(&path, e))?;
    if bytes.len() > 4096 {
        return Ok(false);
    }
    let grant: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(grant["version"] == 1
        && grant["scope"] == kind.name()
        && grant["fingerprint"] == fingerprint)
}

pub(crate) fn record(
    data: &Path,
    root: &Path,
    kind: Kind,
    fingerprint: &str,
    verify: impl FnOnce() -> Result<()>,
) -> Result<PathBuf> {
    let path = path(data, root, kind);
    let _lock = crate::store::Store::lock_configuration(&path)?;
    let parent = path.parent().expect("grant parent");
    fs::create_dir_all(parent).map_err(|e| Error::io(&path, e))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| Error::io(&path, e))?;
    file.write_all(
        json!({"version":1,"scope":kind.name(),"fingerprint":fingerprint})
            .to_string()
            .as_bytes(),
    )
    .map_err(|e| Error::io(&path, e))?;
    file.as_file().sync_all().map_err(|e| Error::io(&path, e))?;
    verify()?;
    crate::cancel::check(Some(&crate::cancel::token()))?;
    file.persist(&path).map_err(|e| Error::io(&path, e.error))?;
    crate::store::sync_directory(parent)?;
    Ok(path)
}

pub(crate) fn revoke(data: &Path, root: &Path, kind: Kind) -> Result<Value> {
    let root = crate::paths::canonical(root)?;
    let path = path(data, &root, kind);
    let _lock = crate::store::Store::lock_configuration(&path)?;
    let removed = match fs::remove_file(&path) {
        Ok(()) => {
            crate::store::sync_directory(path.parent().expect("grant parent"))?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(Error::io(&path, error)),
    };
    Ok(json!({"trusted":false,"removed":removed,"root":root,"grant_path":path}))
}
