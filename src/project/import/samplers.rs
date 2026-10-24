use super::*;

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
        let mut images: BTreeMap<PathBuf, BTreeMap<String, Arc<[u8]>>> = BTreeMap::new();
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
                    if Path::new(uri).is_absolute() {
                        return Err(Error::new(
                            "UNSUPPORTED_FEATURE",
                            "Independent copying of absolute string sampler paths requires a portable binding policy",
                        ));
                    }
                    let path = self.project.resolve_file(&key.0, uri)?;
                    self.project.capture_asset(&path, "image")?;
                    let bytes = self.project.asset(&path)?.bytes.clone();
                    if !(bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                        || bytes.starts_with(&[0xff, 0xd8, 0xff])
                        || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
                    {
                        return Err(copy_error(
                            "Copied sampler images require PNG, JPEG, or WebP data",
                        ));
                    }
                    images
                        .entry(key.0.clone())
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
        let mut directories = BTreeSet::new();
        let mut libraries = Vec::new();
        for (index, (origin, images)) in images.into_iter().enumerate() {
            self.project.check_cancelled()?;
            let depth = images
                .keys()
                .map(|uri| required_depth(uri))
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
            let mut variables = Map::new();
            let mut bindings = Vec::new();
            for (uri, bytes) in images {
                self.project.check_cancelled()?;
                let mut target = anchor.clone();
                let parts = Path::new(&uri).components().collect::<Vec<_>>();
                for (i, part) in parts.iter().enumerate() {
                    match part {
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir => {
                            target.pop();
                        }
                        std::path::Component::Normal(part) => {
                            target.push(part);
                            if i + 1 < parts.len() {
                                directories.insert(member_name(&target)?);
                            }
                        }
                        _ => return Err(copy_error("Sampler image paths must be relative")),
                    }
                }
                insert_member(&mut members, member_name(&target)?, bytes)?;
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
            let bytes: Arc<[u8]> = serde_json::to_vec(
                &json!({"version":self.project.root_document().version(),"variables":variables,"children":[]}),
            )
            .map_err(|_| copy_error("Cannot encode copied sampler variables"))?
            .into();
            insert_member(&mut members, library.clone(), bytes)?;
            libraries.push((library, bindings));
        }
        // A URI such as a/../pixel.png still needs the a directory to exist.
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

fn definition_values(value: &Value) -> Vec<&Value> {
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
