//! Project configuration, explicit font inputs and font-lock verification.

use super::*;

impl Project {
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
    pub(super) fn load_fonts_with_lock(&self, enforce: bool) -> Result<crate::text::Fonts> {
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
}
