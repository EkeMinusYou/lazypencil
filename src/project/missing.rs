use super::*;

pub(super) struct MissingAsset {
    pub path: crate::paths::MissingPath,
    kind: String,
    origin: PathBuf,
    uri: String,
}

impl MissingAsset {
    pub fn dependency(&self) -> Dependency {
        Dependency {
            path: self.path.requested.clone(),
            revision: self.path.revision(),
            kind: self.kind.clone(),
            version: None,
            availability: Some("missing"),
        }
    }
    fn diagnostic(&self) -> Value {
        json!({"code":"ASSET_NOT_FOUND","message":"A declared asset is missing",
            "path":self.path.requested,"resolved_path":self.path.target,"kind":self.kind,"origin":self.origin,"uri":self.uri})
    }
}

impl Project {
    pub(super) fn capture_declared_asset(
        &mut self,
        origin: &Path,
        uri: &str,
        kind: &str,
    ) -> Result<()> {
        self.check_cancelled()?;
        match self.resolve_file(origin, uri) {
            Ok(path) => self.capture_asset(&path, kind),
            Err(error) => self.capture_missing_reference(origin, uri, kind, error),
        }
    }

    pub(super) fn capture_missing_reference(
        &mut self,
        origin: &Path,
        uri: &str,
        kind: &str,
        error: Error,
    ) -> Result<()> {
        if error.code != "IO_ERROR" {
            return Err(error);
        }
        let requested = self.reference_path(origin, uri)?;
        let key = (requested.clone(), kind.to_owned());
        if let Some(existing) = self.missing_assets.get(&key) {
            existing.path.verify()?;
            return Ok(());
        }
        let Some(path) = crate::paths::MissingPath::capture(&requested)? else {
            return Err(error);
        };
        if !self
            .allowed_roots
            .iter()
            .any(|root| path.target.starts_with(root))
        {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "A missing reference escapes the allowed project roots",
            ));
        }
        if self.missing_assets.len() >= 1024 {
            return Err(invalid(
                "A project may report at most 1024 missing asset references",
            ));
        }
        path.verify()?;
        let bytes = serde_json::to_vec(&path)
            .expect("UTF-8 missing reference")
            .len()
            .saturating_add(uri.len())
            .saturating_add(origin.as_os_str().len());
        self.byte_count = self.byte_count.saturating_add(bytes);
        if self.byte_count > MAX_PROJECT_BYTES {
            return Err(invalid("Project dependencies exceed 128 MiB"));
        }
        self.missing_assets.insert(
            key,
            MissingAsset {
                path,
                kind: kind.into(),
                origin: origin.into(),
                uri: uri.into(),
            },
        );
        Ok(())
    }

    pub fn asset_diagnostics(&self) -> Vec<Value> {
        self.missing_assets
            .values()
            .map(MissingAsset::diagnostic)
            .collect()
    }

    pub fn assets_complete(&self) -> bool {
        self.missing_assets.is_empty()
    }

    pub(crate) fn refresh_missing_assets(&mut self) -> Result<()> {
        for asset in self.missing_assets.values_mut() {
            crate::cancel::check(self.cancel.as_ref())?;
            asset.path.verify()?;
            asset.path =
                crate::paths::MissingPath::capture(&asset.path.requested)?.ok_or_else(|| {
                    Error::new(
                        "REVISION_CONFLICT",
                        "A missing asset appeared; reload the document",
                    )
                })?;
        }
        Ok(())
    }

    pub(crate) fn require_assets(&self) -> Result<()> {
        if self.assets_complete() {
            return Ok(());
        }
        Err(Error::new(
            "ASSET_NOT_FOUND",
            "The document has missing library, image, or code assets",
        )
        .details(json!({"assets_complete":false,"asset_diagnostics":self.asset_diagnostics()})))
    }

    pub(crate) fn require_no_new_missing(&self, before: &Project) -> Result<()> {
        let mut allowed = BTreeSet::new();
        for asset in before.missing_assets.values() {
            self.check_cancelled()?;
            allowed.insert((asset.path.current_target()?, asset.kind.clone()));
        }
        for asset in self.missing_assets.values() {
            self.check_cancelled()?;
            if !allowed.contains(&(asset.path.current_target()?, asset.kind.clone())) {
                return Err(Error::new(
                    "ASSET_NOT_FOUND",
                    "An edit cannot introduce a new missing asset",
                )
                .details(asset.diagnostic()));
            }
        }
        Ok(())
    }

    pub(super) fn asset_path_for_edit(&self, origin: &Path, uri: &str) -> Result<PathBuf> {
        match self.resolve_file(origin, uri) {
            Ok(path) => Ok(path),
            Err(error) => {
                if error.code != "IO_ERROR" {
                    return Err(error);
                }
                let requested = self.reference_path(origin, uri)?;
                let path = crate::paths::MissingPath::capture(&requested)?.ok_or(error)?;
                if !self
                    .allowed_roots
                    .iter()
                    .any(|root| path.target.starts_with(root))
                {
                    return Err(Error::new(
                        "ASSET_NOT_FOUND",
                        "A missing reference escapes the allowed project roots",
                    ));
                }
                path.verify()?;
                Ok(path.target)
            }
        }
    }
}
