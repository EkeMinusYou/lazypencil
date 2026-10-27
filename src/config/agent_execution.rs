//! Project agent settings are data until a separate local execution grant is used.
use super::{
    Snapshot,
    trust::{self, Kind},
};
use crate::{Error, Result, document::hash, store::Store};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Agent discovery deliberately avoids font discovery and design-resource loading.
pub(crate) struct Project {
    document: PathBuf,
    discovery: super::Discovery,
    source: Option<(PathBuf, Arc<[u8]>)>,
    pub(crate) settings: Option<super::agents::AgentSettings>,
}

#[derive(Debug, Clone)]
pub struct Selection {
    root: PathBuf,
    fingerprint: String,
}

impl Snapshot {
    pub fn agent_execution_metadata(&self, data: &Path) -> Result<Value> {
        Project {
            document: self.document.clone(),
            discovery: self.discovery.clone(),
            source: self.source.clone(),
            settings: self.settings.agents.clone(),
        }
        .agent_execution_metadata(data)
    }
}

impl Project {
    pub(crate) fn load(document: &Path) -> Result<Self> {
        let document = match std::fs::symlink_metadata(document) {
            Ok(_) => crate::paths::canonical(document)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = document
                    .file_name()
                    .ok_or_else(|| super::invalid("Document has no filename"))?;
                let parent = document
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                crate::paths::canonical(parent)?.join(name)
            }
            Err(error) => return Err(Error::io(document, error)),
        };
        if document.to_str().is_none() {
            return Err(super::invalid("Document paths must be UTF-8"));
        }
        let discovery = super::locate(&document)?;
        let source = discovery
            .source
            .as_ref()
            .map(|path| super::read_configuration(path).map(|bytes| (path.clone(), bytes)))
            .transpose()?;
        let settings: super::ProjectFile = source
            .as_ref()
            .map(|(path, bytes)| super::parse(bytes, path))
            .transpose()?
            .unwrap_or_default();
        super::validate_version(settings.version)?;
        if let Some(agents) = &settings.agents {
            agents.validate()?;
        }
        Ok(Self {
            document,
            discovery,
            source,
            settings: settings.agents,
        })
    }

    fn root(&self) -> &Path {
        &self.discovery.root
    }

    pub(crate) fn base(&self) -> &Path {
        self.source
            .as_ref()
            .and_then(|(path, _)| path.parent())
            .unwrap_or(self.root())
    }

    pub(crate) fn verify_unchanged(&self) -> Result<()> {
        super::verify_project_source(&self.document, &self.discovery, &self.source)
    }

    pub(crate) fn summary(&self, data: &Path) -> Result<Value> {
        Ok(
            json!({"path":self.source.as_ref().map(|(path,_)|path),"hash":self.source.as_ref().map(|(_,bytes)|hash(bytes)),
            "root":self.root(),"settings":{"agents":self.settings},"agent_execution":self.agent_execution_metadata(data)?}),
        )
    }

    pub(crate) fn agent_execution_metadata(&self, data: &Path) -> Result<Value> {
        let fingerprint = self.agent_execution_fingerprint();
        let trusted = match &fingerprint {
            Some(fingerprint) => {
                trust::trusted(data, self.root(), Kind::AgentExecution, fingerprint)?
            }
            None => true,
        };
        Ok(
            json!({"configured":fingerprint.is_some(),"trust_required":fingerprint.is_some(),"trust_scope":Kind::AgentExecution.name(),"fingerprint":fingerprint,"trusted":trusted,"root":self.root()}),
        )
    }

    fn agent_execution_fingerprint(&self) -> Option<String> {
        self.settings
            .as_ref()
            .filter(|settings| settings.configured())
            .map(|settings| {
                hash(
                json!({
                    "version":1,"scope":Kind::AgentExecution.name(),"root":self.root(),
                    "configuration":self.source.as_ref().map(|(path,_)|path),"settings":settings,
                })
                .to_string()
                .as_bytes(),
            )
            })
    }

    pub(crate) fn trust_agent_execution(
        &self,
        data: &Path,
        expected: Option<&str>,
    ) -> Result<Value> {
        let fingerprint = self.agent_execution_fingerprint().ok_or_else(|| {
            Error::new(
                "CONFIGURATION_ERROR",
                "This project has no agent settings to authorize",
            )
        })?;
        if expected.is_some_and(|value| value != fingerprint) {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "Project agent settings changed before authorization",
            ));
        }
        self.verify_unchanged()?;
        let path = trust::record(
            data,
            self.root(),
            Kind::AgentExecution,
            &fingerprint,
            || self.verify_unchanged(),
        )?;
        Ok(
            json!({"trusted":true,"trust_scope":Kind::AgentExecution.name(),"fingerprint":fingerprint,"root":self.root(),"grant_path":path}),
        )
    }

    pub(crate) fn require_agent_execution(&self, data: &Path) -> Result<Selection> {
        let selection = self.selection()?;
        if !trust::trusted(
            data,
            self.root(),
            Kind::AgentExecution,
            &selection.fingerprint,
        )? {
            return Err(Error::new("PROJECT_NOT_TRUSTED", "Project agent settings require a separate local user grant; review config show --executables and run config trust --executables for this document. Design-instruction grants do not authorize executables, and agents must not grant trust themselves")
                .details(self.agent_execution_metadata(data)?));
        }
        Ok(selection)
    }

    pub(crate) fn selection(&self) -> Result<Selection> {
        let fingerprint = self.agent_execution_fingerprint().ok_or_else(|| {
            Error::new(
                "PROJECT_NOT_TRUSTED",
                "Project agent settings are no longer configured",
            )
        })?;
        Ok(Selection {
            root: self.root().into(),
            fingerprint,
        })
    }
}

impl Selection {
    pub(crate) fn metadata(&self) -> Value {
        json!({"scope":Kind::AgentExecution.name(),"root":self.root,"fingerprint":self.fingerprint})
    }

    pub(crate) fn verify(&self, store: &Store) -> Result<()> {
        let configuration = Project::load(store.path())?;
        if self.root != configuration.root()
            || configuration.agent_execution_fingerprint().as_deref()
                != Some(self.fingerprint.as_str())
        {
            return Err(Error::new(
                "PROJECT_NOT_TRUSTED",
                "Project agent settings changed; reload and review them before another turn",
            ));
        }
        configuration.require_agent_execution(store.data_dir())?;
        configuration.verify_unchanged()
    }
}
