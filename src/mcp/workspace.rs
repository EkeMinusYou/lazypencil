//! Connection-local document handles and bounded discovery snapshots.
use super::*;
use crate::documents::{Limits, Listing, Scan};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Open {
    path: PathBuf,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct List {
    #[serde(default = "current")]
    directory: PathBuf,
    #[serde(default = "yes")]
    recursive: bool,
    #[serde(default)]
    query: Option<String>,
    #[serde(default = "hundred")]
    limit: usize,
    #[serde(default)]
    cursor: Option<String>,
}
fn current() -> PathBuf {
    PathBuf::from(".")
}
fn yes() -> bool {
    true
}
pub(super) fn tools() -> Vec<Tool> {
    let create = tool::<Open>(
        ToolKind::CreateDocument,
        "Create an empty .pen file in an existing directory without overwriting anything. Returns document_id for subsequent tools. Paths resolve against the server working directory unless absolute.",
    );
    vec![
        tool::<Open>(
            ToolKind::OpenDocument,
            "Open an existing .pen file and return its connection-local document_id and current metadata. Reopening the same canonical path returns the same ID. This does not execute scripts or initialize a renderer.",
        ),
        create,
        tool::<Empty>(
            ToolKind::CloseDocument,
            "Close this connection-local document handle, cancel its queued/running requests and release its calculations and runtime processes before acknowledgement. Reopen the path for a new ID.",
        ),
        tool::<List>(
            ToolKind::ListDocuments,
            "Discover .pen files below directory (default server working directory), recursively by default. Does not parse documents or follow directory symlinks. Search stops after 10,000 documents, 100,000 entries or 5 seconds; complete and stop_reason identify partial results. query filters collected relative paths. Reuse the same directory, recursive and query with cursor to page this retained listing.",
        ),
    ]
}
struct CachedListing {
    id: String,
    listing: Listing,
    recursive: bool,
    query: Option<String>,
    created: Instant,
    _lease: resources::Lease,
}
pub(super) struct Workspace {
    options: Options,
    documents: HashMap<String, actor::Actor>,
    retired: Vec<actor::Actor>,
    resources: std::sync::Arc<resources::Resources>,
    paths: HashMap<PathBuf, String>,
    listings: Vec<CachedListing>,
    owned: Option<Store>,
    usage: Option<crate::guidance::usage::UsageAccess>,
}
impl Workspace {
    pub fn new(mut options: Options) -> Result<Self> {
        options.roots = options
            .roots
            .iter()
            .map(|path| {
                let path = crate::paths::canonical(path)?;
                if !path.is_dir() {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "MCP roots must be directories",
                    ));
                }
                Ok(path)
            })
            .collect::<Result<_>>()?;
        let mut owned: Option<Store> = None;
        for path in [
            options
                .guidance_audit_manifest
                .as_deref()
                .map(crate::guidance::usage::UsageAccess::document_path),
            options
                .attachments_manifest
                .as_deref()
                .map(crate::attachments::AttachmentAccess::document_path),
        ]
        .into_iter()
        .flatten()
        {
            let store = Store::open(path?, options.data_dir.clone())?;
            if owned
                .as_ref()
                .is_some_and(|previous| previous.path() != store.path())
            {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Connection grants refer to different documents",
                ));
            }
            owned = Some(store);
        }
        let usage = options
            .guidance_audit_manifest
            .as_deref()
            .map(|path| {
                crate::guidance::usage::UsageAccess::open(
                    owned.as_ref().expect("usage document"),
                    path,
                )
            })
            .transpose()?;
        if let Some(path) = &options.attachments_manifest {
            crate::attachments::AttachmentAccess::open(
                owned.as_ref().expect("attachment document"),
                path,
            )?;
        }
        Ok(Self {
            options,
            documents: HashMap::new(),
            retired: Vec::new(),
            resources: std::sync::Arc::new(resources::Resources::default()),
            paths: HashMap::new(),
            listings: vec![],
            owned,
            usage,
        })
    }
    pub fn into_options(self) -> Options {
        self.options
    }
    pub fn usage(&self) -> Option<crate::guidance::usage::UsageAccess> {
        self.usage.clone()
    }
    pub fn has_preview(&self) -> bool {
        self.options.preview_control.is_some()
    }
    fn check_scope(&self, path: &Path) -> Result<()> {
        if !self.options.roots.is_empty()
            && !self.options.roots.iter().any(|root| path.starts_with(root))
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Path is outside the configured MCP roots",
            ));
        }
        Ok(())
    }
    fn open(
        &mut self,
        args: Open,
        create: bool,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult> {
        crate::cancel::check(Some(cancel))?;
        crate::documents::require_pen(&args.path)?;
        let path = if create {
            let parent = args
                .path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            crate::paths::canonical(parent)?.join(
                args.path
                    .file_name()
                    .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Missing document filename"))?,
            )
        } else {
            crate::paths::canonical(&args.path)?
        };
        crate::documents::require_pen(&path)?;
        self.check_scope(&path)?;
        if self
            .owned
            .as_ref()
            .is_some_and(|store| store.path() != path)
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The owned conversation grants authorize only their document",
            ));
        }
        if create && self.options.preview_control.is_some() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "A preview connection cannot create another document",
            ));
        }
        if create && self.paths.contains_key(&path) {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "The destination already exists",
            ));
        }
        let id = if let Some(id) = self.paths.get(&path) {
            id.clone()
        } else {
            if self.documents.len() + self.retired.len() >= 64 {
                return Err(Error::new(
                    "RESOURCE_LIMIT",
                    "At most 64 documents may be open on one connection; close_document to release handles",
                ));
            }
            let store = if create {
                Store::init(&path, self.options.data_dir.clone())?
            } else {
                Store::open(&path, self.options.data_dir.clone())?
            };
            self.check_scope(store.path())?;
            // Validate the stored format without layout, scripts, fonts or network.
            store.read()?;
            crate::cancel::check(Some(cancel))?;
            let attachments = self
                .options
                .attachments_manifest
                .as_deref()
                .map(|path| crate::attachments::AttachmentAccess::open(&store, path))
                .transpose()?;
            let preview_control = self
                .options
                .preview_control
                .as_deref()
                .map(|value| crate::preview_control::Client::parse(value, store.path()))
                .transpose()?;
            let document = actor::Seed {
                store,
                fonts: self.options.fonts.clone(),
                attachments,
                usage: self.usage.clone(),
                preview_control,
            };
            let id = uuid::Uuid::new_v4().to_string();
            let actor = actor::Actor::spawn(id.clone(), document, self.resources.clone())?;
            self.paths.insert(path.clone(), id.clone());
            self.documents.insert(id.clone(), actor);
            id
        };
        let document = self.documents[&id].store.read()?;
        structured(
            json!({"document_id":id,"path":path,"revision":document.revision(),"version":document.value()["version"]}),
        )
    }
    fn list(
        &mut self,
        args: List,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult> {
        if !(1..=500).contains(&args.limit) {
            return Err(Error::new("INVALID_ARGUMENT", "limit must be 1..500"));
        }
        let directory = crate::paths::canonical(&args.directory)?;
        self.check_scope(&directory)?;
        if let Some(owned) = &self.owned
            && owned.path().parent() != Some(directory.as_path())
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Owned discovery is limited to the document directory",
            ));
        }
        self.listings
            .retain(|listing| listing.created.elapsed() < Duration::from_secs(300));
        let (index, offset) = if let Some(cursor) = &args.cursor {
            let (id, offset) = cursor
                .split_once(':')
                .ok_or_else(|| Error::new("INVALID_ARGUMENT", "Invalid document cursor"))?;
            let index = self
                .listings
                .iter()
                .position(|listing| listing.id == id)
                .ok_or_else(|| {
                    Error::new(
                        "SNAPSHOT_UNAVAILABLE",
                        "Document listing expired; restart without cursor",
                    )
                })?;
            let cached = &self.listings[index];
            if cached.listing.root != directory
                || cached.recursive != args.recursive
                || cached.query != args.query
            {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Cursor belongs to another document query",
                ));
            }
            (
                index,
                offset
                    .parse::<usize>()
                    .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid document cursor"))?,
            )
        } else {
            let mut listing =
                Scan::new(&directory, args.recursive, Limits::default())?.finish(cancel);
            crate::cancel::check(Some(cancel))?;
            if let Some(query) = &args.query {
                let query = query.to_lowercase();
                listing
                    .documents
                    .retain(|path| path.to_string_lossy().to_lowercase().contains(&query));
            }
            if let Some(owned) = &self.owned {
                listing
                    .documents
                    .retain(|path| directory.join(path) == owned.path());
            }
            if self.listings.len() >= 4 {
                self.listings.remove(0);
            }
            let bytes = listing
                .documents
                .iter()
                .map(|path| path.as_os_str().len() + 64)
                .sum::<usize>()
                + listing.root.as_os_str().len();
            let lease = loop {
                match self.resources.retained.reserve(bytes) {
                    Ok(lease) => break lease,
                    Err(_) if !self.listings.is_empty() => {
                        self.listings.remove(0);
                    }
                    Err(error) => return Err(error),
                }
            };
            self.listings.push(CachedListing {
                _lease: lease,
                id: uuid::Uuid::new_v4().to_string(),
                listing,
                recursive: args.recursive,
                query: args.query,
                created: Instant::now(),
            });
            (self.listings.len() - 1, 0)
        };
        let cached = &self.listings[index];
        let listing = &cached.listing;
        if offset > listing.documents.len() {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Document cursor is out of range",
            ));
        }
        let end = (offset + args.limit).min(listing.documents.len());
        structured(
            json!({"directory":listing.root,"documents":&listing.documents[offset..end],"total_matches":listing.documents.len(),"complete":listing.complete,"stop_reason":listing.stop_reason,"entries_scanned":listing.entries_scanned,"unreadable_directories":listing.unreadable_directories,"next_cursor":(end < listing.documents.len()).then(|| format!("{}:{end}",cached.id))}),
        )
    }
    pub fn call(
        &mut self,
        name: &str,
        arguments: Value,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult> {
        crate::cancel::check(Some(cancel))?;
        let expired = || {
            Error::new(
                "AGENT_ERROR",
                "The owned MCP connection is no longer active; reconnect to issue a new grant",
            )
        };
        #[cfg(unix)]
        if let Some(usage) = &self.usage {
            usage.check_connection().map_err(|_| expired())?;
        }
        if let Some(owned) = &self.owned
            && let Some(path) = &self.options.attachments_manifest
        {
            crate::attachments::AttachmentAccess::open(owned, path).map_err(|_| expired())?;
        }
        let kind =
            ToolKind::parse(name).ok_or_else(|| Error::new("INVALID_ARGUMENT", "Unknown tool"))?;
        match kind {
            ToolKind::OpenDocument => return self.open(parse(arguments)?, false, cancel),
            ToolKind::CreateDocument => return self.open(parse(arguments)?, true, cancel),
            ToolKind::ListDocuments => return self.list(parse(arguments)?, cancel),
            ToolKind::ListIcons => {
                return crate::icons::list(
                    &crate::store::data_directory(self.options.data_dir.clone())?,
                    &parse(arguments)?,
                )
                .and_then(structured);
            }
            _ => {}
        }
        let value = match kind {
            ToolKind::GetGuidance => {
                let args: GuideArgs = parse(arguments)?;
                crate::guidance::get_guidance(args.id.as_deref())?
            }
            ToolKind::ListStyles => {
                let args: StylesArgs = parse(arguments)?;
                crate::guidance::list_styles(
                    args.use_case.as_deref(),
                    args.limit,
                    args.cursor.as_deref(),
                )?
            }
            ToolKind::GetStyle => {
                let args: StyleArgs = parse(arguments)?;
                crate::guidance::get_style(&args.id)?
            }

            _ => {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "document_id is required; call open_document or create_document first",
                ));
            }
        };
        structured(value)
    }
    pub fn dispatch(&mut self, mut job: Job) {
        self.retired.retain_mut(|actor| !actor.reap());
        if job.cancelled() {
            job.finish(
                Err(Error::new(
                    "CANCELLED",
                    "The request was cancelled before dispatch",
                )),
                None,
            );
            return;
        }
        let kind = ToolKind::parse(&job.name);
        if !matches!(
            kind,
            Some(
                ToolKind::OpenDocument
                    | ToolKind::CreateDocument
                    | ToolKind::ListDocuments
                    | ToolKind::ListIcons
            )
        ) && let Some(id) = job
            .arguments
            .as_object_mut()
            .and_then(|args| args.remove("document_id"))
        {
            let result = (|| {
                let id = id.as_str().ok_or_else(|| {
                    Error::new("INVALID_ARGUMENT", "document_id must be a string")
                })?;
                if kind == Some(ToolKind::CloseDocument) {
                    let _: Empty = parse(job.arguments.clone())?;
                }
                if !self.documents.contains_key(id) {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "Unknown document_id; open the document on this connection first",
                    ));
                }
                Ok(id.to_owned())
            })();
            match result {
                Err(error) => job.finish(Err(error), Some(Instant::now())),
                Ok(id) if kind == Some(ToolKind::CloseDocument) => {
                    let mut actor = self.documents.remove(&id).expect("checked handle");
                    self.paths.retain(|_, value| value != &id);
                    actor.close(job);
                    self.retired.push(actor);
                }
                Ok(id) => self
                    .documents
                    .get_mut(&id)
                    .expect("checked handle")
                    .enqueue(job),
            }
        } else {
            let started = Instant::now();
            let result = self.call(&job.name, job.arguments.clone(), &job.cancel);
            job.finish(result, Some(started));
        }
    }
}
