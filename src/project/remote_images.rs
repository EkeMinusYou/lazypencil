use super::*;
use std::time::{Duration, Instant};

impl Project {
    pub(super) fn declare_remote_image(&mut self, origin: &Path, uri: &str) -> Result<()> {
        let path = crate::remote_images::key(uri)?;
        if !self.remote_references.contains_key(&path) {
            if self.remote_references.len() >= 1024 {
                return Err(invalid("A project may declare at most 1024 HTTP images"));
            }
            self.byte_count = self
                .byte_count
                .saturating_add(uri.len())
                .saturating_add(origin.as_os_str().len())
                .saturating_add(path.as_os_str().len());
            if self.byte_count > MAX_PROJECT_BYTES {
                return Err(invalid("Project dependencies exceed 128 MiB"));
            }
            self.remote_references
                .insert(path, (origin.to_owned(), uri.into()));
        }
        Ok(())
    }

    /// Explicitly acquire declared HTTP image bytes under the captured network policy.
    /// Passive inspection keeps only declarations in the document dependency revision.
    pub fn acquire_remote_images(&mut self) -> Result<()> {
        self.acquire_images = true;
        let references = self.remote_references.values().cloned().collect::<Vec<_>>();
        for (origin, uri) in references {
            self.capture_declared_asset(&origin, &uri, "image")?;
        }
        Ok(())
    }

    /// Image-only resolution never extends the local/code/import read roots.
    pub fn resolve_image(&self, origin: &Path, uri: &str) -> Result<PathBuf> {
        if crate::inline_images::is_data(uri) {
            crate::inline_images::validate(uri)?;
            return Ok(crate::inline_images::identity(uri));
        }
        if crate::remote_images::is_http(uri) {
            return crate::remote_images::key(uri);
        }
        let path = Path::new(uri);
        if (crate::remote_images::is_key(path) || crate::inline_images::is_key(path))
            && self.assets.get(path).is_some_and(Asset::is_captured_image)
        {
            return Ok(path.into());
        }
        self.resolve_file(origin, uri)
    }

    pub(crate) fn capture_image(&mut self, origin: &Path, uri: &str) -> Result<PathBuf> {
        self.check_cancelled()?;
        if crate::inline_images::is_data(uri) {
            let path = crate::inline_images::identity(uri);
            if let Some(error) = self.captured_image_failures.get(&path) {
                return Err(error.clone());
            }
            if !self.assets.contains_key(&path) {
                let bytes = crate::inline_images::decode(uri, self.cancel.as_ref())?;
                self.capture_bytes(path.clone(), "embedded_image", bytes)?;
            }
            return Ok(path);
        }
        if !crate::remote_images::is_http(uri) {
            let path = self.resolve_image(origin, uri)?;
            self.capture_asset(&path, "image")?;
            return Ok(path);
        }
        let path = crate::remote_images::key(uri)?;
        if let Some(error) = self.captured_image_failures.get(&path) {
            return Err(error.clone());
        }
        if self.assets.contains_key(&path) {
            return Ok(path);
        }
        let settings = self.runtime_settings().browser;
        if !self.remote_images_allowed
            || settings.network_policy == crate::config::runtime::PageNetworkPolicy::Offline
        {
            return Err(Error::new(
                "ASSET_NOT_FOUND",
                "HTTP image acquisition is disabled by the network policy",
            ));
        }
        let limit = Duration::from_millis(settings.timeout_ms)
            .min(Duration::from_secs(crate::remote_images::MAX_SECONDS));
        let deadline = *self
            .remote_deadline
            .get_or_insert_with(|| Instant::now() + limit);
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                Error::new(
                    "ASSET_NOT_FOUND",
                    "Project HTTP image acquisition deadline exceeded",
                )
            })?;
        let bytes = crate::remote_images::fetch(
            uri,
            self.remote_worker.as_deref(),
            remaining,
            settings.image_origins.as_deref(),
            self.cancel.as_ref(),
        )?;
        self.capture_bytes(path.clone(), "remote_image", bytes)?;
        self.runtime_assets.insert(path.clone());
        Ok(path)
    }
}
