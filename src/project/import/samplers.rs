use super::*;

type ImageFiles = BTreeMap<String, Arc<[u8]>>;

impl Copier<'_> {
    pub(super) fn scan_sampler_values(&mut self, origin: &Path, node: &Value) -> Result<()> {
        for property in ["fill", "stroke", "effect"] {
            for fill in shader_values(&node[property]) {
                if fill["type"] != "shader" {
                    continue;
                }
                for value in fill
                    .get("uniforms")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(Map::values)
                    .flat_map(shader_values)
                {
                    if let Some(name) = value.as_str().and_then(|value| value.strip_prefix('$')) {
                        let key = self.project.variable_key(origin, name)?;
                        if self.project.documents[&key.0].value()["variables"][&key.1]["type"]
                            == "string"
                        {
                            self.sampler_vars.insert(key);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn prepare_sampler_bindings(&mut self) -> Result<()> {
        let mut queue = self.sampler_vars.iter().cloned().collect::<VecDeque<_>>();
        let mut seen = BTreeSet::new();
        let mut images: BTreeMap<(PathBuf, bool), ImageFiles> = BTreeMap::new();
        while let Some(key) = queue.pop_front() {
            self.project.check_cancelled()?;
            if !seen.insert(key.clone()) {
                continue;
            }
            // Explicitly reused variables retain their existing consumer bindings.
            let Some(definition) = self.var_values.get(&key) else {
                continue;
            };
            for value in definition_values(&definition["value"]) {
                let Some(uri) = value.as_str() else {
                    continue;
                };
                if let Some(name) = uri.strip_prefix('$').filter(|name| !name.is_empty()) {
                    queue.push_back(self.project.variable_key(&key.0, name)?);
                } else if !uri.is_empty() && !uri.starts_with('#') && !uri.starts_with("pencil:") {
                    let path = self
                        .project
                        .resolve_file(&key.0, self.project.sampler_uri(&key.0, uri))?;
                    self.project.capture_asset(&path, "image")?;
                    let bytes = self.project.asset(&path)?.bytes.clone();
                    image_extension(&bytes)?;
                    images
                        .entry((key.0.clone(), Path::new(uri).is_absolute()))
                        .or_default()
                        .insert(uri.into(), bytes);
                }
            }
        }
        self.sampler_vars.extend(seen);
        if images.is_empty() {
            return Ok(());
        }
        let mut members = BTreeMap::new();
        let mut libraries = Vec::new();
        for (index, ((origin, absolute), images)) in images.into_iter().enumerate() {
            self.project.check_cancelled()?;
            let depth = images
                .keys()
                .map(|uri| if absolute { Ok(0) } else { required_depth(uri) })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .max()
                .unwrap_or(0);
            let mut attempt = 0usize;
            let anchor = loop {
                let mut anchor = PathBuf::from(format!("origins/{index}/base"));
                for slot in 0..depth {
                    anchor.push(format!("origin-{attempt}-{slot}"));
                }
                let conflict = images.keys().any(|uri| {
                    crate::managed::normalize(&anchor.join(uri))
                        .is_ok_and(|target| anchor.starts_with(target))
                });
                if !conflict {
                    break anchor;
                }
                attempt += 1;
                if attempt > 4096 {
                    return Err(copy_error(
                        "Cannot allocate a collision-free sampler origin",
                    ));
                }
            };
            let projected = if absolute {
                None
            } else {
                match project_images(&anchor, &images, self.project.cancel.as_ref()) {
                    Ok(files) => Some(files),
                    // Only pure projection failures permit relocation. Existing
                    // files, root restrictions and publication are checked later.
                    Err(error) if matches!(error.code, "ASSET_CONFLICT" | "INVALID_ARGUMENT") => {
                        None
                    }
                    Err(error) => return Err(error),
                }
            };
            let relocate = projected.is_none();
            let mut variables = Map::new();
            let mut bindings = Vec::new();
            let mut relocated = Map::new();
            let mut packed = ImageFiles::new();
            for (uri, bytes) in images {
                self.project.check_cancelled()?;
                // Preserve visible text even when filesystem aliases cannot be
                // reproduced as a directory tree in a portable bundle.
                let portable_uri = if relocate {
                    let extension = image_extension(&bytes)?;
                    format!("images/{}.{extension}", hash(&bytes))
                } else {
                    uri.clone()
                };
                if relocate {
                    relocated.insert(uri.clone(), json!(portable_uri));
                    packed.insert(portable_uri, bytes);
                }
                let mut name = format!("lazypencil_image_{index}_{}", hash(uri.as_bytes()));
                while self.vars.values().any(|value| value == &name)
                    || self
                        .project
                        .documents
                        .values()
                        .any(|document| document.value()["variables"].get(&name).is_some())
                {
                    name.push('_');
                }
                variables.insert(name.clone(), json!({"type":"string","value":uri}));
                bindings.push(((origin.clone(), uri), name));
            }
            let mut files = match projected {
                Some(files) => files,
                None => project_images(&anchor, &packed, self.project.cancel.as_ref())?,
            };
            members.append(&mut files);
            let mut number = 0usize;
            let library = loop {
                let name =
                    member_name(&anchor.join(format!("lazypencil-values-{number}.lib.pen")))?;
                if !members.contains_key(&name)
                    && !members
                        .keys()
                        .any(|value| value.starts_with(&format!("{name}/")))
                {
                    break name;
                }
                number += 1;
            };
            let mut document = json!({"version":self.project.root_document().version(),"variables":variables,"children":[]});
            if !relocated.is_empty() {
                document["metadata"] =
                    json!({"lazypencil":{"sampler_images":{"version":1,"images":relocated}}});
            }
            let bytes: Arc<[u8]> = serde_json::to_vec(&document)
                .map_err(|_| copy_error("Cannot encode copied sampler variables"))?
                .into();
            insert_member(&mut members, library.clone(), bytes)?;
            libraries.push((library, bindings));
        }
        let document = self
            .path
            .ok_or_else(|| copy_error("Copying sampler images requires a bound document"))?;
        let files = crate::managed::File::bundle(document, members, self.project.cancel.as_ref())?;
        for (index, (library, bindings)) in libraries.into_iter().enumerate() {
            let file = files
                .iter()
                .find(|file| file.uri.ends_with(&format!("/{library}")))
                .expect("bundled variable document");
            let mut alias = format!(
                "lazypencil-images-{}-{index}",
                &hash(file.uri.as_bytes())[..16]
            );
            while self.project.imports[&self.project.root]
                .get(&alias)
                .is_some_and(|path| path != &file.path)
                || self.project.root_document().value()["variables"]
                    .as_object()
                    .is_some_and(|values| {
                        values
                            .keys()
                            .any(|name| name.starts_with(&format!("{alias}:")))
                    })
                || self
                    .copied_imports
                    .get(&alias)
                    .is_some_and(|value| value != file.uri.as_str())
            {
                alias.push('_');
            }
            self.copied_imports.insert(alias.clone(), json!(file.uri));
            for (key, name) in bindings {
                self.sampler_bindings
                    .insert(key, format!("${alias}:{name}"));
            }
        }
        for file in files {
            self.files.insert(file.path.clone(), file);
        }
        Ok(())
    }

    pub(super) fn rewrite_sampler_definition(&self, key: &Key, definition: &mut Value) {
        if !self.sampler_vars.contains(key) {
            return;
        }
        for value in definition_values_mut(&mut definition["value"]) {
            if let Some(uri) = value.as_str()
                && let Some(binding) = self.sampler_bindings.get(&(key.0.clone(), uri.into()))
            {
                *value = json!(binding);
            }
        }
    }
}

pub(super) fn definition_values(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(values) => values
            .iter()
            .filter_map(|value| value.get("value"))
            .collect(),
        value => vec![value],
    }
}
fn definition_values_mut(value: &mut Value) -> Vec<&mut Value> {
    match value {
        Value::Array(values) => values
            .iter_mut()
            .filter_map(|value| value.get_mut("value"))
            .collect(),
        value => vec![value],
    }
}

fn project_images(
    anchor: &Path,
    images: &ImageFiles,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<ImageFiles> {
    let mut members = ImageFiles::new();
    let mut directories = BTreeSet::new();
    for (uri, bytes) in images {
        crate::cancel::check(cancel)?;
        let mut target = anchor.to_path_buf();
        let parts = Path::new(uri).components().collect::<Vec<_>>();
        for (index, part) in parts.iter().enumerate() {
            match part {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    target.pop();
                }
                std::path::Component::Normal(part) => {
                    target.push(part);
                    if index + 1 < parts.len() {
                        directories.insert(member_name(&target)?);
                    }
                }
                _ => return Err(copy_error("Sampler image paths must be relative")),
            }
        }
        insert_member(&mut members, member_name(&target)?, bytes.clone())?;
    }
    // Preserve transit directories for paths such as a/../pixel.png.
    for directory in directories {
        if !members
            .keys()
            .any(|name| name.starts_with(&format!("{directory}/")))
        {
            insert_member(
                &mut members,
                format!("{directory}/.lazypencil-directory"),
                Arc::from(&b"{}\n"[..]),
            )?;
        }
    }
    crate::managed::validate_bundle_names(members.keys().map(String::as_str), cancel)?;
    Ok(members)
}

fn required_depth(uri: &str) -> Result<usize> {
    let (mut position, mut depth) = (0i32, 0i32);
    for part in Path::new(uri).components() {
        match part {
            std::path::Component::Normal(_) => position += 1,
            std::path::Component::ParentDir => {
                position -= 1;
                depth = depth.max(-position);
            }
            std::path::Component::CurDir => {}
            _ => return Err(copy_error("Sampler image paths must be relative")),
        }
    }
    if depth > 64 {
        return Err(copy_error(
            "Sampler image paths exceed 64 parent traversals",
        ));
    }
    Ok(depth as usize)
}
fn member_name(path: &Path) -> Result<String> {
    path.components()
        .map(|part| {
            part.as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| copy_error("Sampler paths must be UTF-8"))
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}
fn insert_member(
    members: &mut BTreeMap<String, Arc<[u8]>>,
    name: String,
    bytes: Arc<[u8]>,
) -> Result<()> {
    if members
        .get(&name)
        .is_some_and(|existing| existing.as_ref() != bytes.as_ref())
    {
        return Err(Error::new(
            "ASSET_CONFLICT",
            "Copied sampler paths have conflicting contents",
        ));
    }
    members.insert(name, bytes);
    Ok(())
}
