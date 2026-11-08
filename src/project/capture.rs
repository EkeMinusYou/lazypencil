//! Capture immutable project inputs and declared document dependencies.

use super::*;

impl Project {
    pub fn load(store: &Store) -> Result<Self> {
        Self::load_with_cancellation(store, None)
    }

    pub fn load_with_cancellation(
        store: &Store,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Self> {
        Self::from_document_with_activity(
            store.path(),
            store.read()?,
            &[],
            Some(store.data_dir()),
            &BTreeMap::new(),
            cancel,
            store.asset_activity(),
        )
    }

    /// Embedders use the CLI executable for cancellable HTTP image workers.
    pub fn load_with_remote_worker(
        store: &Store,
        executable: &Path,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Self> {
        Self::capture_inputs(
            store.path(),
            store.read()?,
            &[],
            Some(store.data_dir()),
            &BTreeMap::new(),
            CaptureOptions {
                cancel,
                activity: store.asset_activity(),
                remote_images: true,
                remote_worker: Some(executable),
                acquire_images: true,
            },
        )
    }

    /// Capture a sibling document or retained checkpoint under the existing
    /// maintenance lease, without taking a conflicting new shared lease.
    pub(crate) fn for_asset_retention(
        store: &Store,
        path: &Path,
        document: Document,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Self> {
        Self::capture_inputs(
            path,
            document,
            &[],
            Some(store.data_dir()),
            &BTreeMap::new(),
            CaptureOptions {
                cancel,
                activity: store.asset_activity(),
                remote_images: false,
                remote_worker: None,
                acquire_images: false,
            },
        )
    }

    pub(crate) fn retention_documents(&self) -> &BTreeMap<PathBuf, Document> {
        &self.documents
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
        Self::from_document_with_activity(
            path,
            document,
            additional_roots,
            data,
            overlay,
            cancel,
            crate::store::AssetActivity::acquire()?,
        )
    }

    pub(super) fn from_document_with_activity(
        path: &Path,
        document: Document,
        additional_roots: &[PathBuf],
        data: Option<&Path>,
        overlay: &crate::managed::Overlay,
        cancel: Option<&tokio_util::sync::CancellationToken>,
        activity: Arc<crate::store::AssetActivity>,
    ) -> Result<Self> {
        Self::capture_inputs(
            path,
            document,
            additional_roots,
            data,
            overlay,
            CaptureOptions {
                cancel,
                activity,
                remote_images: true,
                remote_worker: None,
                acquire_images: false,
            },
        )
    }

    pub(super) fn capture_inputs(
        path: &Path,
        document: Document,
        additional_roots: &[PathBuf],
        data: Option<&Path>,
        overlay: &crate::managed::Overlay,
        options: CaptureOptions<'_>,
    ) -> Result<Self> {
        let cancel = options.cancel;
        crate::cancel::check(cancel)?;
        let path = crate::paths::canonical(path)?;
        let configuration = crate::config::Snapshot::load(&path)?;
        let root = configuration.root().to_path_buf();
        let mut allowed_roots = vec![root];
        for root in additional_roots {
            allowed_roots.push(crate::paths::canonical(root)?);
        }
        let mut project = Self {
            _asset_activity: Some(options.activity),
            remote_images_allowed: options.remote_images,
            acquire_images: options.acquire_images,
            remote_references: BTreeMap::new(),
            remote_worker: options.remote_worker.map(Path::to_owned),
            remote_deadline: None,
            captured_image_failures: BTreeMap::new(),
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

    pub(super) fn load_document(
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
                    "Cannot resolve version {} at {}; supported versions are {}",
                    document.version(),
                    path.display(),
                    crate::document::SUPPORTED_DOCUMENT_VERSIONS.join(", ")
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
}
