//! Conversation-owned image files and revocable, document-bound MCP grants.

use crate::{
    Error, Result,
    document::hash,
    store::{Store, atomic_replace, create_private_directory},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_REQUEST_IMAGES: usize = 8;
pub const MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub id: String,
    pub display_name: String,
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bytes: Option<usize>,
}

impl Attachment {
    pub fn from_file(directory: &Path, source: &Path) -> Result<Self> {
        let bytes = read_bounded(source, crate::image_input::MAX_SOURCE_BYTES)?;
        Self::from_bytes(
            directory,
            &bytes,
            &source.file_name().unwrap_or_default().to_string_lossy(),
        )
    }

    pub fn from_bytes(directory: &Path, source: &[u8], display_name: &str) -> Result<Self> {
        let image = crate::image_input::normalize(source, None)?;
        let hash = hash(&image.png);
        create_private_directory(&directory.join("attachments"))?;
        let attachment = Self {
            id: format!("img_{hash}"),
            display_name: display_name.into(),
            hash,
            width: image.width,
            height: image.height,
            bytes: image.png.len(),
            source_bytes: Some(source.len()),
        };
        let path = attachment.path(directory)?;
        if path.exists() {
            attachment.read(directory)?;
        } else {
            atomic_replace(&path, &image.png)?;
        }
        Ok(attachment)
    }

    fn path(&self, directory: &Path) -> Result<PathBuf> {
        if self.hash.len() != 64
            || !self.hash.bytes().all(|b| b.is_ascii_hexdigit())
            || self.id != format!("img_{}", self.hash)
        {
            return Err(invalid("Invalid attachment identity"));
        }
        Ok(directory
            .join("attachments")
            .join(format!("{}.png", self.hash)))
    }

    pub(crate) fn remove_file(&self, directory: &Path) -> Result<()> {
        let path = self.path(directory)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::io(&path, error)),
        }
    }

    pub fn read(&self, directory: &Path) -> Result<Vec<u8>> {
        let bytes = read_bounded(&self.path(directory)?, crate::image_input::MAX_SOURCE_BYTES)?;
        if bytes.len() != self.bytes || hash(&bytes) != self.hash {
            return Err(invalid(
                "A stored attachment changed; it was not sent or imported",
            ));
        }
        Ok(bytes)
    }

    pub fn content(
        &self,
        directory: &Path,
    ) -> Result<agent_client_protocol::schema::v1::ContentBlock> {
        use agent_client_protocol::schema::v1::{ContentBlock, ImageContent};
        Ok(ContentBlock::Image(ImageContent::new(
            base64::engine::general_purpose::STANDARD.encode(self.read(directory)?),
            "image/png",
        )))
    }
}

pub fn prepare(directory: &Path, files: &[PathBuf]) -> Result<Vec<Attachment>> {
    if files.len() > MAX_REQUEST_IMAGES {
        return Err(invalid("A request accepts at most eight images"));
    }
    let mut result = Vec::new();
    let mut source_total = 0usize;
    let mut normalized_total = 0usize;
    for file in files {
        let bytes = read_bounded(file, crate::image_input::MAX_SOURCE_BYTES)?;
        source_total = source_total.saturating_add(bytes.len());
        if source_total > MAX_REQUEST_BYTES {
            return Err(invalid("Input images exceed 50 MiB in total"));
        }
        let attachment = Attachment::from_bytes(
            directory,
            &bytes,
            &file.file_name().unwrap_or_default().to_string_lossy(),
        )?;
        normalized_total += attachment.bytes;
        if normalized_total > MAX_REQUEST_BYTES {
            return Err(invalid("Normalized images exceed 50 MiB in total"));
        }
        result.push(attachment);
    }
    Ok(result)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    document: PathBuf,
    conversation: String,
    attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
}

/// The grant filename is unique per connection. Removing it revokes future requests.
pub struct AttachmentGrant {
    path: PathBuf,
    #[cfg(unix)]
    lease: crate::conversation::lease::Lease,
}
impl AttachmentGrant {
    pub fn create(
        store: &Store,
        directory: &Path,
        conversation: &str,
        attachments: &[Attachment],
    ) -> Result<Self> {
        if attachments.len() > 256 {
            return Err(invalid(
                "A conversation accepts at most 256 retained images",
            ));
        }
        let path = directory.join(format!("grant-{}.json", uuid::Uuid::new_v4()));
        #[cfg(unix)]
        let lease = crate::conversation::lease::Lease::create(&path)?;
        let manifest = Manifest {
            document: store.path().into(),
            conversation: conversation.into(),
            attachments: attachments.into(),
            #[cfg(unix)]
            owner: Some(lease.id().into()),
            #[cfg(not(unix))]
            owner: None,
        };
        atomic_replace(
            &path,
            &serde_json::to_vec(&manifest).map_err(|e| invalid(e.to_string()))?,
        )?;
        Ok(Self {
            path,
            #[cfg(unix)]
            lease,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn update(
        &self,
        store: &Store,
        conversation: &str,
        attachments: &[Attachment],
    ) -> Result<()> {
        if attachments.len() > 256 {
            return Err(invalid(
                "A conversation accepts at most 256 retained images",
            ));
        }
        let manifest = Manifest {
            document: store.path().into(),
            conversation: conversation.into(),
            attachments: attachments.into(),
            #[cfg(unix)]
            owner: Some(self.lease.id().into()),
            #[cfg(not(unix))]
            owner: None,
        };
        atomic_replace(
            &self.path,
            &serde_json::to_vec(&manifest).map_err(|error| invalid(error.to_string()))?,
        )
    }
}
impl Drop for AttachmentGrant {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub struct AttachmentAccess {
    path: PathBuf,
    #[cfg(unix)]
    owner: Option<String>,
}
impl AttachmentAccess {
    #[cfg(unix)]
    pub(crate) fn check_connection(&self, store: &Store) -> Result<()> {
        self.manifest(store).map(|_| ())
    }

    pub fn open(store: &Store, path: &Path) -> Result<Self> {
        let path = crate::paths::canonical(path)?;
        let manifest = Self::read_manifest(&path, store)?;
        let access = Self {
            path,
            #[cfg(unix)]
            owner: manifest.owner,
        };
        #[cfg(not(unix))]
        let _ = manifest;
        access.manifest(store)?;
        Ok(access)
    }
    fn manifest(&self, store: &Store) -> Result<Manifest> {
        let manifest = Self::read_manifest(&self.path, store)?;
        #[cfg(unix)]
        {
            if manifest.owner != self.owner {
                return Err(invalid("The attachment connection changed its identity"));
            }
            crate::conversation::lease::require_live(&self.path, self.owner.as_deref())
                .map_err(|error| invalid(error.message))?;
        }
        Ok(manifest)
    }
    fn read_manifest(path: &Path, store: &Store) -> Result<Manifest> {
        let bytes = read_bounded(path, 256 * 1024)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid attachment grant"))?;
        if manifest.document != store.path() || manifest.attachments.len() > 256 {
            return Err(invalid(
                "The attachment grant belongs to a different document or exceeds its limit",
            ));
        }
        Ok(manifest)
    }
    pub fn metadata(&self, store: &Store) -> Result<Value> {
        let manifest = self.manifest(store)?;
        Ok(json!({"conversation":manifest.conversation,"images":manifest.attachments}))
    }
    pub fn read(&self, store: &Store, id: &str) -> Result<Vec<u8>> {
        let manifest = self.manifest(store)?;
        let attachment = manifest
            .attachments
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| invalid("This attachment ID is not registered for this connection"))?;
        attachment.read(
            self.path
                .parent()
                .ok_or_else(|| invalid("Missing attachment directory"))?,
        )
    }
}

#[cfg(unix)]
pub(crate) fn reclaim_stale_grant(
    path: &Path,
    document: &Path,
    conversation: &str,
) -> Result<bool> {
    let bytes = read_bounded(path, 256 * 1024)?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid attachment grant"))?;
    if manifest.document != document
        || manifest.conversation != conversation
        || manifest.attachments.len() > 256
    {
        return Ok(false);
    }
    match manifest.owner.as_deref() {
        Some(owner) => crate::conversation::lease::remove_stale(path, owner),
        None => Ok(false),
    }
}

pub(crate) fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A named pipe must not block before the regular-file check below.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| Error::io(path, e))?;
    if !file.metadata().map_err(|e| Error::io(path, e))?.is_file() {
        return Err(invalid("Attachment input must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;
    if bytes.len() > max {
        return Err(invalid("The input exceeds its size limit"));
    }
    Ok(bytes)
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new("ATTACHMENT_INVALID", message)
}
