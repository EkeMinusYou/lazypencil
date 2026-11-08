//! Reviewed, source-preserving permission-mode defaults. Never exposed through MCP.
use super::*;
use crate::{prompt::AgentName, store::Store};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Conversation,
    Project,
    Global,
}
impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Conversation => "This conversation only",
            Self::Project => "Save as project default",
            Self::Global => "Save as global default",
        }
    }
}

#[derive(Clone)]
pub struct Prepared {
    path: PathBuf,
    scope: Scope,
    agent_settings: Value,
    document: PathBuf,
    requested: PathBuf,
    discovery: Discovery,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}

fn contents(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_configuration(path).map(|bytes| Some(bytes.to_vec())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(path, error)),
    }
}
fn destination(requested: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(requested) {
        Ok(_) => canonical(requested),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => absolute(requested),
        Err(error) => Err(Error::io(requested, error)),
    }
}

impl Prepared {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn agent_settings(&self) -> &Value {
        &self.agent_settings
    }
    pub fn new(document: &Path, agent: AgentName, mode: &str, scope: Scope) -> Result<Self> {
        Self::prepare(document, agent, mode, scope, None)
    }
    fn prepare(
        document: &Path,
        agent: AgentName,
        mode: &str,
        scope: Scope,
        user_path: Option<&Path>,
    ) -> Result<Self> {
        if scope == Scope::Conversation {
            return Err(invalid(
                "Conversation modes do not require a configuration file",
            ));
        }
        let document = canonical(document)?;
        let discovery = locate(&document)?;
        let requested = if scope == Scope::Project {
            discovery
                .source
                .clone()
                .unwrap_or_else(|| discovery.root.join("lazypencil.toml"))
        } else {
            user_path
                .map(Path::to_path_buf)
                .map(Ok)
                .unwrap_or_else(user_config_path)?
        };
        let path = destination(&requested)?;
        let before = contents(&path)?;
        let text = std::str::from_utf8(before.as_deref().unwrap_or(b"version = 1\n"))
            .map_err(|_| invalid("Configuration must be UTF-8"))?;
        let mut parsed = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| invalid(error.to_string()))?;
        let name = match agent {
            AgentName::Codex => "codex",
            AgentName::Claude => "claude",
        };
        // Table-like access also supports quoted keys, dotted keys and inline tables.
        let root = parsed.as_table_mut();
        let agents = root
            .entry("agents")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| invalid("agents must be a table"))?;
        let profile = agents
            .entry(name)
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_like_mut()
            .ok_or_else(|| invalid("Agent profiles must be tables"))?;
        if let Some(previous) = profile
            .get_mut("mode")
            .and_then(toml_edit::Item::as_value_mut)
        {
            let decoration = previous.decor().clone();
            *previous = toml_edit::Value::from(mode);
            *previous.decor_mut() = decoration;
        } else {
            profile.insert("mode", toml_edit::value(mode));
        }
        let after = parsed.to_string().into_bytes();
        if after.len() > MAX_BYTES {
            return Err(invalid("Configuration exceeds 256 KiB"));
        }
        let settings = if scope == Scope::Project {
            let file: ProjectFile = parse(&after, &path)?;
            validate_version(file.version)?;
            file.agents.unwrap_or_default()
        } else {
            let file: UserFile = parse(&after, &path)?;
            validate_version(file.version)?;
            file.agents
        };
        settings.validate()?;
        Ok(Self {
            path,
            scope,
            agent_settings: serde_json::to_value(settings).expect("agent settings"),
            document,
            requested,
            discovery,
            before,
            after,
        })
    }

    pub(crate) fn matches(&self, document: &Path, agent: AgentName, mode: &str) -> Result<()> {
        let name = match agent {
            AgentName::Codex => "codex",
            AgentName::Claude => "claude",
        };
        if self.document != canonical(document)? || self.agent_settings[name]["mode"] != mode {
            return Err(invalid(
                "Reviewed defaults must match this document, agent and mode",
            ));
        }
        self.verify()
    }

    pub fn verify(&self) -> Result<()> {
        if locate(&self.document)? != self.discovery
            || destination(&self.requested)? != self.path
            || contents(&self.path)? != self.before
        {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "Configuration changed; review the settings again before saving",
            ));
        }
        Ok(())
    }

    /// The project variant is called only after an explicit Trust & apply action.
    pub fn save(&self, store: &Store) -> Result<Option<agent_execution::Selection>> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| invalid("Missing configuration parent"))?;
        crate::store::create_private_directory(parent)?;
        let _lock = Store::lock_configuration(&self.path)?;
        self.verify()?;
        crate::store::atomic_replace(&self.path, &self.after)?;
        if self.scope != Scope::Project {
            return Ok(None);
        }
        let project = agent_execution::Project::load(&self.document)?;
        let summary = project.summary(store.data_dir())?;
        if summary["hash"] != hash(&self.after)
            || summary["root"] != self.discovery.root.to_string_lossy().as_ref()
        {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "Project settings changed after save; review before granting trust",
            ));
        }
        let fingerprint = summary["agent_execution"]["fingerprint"]
            .as_str()
            .ok_or_else(|| invalid("Missing project execution fingerprint"))?;
        project.trust_agent_execution(store.data_dir(), Some(fingerprint))?;
        project.require_agent_execution(store.data_dir()).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    fn fixture() -> (TempDir, Store) {
        let temp = TempDir::new().unwrap();
        let store = Store::init(
            &temp.path().join("design.pen"),
            Some(temp.path().join("data")),
        )
        .unwrap();
        (temp, store)
    }
    #[test]
    fn mode_defaults_preserve_comments_other_agents_and_global_overrides() {
        let (temp, store) = fixture();
        let path = temp.path().join("config.toml");
        let before = "# Keep this comment\nversion=1\n[agents.codex]\nfast = true\nmode = 'read-only' # mode comment\n[agents.claude]\nmode = 'acceptEdits'\n";
        fs::write(&path, before).unwrap();
        let prepared = Prepared::prepare(
            store.path(),
            AgentName::Codex,
            "agent",
            Scope::Global,
            Some(&path),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert!(prepared.save(&store).unwrap().is_none());
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("# Keep this comment") && after.contains("# mode comment"));
        let file: UserFile = parse(after.as_bytes(), &path).unwrap();
        assert_eq!(file.agents.codex.mode.as_deref(), Some("agent"));
        assert_eq!(file.agents.codex.fast, Some(true));
        assert_eq!(file.agents.claude.mode.as_deref(), Some("acceptEdits"));
    }
    #[test]
    fn mode_defaults_support_inline_tables_and_detect_concurrent_edits() {
        let (temp, store) = fixture();
        let path = temp.path().join("config.toml");
        fs::write(&path, "version=1\nagents = { codex = { fast = true } }\n").unwrap();
        let prepared = Prepared::prepare(
            store.path(),
            AgentName::Codex,
            "workspace-write",
            Scope::Global,
            Some(&path),
        )
        .unwrap();
        prepared.save(&store).unwrap();
        let file: UserFile = parse(&fs::read(&path).unwrap(), &path).unwrap();
        assert_eq!(file.agents.codex.mode.as_deref(), Some("workspace-write"));
        let prepared = Prepared::prepare(
            store.path(),
            AgentName::Codex,
            "agent",
            Scope::Global,
            Some(&path),
        )
        .unwrap();
        fs::write(&path, "version=1\n# concurrently changed\n").unwrap();
        assert_eq!(prepared.save(&store).unwrap_err().code, "REVISION_CONFLICT");
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("concurrently changed")
        );
    }
    #[test]
    fn mode_project_review_does_not_grant_trust_until_explicit_save() {
        let (temp, store) = fixture();
        let prepared =
            Prepared::new(store.path(), AgentName::Codex, "agent", Scope::Project).unwrap();
        assert!(!temp.path().join("lazypencil.toml").exists());
        let selection = prepared.save(&store).unwrap().unwrap();
        selection.verify(&store).unwrap();
        fs::write(
            &prepared.path,
            "version=1\n[agents.codex]\nmode='agent-full-access'\n",
        )
        .unwrap();
        assert_eq!(
            selection.verify(&store).unwrap_err().code,
            "PROJECT_NOT_TRUSTED"
        );
    }
    #[cfg(unix)]
    #[test]
    fn mode_defaults_preserve_symlinks_and_reject_retargeting() {
        let (temp, store) = fixture();
        let requested = temp.path().join("config.toml");
        let target = temp.path().join("physical.toml");
        fs::write(&target, "version=1\n").unwrap();
        std::os::unix::fs::symlink(&target, &requested).unwrap();
        let prepared = Prepared::prepare(
            store.path(),
            AgentName::Claude,
            "auto",
            Scope::Global,
            Some(&requested),
        )
        .unwrap();
        prepared.save(&store).unwrap();
        assert!(
            fs::symlink_metadata(&requested)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::read_to_string(&target).unwrap().contains("auto"));
        let prepared = Prepared::prepare(
            store.path(),
            AgentName::Claude,
            "default",
            Scope::Global,
            Some(&requested),
        )
        .unwrap();
        fs::remove_file(&requested).unwrap();
        let other = temp.path().join("other.toml");
        fs::write(&other, "version=1\n").unwrap();
        std::os::unix::fs::symlink(other, &requested).unwrap();
        assert_eq!(prepared.save(&store).unwrap_err().code, "REVISION_CONFLICT");
    }
}
