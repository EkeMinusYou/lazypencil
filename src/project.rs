use crate::{
    Error, Result,
    document::{Document, MAX_NODES, hash},
    store::{Store, read_text},
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_DEPENDENCY_DOCUMENTS: usize = 128;
const MAX_PROJECT_BYTES: usize = 128 * 1024 * 1024;
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;

mod bindings;
mod browser_resources;
pub use browser_resources::{BrowserResourceRoot, BrowserResources};
mod detach;
#[cfg(feature = "rendering")]
mod failed_code;
pub(crate) mod font_files;
#[cfg(feature = "rendering")]
mod generated_refs;
#[cfg(feature = "rendering")]
pub(crate) use failed_code::FailedCodeCapture;
mod browser_context;
mod import;
mod missing;
mod override_context;
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
        let mut count = 0;
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
        let mut count = 0;
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
        let mut count = 0;
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
    pub fn load(store: &Store) -> Result<Self> {
        Self::from_document_with_data(store.path(), store.read()?, &[], Some(store.data_dir()))
    }

    pub fn from_document(
        path: &Path,
        document: Document,
        additional_roots: &[PathBuf],
    ) -> Result<Self> {
        let data = crate::store::default_data_dir().ok();
        Self::from_document_with_data(path, document, additional_roots, data.as_deref())
    }
    pub fn from_document_with_data(
        path: &Path,
        document: Document,
        additional_roots: &[PathBuf],
        data: Option<&Path>,
    ) -> Result<Self> {
        Self::from_document_with_overlay(path, document, additional_roots, data, &BTreeMap::new())
    }

    pub(crate) fn from_document_with_overlay(
        path: &Path,
        document: Document,
        additional_roots: &[PathBuf],
        data: Option<&Path>,
        overlay: &crate::managed::Overlay,
    ) -> Result<Self> {
        Self::from_document_with_overlay_and_cancellation(
            path,
            document,
            additional_roots,
            data,
            overlay,
            None,
        )
    }

    pub(crate) fn from_document_with_overlay_and_cancellation(
        path: &Path,
        document: Document,
        additional_roots: &[PathBuf],
        data: Option<&Path>,
        overlay: &crate::managed::Overlay,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Self> {
        crate::cancel::check(cancel)?;
        let path = crate::paths::canonical(path)?;
        let configuration = crate::config::Snapshot::load(&path)?;
        let root = configuration.root().to_path_buf();
        let mut allowed_roots = vec![root];
        for root in additional_roots {
            allowed_roots.push(crate::paths::canonical(root)?);
        }
        let mut project = Self {
            root: path.clone(),
            documents: BTreeMap::new(),
            imports: BTreeMap::new(),
            sampler_images: BTreeMap::new(),
            browser_resources: BTreeMap::new(),
            browser_document_bytes: BTreeMap::new(),
            allowed_roots,
            assets: BTreeMap::new(),
            missing_assets: BTreeMap::new(),
            runtime_assets: BTreeSet::new(),
            runtime_refs: BTreeMap::new(),
            byte_count: 0,
            resolution_error: None,
            icons: crate::icons::Registry::default(),
            icon_data_dir: data.map(Path::to_path_buf),
            font_files: vec![],
            font_sources: BTreeMap::new(),
            font_bindings: vec![],
            font_settings: configuration.settings.fonts.clone(),
            font_lock_capture: false,
            configuration: Some(configuration),
            overlay: overlay.clone(),
            overlay_published: overlay.is_empty(),
            file_bindings: Default::default(),
            cancel: cancel.cloned(),
        };
        let configuration_sources = project
            .configuration
            .as_ref()
            .expect("configuration")
            .sources();
        for (path, bytes) in configuration_sources {
            project.capture_bytes(path, "configuration", bytes)?;
        }
        if let Some(lock) = project
            .configuration
            .as_ref()
            .and_then(|configuration| configuration.font_lock.clone())
        {
            if project.assets.contains_key(&lock.path) || lock.path == path {
                return Err(Error::new(
                    "CONFIGURATION_ERROR",
                    "Font lockfile must not alias a document or configuration",
                ));
            }
            project.capture_bytes(lock.path, "font_lock", lock.bytes)?;
        }
        let configured_fonts = project
            .configuration
            .as_ref()
            .expect("configuration")
            .font_files
            .clone();
        project.add_font_files(&configured_fonts)?;
        let missing_fonts = project
            .configuration
            .as_ref()
            .expect("configuration")
            .missing_fonts
            .clone();
        for font in &missing_fonts {
            project.capture_missing_font(font)?;
        }
        project.load_document(&path, Some(document), &mut BTreeSet::new())?;
        let mut references = Vec::new();
        let mut libraries = BTreeSet::new();
        match project.resolve_impl(false) {
            Ok(scene) => {
                for node in &scene.roots {
                    collect_resolved_assets(node, &mut references)?;
                    collect_icon_libraries(node, &mut libraries);
                }
            }
            Err(error) => project.resolution_error = Some(error),
        }
        for (origin, uri, kind) in references {
            project.capture_declared_asset(&origin, &uri, &kind)?;
        }
        let design_files = project
            .configuration
            .as_ref()
            .expect("configuration")
            .design
            .as_ref()
            .ok()
            .into_iter()
            .flat_map(|design| &design.resources)
            .map(|file| (file.path.clone(), file.kind, file.bytes.clone()))
            .collect::<Vec<_>>();
        for (path, kind, bytes) in design_files {
            if let Some(document) = project.documents.get(&path) {
                if path != project.root && document.text().as_bytes() != bytes.as_ref() {
                    return Err(Error::new(
                        "REVISION_CONFLICT",
                        "Design example changed while loading the project",
                    ));
                }
                continue;
            }
            if let Some(asset) = project.assets.get(&path) {
                if asset.bytes != bytes {
                    return Err(Error::new(
                        "REVISION_CONFLICT",
                        "Design resource changed while loading the project",
                    ));
                }
            } else {
                project.capture_bytes(path, kind, bytes)?;
            }
        }
        project.prepare_icons(libraries, false)?;
        project.verify_unchanged()?;
        Ok(project)
    }

    fn load_document(
        &mut self,
        path: &Path,
        supplied: Option<Document>,
        active: &mut BTreeSet<PathBuf>,
    ) -> Result<()> {
        self.check_cancelled()?;
        if active.contains(path) {
            return Err(Error::new(
                "REFERENCE_CYCLE",
                format!("Library import cycle at {}", path.display()),
            ));
        }
        if self.documents.contains_key(path) {
            return Ok(());
        }
        if self.documents.len() >= MAX_DEPENDENCY_DOCUMENTS {
            return Err(Error::new(
                "INVALID_DOCUMENT",
                "Too many imported documents",
            ));
        }
        let document = match supplied {
            Some(doc) => doc,
            None => Document::parse(match self.overlay.get(path) {
                Some(file) => std::str::from_utf8(&file.bytes)
                    .map_err(|_| invalid("A staged document must be UTF-8"))?
                    .to_owned(),
                None => read_text(path)?,
            })?,
        };
        if document.codec().is_none() {
            return Err(Error::new(
                "UNSUPPORTED_VERSION",
                format!(
                    "Cannot resolve version {} at {}",
                    document.version(),
                    path.display()
                ),
            ));
        }
        self.byte_count += document.text().len();
        if self.byte_count > MAX_PROJECT_BYTES {
            return Err(invalid("Project dependencies exceed 128 MiB"));
        }
        let declarations = match document.value().get("imports") {
            Some(value) => value
                .as_object()
                .ok_or_else(|| invalid("imports must be an object"))?
                .clone(),
            None => Map::new(),
        };
        self.sampler_images
            .insert(path.into(), sampler_images::parse(&document)?);
        self.documents.insert(path.into(), document);
        self.capture_browser_resources(path)?;
        active.insert(path.into());
        let mut imports = BTreeMap::new();
        for (alias, uri) in declarations {
            if alias.is_empty() || alias.contains(':') || alias.contains('/') {
                return Err(invalid(
                    "Library aliases must be nonempty and contain neither ':' nor '/'",
                ));
            }
            let uri = uri
                .as_str()
                .ok_or_else(|| invalid("A library URI must be a string"))?;
            let target = match self.resolve_file(path, uri) {
                Ok(target) => target,
                Err(error) => {
                    if self
                        .reference_path(path, uri)?
                        .extension()
                        .is_none_or(|extension| !extension.eq_ignore_ascii_case("pen"))
                    {
                        return Err(invalid("Imported libraries must be .pen documents"));
                    }
                    self.capture_missing_reference(path, uri, "library", error)?;
                    // Keep the declaration in the original document, but do
                    // not invent definitions or a lookup edge to an absent
                    // document. Structural repairs must resolve their result.
                    continue;
                }
            };
            if target
                .extension()
                .and_then(|s| s.to_str())
                .is_none_or(|s| !s.eq_ignore_ascii_case("pen"))
            {
                return Err(invalid("Imported libraries must be .pen documents"));
            }
            self.load_document(&target, None, active)?;
            imports.insert(alias, target);
        }
        self.imports.insert(path.into(), imports);
        active.remove(path);
        Ok(())
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

    pub fn add_font_files(&mut self, paths: &[PathBuf]) -> Result<()> {
        let mut explicit = Vec::new();
        for path in paths {
            self.check_cancelled()?;
            let requested = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()
                    .map_err(|error| Error::io(path, error))?
                    .join(path)
            };
            let canonical = crate::paths::canonical(&requested)?;
            if canonical.to_str().is_none() {
                return Err(Error::new("INVALID_ARGUMENT", "Font paths must be UTF-8"));
            }
            if let Some(asset) = self.assets.get_mut(&canonical) {
                if asset.kind == "font_lock" {
                    return Err(Error::new(
                        "CONFIGURATION_ERROR",
                        "Font lockfile must not alias a configured font",
                    ));
                }
                if asset.kind == "browser_resource" {
                    asset.kind = "font".into();
                }
            } else if !self.font_sources.contains_key(&canonical) {
                let font = if let Some(file) = self.overlay.get(&canonical) {
                    font_files::FontFile::from_bytes(canonical.clone(), file.bytes.clone())
                } else {
                    font_files::FontFile::capture(&canonical, self.cancel.as_ref())?
                };
                self.byte_count = self
                    .byte_count
                    .checked_add(font.length)
                    .filter(|total| *total <= MAX_PROJECT_BYTES)
                    .ok_or_else(|| invalid("Project dependencies exceed 128 MiB"))?;
                self.font_sources.insert(canonical.clone(), font);
            }
            if !self
                .font_bindings
                .iter()
                .any(|binding| binding.0 == requested)
            {
                self.font_bindings.push((requested, canonical.clone()));
            }
            explicit.push(canonical);
        }
        explicit.extend(self.font_files.iter().cloned());
        let mut seen = BTreeSet::new();
        explicit.retain(|path| seen.insert(path.clone()));
        self.font_files = explicit;
        Ok(())
    }

    pub fn configuration(&self) -> Value {
        self.configuration
            .as_ref()
            .map(|configuration| {
                let mut value = configuration.summary();
                value["effective_font_files"] = json!(self.font_files);
                if let Some(data) = &self.icon_data_dir {
                    value["design"]["trusted"] = json!(
                        configuration
                            .design_catalog()
                            .and_then(|design| design.trusted(data))
                            .unwrap_or(false)
                    );
                }
                value
            })
            .unwrap_or(Value::Null)
    }

    pub fn configuration_lock_paths(&self) -> Vec<PathBuf> {
        self.configuration
            .as_ref()
            .map(|configuration| configuration.lock_paths().to_vec())
            .unwrap_or_default()
    }

    pub fn design_catalog(&self) -> Result<&crate::guidance::ProjectCatalog> {
        self.configuration
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "INVALID_ARGUMENT",
                    "Project design data requires a file-backed project",
                )
            })?
            .design_catalog()
    }

    pub fn font_settings(&self) -> &crate::config::FontSettings {
        &self.font_settings
    }

    pub fn script_settings(&self) -> crate::config::runtime::ScriptSettings {
        self.configuration
            .as_ref()
            .map_or_else(Default::default, |configuration| {
                configuration.settings.runtime.javascript
            })
    }

    pub fn runtime_settings(&self) -> crate::config::runtime::RuntimeSettings {
        self.configuration
            .as_ref()
            .map_or_else(Default::default, |configuration| {
                configuration.settings.runtime.clone()
            })
    }

    #[cfg(feature = "rendering")]
    pub fn load_fonts(&self) -> Result<crate::text::Fonts> {
        self.load_fonts_with_lock(true)
    }

    #[cfg(feature = "rendering")]
    fn load_fonts_with_lock(&self, enforce: bool) -> Result<crate::text::Fonts> {
        self.check_cancelled()?;
        if let Some(configuration) = &self.configuration {
            if enforce {
                configuration.require_fonts()?;
            } else {
                configuration.require_unlocked_fonts()?;
            }
        }
        let sources = self
            .font_files
            .iter()
            .map(|path| {
                self.font_sources
                    .get(path)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| {
                        Ok(font_files::FontFile::from_bytes(
                            path.clone(),
                            self.asset(path)?.bytes.clone(),
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let lock = if enforce {
            self.configuration
                .as_ref()
                .and_then(|configuration| configuration.font_lock.as_ref())
                .map(|input| crate::font_lock::Manifest::parse(&input.bytes))
                .transpose()?
        } else {
            None
        };
        if let Some(lock) = &lock {
            lock.require_inputs(&self.font_input_hashes()?)?;
        }
        let mut fonts = crate::text::Fonts::from_files(&sources, self.cancel.as_ref())?;
        fonts.configure(self.font_settings())?;
        if let Some(lock) = lock {
            fonts.enforce_lock(lock)?;
        }
        self.check_cancelled()?;
        Ok(fonts)
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn capture_fonts(&mut self) -> Result<crate::text::Fonts> {
        let fonts = self.load_fonts_with_lock(false)?;
        // Only explicit baseline acquisition may resolve past its missing lock.
        // The missing input remains a dependency and is still rechecked at publication.
        self.font_lock_capture = true;
        Ok(fonts)
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn font_input_hashes(&self) -> Result<Vec<String>> {
        self.font_files
            .iter()
            .map(|path| {
                self.font_sources
                    .get(path)
                    .map(|font| Ok(font.revision.clone()))
                    .unwrap_or_else(|| self.asset(path).map(|asset| asset.revision.clone()))
            })
            .collect()
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn font_lock_path(&self) -> Option<PathBuf> {
        self.configuration
            .as_ref()
            .and_then(crate::config::Snapshot::font_lock_path)
    }

    fn reference_path(&self, origin: &Path, uri: &str) -> Result<PathBuf> {
        if uri
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
        {
            return crate::paths::local_file_url(uri);
        }
        if uri.contains("://") {
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

    fn lookup_node(&self, origin: &Path, name: &str) -> Result<(PathBuf, Value)> {
        if let Some((path, id)) = self.runtime_refs.get(name) {
            return Ok((path.clone(), self.documents[path].node(id)?.clone()));
        }
        if let Ok(node) = self.documents[origin].node(name) {
            return Ok((origin.into(), node.clone()));
        }
        if let Some((alias, member)) = name.split_once(':')
            && let Some(path) = self.imported_path(origin, alias)?
        {
            return self.lookup_node(path, member);
        }
        let mut matches = Vec::new();
        for path in self
            .imports
            .get(origin)
            .into_iter()
            .flat_map(BTreeMap::values)
            .collect::<BTreeSet<_>>()
        {
            if let Ok(node) = self.documents[path].node(name) {
                matches.push((path.clone(), node.clone()));
            }
        }
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(Error::new(
                "ASSET_NOT_FOUND",
                format!("Component {name:?} does not exist in this document or its imports"),
            )),
            _ => Err(invalid(format!(
                "Ambiguous imported component {name:?}; specify its library alias"
            ))),
        }
    }

    pub fn resolve(&self) -> Result<ResolvedScene> {
        self.require_assets()?;
        self.resolve_impl(true)
    }

    pub(crate) fn resolve_for_edit(&self) -> Result<ResolvedScene> {
        self.resolve_impl(true)
    }

    fn prepare_icons(&mut self, libraries: BTreeSet<String>, runtime: bool) -> Result<()> {
        let libraries = libraries
            .into_iter()
            .filter(|name| !self.icons.contains(name))
            .collect();
        let (registry, files) =
            crate::icons::Registry::prepare(libraries, self.icon_data_dir.as_deref());
        for (path, bytes) in files {
            if self.assets.contains_key(&path) {
                continue;
            }
            self.byte_count += bytes.len();
            if self.byte_count > MAX_PROJECT_BYTES {
                return Err(invalid("Project dependencies exceed 128 MiB"));
            }
            if runtime {
                self.runtime_assets.insert(path.clone());
            }
            self.assets.insert(
                path,
                Asset {
                    revision: hash(&bytes),
                    bytes,
                    kind: "icon_pack".into(),
                },
            );
        }
        self.icons.extend(registry);
        Ok(())
    }
    fn bind_icons(&self, node: &mut ResolvedNode) {
        if node.kind() == "icon" {
            let icon = self.icons.resolve(&node.properties);
            node.icon_metadata = Some(
                icon.as_ref()
                    .map(|icon| icon.metadata())
                    .unwrap_or_else(|error| json!({"error":error})),
            );
            node.icon = Some(icon);
        }
        for child in &mut node.children {
            self.bind_icons(child);
        }
    }
    fn resolve_impl(&self, icons: bool) -> Result<ResolvedScene> {
        for document in self.documents.values() {
            self.check_cancelled()?;
            crate::operations::validate_local(document)?;
        }
        let mut count = 0;
        let mut roots = Vec::new();
        for node in self.root_document().value()["children"]
            .as_array()
            .expect("parsed document")
        {
            let mut resolved =
                self.expand(&self.root, node, None, &[], &mut Vec::new(), &mut count)?;
            assign_addresses(&mut resolved, None, &[]);
            self.resolve_properties(&mut resolved, &self.default_theme(&self.root)?)?;
            if icons {
                self.bind_icons(&mut resolved);
            }
            roots.push(resolved);
        }
        self.verify_unchanged()?;
        Ok(ResolvedScene {
            revision: self.root_document().revision(),
            dependency_revision: self.dependency_revision(),
            dependencies: self.dependencies(),
            roots,
        })
    }

    pub fn diagnostics(&self) -> Vec<crate::document::Diagnostic> {
        self.documents
            .values()
            .flat_map(Document::diagnostics)
            .collect()
    }

    fn expand(
        &self,
        origin: &Path,
        node: &Value,
        instance: Option<&str>,
        path: &[String],
        active: &mut Vec<(PathBuf, String)>,
        count: &mut usize,
    ) -> Result<ResolvedNode> {
        self.check_cancelled()?;
        *count += 1;
        if *count > MAX_NODES || active.len() > 128 || path.len() > 256 {
            return Err(invalid(
                "Expanded document exceeds its node or reference depth limit",
            ));
        }
        let id = node["id"]
            .as_str()
            .ok_or_else(|| invalid("A component child requires an ID"))?;
        let address = NodeAddress {
            node_id: instance.unwrap_or(id).into(),
            instance_path: if instance.is_some() {
                path.to_vec()
            } else {
                Vec::new()
            },
        };
        if node["type"] == "ref" {
            let reference = node["ref"]
                .as_str()
                .ok_or_else(|| invalid("An instance requires a component reference"))?;
            let (source, definition) = self.lookup_node(origin, reference)?;
            if definition["reusable"] != true {
                return Err(invalid(format!("Component {reference:?} is not reusable")));
            }
            let key = (
                source.clone(),
                definition["id"]
                    .as_str()
                    .expect("indexed definition")
                    .to_string(),
            );
            if active.contains(&key) {
                return Err(Error::new(
                    "REFERENCE_CYCLE",
                    format!("Component reference cycle at {reference}"),
                ));
            }
            active.push(key);
            let root = instance.unwrap_or(id);
            let mut resolved =
                self.expand(&source, &definition, Some(root), path, active, count)?;
            resolved.address = address;
            resolved.component_key = id.into();
            resolved.component_source = origin.into();
            resolved.instance_identity = Some((origin.into(), id.into()));
            resolved.is_instance = true;
            let props = node
                .as_object()
                .ok_or_else(|| invalid("Instance must be an object"))?;
            self.apply_properties(&mut resolved, props, origin, active, count)?;
            if let Some(overrides) = node.get("descendants") {
                let overrides = overrides
                    .as_object()
                    .ok_or_else(|| invalid("descendants must be an object"))?;
                let mut entries = overrides.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(path, _)| path.split('/').count());
                for (path, changes) in entries {
                    let parts = path.split('/').collect::<Vec<_>>();
                    let target = find_descendant_mut(&mut resolved, &parts).ok_or_else(|| {
                        invalid(format!("Missing descendant override path {path:?}"))
                    })?;
                    if changes.get("type").is_some() {
                        let old_key = target.component_key.clone();
                        let old_source = target.component_source.clone();
                        let old_address = target.address.clone();
                        let mut replacement = self.expand(
                            origin,
                            changes,
                            Some(&old_address.node_id),
                            &old_address.instance_path,
                            active,
                            count,
                        )?;
                        replacement.component_key = old_key;
                        replacement.component_source = old_source;
                        replacement.address = old_address;
                        *target = replacement;
                    } else {
                        self.apply_properties(
                            target,
                            changes
                                .as_object()
                                .ok_or_else(|| invalid("An override must be an object"))?,
                            origin,
                            active,
                            count,
                        )?;
                    }
                }
            }
            active.pop();
            return Ok(resolved);
        }
        let mut properties = node
            .as_object()
            .ok_or_else(|| invalid("Node must be an object"))?
            .clone();
        let children = properties.remove("children");
        let property_sources = properties
            .keys()
            .map(|key| (key.clone(), origin.into()))
            .collect();
        let mut resolved = ResolvedNode {
            address,
            source_path: origin.into(),
            source_id: id.into(),
            component_key: id.into(),
            component_source: origin.into(),
            instance_identity: None,
            is_instance: false,
            properties,
            property_sources,
            script_scope: None,
            script_input_scope: None,
            script_bindings: None,
            input_context: None,
            browser_scope: None,
            browser_url_scope: None,
            #[cfg(feature = "rendering")]
            script_inputs_resolved: false,
            theme: BTreeMap::new(),
            children: Vec::new(),
            generated_by: None,
            icon: None,
            icon_metadata: None,
        };
        self.bind_script_scope(&mut resolved)?;
        self.bind_browser_scope(&mut resolved)?;
        if let Some(children) = children {
            for child in children
                .as_array()
                .ok_or_else(|| invalid("children must be an array"))?
            {
                let mut child_path = path.to_vec();
                child_path.push(
                    child["id"]
                        .as_str()
                        .ok_or_else(|| invalid("Missing child ID"))?
                        .into(),
                );
                resolved.children.push(self.expand(
                    origin,
                    child,
                    instance,
                    &child_path,
                    active,
                    count,
                )?);
            }
        }
        Ok(resolved)
    }

    fn apply_properties(
        &self,
        node: &mut ResolvedNode,
        changes: &Map<String, Value>,
        origin: &Path,
        active: &mut Vec<(PathBuf, String)>,
        count: &mut usize,
    ) -> Result<()> {
        let carrier = script_context::override_value(changes)?;
        let browser_carrier = browser_context::override_value(changes)?;
        if carrier.is_some() && node.kind() != "script" {
            return Err(invalid("Copied script overrides require a script target"));
        }
        if browser_carrier.is_some() && node.kind() != "browser" {
            return Err(invalid("Copied browser overrides require a browser target"));
        }
        let carrier_metadata = carrier
            .as_ref()
            .map(|carrier| carrier.metadata.as_ref())
            .or_else(|| {
                browser_carrier
                    .as_ref()
                    .map(|carrier| carrier.metadata.as_ref())
            });
        let metadata_changed =
            carrier_metadata.map_or(changes.contains_key("metadata"), |value| value.is_some());
        for (key, value) in changes {
            if matches!(
                key.as_str(),
                "id" | "type" | "ref" | "descendants" | "children"
            ) {
                continue;
            }
            let value = if key == "metadata"
                && let Some(metadata) = carrier_metadata
            {
                let Some(value) = metadata else {
                    continue;
                };
                value
            } else {
                value
            };
            if key == "theme" {
                let map = value
                    .as_object()
                    .ok_or_else(|| invalid("theme must be an object"))?;
                let target = node.properties.entry("theme").or_insert_with(|| json!({}));
                target
                    .as_object_mut()
                    .ok_or_else(|| invalid("theme must be an object"))?
                    .extend(map.clone());
            } else {
                node.properties.insert(key.clone(), value.clone());
            }
            node.property_sources.insert(key.clone(), origin.into());
        }
        if let Some(children) = changes.get("children") {
            node.children.clear();
            for child in children
                .as_array()
                .ok_or_else(|| invalid("Replacement children must be an array"))?
            {
                let mut path = node.address.instance_path.clone();
                path.push(
                    child["id"]
                        .as_str()
                        .ok_or_else(|| invalid("Replacement child requires an ID"))?
                        .into(),
                );
                node.children.push(self.expand(
                    origin,
                    child,
                    Some(&node.address.node_id),
                    &path,
                    active,
                    count,
                )?);
            }
        }
        // Ordinary metadata does not own execution context. Reset explicit code
        // and input edits independently, then bind only reserved context fields.
        if changes.contains_key("scriptUri") {
            node.script_scope = None;
            node.script_bindings = None;
        }
        if changes.contains_key("inputs") {
            node.input_context = None;
            node.script_input_scope = None;
            if let Some(bindings) = &mut node.script_bindings {
                bindings.inputs_bound = false;
            }
        }
        if metadata_changed {
            let (output, input) =
                script_context::metadata_contexts(node.properties.get("metadata"));
            if output || input {
                let previous_output =
                    (!output).then(|| (node.script_bindings.take(), node.script_scope.take()));
                let previous_input =
                    (!input).then(|| (node.input_context.take(), node.script_input_scope.take()));
                self.bind_script_scope(node)?;
                if let Some((mut bindings, scope)) = previous_output {
                    if input && let Some(bindings) = &mut bindings {
                        bindings.inputs_bound = false;
                    }
                    node.script_bindings = bindings;
                    node.script_scope = scope;
                }
                if let Some((context, scope)) = previous_input {
                    node.input_context = context;
                    node.script_input_scope = scope;
                }
            }
        }
        if let Some(carrier) = carrier {
            if let Some(value) = carrier.output {
                node.script_bindings = bindings::bind_value(self, node, &value, origin)?;
                node.script_scope = None;
            }
            if let Some(value) = carrier.input {
                node.input_context = script_context::bind(self, node, &value, origin)?;
            }
        }
        if changes.contains_key("url") {
            node.browser_scope = None;
            node.browser_url_scope = None;
        }
        if metadata_changed
            && node
                .properties
                .get("metadata")
                .and_then(|value| value.pointer("/lazypencil/browser_scope"))
                .is_some()
        {
            self.bind_browser_scope(node)?;
        }
        if let Some(scope) = browser_carrier.and_then(|carrier| carrier.scope) {
            self.apply_browser_scope(node, &scope, origin)?;
        }
        Ok(())
    }

    fn default_theme(&self, source: &Path) -> Result<BTreeMap<String, String>> {
        let mut result = BTreeMap::new();
        if let Some(axes) = self.documents[source].value().get("themes") {
            for (name, values) in axes
                .as_object()
                .ok_or_else(|| invalid("themes must be an object"))?
            {
                let first = values
                    .as_array()
                    .and_then(|v| v.first())
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid(format!("Theme axis {name:?} needs a nonempty string array"))
                    })?;
                if !values
                    .as_array()
                    .expect("checked array")
                    .iter()
                    .all(Value::is_string)
                {
                    return Err(invalid("Theme values must be strings"));
                }
                result.insert(name.clone(), first.into());
            }
        }
        Ok(result)
    }

    fn resolve_properties(
        &self,
        node: &mut ResolvedNode,
        inherited: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.check_cancelled()?;
        let theme = self.effective_theme(node, inherited)?;
        node.theme = theme.clone();
        let is_script = node.kind() == "script";
        if is_script {
            // Input names belong to the captured header. Stale saved controls
            // must not resolve variables or create runtime dependencies.
            let header = node
                .properties
                .get("scriptUri")
                .and_then(Value::as_str)
                .and_then(|uri| {
                    self.resolve_file(node.property_source("scriptUri"), uri)
                        .ok()
                })
                .and_then(|path| self.assets.get(&path))
                .and_then(|asset| std::str::from_utf8(&asset.bytes).ok())
                .and_then(|source| crate::script::header::Header::parse(source).ok());
            if let Some(values) = node.properties.get("inputs") {
                let values = values
                    .as_object()
                    .ok_or_else(|| invalid("Script inputs must be an object"))?;
                let values = match &header {
                    Some(header) => self.resolve_script_input_values(node, header, values)?,
                    // Generated code may not be captured yet. Its declared
                    // inputs are resolved once after capture, before execution.
                    None => values.clone(),
                };
                node.properties
                    .insert("inputs".into(), Value::Object(values));
            }
            #[cfg(feature = "rendering")]
            {
                node.script_inputs_resolved = header.is_some();
            }
        }
        for (key, value) in &mut node.properties {
            if literal_property(key) || key == "inputs" && is_script {
                continue;
            }
            let origin = node.property_sources.get(key).unwrap_or(&node.source_path);
            *value = self.resolve_value(origin, value, &theme, &mut Vec::new())?;
        }
        for child in &mut node.children {
            self.resolve_properties(child, &theme)?;
        }
        Ok(())
    }

    pub(crate) fn resolve_script_input_values(
        &self,
        node: &ResolvedNode,
        header: &crate::script::header::Header,
        values: &Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        values
            .iter()
            .map(|(name, value)| {
                let value = if header.inputs.iter().any(|input| &input.name == name) {
                    let (origin, value) = if let Some(context) = &node.input_context {
                        (context.origin.as_path(), context.expression(name, value)?)
                    } else {
                        (node.property_source("inputs"), value.clone())
                    };
                    self.resolve_value(origin, &value, &node.theme, &mut Vec::new())?
                } else {
                    value.clone()
                };
                Ok((name.clone(), value))
            })
            .collect()
    }

    fn effective_theme(
        &self,
        node: &ResolvedNode,
        inherited: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>> {
        self.check_cancelled()?;
        let mut theme = self.default_theme(&node.source_path)?;
        theme.extend(inherited.clone());
        if let Some(overrides) = node.properties.get("theme") {
            for (axis, value) in overrides
                .as_object()
                .ok_or_else(|| invalid("theme must be an object"))?
            {
                theme.insert(
                    axis.clone(),
                    value
                        .as_str()
                        .ok_or_else(|| invalid("Theme axis values must be strings"))?
                        .into(),
                );
            }
        }
        Ok(theme)
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn prepare_script_refs(
        &mut self,
        node: &ResolvedNode,
        header: &crate::script::header::Header,
        mut inputs: Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        for input in &header.inputs {
            if matches!(input.kind, crate::script::header::InputKind::Ref)
                && let Some(value) = inputs.get(&input.name)
            {
                let saved = node
                    .properties
                    .get("inputs")
                    .and_then(|values| values.get(&input.name))
                    .is_some_and(|value| !value.is_null());
                let origin = if saved {
                    node.script_input_origin()
                } else {
                    node.script_origin()
                };
                let reference = value.as_str().expect("validated ref input");
                if reference.is_empty() {
                    continue;
                }
                let (origin, reference) = if self.runtime_refs.contains_key(reference) {
                    (origin, reference)
                } else if saved && let Some(context) = &node.input_context {
                    (context.origin.as_path(), context.component(reference)?)
                } else if let Some(bindings) = &node.script_bindings
                    && (!saved || bindings.inputs_bound)
                {
                    (
                        bindings.origin.as_path(),
                        bindings.input_component(reference, saved)?,
                    )
                } else {
                    (origin, reference)
                };
                let (source, definition) = self.lookup_node(origin, reference)?;
                if definition["reusable"] != true {
                    return Err(invalid(
                        "Script ref input must reference a reusable component",
                    ));
                }
                let same_scope = self
                    .lookup_node(node.script_origin(), reference)
                    .is_ok_and(|(path, other)| path == source && other["id"] == definition["id"]);
                if !same_scope
                    || node.script_bindings.is_some()
                    || value.as_str() != Some(reference)
                {
                    let id = definition["id"]
                        .as_str()
                        .expect("definition ID")
                        .to_string();
                    let handle = format!(
                        "@runtime/component/{}",
                        hash(json!([source, id]).to_string().as_bytes())
                    );
                    self.runtime_refs.insert(handle.clone(), (source, id));
                    inputs.insert(input.name.clone(), json!(handle));
                }
            }
        }
        Ok(inputs)
    }

    #[cfg(feature = "rendering")]
    pub(crate) fn generated_children(
        &mut self,
        script: &ResolvedNode,
        nodes: &[Value],
    ) -> Result<Vec<ResolvedNode>> {
        let origin = script
            .script_bindings
            .as_ref()
            .map(|bindings| bindings.origin.clone())
            .unwrap_or_else(|| script.script_origin().to_path_buf());
        let mut nodes = nodes.to_vec();
        let mut counter = 0;
        let mut ids = BTreeSet::new();
        let mut generated_names = BTreeMap::new();
        for node in &mut nodes {
            assign_generated_ids(node, &mut counter, &mut ids, &mut generated_names)?;
        }
        if let Some(bindings) = &script.script_bindings {
            let mut inherited_bytes = 0;
            let mut contexts = Vec::with_capacity(nodes.len());
            for node in &mut nodes {
                self.check_cancelled()?;
                contexts.push(bindings.rewrite(
                    node,
                    &self.runtime_refs,
                    &mut inherited_bytes,
                    self,
                    &generated_names,
                )?);
            }
            for (node, context) in nodes.iter_mut().zip(&contexts) {
                self.bind_generated_ref_contexts(
                    node,
                    bindings,
                    &script.theme,
                    &mut inherited_bytes,
                    context,
                )?;
            }
        } else {
            for node in &mut nodes {
                bindings::rewrite_generated_paths(
                    self,
                    node,
                    &origin,
                    &self.runtime_refs,
                    &generated_names,
                )?;
            }
        }
        let mut count = 0;
        let mut result = Vec::new();
        let mut generated = 0;
        for node in nodes {
            let mut node = self.expand(&origin, &node, None, &[], &mut Vec::new(), &mut count)?;
            self.resolve_properties(&mut node, &script.theme)?;
            mark_generated(&mut node, &script.address, &script.id(), &mut generated);
            result.push(node);
        }
        let mut assets = Vec::new();
        let mut libraries = BTreeSet::new();
        for node in &result {
            collect_resolved_assets(node, &mut assets)?;
            collect_icon_libraries(node, &mut libraries);
        }
        self.prepare_icons(libraries, true)?;
        for node in &mut result {
            self.bind_icons(node);
        }
        for (origin, uri, kind) in assets {
            let path = self.resolve_file(&origin, &uri)?;
            if !self.assets.contains_key(&path) {
                self.capture_asset(&path, &kind)?;
                self.runtime_assets.insert(path);
            }
        }
        self.verify_unchanged()?;
        Ok(result)
    }

    fn resolve_value(
        &self,
        origin: &Path,
        value: &Value,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
    ) -> Result<Value> {
        self.check_cancelled()?;
        Ok(match value {
            Value::String(value) if value.starts_with('$') && value.len() > 1 => {
                self.variable(origin, &value[1..], theme, active)?
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(k, v)| {
                        Ok((
                            k.clone(),
                            if k == "uniforms" && v.is_object() {
                                Value::Object(
                                    v.as_object()
                                        .expect("checked object")
                                        .iter()
                                        .map(|(name, value)| {
                                            Ok((
                                                name.clone(),
                                                if object.get("type").and_then(Value::as_str)
                                                    == Some("shader")
                                                {
                                                    self.resolve_shader_uniform(
                                                        origin, value, theme, active,
                                                    )?
                                                } else {
                                                    self.resolve_value(
                                                        origin, value, theme, active,
                                                    )?
                                                },
                                            ))
                                        })
                                        .collect::<Result<_>>()?,
                                )
                            } else if literal_property(k) {
                                v.clone()
                            } else {
                                self.resolve_value(origin, v, theme, active)?
                            },
                        ))
                    })
                    .collect::<Result<_>>()?,
            ),
            Value::Array(array) => Value::Array(
                array
                    .iter()
                    .map(|v| self.resolve_value(origin, v, theme, active))
                    .collect::<Result<_>>()?,
            ),
            _ => value.clone(),
        })
    }

    fn variable(
        &self,
        origin: &Path,
        name: &str,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
    ) -> Result<Value> {
        self.variable_with_file_origin(origin, name, theme, active, None)
    }

    fn resolve_shader_uniform(
        &self,
        origin: &Path,
        value: &Value,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
    ) -> Result<Value> {
        if let Value::Array(values) = value {
            return Ok(Value::Array(
                values
                    .iter()
                    .map(|value| self.resolve_shader_uniform(origin, value, theme, active))
                    .collect::<Result<_>>()?,
            ));
        }
        let Some(name) = value
            .as_str()
            .and_then(|value| value.strip_prefix('$'))
            .filter(|name| !name.is_empty())
        else {
            return self.resolve_value(origin, value, theme, active);
        };
        let mut source = origin.to_path_buf();
        let result =
            self.variable_with_file_origin(origin, name, theme, active, Some(&mut source))?;
        if let Some(uri) = result
            .as_str()
            .filter(|uri| !uri.is_empty() && !uri.starts_with('#') && !uri.starts_with("pencil:"))
        {
            // Anchor the value without opening inactive assets. Capture and rendering
            // perform canonicalization, root checks, and dependency verification.
            let uri = self.sampler_uri(&source, uri);
            if let Ok(path) = self.reference_path(&source, uri) {
                return Ok(json!(
                    path.to_str()
                        .ok_or_else(|| invalid("Shader image paths must be UTF-8"))?
                ));
            }
        }
        Ok(result)
    }

    fn variable_with_file_origin(
        &self,
        origin: &Path,
        name: &str,
        theme: &BTreeMap<String, String>,
        active: &mut Vec<(PathBuf, String)>,
        file_origin: Option<&mut PathBuf>,
    ) -> Result<Value> {
        self.check_cancelled()?;
        let (source, definition) = self.lookup_variable(origin, name)?;
        let key = (source.clone(), name.to_string());
        if active.contains(&key) || active.len() > 128 {
            return Err(Error::new(
                "REFERENCE_CYCLE",
                format!("Variable alias cycle at ${name}"),
            ));
        }
        active.push(key);
        let raw = definition
            .get("value")
            .ok_or_else(|| invalid(format!("Variable {name:?} has no value")))?;
        let value = if let Some(values) = raw.as_array() {
            let mut selected = None;
            for entry in values {
                let matches = match entry.get("theme") {
                    None => true,
                    Some(condition) => {
                        let condition = condition
                            .as_object()
                            .ok_or_else(|| invalid("Variable theme condition must be an object"))?;
                        condition.iter().all(|(k, v)| {
                            v.as_str()
                                .is_some_and(|v| theme.get(k).is_some_and(|t| t == v))
                        })
                    }
                };
                if matches {
                    selected = entry.get("value");
                }
            }
            selected
                .ok_or_else(|| invalid(format!("Variable {name:?} has no value for this theme")))?
        } else {
            raw
        };
        let result = if let Some(alias) = value
            .as_str()
            .and_then(|value| value.strip_prefix('$'))
            .filter(|name| !name.is_empty())
        {
            self.variable_with_file_origin(&source, alias, theme, active, file_origin)?
        } else {
            if let Some(origin) = file_origin {
                *origin = source.clone();
            }
            self.resolve_value(&source, value, theme, active)?
        };
        active.pop();
        let valid = match definition["type"].as_str() {
            Some("number") => result.is_number(),
            Some("string" | "color") => result.is_string(),
            Some("boolean") => result.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(invalid(format!(
                "Variable {name:?} resolves to the wrong type"
            )));
        }
        Ok(result)
    }

    fn lookup_variable(&self, origin: &Path, name: &str) -> Result<(PathBuf, Value)> {
        if let Some(value) = self.documents[origin]
            .value()
            .get("variables")
            .and_then(|v| v.get(name))
        {
            return Ok((origin.into(), value.clone()));
        }
        if let Some((alias, member)) = name.split_once(':')
            && let Some(path) = self.imported_path(origin, alias)?
        {
            return self.lookup_variable(path, member);
        }
        let mut matches = Vec::new();
        for path in self
            .imports
            .get(origin)
            .into_iter()
            .flat_map(BTreeMap::values)
            .collect::<BTreeSet<_>>()
        {
            if let Some(value) = self.documents[path]
                .value()
                .get("variables")
                .and_then(|v| v.get(name))
            {
                matches.push((path.clone(), value.clone()));
            }
        }
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(invalid(format!("Missing variable ${name}"))),
            _ => Err(invalid(format!(
                "Ambiguous imported variable ${name}; specify its library alias"
            ))),
        }
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

fn literal_property(key: &str) -> bool {
    matches!(
        key,
        "id" | "type"
            | "name"
            | "context"
            | "ref"
            | "metadata"
            | "scriptUri"
            | "url"
            | "geometry"
            | "href"
            | "descendants"
            | "slot"
            | "theme"
    )
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
    let mut bytes = Vec::new();
    file.take(MAX_ASSET_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(invalid("An asset exceeds 64 MiB"));
    }
    Ok(bytes)
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
            let fills = payload
                .as_array()
                .map(|v| v.iter().collect::<Vec<_>>())
                .unwrap_or_else(|| vec![payload]);
            for fill in fills {
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
                        .flat_map(shader_values)
                        .filter_map(Value::as_str)
                    {
                        if !uniform.starts_with(['#', '$'])
                            && !uniform.starts_with("pencil:")
                            && !uniform.is_empty()
                        {
                            output.push((origin.into(), uniform.into(), "image".into()));
                        }
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

fn shader_values(value: &Value) -> &[Value] {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value))
}

fn shader_values_mut(value: &mut Value) -> &mut [Value] {
    match value {
        Value::Array(values) => values,
        value => std::slice::from_mut(value),
    }
}
