//! Layered user/project data settings, captured with their discovery inputs.

use crate::{Error, Result, document::hash};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_BYTES: usize = 256 * 1024;
pub mod agents;
pub mod conversations;
pub mod keys;
pub mod runtime;
const TEMPLATE: &str = "version = 1\n\n[fonts]\n# Omitted fields inherit lower-priority settings.\n# Paths resolve relative to this configuration file.\n# files = []\n# search_paths = []\n# default_family = \"Noto Sans JP\"\n# locale = \"ja\"\n# Set fallbacks explicitly to replace the default glyph fallback chain.\n# fallbacks = [\"Noto Sans JP\"]\n";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    pub fonts: FontSettings,
    pub runtime: runtime::RuntimeSettings,
    pub design: crate::guidance::DesignSettings,
}
impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            version: 1,
            fonts: FontSettings::default(),
            runtime: Default::default(),
            design: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FontSettings {
    #[schemars(length(max = 256))]
    pub files: Vec<PathBuf>,
    #[schemars(length(max = 32))]
    pub search_paths: Vec<PathBuf>,
    /// A family name containing 1..256 UTF-8 bytes.
    #[schemars(length(min = 1, max = 256))]
    pub default_family: String,
    #[schemars(length(max = 256))]
    pub fallbacks: Option<Vec<String>>,
    /// A hyphenated language tag containing 1..128 ASCII bytes.
    #[schemars(length(min = 1, max = 128))]
    pub locale: String,
}
impl Default for FontSettings {
    fn default() -> Self {
        Self {
            files: vec![],
            search_paths: vec![],
            default_family: "Noto Sans JP".into(),
            fallbacks: None,
            locale: "ja".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfig {
    #[schemars(range(min = 1, max = 1))]
    pub version: u32,
    pub fonts: FontSettings,
    pub runtime: runtime::RuntimeSettings,
    pub agents: agents::AgentSettings,
    pub tui: keys::TuiSettings,
    pub conversations: conversations::ConversationSettings,
}
impl Default for UserConfig {
    fn default() -> Self {
        Self {
            version: 1,
            fonts: FontSettings::default(),
            runtime: Default::default(),
            agents: Default::default(),
            tui: Default::default(),
            conversations: Default::default(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FontOverrides {
    files: Option<Vec<PathBuf>>,
    search_paths: Option<Vec<PathBuf>>,
    default_family: Option<String>,
    fallbacks: Option<Vec<String>>,
    locale: Option<String>,
}
impl FontOverrides {
    fn apply(
        &self,
        settings: &mut FontSettings,
        origins: &mut BTreeMap<&'static str, &'static str>,
        origin: &'static str,
    ) {
        if let Some(value) = &self.files {
            settings.files = value.clone();
            origins.insert("files", origin);
        }
        if let Some(value) = &self.search_paths {
            settings.search_paths = value.clone();
            origins.insert("search_paths", origin);
        }
        if let Some(value) = &self.default_family {
            settings.default_family = value.clone();
            origins.insert("default_family", origin);
        }
        if let Some(value) = &self.fallbacks {
            settings.fallbacks = Some(value.clone());
            origins.insert("fallbacks", origin);
        }
        if let Some(value) = &self.locale {
            settings.locale = value.clone();
            origins.insert("locale", origin);
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProjectFile {
    version: u32,
    fonts: FontOverrides,
    runtime: runtime::RuntimeOverrides,
    design: crate::guidance::DesignSettings,
}
impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: 1,
            fonts: Default::default(),
            runtime: Default::default(),
            design: Default::default(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UserFile {
    version: u32,
    fonts: FontOverrides,
    runtime: runtime::RuntimeOverrides,
    agents: agents::AgentSettings,
    tui: keys::TuiSettings,
    conversations: conversations::ConversationSettings,
}
impl Default for UserFile {
    fn default() -> Self {
        Self {
            version: 1,
            fonts: Default::default(),
            runtime: Default::default(),
            agents: Default::default(),
            tui: Default::default(),
            conversations: Default::default(),
        }
    }
}

struct UserSource {
    requested: Option<PathBuf>,
    source: Option<(PathBuf, Arc<[u8]>)>,
    missing: Option<crate::paths::MissingPath>,
    settings: UserConfig,
    fonts: FontOverrides,
    runtime: runtime::RuntimeOverrides,
    automatic: bool,
}

/// A selected user file and its effective font discovery, without a document.
pub struct UserSnapshot {
    source: UserSource,
    pub settings: UserConfig,
    pub font_files: Vec<PathBuf>,
    directories: Vec<Directory>,
    bindings: Vec<(PathBuf, PathBuf)>,
}

/// User retention policy without font discovery, adapters or project loading.
pub struct ConversationSnapshot {
    source: UserSource,
    pub settings: conversations::ConversationSettings,
}

impl ConversationSnapshot {
    pub fn load() -> Result<Self> {
        let source = UserSource::load(Some(user_config_path()?), true)?;
        Ok(Self {
            settings: source.settings.conversations.clone(),
            source,
        })
    }

    pub fn load_file(path: Option<PathBuf>) -> Result<Self> {
        let source = UserSource::load(path, false)?;
        Ok(Self {
            settings: source.settings.conversations.clone(),
            source,
        })
    }

    pub fn verify_unchanged(&self) -> Result<()> {
        self.source.verify().map_err(|_| {
            Error::new(
                "REVISION_CONFLICT",
                "User conversation settings changed; review pruning again",
            )
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Discovery {
    root: PathBuf,
    source: Option<PathBuf>,
    candidates: Vec<PathBuf>,
}
#[derive(Clone, PartialEq, Eq)]
struct Directory {
    requested: PathBuf,
    root: PathBuf,
    files: Vec<PathBuf>,
}

pub struct Snapshot {
    document: PathBuf,
    discovery: Discovery,
    pub settings: ProjectConfig,
    pub source: Option<(PathBuf, Arc<[u8]>)>,
    pub font_files: Vec<PathBuf>,
    pub design: Result<crate::guidance::ProjectCatalog>,
    user: UserSource,
    font_origins: BTreeMap<&'static str, &'static str>,
    runtime_origins: BTreeMap<&'static str, &'static str>,
    lock_paths: Vec<PathBuf>,
    directories: Vec<Directory>,
    bindings: Vec<(PathBuf, PathBuf)>,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("CONFIGURATION_ERROR", message)
}
fn conflict() -> Error {
    Error::new(
        "REVISION_CONFLICT",
        "Project configuration or font discovery changed; reload the project",
    )
}

fn canonical(path: &Path) -> Result<PathBuf> {
    let path = crate::paths::canonical(path)?;
    if path.to_str().is_none() {
        return Err(invalid("Configuration and font paths must be UTF-8"));
    }
    Ok(path)
}

fn locate(document: &Path) -> Result<Discovery> {
    let parent = document
        .parent()
        .ok_or_else(|| invalid("Document has no configuration directory"))?;
    let mut candidates = Vec::new();
    for directory in parent.ancestors() {
        let path = directory.join("lazypencil.toml");
        candidates.push(path.clone());
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                let source = canonical(&path).map_err(|_| {
                    invalid(format!(
                        "Cannot resolve project configuration {}",
                        path.display()
                    ))
                })?;
                return Ok(Discovery {
                    root: directory.into(),
                    source: Some(source),
                    candidates,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(invalid(format!(
                    "Cannot inspect project configuration {}: {error}",
                    path.display()
                )));
            }
        }
        if directory.join(".git").exists() {
            return Ok(Discovery {
                root: directory.into(),
                source: None,
                candidates,
            });
        }
    }
    Ok(Discovery {
        root: parent.into(),
        source: None,
        candidates,
    })
}

fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8], path: &Path) -> Result<T> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("Configuration must be UTF-8"))?;
    toml::from_str(text).map_err(|error: toml::de::Error| {
        // Preserve locations without including a source excerpt or arbitrary values.
        let span = error.span().unwrap_or(0..0);
        let start = span.start.min(bytes.len());
        let line = bytes[..start].iter().filter(|byte| **byte == b'\n').count() + 1;
        invalid("Invalid configuration; expected version 1 and supported fields for this scope")
            .details(json!({"path":path,"byte_start":span.start,"byte_end":span.end,"line":line}))
    })
}

fn validate_version(version: u32) -> Result<()> {
    if version != 1 {
        return Err(invalid(
            "Unsupported configuration version; supported version is 1",
        ));
    }
    Ok(())
}

fn validate_fonts(fonts: &FontSettings) -> Result<()> {
    if fonts.files.len() > 256
        || fonts.search_paths.len() > 32
        || fonts
            .fallbacks
            .as_ref()
            .is_some_and(|names| names.len() > 256)
    {
        return Err(invalid(
            "Font configuration exceeds 256 files/fallbacks or 32 search directories",
        ));
    }
    if fonts.files.iter().chain(&fonts.search_paths).any(|path| {
        path.as_os_str().is_empty()
            || path.as_os_str().as_encoded_bytes().len() > 4096
            || path.to_string_lossy().contains("://")
    }) {
        return Err(invalid(
            "Configured font paths must be nonempty local paths up to 4096 bytes",
        ));
    }
    if std::iter::once(&fonts.default_family)
        .chain(fonts.fallbacks.iter().flatten())
        .any(|name| name.is_empty() || name.len() > 256)
    {
        return Err(invalid("Font families must contain 1..256 UTF-8 bytes"));
    }
    if fonts.locale.is_empty()
        || fonts.locale.len() > 128
        || fonts.locale.split('-').any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        || !fonts.locale.as_bytes()[0].is_ascii_alphabetic()
    {
        return Err(invalid(
            "Font locale must be a language tag such as ja, en-US, or zh-Hant",
        ));
    }
    Ok(())
}

fn directory(requested: &Path) -> Result<Directory> {
    let root = canonical(requested).map_err(|_| {
        invalid(format!(
            "Cannot resolve font search directory {}",
            requested.display()
        ))
    })?;
    let mut files = Vec::new();
    let mut count = 0usize;
    fn visit(path: &Path, depth: usize, count: &mut usize, files: &mut Vec<PathBuf>) -> Result<()> {
        if depth > 16 {
            return Err(invalid("Font search exceeds 16 directory levels"));
        }
        let mut entries = fs::read_dir(path)
            .map_err(|error| {
                invalid(format!(
                    "Cannot read font search directory {}: {error}",
                    path.display()
                ))
            })?
            .take(4097)
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| invalid(error.to_string()))?;
        if entries.len() > 4096 {
            return Err(invalid("Font search exceeds 4096 directory entries"));
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            *count += 1;
            if *count > 4096 {
                return Err(invalid("Font search exceeds 4096 directory entries"));
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let kind = entry
                .file_type()
                .map_err(|error| invalid(error.to_string()))?;
            let path = entry.path();
            if kind.is_dir() {
                visit(&path, depth + 1, count, files)?;
            } else if path
                .extension()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    ["ttf", "otf", "ttc", "otc"]
                        .iter()
                        .any(|extension| name.eq_ignore_ascii_case(extension))
                })
            {
                let canonical = canonical(&path).map_err(|_| {
                    invalid(format!("Cannot resolve discovered font {}", path.display()))
                })?;
                let metadata =
                    fs::metadata(&canonical).map_err(|error| invalid(error.to_string()))?;
                if metadata.is_file() {
                    files.push(canonical);
                } else if !metadata.is_dir() {
                    return Err(invalid("A discovered font is not a regular file"));
                }
            }
            if files.len() > 256 {
                return Err(invalid("Font search exceeds 256 files"));
            }
        }
        Ok(())
    }
    visit(&root, 0, &mut count, &mut files)?;
    let mut seen = BTreeSet::new();
    files.retain(|path| seen.insert(path.clone()));
    Ok(Directory {
        requested: requested.into(),
        root,
        files,
    })
}

pub fn user_config_path() -> Result<PathBuf> {
    let directory = std::env::var_os("LAZYPENCIL_CONFIG_DIR")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| {
            directories::ProjectDirs::from("dev", "lazypencil", "lazypencil")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .ok_or_else(|| {
                    invalid(
                        "Cannot determine user configuration directory; set LAZYPENCIL_CONFIG_DIR",
                    )
                })
        })?;
    if directory.as_os_str().is_empty()
        || directory.to_str().is_none()
        || directory.as_os_str().as_encoded_bytes().len() > 4096
        || directory.to_string_lossy().contains("://")
    {
        return Err(invalid(
            "Configuration directory must be a nonempty local UTF-8 path up to 4096 bytes",
        ));
    }
    Ok(absolute(&directory)?.join("config.toml"))
}

fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.into()
    } else {
        std::env::current_dir()
            .map_err(|error| Error::io(path, error))?
            .join(path)
    };
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(invalid(
            "Configuration paths must resolve to absolute UTF-8 paths",
        ));
    }
    Ok(path)
}

fn read_configuration(path: &Path) -> Result<Arc<[u8]>> {
    let mut bytes = Vec::new();
    crate::store::open_regular_file(path)
        .map_err(|_| {
            invalid(format!(
                "Cannot read configuration {} as a regular file",
                path.display()
            ))
        })?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| invalid(error.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("Configuration exceeds 256 KiB"));
    }
    Ok(bytes.into())
}

fn origins() -> BTreeMap<&'static str, &'static str> {
    [
        "files",
        "search_paths",
        "default_family",
        "fallbacks",
        "locale",
    ]
    .into_iter()
    .map(|key| (key, "builtin"))
    .collect()
}

impl UserSource {
    fn load(requested: Option<PathBuf>, automatic: bool) -> Result<Self> {
        let requested = requested.as_deref().map(absolute).transpose()?;
        let mut result = Self {
            requested,
            source: None,
            missing: None,
            settings: UserConfig::default(),
            fonts: FontOverrides::default(),
            runtime: Default::default(),
            automatic,
        };
        if let Some(requested) = &result.requested {
            match fs::symlink_metadata(requested) {
                Ok(_) => {
                    let path = canonical(requested).map_err(|_| {
                        invalid(format!(
                            "Cannot resolve user configuration {}",
                            requested.display()
                        ))
                    })?;
                    let bytes = read_configuration(&path)?;
                    let parsed: UserFile = parse(&bytes, &path)?;
                    validate_version(parsed.version)?;
                    parsed
                        .fonts
                        .apply(&mut result.settings.fonts, &mut origins(), "user");
                    validate_fonts(&result.settings.fonts)?;
                    parsed.runtime.apply(
                        &mut result.settings.runtime,
                        &mut runtime::origins(),
                        "user",
                    )?;
                    parsed.agents.validate()?;
                    result.settings.agents = parsed.agents;
                    parsed.tui.validate()?;
                    result.settings.tui = parsed.tui;
                    parsed.conversations.retention.validate()?;
                    result.settings.conversations = parsed.conversations;
                    result.fonts = parsed.fonts;
                    result.runtime = parsed.runtime;
                    result.source = Some((path, bytes));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    result.missing =
                        Some(crate::paths::MissingPath::capture(requested)?.ok_or_else(conflict)?);
                }
                Err(error) => {
                    return Err(invalid(format!(
                        "Cannot inspect user configuration {}: {error}",
                        requested.display()
                    )));
                }
            }
        }
        Ok(result)
    }
    fn base(&self) -> Option<&Path> {
        self.source.as_ref().and_then(|(path, _)| path.parent())
    }
    fn verify(&self) -> Result<()> {
        if self.automatic && Some(user_config_path().map_err(|_| conflict())?) != self.requested {
            return Err(conflict());
        }
        if let Some(missing) = &self.missing {
            missing.verify().map_err(|_| conflict())?;
        }
        if let Some((path, expected)) = &self.source
            && (canonical(self.requested.as_ref().expect("selected user file"))
                .map_err(|_| conflict())?
                != *path
                || read_configuration(path).map_err(|_| conflict())?.as_ref() != expected.as_ref())
        {
            return Err(conflict());
        }
        Ok(())
    }
    fn summary(&self) -> Value {
        json!({"enabled":self.requested.is_some(),"requested_path":self.requested,
            "path":self.source.as_ref().map(|(path,_)|path),"hash":self.source.as_ref().map(|(_,bytes)|hash(bytes)),"settings":self.settings})
    }
}

struct FontInputs {
    files: Vec<PathBuf>,
    directories: Vec<Directory>,
    bindings: Vec<(PathBuf, PathBuf)>,
}

fn font_inputs(
    settings: &FontSettings,
    origins: &BTreeMap<&'static str, &'static str>,
    bases: &[(&str, &Path)],
) -> Result<FontInputs> {
    let mut font_files = Vec::new();
    let mut bindings = Vec::new();
    let mut directories = Vec::new();
    // More specific discovery precedes inherited explicit files when fields mix.
    for (origin, base) in bases {
        if origins["files"] == *origin {
            for path in &settings.files {
                let requested = base.join(path);
                let canonical = canonical(&requested).map_err(|_| {
                    invalid(format!(
                        "Cannot resolve configured font {}",
                        requested.display()
                    ))
                })?;
                if !fs::metadata(&canonical).is_ok_and(|metadata| metadata.is_file()) {
                    return Err(invalid("Configured font input must be a regular file"));
                }
                bindings.push((requested, canonical.clone()));
                font_files.push(canonical);
            }
        }
        if origins["search_paths"] == *origin {
            for path in &settings.search_paths {
                let snapshot = directory(&base.join(path))?;
                font_files.extend(snapshot.files.iter().cloned());
                directories.push(snapshot);
            }
        }
    }
    let mut seen = BTreeSet::new();
    font_files.retain(|path| seen.insert(path.clone()));
    if font_files.len() > 256 {
        return Err(invalid("Configured font files exceed 256"));
    }
    Ok(FontInputs {
        files: font_files,
        directories,
        bindings,
    })
}

fn verify_fonts(bindings: &[(PathBuf, PathBuf)], directories: &[Directory]) -> Result<()> {
    for (requested, expected) in bindings {
        if canonical(requested).map_err(|_| conflict())? != *expected {
            return Err(conflict());
        }
    }
    for snapshot in directories {
        if directory(&snapshot.requested).map_err(|_| conflict())? != *snapshot {
            return Err(conflict());
        }
    }
    Ok(())
}

impl UserSnapshot {
    pub fn load() -> Result<Self> {
        Self::load_inner(Some(user_config_path()?), true)
    }
    pub fn load_file(path: Option<PathBuf>) -> Result<Self> {
        Self::load_inner(path, false)
    }
    fn load_inner(path: Option<PathBuf>, automatic: bool) -> Result<Self> {
        let source = UserSource::load(path, automatic)?;
        let mut origins = origins();
        let mut fonts = FontSettings::default();
        source.fonts.apply(&mut fonts, &mut origins, "user");
        let inputs = font_inputs(
            &fonts,
            &origins,
            &source
                .base()
                .map(|base| vec![("user", base)])
                .unwrap_or_default(),
        )?;
        let settings = source.settings.clone();
        Ok(Self {
            source,
            settings,
            font_files: inputs.files,
            directories: inputs.directories,
            bindings: inputs.bindings,
        })
    }
    pub fn verify_unchanged(&self) -> Result<()> {
        self.source.verify()?;
        verify_fonts(&self.bindings, &self.directories)
    }
    pub fn summary(&self) -> Value {
        let mut value = self.source.summary();
        value["font_files"] = json!(self.font_files);
        value
    }
}

impl Snapshot {
    pub fn load(document: &Path) -> Result<Self> {
        Self::load_inner(document, Some(user_config_path()?), true)
    }
    pub fn load_with_user_file(document: &Path, user: Option<PathBuf>) -> Result<Self> {
        Self::load_inner(document, user, false)
    }
    fn load_inner(document: &Path, user_file: Option<PathBuf>, automatic: bool) -> Result<Self> {
        let discovery = locate(document)?;
        let source = discovery
            .source
            .as_ref()
            .map(|path| read_configuration(path).map(|bytes| (path.clone(), bytes)))
            .transpose()?;
        let project: ProjectFile = source
            .as_ref()
            .map(|(path, bytes)| parse(bytes, path))
            .transpose()?
            .unwrap_or_default();
        validate_version(project.version)?;
        let mut project_fonts = FontSettings::default();
        project
            .fonts
            .apply(&mut project_fonts, &mut origins(), "project");
        validate_fonts(&project_fonts)?;
        let user = UserSource::load(user_file, automatic)?;
        let mut settings = ProjectConfig::default();
        let mut font_origins = origins();
        user.fonts
            .apply(&mut settings.fonts, &mut font_origins, "user");
        project
            .fonts
            .apply(&mut settings.fonts, &mut font_origins, "project");
        settings.design = project.design;
        validate_fonts(&settings.fonts)?;
        let mut runtime_origins = runtime::origins();
        user.runtime
            .apply(&mut settings.runtime, &mut runtime_origins, "user")?;
        project
            .runtime
            .apply(&mut settings.runtime, &mut runtime_origins, "project")?;
        let base = source
            .as_ref()
            .and_then(|(path, _)| path.parent())
            .unwrap_or(&discovery.root);
        let mut bases = vec![("project", base)];
        if let Some(base) = user.base() {
            bases.push(("user", base));
        }
        let inputs = font_inputs(&settings.fonts, &font_origins, &bases)?;
        let design = crate::guidance::ProjectCatalog::load(
            &discovery.root,
            base,
            &settings.design,
            source
                .as_ref()
                .map(|(path, bytes)| (path.as_path(), hash(bytes))),
        );
        let mut lock_paths = discovery.candidates.clone();
        lock_paths.extend(user.requested.iter().cloned());
        Ok(Self {
            document: document.into(),
            discovery,
            settings,
            source,
            font_files: inputs.files,
            design,
            user,
            font_origins,
            runtime_origins,
            lock_paths,
            directories: inputs.directories,
            bindings: inputs.bindings,
        })
    }
    pub fn root(&self) -> &Path {
        &self.discovery.root
    }
    pub fn lock_paths(&self) -> &[PathBuf] {
        &self.lock_paths
    }
    pub fn verify_unchanged(&self) -> Result<()> {
        self.user.verify()?;
        if let Ok(design) = &self.design {
            design.verify_unchanged()?;
        }
        if locate(&self.document).map_err(|_| conflict())? != self.discovery {
            return Err(conflict());
        }
        if let Some((path, expected)) = &self.source {
            let mut bytes = Vec::new();
            crate::store::open_regular_file(path)
                .map_err(|_| conflict())?
                .take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| conflict())?;
            if hash(&bytes) != hash(expected) {
                return Err(conflict());
            }
        }
        verify_fonts(&self.bindings, &self.directories)
    }
    pub fn summary(&self) -> Value {
        let design = self
            .design
            .as_ref()
            .map(|design| design.metadata())
            .unwrap_or_else(|error| json!({"available":false,"trusted":false,"error":error}));
        json!({"path":self.source.as_ref().map(|(path,_)|path),"hash":self.source.as_ref().map(|(_,bytes)|hash(bytes)),"root":self.root(),"settings":self.settings,"font_files":self.font_files,"font_origins":self.font_origins,"runtime_origins":self.runtime_origins,"user":self.user.summary(),"design":design})
    }
    pub fn sources(&self) -> Vec<(PathBuf, Arc<[u8]>)> {
        self.source
            .iter()
            .chain(self.user.source.iter())
            .cloned()
            .collect()
    }
    pub fn design_catalog(&self) -> Result<&crate::guidance::ProjectCatalog> {
        self.design.as_ref().map_err(Clone::clone)
    }
}

pub fn initialize(path: &Path) -> Result<Value> {
    let cancel = crate::cancel::token();
    crate::cancel::check(Some(&cancel))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid("Configuration needs a filename"))?;
    if name != "lazypencil.toml" {
        return Err(invalid("Configuration filename must be lazypencil.toml"));
    }
    let parent = crate::paths::canonical(
        path.parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let path = parent.join(name);
    let _lock = crate::store::Store::lock_configuration(&path)?;
    write_configuration(&path, "project")
}

pub fn initialize_user() -> Result<Value> {
    let cancel = crate::cancel::token();
    crate::cancel::check(Some(&cancel))?;
    let requested = user_config_path()?;
    if fs::symlink_metadata(&requested).is_ok() {
        return Err(invalid(
            "Configuration already exists; init never overwrites it",
        ));
    }
    let missing = crate::paths::MissingPath::capture(&requested)?.ok_or_else(conflict)?;
    let _lock = crate::store::Store::lock_configuration(&requested)?;
    missing.verify()?;
    let path = &missing.target;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("User configuration needs a parent directory"))?;
    crate::store::create_private_directory(parent)?;
    missing.verify()?;
    let mut value = write_configuration(path, "user")?;
    value["requested_path"] = json!(requested);
    Ok(value)
}

fn write_configuration(path: &Path, scope: &str) -> Result<Value> {
    let cancel = crate::cancel::token();
    crate::cancel::check(Some(&cancel))?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Configuration needs a parent directory"))?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| Error::io(parent, error))?;
    let template = if scope == "user" {
        format!(
            "{TEMPLATE}{}{}{}{}",
            runtime::TEMPLATE,
            agents::TEMPLATE,
            keys::TEMPLATE,
            conversations::TEMPLATE
        )
    } else {
        format!("{TEMPLATE}{}", runtime::TEMPLATE)
    };
    file.write_all(template.as_bytes())
        .map_err(|error| Error::io(path, error))?;
    file.as_file()
        .sync_all()
        .map_err(|error| Error::io(path, error))?;
    crate::cancel::check(Some(&cancel))?;
    file.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            invalid("Configuration already exists; init never overwrites it")
        } else {
            Error::io(path, error.error)
        }
    })?;
    crate::store::sync_directory(parent)?;
    Ok(json!({"path":path,"hash":hash(template.as_bytes()),"version":1,"scope":scope}))
}

pub fn revoke_design_trust(document: &Path, data: &Path) -> Result<Value> {
    let parent = document
        .parent()
        .ok_or_else(|| invalid("Document has no configuration directory"))?;
    let mut removed = 0usize;
    for directory in parent.ancestors() {
        let result = crate::guidance::ProjectCatalog::revoke(directory, data)?;
        if result["removed"] == true {
            removed += 1;
        }
        if directory.join(".git").exists() {
            break;
        }
    }
    Ok(json!({"trusted":false,"removed_grants":removed,"document":document}))
}
