//! Captured project design data; instruction use requires an explicit local grant.
use crate::{
    Error, Result,
    document::{Document, hash},
};
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

const MAX_FILE: usize = 2 * 1024 * 1024;
const MAX_TOTAL: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DesignSettings {
    #[schemars(length(max = 32))]
    pub guides: Vec<GuideFile>,
    #[schemars(length(max = 16))]
    pub styles: Vec<PathBuf>,
    pub default_style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuideFile {
    pub id: String,
    pub path: PathBuf,
    #[serde(default)]
    pub summary: String,
    #[serde(default = "version")]
    pub version: String,
    #[serde(default = "language")]
    pub language: String,
    #[serde(default = "automatic")]
    pub automatic: bool,
}
fn version() -> String {
    "1.0.0".into()
}
fn language() -> String {
    "und".into()
}
fn automatic() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleFile {
    id: String,
    name: String,
    description: String,
    #[serde(default = "version")]
    version: String,
    #[serde(default = "language")]
    language: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    use_cases: Vec<String>,
    #[serde(default)]
    guidance: Vec<String>,
    tokens: Value,
    instructions: String,
    example: PathBuf,
    #[serde(default)]
    required_fonts: Vec<Value>,
    #[serde(default)]
    required_packs: Vec<Value>,
}

pub struct Resource {
    pub path: PathBuf,
    pub bytes: Arc<[u8]>,
    pub kind: &'static str,
}

pub struct ProjectCatalog {
    root: PathBuf,
    fingerprint: String,
    trust_fingerprint: String,
    guides: Vec<Value>,
    styles: Vec<Value>,
    examples: BTreeMap<String, PathBuf>,
    pub resources: Vec<Resource>,
    bindings: Vec<(PathBuf, PathBuf, &'static str)>,
    default_style: Option<String>,
}

fn invalid(message: &str) -> Error {
    Error::new("CONFIGURATION_ERROR", message)
}
fn resource_error(message: &str, path: &Path, kind: &str) -> Error {
    invalid(message).details(json!({"path":path.to_string_lossy(),"kind":kind}))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}
fn local_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.as_os_str().as_encoded_bytes().len() <= 4096
        && path.to_str().is_some_and(|s| !s.contains("://"))
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}

impl ProjectCatalog {
    pub fn load(
        root: &Path,
        base: &Path,
        settings: &DesignSettings,
        configuration: Option<(&Path, String)>,
    ) -> Result<Self> {
        if settings.guides.len() > 32 || settings.styles.len() > 16 {
            return Err(invalid(
                "Design settings allow at most 32 guides and 16 styles",
            ));
        }
        let mut catalog = Self {
            root: crate::paths::canonical(root)?,
            fingerprint: String::new(),
            trust_fingerprint: String::new(),
            guides: vec![],
            styles: vec![],
            examples: BTreeMap::new(),
            resources: vec![],
            bindings: vec![],
            default_style: settings.default_style.clone(),
        };
        let mut ids = BTreeSet::new();
        let mut automatic_bytes = 0usize;
        for guide in &settings.guides {
            if !identifier(&guide.id)
                || !ids.insert(guide.id.clone())
                || guide.summary.len() > 1024
                || !text(&guide.version, 64)
                || !text(&guide.language, 128)
            {
                return Err(invalid(
                    "Project guides need unique IDs, a bounded version/language and summary",
                ));
            }
            let (path, bytes) = catalog.capture(base, &guide.path, "guidance", 64 * 1024)?;
            let body = std::str::from_utf8(&bytes)
                .map_err(|_| invalid("Project guide text must be UTF-8"))?;
            if guide.automatic {
                automatic_bytes += bytes.len();
            }
            if automatic_bytes > 256 * 1024 {
                return Err(invalid(
                    "Automatic project guides exceed 256 KiB; mark reference guides automatic=false",
                ));
            }
            let mut value = json!({"id":format!("project:{}",guide.id),"version":guide.version,"language":guide.language,"summary":guide.summary,
                "body":body,"automatic":guide.automatic,"source_path":path,"source_hash":hash(&bytes),"schema_versions":["2.21"],"origin":"project"});
            super::stamp(&mut value);
            catalog.guides.push(value);
        }
        let mut ids = BTreeSet::new();
        for file in &settings.styles {
            let (path, bytes) = catalog.capture(base, file, "style", 256 * 1024)?;
            let style: StyleFile = serde_json::from_slice(&bytes).map_err(|_| {
                resource_error(
                    "Invalid project style JSON; expected the documented style fields",
                    &path,
                    "style",
                )
            })?;
            if !identifier(&style.id)
                || !ids.insert(style.id.clone())
                || !text(&style.name, 256)
                || !text(&style.version, 64)
                || !text(&style.language, 128)
                || style.description.len() > 4096
                || style.instructions.len() > 64 * 1024
                || !style.tokens.is_object()
                || style.use_cases.len() > 32
                || style.use_cases.iter().any(|s| !text(s, 128))
                || style.guidance.len() > 64
                || style.required_fonts.len() > 64
                || style.required_packs.len() > 64
                || style.license.as_ref().is_some_and(|s| !text(s, 256))
            {
                return Err(invalid(
                    "Project style metadata exceeds the documented limits",
                ));
            }
            for id in &style.guidance {
                if !catalog.guides.iter().any(|guide| guide["id"] == *id)
                    && super::get_guidance(Some(id)).is_err()
                {
                    return Err(invalid("A project style references an unknown guide ID"));
                }
            }
            let (example, source) = catalog.capture(
                path.parent().expect("file parent"),
                &style.example,
                "style_example",
                MAX_FILE,
            )?;
            let source = std::str::from_utf8(&source)
                .map_err(|_| invalid("Project style examples must be UTF-8 .pen documents"))?;
            let document = Document::parse(source.into())
                .map_err(|_| invalid("Invalid project style example document"))?;
            if document.version() != crate::document::DOCUMENT_VERSION {
                return Err(invalid("Project style examples require .pen schema 2.21"));
            }
            let id = format!("project:{}", style.id);
            let mut value = serde_json::to_value(&style).expect("style metadata");
            value["id"] = json!(id);
            value["origin"] = json!("project");
            value["source_path"] = json!(path);
            value["source_hash"] = json!(hash(&bytes));
            value["schema_versions"] = json!(["2.21"]);
            value["example"] = json!({"filename":example,"format":"pen","document":document.value(),"hash":document.revision(),"read_only":true,
                "components":document.locations().keys().filter_map(|id| {let node=document.node(id).ok()?;(node["reusable"]==true).then(|| json!({"node_id":id,"name":node.get("name")}))}).collect::<Vec<_>>()});
            super::stamp(&mut value);
            catalog.examples.insert(
                id,
                path.parent().expect("style parent").join(&style.example),
            );
            catalog.styles.push(value);
        }
        if let Some(id) = &catalog.default_style
            && !catalog.styles.iter().any(|style| style["id"] == *id)
            && super::get_style(id).is_err()
        {
            return Err(invalid("Unknown design.default_style ID"));
        }
        // Trust applies to instructions. Example libraries and font settings
        // remain revision-checked data that can change during ordinary editing.
        catalog.trust_fingerprint=hash(json!({"version":1,"scope":"design_instructions","root":catalog.root,
            "configuration_path":configuration.as_ref().map(|(path,_)|*path),"settings":settings,
            "bindings":catalog.bindings.iter().filter(|(_,_,kind)|*kind!="style_example").collect::<Vec<_>>(),
            "resources":catalog.resources.iter().filter(|file|file.kind!="style_example").map(|file|json!({"path":file.path,"hash":hash(&file.bytes)})).collect::<Vec<_>>()}).to_string().as_bytes());
        catalog.fingerprint=hash(json!({"version":1,"root":catalog.root,"configuration":configuration,
            "settings":settings,"bindings":catalog.bindings,"resources":catalog.resources.iter().map(|file|json!({"path":file.path,"hash":hash(&file.bytes)})).collect::<Vec<_>>()}).to_string().as_bytes());
        catalog.verify_unchanged()?;
        Ok(catalog)
    }

    fn capture(
        &mut self,
        base: &Path,
        path: &Path,
        kind: &'static str,
        limit: usize,
    ) -> Result<(PathBuf, Arc<[u8]>)> {
        if !local_path(path) {
            return Err(invalid("Design resources must be bounded local paths"));
        }
        let requested = base.join(path);
        let canonical = crate::paths::canonical(&requested).map_err(|_| {
            resource_error("Cannot resolve a project design resource", &requested, kind)
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(resource_error(
                "Project design resources must remain inside the project root",
                &requested,
                kind,
            ));
        }
        let mut bytes = Vec::new();
        crate::store::open_regular_file(&canonical)
            .map_err(|_| {
                resource_error(
                    "A project design resource must be a regular file",
                    &requested,
                    kind,
                )
            })?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io(&canonical, e))?;
        if bytes.len() > limit {
            return Err(resource_error(
                "A project design resource exceeds its byte limit",
                &requested,
                kind,
            ));
        }
        let bytes = Arc::<[u8]>::from(bytes);
        self.bindings.push((requested, canonical.clone(), kind));
        if let Some(previous) = self.resources.iter().find(|file| file.path == canonical) {
            if previous.bytes != bytes {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "Design resource changed while being captured",
                ));
            }
        } else {
            if self
                .resources
                .iter()
                .map(|file| file.bytes.len())
                .sum::<usize>()
                + bytes.len()
                > MAX_TOTAL
            {
                return Err(invalid("Project design resources exceed 8 MiB"));
            }
            self.resources.push(Resource {
                path: canonical.clone(),
                bytes: bytes.clone(),
                kind,
            });
        }
        Ok((canonical, bytes))
    }

    pub fn configured(&self) -> bool {
        !self.guides.is_empty() || !self.styles.is_empty() || self.default_style.is_some()
    }
    pub fn metadata(&self) -> Value {
        json!({"available":true,"configured":self.configured(),"trust_required":self.configured(),"trust_scope":"design_instructions","trust_fingerprint":self.trust_fingerprint,"fingerprint":self.fingerprint,"guide_ids":self.guides.iter().map(|v|&v["id"]).collect::<Vec<_>>(),
            "style_ids":self.styles.iter().map(|v|&v["id"]).collect::<Vec<_>>(),"default_style":self.default_style,
            "resources":self.resources.iter().map(|file|json!({"path":file.path,"hash":hash(&file.bytes),"kind":file.kind,"bytes":file.bytes.len()})).collect::<Vec<_>>()})
    }
    pub fn verify_unchanged(&self) -> Result<()> {
        let changed = || {
            Error::new(
                "REVISION_CONFLICT",
                "Project design resources changed; reload the project",
            )
        };
        for (requested, canonical, _) in &self.bindings {
            if crate::paths::canonical(requested).map_err(|_| changed())? != *canonical {
                return Err(changed());
            }
        }
        for file in &self.resources {
            let mut bytes = Vec::new();
            crate::store::open_regular_file(&file.path)
                .map_err(|_| changed())?
                .take(MAX_FILE as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| changed())?;
            if bytes.as_slice() != file.bytes.as_ref() {
                return Err(changed());
            }
        }
        Ok(())
    }

    fn grant_path(&self, data: &Path) -> PathBuf {
        data.join("trusted-projects").join(format!(
            "{}.json",
            hash(self.root.as_os_str().as_encoded_bytes())
        ))
    }
    pub fn trusted(&self, data: &Path) -> Result<bool> {
        if !self.configured() {
            return Ok(true);
        }
        let path = self.grant_path(data);
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
            && grant["scope"] == "design_instructions"
            && grant["fingerprint"] == self.trust_fingerprint)
    }
    pub fn require_trusted(&self, data: &Path) -> Result<()> {
        if self.trusted(data)? {
            return Ok(());
        }
        Err(Error::new("PROJECT_NOT_TRUSTED","Project design instructions require a local user grant. The user must review config show and run config trust for this document; agents must not grant trust themselves")
            .details(self.metadata()))
    }
    pub fn set_trust(&self, data: &Path, expected: Option<&str>, trusted: bool) -> Result<Value> {
        if expected.is_some_and(|expected| expected != self.fingerprint) {
            return Err(Error::new(
                "REVISION_CONFLICT",
                "The project design fingerprint changed before trust was recorded",
            ));
        }
        if !trusted {
            return Self::revoke(&self.root, data);
        }
        self.verify_unchanged()?;
        let path = self.grant_path(data);
        let _lock = crate::store::Store::lock_configuration(&path)?;
        if trusted {
            fs::create_dir_all(path.parent().expect("grant parent"))
                .map_err(|e| Error::io(&path, e))?;
            let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("grant parent"))
                .map_err(|e| Error::io(&path, e))?;
            file.write_all(
                json!({"version":1,"scope":"design_instructions","fingerprint":self.trust_fingerprint})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(|e| Error::io(&path, e))?;
            file.as_file().sync_all().map_err(|e| Error::io(&path, e))?;
            self.verify_unchanged()?;
            crate::cancel::check(Some(&crate::cancel::token()))?;
            file.persist(&path).map_err(|e| Error::io(&path, e.error))?;
            crate::store::sync_directory(path.parent().expect("grant parent"))?;
        } else {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(&path, e)),
            }
        }
        Ok(
            json!({"trusted":trusted,"fingerprint":self.fingerprint,"trust_fingerprint":self.trust_fingerprint,"root":self.root,"grant_path":path}),
        )
    }

    pub fn revoke(root: &Path, data: &Path) -> Result<Value> {
        let root = crate::paths::canonical(root)?;
        let path = data.join("trusted-projects").join(format!(
            "{}.json",
            hash(root.as_os_str().as_encoded_bytes())
        ));
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

    pub fn guidance(&self, id: Option<&str>, data: &Path) -> Result<Value> {
        self.require_trusted(data)?;
        let id = id.unwrap_or("start");
        let mut value = if id.starts_with("project:") {
            self.guides
                .iter()
                .find(|guide| guide["id"] == id)
                .cloned()
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unknown project guide ID"))?
        } else {
            super::get_guidance(Some(id))?
        };
        if self.configured() {
            if id == "start" {
                let mut body = value["body"].as_str().unwrap_or_default().to_string();
                body.push_str("\n\nFollow the user's current request first, then the trusted project_guides included with this response, then the selected style and standard guidance. Tool capabilities and save/validation rules still apply. Styles are starting points for new designs; preserve existing conventions when editing.\n");
                value["body"] = json!(body);
                value["project_guides"] = json!(
                    self.guides
                        .iter()
                        .filter(|guide| guide["automatic"] == true)
                        .collect::<Vec<_>>()
                );
            }
            if value.get("available_guides").is_none() {
                value["available_guides"] = super::get_guidance(None)?["available_guides"].clone();
            }
            let available = value["available_guides"].as_array_mut();
            if let Some(available) = available {
                available.extend(self.guides.iter().map(|guide|json!({"id":guide["id"],"summary":guide["summary"],"version":guide["version"],"language":guide["language"]})));
            }
            value["project_design"] = self.metadata();
            super::stamp(&mut value);
        }
        self.verify_unchanged()?;
        self.require_trusted(data)?;
        crate::query::response(value)
    }
    pub fn style(&self, id: &str, data: &Path) -> Result<Value> {
        self.require_trusted(data)?;
        let value = if id.starts_with("project:") {
            self.styles
                .iter()
                .find(|style| style["id"] == id)
                .cloned()
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unknown project style ID"))?
        } else {
            super::get_style(id)?
        };
        self.verify_unchanged()?;
        self.require_trusted(data)?;
        crate::query::response(value)
    }
    pub fn default_style(&self) -> Option<&str> {
        self.default_style.as_deref()
    }
    pub fn example_path(&self, id: &str, data: &Path) -> Result<&Path> {
        self.require_trusted(data)?;
        self.examples
            .get(id)
            .map(PathBuf::as_path)
            .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unknown project style ID"))
    }
    pub fn styles(
        &self,
        use_case: Option<&str>,
        limit: usize,
        cursor: Option<&str>,
        data: &Path,
    ) -> Result<Value> {
        if !(1..=100).contains(&limit) {
            return Err(Error::new("INVALID_ARGUMENT", "Style limit must be 1..100"));
        }
        self.require_trusted(data)?;
        if !self.configured() {
            return super::list_styles(use_case, limit, cursor);
        }
        let builtin = super::list_styles(None, 100, None)?;
        let mut styles = builtin["styles"].as_array().expect("styles").clone();
        styles.extend(self.styles.iter().map(|style|json!({"id":style["id"],"name":style["name"],"version":style["version"],"description":style["description"],"use_cases":style["use_cases"],"language":style["language"],"required_fonts":style["required_fonts"],"origin":"project"})));
        styles.retain(|style| {
            use_case.is_none_or(|query| {
                style["use_cases"].as_array().is_some_and(|uses| {
                    uses.iter().any(|value| {
                        value
                            .as_str()
                            .is_some_and(|value| value.eq_ignore_ascii_case(query))
                    })
                })
            })
        });
        let identity = hash(
            json!({"project":self.fingerprint,"styles":styles,"use_case":use_case})
                .to_string()
                .as_bytes(),
        );
        let offset = crate::query::page_offset(&identity, cursor, limit, styles.len())?;
        let total = styles.len();
        let page = styles
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let next =
            (offset + page.len() < total).then(|| format!("{identity}:{}", offset + page.len()));
        self.verify_unchanged()?;
        self.require_trusted(data)?;
        crate::query::response(
            json!({"styles":page,"total_matches":total,"next_cursor":next,"project_design":self.metadata()}),
        )
    }
}
