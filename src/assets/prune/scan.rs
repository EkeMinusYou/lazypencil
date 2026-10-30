use super::*;
use crate::{document::Document, project::Project, store::HistoryEntry};
use std::fs;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Source {
    File { content: Content },
    Directory { entries: Value },
    Missing { snapshot: crate::paths::MissingPath },
    Alias { canonical: PathBuf },
}

#[derive(Default)]
pub(super) struct Scan {
    pub sources: BTreeMap<PathBuf, Source>,
    pub identities: BTreeSet<String>,
    pub owned: Vec<Candidate>,
    pub pending: Vec<trash::Journal>,
    pub pending_payloads: BTreeMap<PathBuf, BTreeSet<String>>,
    pub retained: BTreeMap<PathBuf, String>,
    pub uncertain: BTreeSet<String>,
    pub references: BTreeSet<String>,
    pub paths: BTreeSet<PathBuf>,
    projects: Vec<Project>,
    documents: Vec<(PathBuf, Document)>,
    owners: BTreeMap<String, PathBuf>,
}

impl Scan {
    pub fn protected(&self, asset: &Candidate) -> bool {
        !self.uncertain.is_empty()
            || self.references.contains(&asset.digest)
            || self.paths.iter().any(|path| {
                path == &asset.path
                    || asset.bundle && path.starts_with(&asset.path)
                    || asset.path.starts_with(path)
            })
    }

    pub fn verify(&self, budget: &mut Budget<'_>) -> Result<()> {
        for (path, source) in &self.sources {
            budget.check()?;
            let matches = match source {
                Source::File { content } => Content::of(&budget.read(path, MAX_FILE)?) == *content,
                Source::Directory { entries } => {
                    json!(filesystem::entries(path, MAX_ENTRIES, budget.cancel)?) == *entries
                }
                Source::Missing { snapshot } => {
                    snapshot.verify()?;
                    true
                }
                Source::Alias { canonical } => {
                    crate::paths::canonical(path).is_ok_and(|current| &current == canonical)
                }
            };
            if !matches {
                return Err(changed());
            }
        }
        for project in &self.projects {
            budget.check()?;
            project.verify_unchanged()?;
        }
        budget.check()
    }

    fn entries(&mut self, path: &Path, budget: &mut Budget<'_>) -> Result<Vec<filesystem::Entry>> {
        let entries = filesystem::entries(path, MAX_ENTRIES, budget.cancel)?;
        budget.entries(entries.len())?;
        self.sources.insert(
            path.into(),
            Source::Directory {
                entries: json!(entries),
            },
        );
        Ok(entries)
    }

    fn missing(&mut self, path: &Path, budget: &Budget<'_>) -> Result<bool> {
        budget.check()?;
        if let Some(snapshot) = crate::paths::MissingPath::capture(path)? {
            snapshot.verify()?;
            self.sources
                .insert(path.into(), Source::Missing { snapshot });
            Ok(true)
        } else {
            filesystem::entries(path, MAX_ENTRIES, budget.cancel)?;
            Ok(false)
        }
    }

    fn read(&mut self, path: &Path, limit: usize, budget: &mut Budget<'_>) -> Result<Vec<u8>> {
        let bytes = budget.read(path, limit)?;
        let source = Source::File {
            content: Content::of(&bytes),
        };
        if let Some(previous) = self.sources.insert(path.into(), source.clone())
            && serde_json::to_value(previous).unwrap() != serde_json::to_value(source).unwrap()
        {
            return Err(changed());
        }
        Ok(bytes)
    }

    fn tree(
        &mut self,
        path: &Path,
        data: &Path,
        depth: usize,
        budget: &mut Budget<'_>,
    ) -> Result<()> {
        if depth > 64 {
            return Err(Error::new(
                "RESOURCE_LIMIT",
                "Project tree exceeds 64 levels",
            ));
        }
        if path.ends_with("assets/lazypencil") {
            return self.assets(path, budget);
        }
        for entry in self.entries(path, budget)? {
            let child = path.join(entry.name);
            if child == data {
                continue;
            }
            match entry.kind {
                filesystem::Kind::Directory => self.tree(&child, data, depth + 1, budget)?,
                filesystem::Kind::File
                    if child
                        .extension()
                        .is_some_and(|extension| extension == "pen") =>
                {
                    let bytes = self.read(&child, crate::document::MAX_DOCUMENT_BYTES, budget)?;
                    match document(bytes) {
                        Ok(document) => self.documents.push((child, document)),
                        Err(error) => {
                            self.uncertain
                                .insert(format!("{}: {}", child.display(), error.code));
                        }
                    }
                }
                filesystem::Kind::Link => {
                    self.uncertain
                        .insert(format!("{}: unscanned symbolic link", child.display()));
                }
                _ => (),
            }
        }
        Ok(())
    }

    fn assets(&mut self, path: &Path, budget: &mut Budget<'_>) -> Result<()> {
        let entries = filesystem::entries(path, MAX_ENTRIES, budget.cancel)?;
        budget.entries(entries.len())?;
        let mut pending = BTreeSet::new();
        for entry in entries {
            let child = path.join(&entry.name);
            if entry.name.starts_with(TRASH) {
                match trash::load(&child, budget) {
                    Ok(journal) => {
                        if pending.insert(journal.directory()) {
                            self.pending_payloads.insert(
                                journal.directory(),
                                trash::payload_state(&journal, budget)?,
                            );
                            self.pending.push(journal);
                        }
                    }
                    Err(error) => {
                        self.uncertain
                            .insert(format!("{}: {}", child.display(), error.code));
                    }
                }
            } else if digest_name(&entry.name).is_some() {
                match candidate(&child, budget) {
                    Ok(asset) => self.owned.push(asset),
                    Err(error) => {
                        self.retained
                            .insert(child, format!("Ownership not verified: {}", error.code));
                    }
                }
            } else {
                self.retained
                    .insert(child, "Not an owned content-addressed asset".into());
            }
        }
        Ok(())
    }

    fn raw_references(
        &mut self,
        owner: &Path,
        value: &Value,
        budget: &mut Budget<'_>,
    ) -> Result<()> {
        budget.check()?;
        match value {
            Value::String(text) => {
                // Check every stored string: inactive themes, disabled nodes,
                // unused definitions and unknown metadata can retain assets.
                for part in text.split(|character: char| !character.is_ascii_hexdigit()) {
                    if part.len() == 64 {
                        self.references.insert(part.to_ascii_lowercase());
                    }
                }
                let path = if text.starts_with("file:") {
                    crate::paths::local_file_url(text).ok()
                } else if !text.is_empty()
                    && !text.contains("://")
                    && !crate::inline_images::is_data(text)
                    && !text.starts_with('$')
                {
                    owner.parent().map(|parent| parent.join(text))
                } else {
                    None
                };
                if let Some(path) = path
                    && let Ok(path) = crate::paths::canonical(&path)
                {
                    self.paths.insert(path);
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.raw_references(owner, value, budget)?;
                }
            }
            Value::Object(values) => {
                if values.get("type").is_some_and(|kind| kind == "script") {
                    // A program can synthesize arbitrary future URIs. One
                    // execution, or a possibly stale raw binding, is not a
                    // proof that its complete reference closure is bounded.
                    self.uncertain.insert(format!(
                        "{}: dynamic script reference closure",
                        owner.display()
                    ));
                }
                for (key, value) in values {
                    self.raw_references(owner, &Value::String(key.clone()), budget)?;
                    self.raw_references(owner, value, budget)?;
                }
            }
            _ => (),
        }
        Ok(())
    }

    fn capture(
        &mut self,
        store: &Store,
        path: &Path,
        document: Document,
        budget: &mut Budget<'_>,
    ) -> Result<()> {
        for diagnostic in document.diagnostics() {
            self.uncertain
                .insert(format!("{}: {}", path.display(), diagnostic.code));
        }
        self.raw_references(path, document.value(), budget)?;
        self.owners
            .insert(hash(path.as_os_str().as_encoded_bytes()), path.into());
        budget.entries(1)?;
        if self.projects.len() >= 256 {
            return Err(Error::new(
                "RESOURCE_LIMIT",
                "Asset maintenance exceeds 256 project snapshots",
            ));
        }
        match Project::for_asset_retention(store, path, document, budget.cancel) {
            Ok(project) => {
                if let Some(error) = project.resolution_error() {
                    self.uncertain
                        .insert(format!("{}: {}", path.display(), error.code));
                }
                self.identities.insert(project.read_identity()?);
                for (owner, document) in project.retention_documents() {
                    self.raw_references(owner, document.value(), budget)?;
                    self.owners
                        .insert(hash(owner.as_os_str().as_encoded_bytes()), owner.clone());
                    // The supplied root may be a historical snapshot. Its
                    // bytes are already bound through the checkpoint file.
                    if owner != path {
                        let bytes =
                            self.read(owner, crate::document::MAX_DOCUMENT_BYTES, budget)?;
                        if hash(&bytes) != document.revision() {
                            return Err(changed());
                        }
                    }
                }
                for dependency in project.dependencies() {
                    if !dependency.is_file() {
                        continue;
                    }
                    self.paths.insert(dependency.path.clone());
                    if dependency.availability.is_none() && dependency.kind != "font" {
                        let bytes = self.read(&dependency.path, MAX_FILE, budget)?;
                        if hash(&bytes) != dependency.revision {
                            return Err(changed());
                        }
                    }
                }
                self.projects.push(project);
            }
            Err(error) if matches!(error.code, "CANCELLED" | "REVISION_CONFLICT") => {
                return Err(error);
            }
            Err(error) => {
                self.uncertain
                    .insert(format!("{}: {}", path.display(), error.code));
            }
        }
        budget.check()
    }

    fn histories(&mut self, store: &Store, data: &Path, budget: &mut Budget<'_>) -> Result<()> {
        let documents = data.join("documents");
        if self.missing(&documents, budget)? {
            return Ok(());
        }
        for namespace in self.entries(&documents, budget)? {
            let path = documents.join(&namespace.name);
            if namespace.kind != filesystem::Kind::Directory {
                self.uncertain
                    .insert(format!("{}: unknown document namespace", path.display()));
                continue;
            }
            let history = path.join("history");
            if self.missing(&history, budget)? {
                continue;
            }
            for entry in self.entries(&history, budget)? {
                let checkpoint = history.join(&entry.name);
                if entry.kind != filesystem::Kind::Directory {
                    self.uncertain
                        .insert(format!("{}: unknown checkpoint", checkpoint.display()));
                    continue;
                }
                self.entries(&checkpoint, budget)?;
                let metadata = self.read(
                    &checkpoint.join("metadata.json"),
                    crate::document::MAX_DOCUMENT_BYTES,
                    budget,
                )?;
                let entry: HistoryEntry = serde_json::from_slice(&metadata).map_err(|_| {
                    Error::new("RECOVERY_REQUIRED", "Cannot verify checkpoint metadata")
                })?;
                if entry.id != checkpoint.file_name().unwrap().to_string_lossy()
                    || uuid::Uuid::parse_str(&entry.id).is_err()
                {
                    return Err(Error::new(
                        "RECOVERY_REQUIRED",
                        "Checkpoint identity mismatch",
                    ));
                }
                let owner = entry
                    .document_path
                    .or_else(|| self.owners.get(&namespace.name).cloned());
                if let Some(owner) = &owner
                    && (!owner.is_absolute()
                        || hash(owner.as_os_str().as_encoded_bytes()) != namespace.name)
                {
                    return Err(Error::new("RECOVERY_REQUIRED", "Checkpoint owner mismatch"));
                }
                for (name, revision) in [
                    ("before.pen", entry.before_revision),
                    ("after.pen", entry.after_revision),
                ] {
                    let path = checkpoint.join(name);
                    let bytes = self.read(&path, crate::document::MAX_DOCUMENT_BYTES, budget)?;
                    if hash(&bytes) != revision {
                        return Err(Error::new(
                            "RECOVERY_REQUIRED",
                            "Checkpoint content mismatch",
                        ));
                    }
                    let document = document(bytes)?;
                    match &owner {
                        Some(owner) => self.capture(store, owner, document, budget)?,
                        None => {
                            self.raw_references(&path, document.value(), budget)?;
                            self.uncertain.insert(format!(
                                "{}: legacy checkpoint owner is unknown",
                                checkpoint.display()
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn document(bytes: Vec<u8>) -> Result<Document> {
    Document::parse(
        String::from_utf8(bytes)
            .map_err(|_| Error::new("INVALID_DOCUMENT", "Document must be UTF-8"))?,
    )
}

pub(super) fn collect(store: &Store, root: &Path, budget: &mut Budget<'_>) -> Result<Scan> {
    let absent_data = crate::paths::MissingPath::capture(store.data_dir())?;
    let data = match &absent_data {
        Some(snapshot) => snapshot.target.clone(),
        None => crate::paths::canonical(store.data_dir())?,
    };
    if data == root {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Project root must differ from the user data directory",
        ));
    }
    let mut scan = Scan::default();
    if let Some(snapshot) = absent_data {
        scan.sources
            .insert(store.data_dir().into(), Source::Missing { snapshot });
    } else if data != store.data_dir() {
        scan.sources.insert(
            store.data_dir().into(),
            Source::Alias {
                canonical: data.clone(),
            },
        );
    }
    scan.tree(root, &data, 0, budget)?;
    scan.paths.insert(store.path().into());
    if !scan.documents.iter().any(|(path, _)| path == store.path()) {
        let bytes = scan.read(store.path(), crate::document::MAX_DOCUMENT_BYTES, budget)?;
        scan.documents.push((store.path().into(), document(bytes)?));
    }
    for (path, document) in std::mem::take(&mut scan.documents) {
        scan.capture(store, &path, document, budget)?;
    }
    scan.histories(store, &data, budget)?;
    for journal in &scan.pending {
        if journal.root != root || journal.document != store.path() {
            return Err(Error::new(
                "RECOVERY_REQUIRED",
                "A pending asset journal belongs to another maintenance scope",
            ));
        }
    }
    for asset in &scan.owned {
        if scan.protected(asset) {
            scan.retained.insert(
                asset.path.clone(),
                if scan.uncertain.is_empty() {
                    "Referenced by a document, dependency, stored theme or retained checkpoint"
                        .into()
                } else {
                    "Reference closure is uncertain; retained conservatively".into()
                },
            );
        }
    }
    scan.verify(budget)?;
    Ok(scan)
}

pub(super) fn candidate(path: &Path, budget: &mut Budget<'_>) -> Result<Candidate> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(changed)?;
    let digest = digest_name(name).ok_or_else(changed)?.to_owned();
    let bundle = name.ends_with(".bundle");
    let mut files = BTreeMap::new();
    if bundle {
        let bytes = budget.read(&path.join("bundle.json"), 2 * 1024 * 1024)?;
        if hash(&bytes) != digest {
            return Err(changed());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Manifest {
            version: u32,
            files: BTreeMap<String, Content>,
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| changed())?;
        if manifest.version != 1
            || manifest.files.is_empty()
            || manifest.files.len() > 4097
            || manifest
                .files
                .values()
                .any(|file| file.bytes > MAX_FILE as u64)
            || manifest.files.values().map(|file| file.bytes).sum::<u64>() > 128 * 1024 * 1024
            || manifest.files.keys().map(String::len).sum::<usize>() > 1024 * 1024
        {
            return Err(changed());
        }
        crate::managed::validate_bundle_names(
            manifest.files.keys().map(String::as_str),
            budget.cancel,
        )?;
        files = manifest.files;
        files.insert("bundle.json".into(), Content::of(&bytes));
    } else {
        let bytes = budget.read(path, MAX_FILE)?;
        if bytes.is_empty() || hash(&bytes) != digest {
            return Err(changed());
        }
        files.insert(String::new(), Content::of(&bytes));
    }
    let asset = Candidate {
        path: path.into(),
        digest,
        bundle,
        files,
    };
    verify_candidate(&asset, false, budget)?;
    Ok(asset)
}

pub(super) fn verify_candidate(
    asset: &Candidate,
    partial: bool,
    budget: &mut Budget<'_>,
) -> Result<()> {
    candidate_state(asset, partial, budget).map(|_| ())
}

pub(super) fn candidate_state(
    asset: &Candidate,
    partial: bool,
    budget: &mut Budget<'_>,
) -> Result<BTreeSet<String>> {
    let mut seen = BTreeSet::new();
    if asset.bundle {
        verify_tree(asset, &asset.path, 0, &mut seen, budget)?;
    } else {
        match budget.read(&asset.path, MAX_FILE) {
            Ok(bytes) if asset.files.get("") == Some(&Content::of(&bytes)) => {
                seen.insert(String::new());
            }
            Ok(_) => return Err(changed()),
            Err(error)
                if partial
                    && !asset
                        .path
                        .try_exists()
                        .map_err(|io| Error::io(&asset.path, io))? =>
            {
                // A symlink is not an absent payload.
                if fs::symlink_metadata(&asset.path).is_ok() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    if !partial && seen.len() != asset.files.len() {
        return Err(changed());
    }
    Ok(seen)
}

fn verify_tree(
    asset: &Candidate,
    path: &Path,
    depth: usize,
    seen: &mut BTreeSet<String>,
    budget: &mut Budget<'_>,
) -> Result<()> {
    if depth > 128 {
        return Err(changed());
    }
    let entries = filesystem::entries(path, 8194, budget.cancel)?;
    budget.entries(entries.len())?;
    for entry in entries {
        let child = path.join(entry.name);
        let relative = child
            .strip_prefix(&asset.path)
            .map_err(|_| changed())?
            .to_str()
            .ok_or_else(changed)?
            .to_owned();
        match entry.kind {
            filesystem::Kind::Directory
                if asset
                    .files
                    .keys()
                    .any(|name| name.starts_with(&format!("{relative}/"))) =>
            {
                verify_tree(asset, &child, depth + 1, seen, budget)?;
            }
            filesystem::Kind::File => {
                if asset.files.get(&relative) != Some(&Content::of(&budget.read(&child, MAX_FILE)?))
                {
                    return Err(changed());
                }
                seen.insert(relative);
            }
            _ => return Err(changed()),
        }
    }
    Ok(())
}
