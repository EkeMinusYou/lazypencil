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
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_REQUEST_IMAGES: usize = 8;
pub const MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;
const MAX_GRANT_BYTES: usize = 512 * 1024;

/// The encoded input is retained privately; only the normalized PNG is sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalImage {
    pub hash: String,
    pub mime_type: String,
    pub bytes: usize,
    /// Dimensions after applying the source's orientation metadata.
    pub width: u32,
    pub height: u32,
}

impl OriginalImage {
    fn path(&self, directory: &Path) -> Result<PathBuf> {
        if self.bytes == 0
            || self.bytes > crate::image_input::MAX_SOURCE_BYTES
            || self.width == 0
            || self.height == 0
            || u64::from(self.width) * u64::from(self.height) > crate::image_input::MAX_IMAGE_PIXELS
        {
            return Err(invalid("Invalid original image metadata"));
        }
        let extension = match self.mime_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => return Err(invalid("Invalid original image format")),
        };
        image_path(directory, &self.hash, extension)
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<OriginalImage>,
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
        if display_name.len() > 1024 {
            return Err(invalid(
                "Attachment display names must not exceed 1024 bytes",
            ));
        }
        let image = crate::image_input::normalize(source, None)?;
        let hash = hash(&image.png);
        create_private_directory(&directory.join("attachments"))?;
        let original = OriginalImage {
            hash: crate::document::hash(source),
            mime_type: image.source_mime.into(),
            bytes: source.len(),
            width: image.width,
            height: image.height,
        };
        let attachment = Self {
            id: identity(&hash, Some(&original)),
            display_name: display_name.into(),
            hash,
            width: image.width,
            height: image.height,
            bytes: image.png.len(),
            source_bytes: Some(source.len()),
            original: Some(original),
        };
        // PNG input and its normalized output share one blob when identical.
        // Distinct encodings with the same pixels retain separate originals.
        persist_image(
            &attachment
                .original
                .as_ref()
                .expect("original")
                .path(directory)?,
            source,
        )?;
        persist_image(&attachment.path(directory)?, &image.png)?;
        Ok(attachment)
    }

    fn path(&self, directory: &Path) -> Result<PathBuf> {
        if self.hash.len() != 64
            || !self.hash.bytes().all(|b| b.is_ascii_hexdigit())
            || self.id != identity(&self.hash, self.original.as_ref())
            || self.bytes == 0
            || self.bytes > crate::image_input::MAX_SOURCE_BYTES
            || self
                .source_bytes
                .is_some_and(|bytes| bytes > crate::image_input::MAX_SOURCE_BYTES)
            || self.width == 0
            || self.height == 0
            || u64::from(self.width) * u64::from(self.height) > crate::image_input::MAX_IMAGE_PIXELS
        {
            return Err(invalid("Invalid attachment identity"));
        }
        if let Some(original) = &self.original {
            original.path(directory)?;
        }
        image_path(directory, &self.hash, "png")
    }

    pub(crate) fn reclaim_files<'a>(
        &self,
        directory: &Path,
        retained: impl Iterator<Item = &'a Self>,
    ) -> Result<()> {
        let mut live = BTreeSet::new();
        for attachment in retained {
            live.insert(attachment.path(directory)?);
            if let Some(original) = &attachment.original {
                live.insert(original.path(directory)?);
            }
        }
        let mut retired = BTreeSet::from([self.path(directory)?]);
        if let Some(original) = &self.original {
            retired.insert(original.path(directory)?);
        }
        for path in retired.difference(&live) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::io(path, error)),
            }
        }
        Ok(())
    }

    pub fn read_original(&self, directory: &Path) -> Result<Vec<u8>> {
        let original = self.original.as_ref().ok_or_else(|| {
            Error::new(
                "ATTACHMENT_ORIGINAL_UNAVAILABLE",
                "This older attachment has no captured original",
            )
        })?;
        self.path(directory)?;
        let bytes = read_bounded(
            &original.path(directory)?,
            crate::image_input::MAX_SOURCE_BYTES,
        )?;
        if bytes.len() != original.bytes || hash(&bytes) != original.hash {
            return Err(invalid("A stored original image changed"));
        }
        Ok(bytes)
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

fn identity(normalized: &str, original: Option<&OriginalImage>) -> String {
    match original {
        Some(original) => format!(
            "img_{}",
            hash(format!("v2:{normalized}:{}", original.hash).as_bytes())
        ),
        None => format!("img_{normalized}"),
    }
}

fn image_path(directory: &Path, digest: &str, extension: &str) -> Result<PathBuf> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("Invalid stored image hash"));
    }
    Ok(directory
        .join("attachments")
        .join(format!("{digest}.{extension}")))
}

fn persist_image(path: &Path, bytes: &[u8]) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || read_bounded(path, crate::image_input::MAX_SOURCE_BYTES)? != bytes
            {
                return Err(invalid(
                    "An existing stored image changed; it was not overwritten",
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => atomic_replace(path, bytes),
        Err(error) => Err(Error::io(path, error)),
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
        let bytes = read_bounded(path, MAX_GRANT_BYTES)?;
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
    let bytes = read_bounded(path, MAX_GRANT_BYTES)?;
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
