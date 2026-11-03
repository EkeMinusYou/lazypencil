use crate::{
    Error, Result,
    document::{Document, MAX_NODES, hash},
    store::{Store, read_text},
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_DEPENDENCY_DOCUMENTS: usize = 128;
const MAX_PROJECT_BYTES: usize = 128 * 1024 * 1024;
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;

mod bindings;
mod browser_resources;
mod capture;
mod configuration;
mod resolution;
pub(crate) use browser_resources::filesystem as secure_filesystem;
pub use browser_resources::{BrowserResourceRoot, BrowserResources};
mod detach;
mod expansion;
pub(crate) use expansion::{Expansion, MAX_EXPANSION_BYTES};
#[cfg(feature = "rendering")]
mod failed_code;
pub(crate) mod font_files;
#[cfg(feature = "rendering")]
mod generated_refs;
#[cfg(feature = "rendering")]
pub(crate) use failed_code::FailedCodeCapture;
mod browser_context;
mod copy;
mod import;
mod lookup;
mod missing;
mod properties;
use properties::literal_property;
pub(crate) use properties::literal_variable_property;
mod override_context;
mod remote_images;
pub(crate) use override_context::preserve_metadata as preserve_execution_metadata;
mod sampler_images;
mod script_context;
pub(crate) use browser_context::revoke as revoke_copied_browser_context;
pub(crate) use script_context::revoke as revoke_copied_script_context;
mod scope;
mod variables;
pub use variables::VariablesRequest;

#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    pub path: PathBuf,
    pub revision: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability: Option<&'static str>,
}

pub struct Asset {
    pub bytes: Arc<[u8]>,
    pub kind: String,
    pub revision: String,
}

impl Dependency {
    pub(crate) fn is_file(&self) -> bool {
        !matches!(self.kind.as_str(), "remote_image" | "embedded_image")
    }
}

impl Asset {
    fn is_captured_image(&self) -> bool {
        matches!(self.kind.as_str(), "remote_image" | "embedded_image")
    }
}

fn captured_image_kind(path: &Path) -> &'static str {
    if crate::inline_images::is_key(path) {
        "embedded_image"
    } else {
        "remote_image"
    }
}

struct CaptureOptions<'a> {
    cancel: Option<&'a tokio_util::sync::CancellationToken>,
    activity: Arc<crate::store::AssetActivity>,
    remote_images: bool,
    remote_worker: Option<&'a Path>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeAddress {
    pub node_id: String,
    pub instance_path: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedNode {
    pub address: NodeAddress,
    pub source_path: PathBuf,
    pub source_id: String,
    pub component_key: String,
    #[serde(skip_serializing)]
    component_source: PathBuf,
    #[serde(skip_serializing)]
    instance_identity: Option<(PathBuf, String)>,
    pub is_instance: bool,
    pub properties: Map<String, Value>,
    pub property_sources: BTreeMap<String, PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    script_scope: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    script_input_scope: Option<PathBuf>,
    #[serde(skip)]
    script_bindings: Option<bindings::Bound>,
    #[serde(skip)]
    input_context: Option<script_context::Bound>,
    #[serde(skip)]
    browser_scope: Option<PathBuf>,
    #[serde(skip)]
    browser_url_scope: Option<PathBuf>,
    #[cfg(feature = "rendering")]
    #[serde(skip_serializing)]
    pub(crate) script_inputs_resolved: bool,
    pub theme: BTreeMap<String, String>,
    pub children: Vec<ResolvedNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by: Option<NodeAddress>,
    #[serde(skip_serializing)]
    pub icon: Option<Result<Arc<crate::icons::ResolvedIcon>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_metadata: Option<Value>,
}

impl ResolvedNode {
    pub fn id(&self) -> String {
        if self.address.instance_path.is_empty() {
            self.address.node_id.clone()
        } else {
            format!(
                "{}/{}",
                self.address.node_id,
                self.address.instance_path.join("/")
            )
        }
    }

    pub fn kind(&self) -> &str {
        self.properties
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub fn property_source(&self, key: &str) -> &Path {
        self.property_sources
            .get(key)
            .map(PathBuf::as_path)
            .unwrap_or(&self.source_path)
    }

    pub fn script_origin(&self) -> &Path {
        self.script_scope
            .as_deref()
            .unwrap_or_else(|| self.property_source("scriptUri"))
    }

    pub fn browser_origin(&self) -> &Path {
        self.browser_scope
            .as_deref()
            .unwrap_or_else(|| self.browser_url_origin())
    }

    fn browser_url_origin(&self) -> &Path {
        self.browser_url_scope
            .as_deref()
            .unwrap_or_else(|| self.property_source("url"))
    }

    pub fn script_input_origin(&self) -> &Path {
        self.input_context
            .as_ref()
            .map(|context| context.origin.as_path())
            .or(self.script_input_scope.as_deref())
            .unwrap_or_else(|| self.property_source("inputs"))
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn script_bindings_revision(&self) -> Option<String> {
        if self.input_context.is_some() {
            return Some(hash(
                json!({"output":self.script_bindings,"input":self.input_context})
                    .to_string()
                    .as_bytes(),
            ));
        }
        self.script_bindings.as_ref().map(|bindings| {
            hash(
                serde_json::to_string(bindings)
                    .expect("script bindings")
                    .as_bytes(),
            )
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedScene {
    pub revision: String,
    pub dependency_revision: String,
    pub dependencies: Vec<Dependency>,
    pub roots: Vec<ResolvedNode>,
}

pub struct Project {
    root: PathBuf,
    documents: BTreeMap<PathBuf, Document>,
    imports: BTreeMap<PathBuf, BTreeMap<String, PathBuf>>,
    sampler_images: BTreeMap<PathBuf, BTreeMap<String, String>>,
    browser_resources: BTreeMap<PathBuf, browser_resources::Resources>,
    browser_document_bytes: BTreeMap<PathBuf, Arc<[u8]>>,
    allowed_roots: Vec<PathBuf>,
    assets: BTreeMap<PathBuf, Asset>,
    missing_assets: BTreeMap<(PathBuf, String), missing::MissingAsset>,
    runtime_assets: BTreeSet<PathBuf>,
    runtime_refs: BTreeMap<String, (PathBuf, String)>,
    byte_count: usize,
    resolution_error: Option<Error>,
    icons: crate::icons::Registry,
    icon_data_dir: Option<PathBuf>,
    configuration: Option<crate::config::Snapshot>,
    font_files: Vec<PathBuf>,
    font_sources: BTreeMap<PathBuf, font_files::FontFile>,
    font_bindings: Vec<(PathBuf, PathBuf)>,
    font_settings: crate::config::FontSettings,
    font_lock_capture: bool,
    overlay: crate::managed::Overlay,
    overlay_published: bool,
    file_bindings: std::sync::Mutex<BTreeMap<PathBuf, PathBuf>>,
    cancel: Option<tokio_util::sync::CancellationToken>,
    remote_images_allowed: bool,
    remote_worker: Option<PathBuf>,
    remote_deadline: Option<std::time::Instant>,
    captured_image_failures: BTreeMap<PathBuf, Error>,
    // Fields drop in declaration order. Release the lease after captured inputs.
    _asset_activity: Option<Arc<crate::store::AssetActivity>>,
}

impl Project {
    pub(crate) fn set_cancellation(
        &mut self,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) {
        self.cancel = cancel.cloned();
    }

    pub(crate) fn check_cancelled(&self) -> Result<()> {
        crate::cancel::check(self.cancel.as_ref())
    }

    pub(crate) fn local(document: Document) -> Result<Self> {
        browser_resources::validate(&document)?;
        if document
            .value()
            .get("imports")
            .and_then(Value::as_object)
            .is_some_and(|imports| !imports.is_empty())
        {
            return Err(Error::new(
                "INVALID_OPERATION",
                "This operation needs a project path to resolve imported components",
            ));
        }
        let root = PathBuf::new();
        let byte_count = document.text().len();
        let sampler_images = [(root.clone(), sampler_images::parse(&document)?)]
            .into_iter()
            .collect();
        Ok(Self {
            _asset_activity: None,
            root: root.clone(),
            documents: [(root.clone(), document)].into_iter().collect(),
            imports: [(root, BTreeMap::new())].into_iter().collect(),
            sampler_images,
            browser_resources: BTreeMap::new(),
            browser_document_bytes: BTreeMap::new(),
            allowed_roots: Vec::new(),
            assets: BTreeMap::new(),
            missing_assets: BTreeMap::new(),
            runtime_assets: BTreeSet::new(),
            runtime_refs: BTreeMap::new(),
            byte_count,
            resolution_error: None,
            icons: crate::icons::Registry::default(),
            icon_data_dir: None,
            configuration: None,
            font_files: vec![],
            font_sources: BTreeMap::new(),
            font_bindings: vec![],
            font_settings: Default::default(),
            font_lock_capture: false,
            overlay: BTreeMap::new(),
            overlay_published: true,
            cancel: None,
            remote_images_allowed: false,
            remote_worker: None,
            remote_deadline: None,
            captured_image_failures: BTreeMap::new(),
            file_bindings: Default::default(),
        })
    }

    pub fn expanded_node(&self, address: &NodeAddress) -> Result<ResolvedNode> {
        self.expanded_library_node(None, address)
    }

    pub fn library_document(&self, alias: Option<&str>) -> Result<&Document> {
        Ok(&self.documents[self.library_path(alias)?])
    }

    fn imported_path(&self, origin: &Path, alias: &str) -> Result<Option<&PathBuf>> {
        let path = self
            .imports
            .get(origin)
            .and_then(|imports| imports.get(alias));
        if path.is_none()
            && self.documents[origin].value()["imports"]
                .get(alias)
                .is_some()
        {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "The declared library is missing",
            ));
        }
        Ok(path)
    }

    fn library_path(&self, alias: Option<&str>) -> Result<&PathBuf> {
        let mut path = &self.root;
        if let Some(alias) = alias {
            for part in alias.split(':') {
                path = self.imported_path(path, part)?.ok_or_else(|| {
                    Error::new("INVALID_ARGUMENT", "Unknown declared library alias")
                })?;
            }
        }
        Ok(path)
    }

    pub fn expanded_library_node(
        &self,
        alias: Option<&str>,
        address: &NodeAddress,
    ) -> Result<ResolvedNode> {
        let origin = self.library_path(alias)?;
        let node = self.documents[origin].node(&address.node_id)?;
        if !address.instance_path.is_empty() && node["type"] != "ref" {
            return Err(Error::new(
                "INVALID_OPERATION",
                "An instance path requires a root instance",
            ));
        }
        let mut count = Expansion::default();
        let mut expanded = self.expand(origin, node, None, &[], &mut Vec::new(), &mut count)?;
        assign_addresses(&mut expanded, None, &[]);
        if address.instance_path.is_empty() {
            return Ok(expanded);
        }
        let path = address
            .instance_path
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        find_descendant_mut(&mut expanded, &path)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "INVALID_OPERATION",
                    "The requested inherited descendant does not exist",
                )
            })
    }

    pub fn expanded_library_roots(&self, alias: Option<&str>) -> Result<Vec<ResolvedNode>> {
        let origin = self.library_path(alias)?;
        let mut count = Expansion::default();
        self.documents[origin].value()["children"]
            .as_array()
            .expect("parsed document")
            .iter()
            .map(|node| {
                let mut expanded =
                    self.expand(origin, node, None, &[], &mut Vec::new(), &mut count)?;
                assign_addresses(&mut expanded, None, &[]);
                Ok(expanded)
            })
            .collect()
    }

    pub(crate) fn validate_structure(&self) -> Result<()> {
        let mut count = Expansion::default();
        for node in self.root_document().value()["children"]
            .as_array()
            .expect("parsed document")
        {
            self.expand(&self.root, node, None, &[], &mut Vec::new(), &mut count)?;
        }
        Ok(())
    }

    fn capture_detached_scripts(&mut self, address: &NodeAddress) -> Result<()> {
        fn codes(node: &ResolvedNode, files: &mut BTreeSet<(PathBuf, String)>) {
            if node.kind() == "script"
                && let Some(uri) = node.properties.get("scriptUri").and_then(Value::as_str)
            {
                files.insert((node.property_source("scriptUri").into(), uri.into()));
            }
            for child in &node.children {
                codes(child, files);
            }
        }
        let root = self.expanded_node(&NodeAddress {
            node_id: address.node_id.clone(),
            instance_path: vec![],
        })?;
        let mut files = BTreeSet::new();
        codes(&root, &mut files);
        // Projection visits raw definitions before applying their outer overrides.
        // Their headers are needed to relocate saved controls even when another
        // code URI is effective in the final instance. Traverse only this source
        // closure; capture bytes without executing any JavaScript.
        fn raw_codes(
            project: &Project,
            origin: &Path,
            value: &Value,
            files: &mut BTreeSet<(PathBuf, String)>,
            pending: &mut VecDeque<(PathBuf, String)>,
            visited: &mut BTreeSet<(PathBuf, String)>,
            count: &mut usize,
        ) -> Result<()> {
            project.check_cancelled()?;
            if let Some(id) = value.get("id").and_then(Value::as_str)
                && !visited.insert((origin.into(), id.into()))
            {
                return Ok(());
            }
            *count += 1;
            if *count > MAX_NODES {
                return Err(invalid("Detached source projection exceeds its node limit"));
            }
            if let Some(uri) = value.get("scriptUri").and_then(Value::as_str) {
                files.insert((origin.into(), uri.into()));
            }
            if value["type"] == "ref"
                && let Some(reference) = value.get("ref").and_then(Value::as_str)
            {
                let (source, definition) = project.lookup_node(origin, reference)?;
                pending.push_back((
                    source,
                    definition["id"]
                        .as_str()
                        .expect("indexed definition")
                        .into(),
                ));
            }
            for child in value
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .chain(
                    value
                        .get("descendants")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(Map::values),
                )
            {
                raw_codes(project, origin, child, files, pending, visited, count)?;
            }
            Ok(())
        }
        let mut pending = VecDeque::from([(self.root.clone(), address.node_id.clone())]);
        let mut visited = BTreeSet::new();
        let mut count = 0;
        while let Some((origin, id)) = pending.pop_front() {
            raw_codes(
                self,
                &origin,
                self.documents[&origin].node(&id)?,
                &mut files,
                &mut pending,
                &mut visited,
                &mut count,
            )?;
        }
        for (origin, uri) in files {
            self.check_cancelled()?;
            let path = self.resolve_file(&origin, &uri)?;
            self.capture_asset(&path, "script")?;
        }
        Ok(())
    }

    /// Materialize one instance boundary, including overrides from its ancestors.
    pub(crate) fn detached_template(&mut self, address: &NodeAddress) -> Result<Value> {
        if !self.expanded_node(address)?.is_instance {
            return Err(Error::new(
                "INVALID_OPERATION",
                "Detach requires an instance",
            ));
        }
        self.capture_detached_scripts(address)?;
        let instance =
            self.relocate_detached_node(&self.root, self.root_document().node(&address.node_id)?)?;
        let (mut value, mut aliases) = self.detach_instance_projection(&instance, 0)?;
        for (index, _) in address.instance_path.iter().enumerate() {
            let original = self.expanded_node(&NodeAddress {
                node_id: address.node_id.clone(),
                instance_path: address.instance_path[..=index].to_vec(),
            })?;
            let scoped = self.detached_id(&original.component_source, &original.component_key);
            let mut key = scoped.as_str();
            for _ in 0..128 {
                if let Some(next) = aliases.get(key) {
                    key = next;
                } else {
                    break;
                }
            }
            let instance = find_stored_descendant(&mut value, key)
                .ok_or_else(|| invalid("Missing inherited instance during detach"))?;
            (value, aliases) = self.detach_instance_projection(instance, 0)?;
        }
        self.bind_detached_internal_refs(&mut value)?;
        let scene = self.resolve_for_edit()?;
        fn find<'a>(
            node: &'a ResolvedNode,
            address: &NodeAddress,
            inherited: &'a BTreeMap<String, String>,
        ) -> Option<(&'a ResolvedNode, &'a BTreeMap<String, String>)> {
            if node.address.node_id == address.node_id
                && node.address.instance_path == address.instance_path
            {
                return Some((node, inherited));
            }
            node.children
                .iter()
                .find_map(|child| find(child, address, &node.theme))
        }
        let default_theme = self.default_theme(&self.root)?;
        let (original, inherited) = scene
            .roots
            .iter()
            .find_map(|node| find(node, address, &default_theme))
            .ok_or_else(|| invalid("Missing selected instance in resolved scene"))?;
        self.align_detached_themes(&mut value, original, inherited)?;
        Ok(value)
    }

    fn detach_instance_projection(
        &self,
        instance: &Value,
        depth: usize,
    ) -> Result<(Value, BTreeMap<String, String>)> {
        if depth > 128 || instance["type"] != "ref" {
            return Err(Error::new(
                "INVALID_OPERATION",
                "Detach requires an instance within the reference depth limit",
            ));
        }
        let reference = instance["ref"]
            .as_str()
            .ok_or_else(|| invalid("An instance requires a reference"))?;
        let (source, definition) = self.lookup_node(&self.root, reference)?;
        let definition = self.relocate_detached_node(&source, &definition)?;
        let (mut value, mut aliases) = if definition["type"] == "ref" {
            self.detach_instance_projection(&definition, depth + 1)?
        } else {
            (definition, BTreeMap::new())
        };
        value
            .as_object_mut()
            .expect("node object")
            .remove("reusable");
        merge_detached_properties(&mut value, instance)?;
        value["id"] = instance["id"].clone();
        if let Some(overrides) = instance.get("descendants").and_then(Value::as_object) {
            let mut entries = overrides.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(path, _)| path.split('/').count());
            for (path, changes) in entries {
                let (first, remaining) = path
                    .split_once('/')
                    .map_or((path.as_str(), None), |(a, b)| (a, Some(b)));
                let mut key = first;
                for _ in 0..128 {
                    if let Some(next) = aliases.get(key) {
                        key = next;
                    } else {
                        break;
                    }
                }
                let target = find_stored_descendant(&mut value, key)
                    .ok_or_else(|| invalid(format!("Missing descendant override path {path:?}")))?;
                if let Some(remaining) = remaining {
                    if target["type"] != "ref" {
                        return Err(invalid(
                            "Only nested instances introduce an override path prefix",
                        ));
                    }
                    let target = target.as_object_mut().expect("node object");
                    let descendants = target
                        .entry("descendants")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .ok_or_else(|| invalid("descendants must be an object"))?;
                    if changes.get("type").is_some() || !descendants.contains_key(remaining) {
                        descendants.insert(remaining.into(), changes.clone());
                    } else {
                        merge_detached_properties(
                            descendants.get_mut(remaining).expect("existing override"),
                            changes,
                        )?;
                    }
                } else if changes.get("type").is_some() {
                    let old = target["id"].as_str().expect("node ID").to_string();
                    *target = changes.clone();
                    aliases.insert(
                        old,
                        target["id"].as_str().expect("replacement ID").to_string(),
                    );
                } else {
                    merge_detached_properties(target, changes)?;
                }
            }
        }
        Ok((value, aliases))
    }
    pub fn root_document(&self) -> &Document {
        &self.documents[&self.root]
    }

    pub fn resolution_error(&self) -> Option<&Error> {
        self.resolution_error.as_ref()
    }

    pub fn components(&self) -> Vec<Value> {
        let mut result = Vec::new();
        let mut aliases = BTreeMap::from([(self.root.clone(), String::new())]);
        let mut queue = VecDeque::from([self.root.clone()]);
        while let Some(path) = queue.pop_front() {
            let prefix = aliases[&path].clone();
            for (alias, target) in self.imports.get(&path).into_iter().flatten() {
                if !aliases.contains_key(target) {
                    let alias = if prefix.is_empty() {
                        alias.clone()
                    } else {
                        format!("{prefix}:{alias}")
                    };
                    aliases.insert(target.clone(), alias);
                    queue.push_back(target.clone());
                }
            }
        }
        for (path, document) in &self.documents {
            let alias = aliases.get(path).filter(|alias| !alias.is_empty());
            for id in document.locations().keys() {
                let node = document.node(id).expect("indexed component");
                if node["reusable"] == true {
                    result.push(
                        json!({"node_id":id,"name":node.get("name"),"type":node["type"],
                        "source_path":path,"library_alias":alias,"local":path==&self.root,
                        "slot":node.get("slot")}),
                    );
                }
            }
        }
        result
    }

    pub fn dependencies(&self) -> Vec<Dependency> {
        let mut dependencies = self
            .documents
            .iter()
            .filter(|(path, _)| *path != &self.root)
            .map(|(path, document)| Dependency {
                path: path.clone(),
                revision: document.revision(),
                kind: "document".into(),
                version: Some(document.version().into()),
                availability: None,
            })
            .chain(
                self.assets
                    .iter()
                    .filter(|(path, _)| !self.runtime_assets.contains(*path))
                    .map(|(path, asset)| Dependency {
                        path: path.clone(),
                        revision: asset.revision.clone(),
                        kind: asset.kind.clone(),
                        version: None,
                        availability: None,
                    }),
            )
            .chain(self.missing_assets.values().map(|asset| asset.dependency()))
            .chain(self.captured_image_failures.iter().map(|(path, error)| {
                Dependency {
                    path: path.clone(),
                    revision: hash(
                        json!({"path":path,"availability":"missing","error":error})
                            .to_string()
                            .as_bytes(),
                    ),
                    kind: captured_image_kind(path).into(),
                    version: None,
                    availability: Some("missing"),
                }
            }))
            .chain(self.font_sources.values().map(|font| Dependency {
                path: font.path.clone(),
                revision: font.revision.clone(),
                kind: "font".into(),
                version: None,
                availability: None,
            }))
            .collect::<Vec<_>>();
        dependencies.sort_by(|a, b| a.path.cmp(&b.path));
        dependencies
    }

    pub fn dependency_revision(&self) -> String {
        if let Some(resources) = browser_resources::revision(self) {
            return hash(
                &serde_json::to_vec(&(self.dependencies(), resources))
                    .expect("serializable browser dependencies"),
            );
        }
        hash(
            serde_json::to_vec(&self.dependencies())
                .expect("serializable dependencies")
                .as_slice(),
        )
    }

    pub fn runtime_dependencies(&self) -> Vec<Dependency> {
        self.runtime_assets
            .iter()
            .map(|path| {
                let asset = &self.assets[path];
                Dependency {
                    path: path.clone(),
                    revision: asset.revision.clone(),
                    kind: asset.kind.clone(),
                    version: None,
                    availability: None,
                }
            })
            .collect()
    }

    pub fn runtime_dependency_revision(&self) -> String {
        hash(&serde_json::to_vec(&self.runtime_dependencies()).expect("serializable dependencies"))
    }

    pub fn require_runtime_dependencies(&self, expected: &str) -> Result<()> {
        let actual = self.runtime_dependency_revision();
        if expected != actual {
            return Err(Error::new("REVISION_CONFLICT", "A runtime dependency changed; obtain a fresh calculation snapshot")
                .details(json!({"expected_runtime_dependency_revision":expected,"actual_runtime_dependency_revision":actual})));
        }
        Ok(())
    }

    pub fn require_dependencies(&self, expected: Option<&str>) -> Result<()> {
        let actual = self.dependency_revision();
        if !self.dependencies().is_empty() && expected.is_none() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "This document has external dependencies; supply expected_dependency_revision",
            ));
        }
        if expected.is_some_and(|e| e != actual) {
            return Err(Error::new("REVISION_CONFLICT","A dependency changed; re-read the dependency revision")
                .details(json!({"expected_dependency_revision":expected,"actual_dependency_revision":actual})));
        }
        Ok(())
    }

    pub fn verify_unchanged(&self) -> Result<()> {
        self.check_cancelled()?;
        for asset in self.missing_assets.values() {
            self.check_cancelled()?;
            asset.path.verify()?;
        }
        let bindings = self
            .file_bindings
            .lock()
            .map_err(|_| invalid("Cannot access project file bindings"))?
            .clone();
        for (requested, expected) in bindings {
            self.check_cancelled()?;
            if !self.overlay_published && self.overlay.contains_key(&expected) {
                continue;
            }
            if !crate::paths::canonical(&requested).is_ok_and(|actual| actual == expected) {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "A library or asset path changed its target; reload the project",
                ));
            }
        }
        if let Some(configuration) = &self.configuration {
            configuration.verify_unchanged()?;
        }
        for (requested, canonical) in &self.font_bindings {
            if !crate::paths::canonical(requested).is_ok_and(|current| current == *canonical) {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "A configured font path changed; reload the project",
                ));
            }
        }
        self.icons.verify_unchanged()?;
        for font in self.font_sources.values() {
            if !self.overlay_published && self.overlay.contains_key(&font.path) {
                continue;
            }
            self.verify_published_overlay(&font.path)?;
            font.verify(self.cancel.as_ref())?;
        }
        self.verify_browser_resources()?;
        for (path, document) in &self.documents {
            self.check_cancelled()?;
            if path == &self.root {
                continue;
            }
            if !self.overlay_published && self.overlay.contains_key(path) {
                continue;
            }
            self.verify_published_overlay(path)?;
            if hash(read_text(path)?.as_bytes()) != document.revision() {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    format!(
                        "Dependency changed during the operation: {}",
                        path.display()
                    ),
                ));
            }
        }
        let browser_paths = browser_resources::protected_paths(self);
        for (path, asset) in &self.assets {
            self.check_cancelled()?;
            if asset.is_captured_image() {
                continue;
            }
            if !self.overlay_published && self.overlay.contains_key(path) {
                continue;
            }
            self.verify_published_overlay(path)?;
            let revision = if browser_paths.contains(path.as_path()) {
                browser_resources::file_revision(path, self.cancel.as_ref())?
            } else {
                crate::store::content::file_hash(path, MAX_ASSET_BYTES, self.cancel.as_ref())?
            };
            if revision != asset.revision {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    format!("Asset changed during the operation: {}", path.display()),
                ));
            }
        }
        Ok(())
    }

    pub fn asset(&self, path: &Path) -> Result<&Asset> {
        self.assets.get(path).ok_or_else(|| {
            Error::new(
                "ASSET_NOT_FOUND",
                "Asset was not captured in this project snapshot",
            )
        })
    }

    pub fn capture_asset(&mut self, path: &Path, kind: &str) -> Result<()> {
        self.check_cancelled()?;
        if self.assets.get(path).is_some_and(Asset::is_captured_image) {
            return if kind == "image" {
                Ok(())
            } else {
                Err(Error::new(
                    "ASSET_NOT_FOUND",
                    "Captured images cannot be used as code or local files",
                ))
            };
        }
        if let Some(file) = self.overlay.get(path).cloned() {
            if !self.assets.contains_key(path) {
                let kind = if let Some(font) = self.font_sources.remove(path) {
                    self.byte_count -= font.length;
                    "font"
                } else {
                    kind
                };
                self.capture_bytes(path.to_owned(), kind, file.bytes.clone())?;
            }
            return Ok(());
        }
        let path = crate::paths::canonical(path)?;
        if self
            .assets
            .get(&path)
            .is_some_and(|asset| asset.kind == "font_lock")
            && kind != "font_lock"
        {
            return Err(Error::new(
                "CONFIGURATION_ERROR",
                "Font lockfile must not alias another project asset",
            ));
        }
        if let Some(font) = self.font_sources.get(&path) {
            let bytes = font.read(self.cancel.as_ref())?;
            self.byte_count -= font.length;
            self.font_sources.remove(&path);
            self.capture_bytes(path.clone(), "font", bytes)?;
        }
        self.runtime_assets.remove(&path);
        if let Some(asset) = self.assets.get_mut(&path) {
            if asset.kind == "browser_resource" && kind != "browser_resource" {
                asset.kind = kind.into();
            }
            return Ok(());
        }
        let bytes = read_binary(&path)?;
        self.capture_bytes(path, kind, bytes.into())
    }

    fn capture_bytes(&mut self, path: PathBuf, kind: &str, bytes: Arc<[u8]>) -> Result<()> {
        self.check_cancelled()?;
        self.byte_count += bytes.len();
        if self.byte_count > MAX_PROJECT_BYTES {
            return Err(invalid("Project dependencies exceed 128 MiB"));
        }
        let revision = hash(&bytes);
        self.assets.insert(
            path,
            Asset {
                bytes,
                kind: kind.into(),
                revision,
            },
        );
        Ok(())
    }

    fn reference_path(&self, origin: &Path, uri: &str) -> Result<PathBuf> {
        if uri
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
        {
            return crate::paths::local_file_url(uri);
        }
        if crate::inline_images::is_data(uri) || uri.contains("://") {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "Only explicit local file references are allowed here",
            ));
        }
        Ok(origin
            .parent()
            .ok_or_else(|| invalid("Missing reference base directory"))?
            .join(uri))
    }

    pub fn resolve_file(&self, origin: &Path, uri: &str) -> Result<PathBuf> {
        let target = self.reference_path(origin, uri)?;
        let staged = if self.overlay.is_empty() {
            target.clone()
        } else {
            crate::managed::normalize(&target)?
        };
        let canonical = if let Some(file) = self.overlay.get(&staged) {
            file.for_document(&self.root)?;
            self.file_bindings
                .lock()
                .map_err(|_| invalid("Cannot access staged bindings"))?
                .insert(target.clone(), staged.clone());
            staged
        } else {
            let canonical = crate::paths::canonical(&target)?;
            let mut bindings = self
                .file_bindings
                .lock()
                .map_err(|_| invalid("Cannot access project file bindings"))?;
            if bindings
                .get(&target)
                .is_some_and(|expected| expected != &canonical)
            {
                return Err(Error::new(
                    "REVISION_CONFLICT",
                    "A library or asset path changed while resolving the project",
                ));
            }
            bindings.insert(target, canonical.clone());
            canonical
        };
        if !self
            .allowed_roots
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "A reference escapes the allowed project roots",
            ));
        }
        Ok(canonical)
    }

    pub(crate) fn live_dependencies(&self) -> Vec<Dependency> {
        self.dependencies()
            .into_iter()
            .filter(|d| self.overlay_published || !self.overlay.contains_key(&d.path))
            .collect()
    }

    pub(crate) fn publish_overlay(&mut self) -> Result<()> {
        self.overlay_published = true;
        self.verify_unchanged()
    }

    fn verify_published_overlay(&self, path: &Path) -> Result<()> {
        if self.overlay_published
            && let Some(file) = self.overlay.get(path)
        {
            file.verify(true)?;
        }
        Ok(())
    }

    pub(crate) fn read_identity(&self) -> Result<String> {
        let bindings = self
            .file_bindings
            .lock()
            .map_err(|_| invalid("Cannot access project file bindings"))?;
        Ok(hash(json!({"dependencies":self.dependency_revision(),"bindings":bindings.iter().collect::<Vec<_>>(),"configuration_candidates":self.configuration_lock_paths()}).to_string().as_bytes()))
    }

    pub(crate) fn captured_bytes(&self) -> usize {
        self.byte_count
    }
}

fn find_descendant_mut<'a>(
    root: &'a mut ResolvedNode,
    path: &[&str],
) -> Option<&'a mut ResolvedNode> {
    let (first, rest) = path.split_first()?;
    for child in &mut root.children {
        if child.component_key == *first {
            return if rest.is_empty() {
                Some(child)
            } else if child.is_instance {
                find_descendant_mut(child, rest)
            } else {
                None
            };
        }
        if !child.is_instance
            && let Some(found) = find_descendant_mut(child, path)
        {
            return Some(found);
        }
    }
    None
}

fn assign_addresses(node: &mut ResolvedNode, instance: Option<&str>, prefix: &[String]) {
    if let Some(instance) = instance {
        let mut path = prefix.to_vec();
        path.push(node.component_key.clone());
        node.address = NodeAddress {
            node_id: instance.into(),
            instance_path: path,
        };
    }
    let root = if node.is_instance {
        Some(node.address.node_id.clone())
    } else {
        instance.map(str::to_string)
    };
    let child_prefix = if node.is_instance {
        node.address.instance_path.clone()
    } else {
        prefix.to_vec()
    };
    for child in &mut node.children {
        assign_addresses(child, root.as_deref(), &child_prefix);
    }
}

#[cfg(feature = "rendering")]
fn assign_generated_ids(
    node: &mut Value,
    counter: &mut usize,
    ids: &mut BTreeSet<String>,
    names: &mut BTreeMap<String, String>,
) -> Result<()> {
    if let Some(id) = node.get("id") {
        let id = id
            .as_str()
            .filter(|id| !id.is_empty() && !id.contains('/'))
            .ok_or_else(|| invalid("Generated IDs must be nonempty strings without '/'"))?;
        if !ids.insert(id.into()) {
            return Err(invalid("Duplicate ID in script output"));
        }
        names.insert(format!("generated_node_{}", *counter), id.into());
    }
    node["id"] = json!(format!("generated_node_{}", *counter));
    *counter += 1;
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            assign_generated_ids(child, counter, ids, names)?;
        }
    }
    if node["type"] == "ref" {
        assign_generated_override_ids(node, counter, ids, names)?;
    }
    Ok(())
}

#[cfg(feature = "rendering")]
fn assign_generated_override_ids(
    node: &mut Value,
    counter: &mut usize,
    ids: &mut BTreeSet<String>,
    names: &mut BTreeMap<String, String>,
) -> Result<()> {
    for changes in node
        .get_mut("descendants")
        .and_then(Value::as_object_mut)
        .into_iter()
        .flat_map(Map::values_mut)
    {
        if changes.get("type").is_some() {
            assign_generated_ids(changes, counter, ids, names)?;
        } else {
            if let Some(children) = changes.get_mut("children").and_then(Value::as_array_mut) {
                for child in children {
                    assign_generated_ids(child, counter, ids, names)?;
                }
            }
            assign_generated_override_ids(changes, counter, ids, names)?;
        }
    }
    Ok(())
}

#[cfg(feature = "rendering")]
fn mark_generated(node: &mut ResolvedNode, owner: &NodeAddress, prefix: &str, counter: &mut usize) {
    node.address = NodeAddress {
        node_id: format!("{prefix}/$generated/{}", *counter),
        instance_path: Vec::new(),
    };
    node.generated_by = Some(owner.clone());
    *counter += 1;
    for child in &mut node.children {
        mark_generated(child, owner, prefix, counter);
    }
}

fn find_stored_descendant<'a>(node: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    for child in node.get_mut("children")?.as_array_mut()? {
        if child["id"] == id {
            return Some(child);
        }
        if child["type"] != "ref"
            && let Some(found) = find_stored_descendant(child, id)
        {
            return Some(found);
        }
    }
    None
}

fn merge_detached_properties(target: &mut Value, changes: &Value) -> Result<()> {
    let target = target
        .as_object_mut()
        .ok_or_else(|| invalid("A node or override must be an object"))?;
    for (key, value) in changes
        .as_object()
        .ok_or_else(|| invalid("An override must be an object"))?
    {
        if matches!(key.as_str(), "id" | "type" | "ref" | "descendants") {
            continue;
        }
        if key == "theme" {
            let theme = target
                .entry(key.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| invalid("theme must be an object"))?;
            theme.extend(
                value
                    .as_object()
                    .ok_or_else(|| invalid("theme must be an object"))?
                    .clone(),
            );
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("INVALID_DOCUMENT", message)
}

fn read_binary(path: &Path) -> Result<Vec<u8>> {
    if !std::fs::metadata(path)
        .map_err(|e| Error::io(path, e))?
        .is_file()
    {
        return Err(Error::new(
            "INVALID_DOCUMENT",
            "An asset must be a regular file",
        ));
    }
    let file = crate::store::open_regular_file(path)?;
    crate::store::content::read_bounded(
        file,
        path,
        MAX_ASSET_BYTES,
        None,
        invalid("An asset exceeds 64 MiB"),
    )
}

fn collect_icon_libraries(node: &ResolvedNode, libraries: &mut BTreeSet<String>) {
    if node.kind() == "icon" {
        libraries.insert(
            node.properties
                .get("library")
                .and_then(Value::as_str)
                .unwrap_or("lucide")
                .to_string(),
        );
    }
    for child in &node.children {
        collect_icon_libraries(child, libraries);
    }
}

fn collect_resolved_assets(
    node: &ResolvedNode,
    output: &mut Vec<(PathBuf, String, String)>,
) -> Result<()> {
    if node.properties.get("enabled") == Some(&Value::Bool(false)) {
        return Ok(());
    }
    for key in ["fill", "stroke", "effect", "scriptUri"] {
        if let Some(value) = node.properties.get(key) {
            let property = Value::Object([(key.to_string(), value.clone())].into_iter().collect());
            collect_assets(&property, node.property_source(key), output)?;
        }
    }
    for child in &node.children {
        collect_resolved_assets(child, output)?;
    }
    Ok(())
}

fn collect_assets(
    value: &Value,
    origin: &Path,
    output: &mut Vec<(PathBuf, String, String)>,
) -> Result<()> {
    if let Some(uri) = value.get("scriptUri") {
        let uri = uri
            .as_str()
            .ok_or_else(|| invalid("scriptUri must be a string"))?;
        if !uri.is_empty() {
            output.push((origin.into(), uri.into(), "script".into()));
        }
    }
    for key in ["fill", "stroke", "effect"] {
        if let Some(payload) = value.get(key) {
            for fill in property_values(payload) {
                if fill["enabled"] == false {
                    continue;
                }
                let kind = match fill["type"].as_str() {
                    Some("image") => "image",
                    Some("shader") => "shader",
                    _ => continue,
                };
                if let Some(uri) = fill.get("url") {
                    let uri = uri
                        .as_str()
                        .ok_or_else(|| invalid("An asset URL must be a string"))?;
                    if !uri.is_empty() {
                        output.push((origin.into(), uri.into(), kind.into()));
                    }
                }
                if kind == "shader"
                    && let Some(uniforms) = fill.get("uniforms").and_then(Value::as_object)
                {
                    for uniform in uniforms
                        .values()
                        .flat_map(property_values)
                        .filter_map(shader_asset_uri)
                    {
                        output.push((origin.into(), uniform.into(), "image".into()));
                    }
                }
            }
        }
    }
    for child in value
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_assets(child, origin, output)?;
    }
    for changes in value
        .get("descendants")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(Map::values)
    {
        collect_assets(changes, origin, output)?;
    }
    Ok(())
}

fn shader_asset_uri(value: &Value) -> Option<&str> {
    value.as_str().filter(|uri| {
        !uri.is_empty() && !uri.starts_with(['#', '$']) && !uri.starts_with("pencil:")
    })
}

fn property_values(value: &Value) -> &[Value] {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value))
}

fn property_values_mut(value: &mut Value) -> &mut [Value] {
    match value {
        Value::Array(values) => values,
        value => std::slice::from_mut(value),
    }
}
