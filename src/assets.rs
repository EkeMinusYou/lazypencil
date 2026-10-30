use crate::{Error, Result, document::hash, project::Project, store::Store};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read, path::Path};
use tokio_util::sync::CancellationToken;

const MAX_CODE_BYTES: usize = 1024 * 1024;

pub mod prune;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Image,
    Script,
    Shader,
}

impl std::str::FromStr for AssetKind {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "image" => Ok(Self::Image),
            "script" => Ok(Self::Script),
            "shader" => Ok(Self::Shader),
            _ => Err(Error::new(
                "INVALID_ARGUMENT",
                "Asset kind must be image, script, or shader",
            )),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum AssetSource {
    Text { text: String },
    Data { mime_type: String, data: String },
    Attachment { attachment_id: String },
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PutAssetRequest {
    pub kind: AssetKind,
    pub source: AssetSource,
}

#[derive(Debug, Serialize)]
pub struct ManagedAsset {
    pub kind: AssetKind,
    pub uri: String,
    pub hash: String,
    pub mime_type: String,
    pub bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub reused: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadAssetRequest {
    pub hash: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "chunk_size")]
    pub limit: usize,
}
fn chunk_size() -> usize {
    65536
}

pub fn put_asset(store: &Store, request: &PutAssetRequest) -> Result<ManagedAsset> {
    put_asset_with_cancellation(store, request, None)
}

pub fn put_asset_with_cancellation(
    store: &Store,
    request: &PutAssetRequest,
    cancel: Option<&CancellationToken>,
) -> Result<ManagedAsset> {
    crate::cancel::check(cancel)?;
    match &request.source {
        AssetSource::Text { text } => {
            if matches!(request.kind, AssetKind::Image) {
                return Err(Error::new(
                    "ATTACHMENT_INVALID",
                    "Image input requires MIME-typed Base64 data or a registered attachment",
                ));
            }
            put_bytes_with_cancellation(store, request.kind, text.as_bytes(), None, cancel)
        }
        AssetSource::Data { mime_type, data } => {
            let max = if matches!(request.kind, AssetKind::Image) {
                crate::image_input::MAX_SOURCE_BYTES
            } else {
                MAX_CODE_BYTES
            };
            if data.len() > max.saturating_add(2) / 3 * 4 {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Encoded asset exceeds its size limit",
                ));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Invalid Base64 asset data"))?;
            put_bytes_with_cancellation(store, request.kind, &bytes, Some(mime_type), cancel)
        }
        AssetSource::Attachment { .. } => Err(Error::new(
            "ATTACHMENT_INVALID",
            "This attachment ID was not registered for the bound session",
        )),
    }
}

pub fn put_file(
    store: &Store,
    kind: AssetKind,
    path: &Path,
    mime: Option<&str>,
) -> Result<ManagedAsset> {
    put_file_with_cancellation(store, kind, path, mime, None)
}

pub fn put_file_with_cancellation(
    store: &Store,
    kind: AssetKind,
    path: &Path,
    mime: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<ManagedAsset> {
    crate::cancel::check(cancel)?;
    let max = if matches!(kind, AssetKind::Image) {
        crate::image_input::MAX_SOURCE_BYTES
    } else {
        MAX_CODE_BYTES
    };
    let mut bytes = Vec::new();
    crate::store::open_regular_file(path)
        .map_err(|error| {
            if error.code == "INVALID_DOCUMENT" {
                Error::new("INVALID_ARGUMENT", "Asset input must be a regular file")
            } else {
                error
            }
        })?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io(path, e))?;
    if bytes.len() > max {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Asset input exceeds its size limit",
        ));
    }
    put_bytes_with_cancellation(store, kind, &bytes, mime, cancel)
}

pub fn put_bytes(
    store: &Store,
    kind: AssetKind,
    bytes: &[u8],
    mime: Option<&str>,
) -> Result<ManagedAsset> {
    put_bytes_with_cancellation(store, kind, bytes, mime, None)
}

pub fn put_bytes_with_cancellation(
    store: &Store,
    kind: AssetKind,
    bytes: &[u8],
    mime: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<ManagedAsset> {
    crate::cancel::check(cancel)?;
    store.read()?.ensure_writable()?;
    let (bytes, extension, mime_type, width, height) = match kind {
        AssetKind::Image => {
            let image = crate::image_input::normalize(bytes, mime)?;
            (
                image.png,
                "png",
                "image/png",
                Some(image.width),
                Some(image.height),
            )
        }
        AssetKind::Script | AssetKind::Shader => {
            if bytes.is_empty() || bytes.len() > MAX_CODE_BYTES {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Code assets must contain 1..1048576 bytes",
                ));
            }
            std::str::from_utf8(bytes)
                .map_err(|_| Error::new("INVALID_ARGUMENT", "Code assets must be UTF-8"))?;
            let (extension, expected) = if matches!(kind, AssetKind::Script) {
                ("js", "text/javascript")
            } else {
                ("glsl", "text/plain")
            };
            if mime.is_some_and(|m| m != expected && m != "text/plain") {
                return Err(Error::new(
                    "INVALID_ARGUMENT",
                    "Unexpected MIME type for a code asset",
                ));
            }
            (bytes.to_vec(), extension, expected, None, None)
        }
    };
    crate::cancel::check(cancel)?;
    let digest = hash(&bytes);
    let file = crate::managed::File::new(store.path(), bytes.into(), extension)?;
    let reused = file.publish(store, cancel)?;
    Ok(ManagedAsset {
        kind,
        uri: file.uri,
        hash: digest,
        mime_type: mime_type.into(),
        bytes: file.bytes.len(),
        width,
        height,
        reused,
    })
}

pub fn read_asset(project: &Project, request: &ReadAssetRequest) -> Result<Value> {
    if request.limit == 0 || request.limit > 1024 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Asset chunk limit must be 1..1048576 bytes",
        ));
    }
    let dependency = project
        .dependencies()
        .into_iter()
        .chain(project.runtime_dependencies())
        .find(|d| d.revision == request.hash && matches!(d.kind.as_str(), "script" | "shader"))
        .ok_or_else(|| {
            Error::new(
                "ASSET_NOT_FOUND",
                "No captured script or shader has this hash",
            )
        })?;
    let asset = project.asset(&dependency.path)?;
    read_code(
        asset,
        &dependency.kind,
        request,
        &project.root_document().revision(),
        &project.dependency_revision(),
    )
}

pub(crate) fn read_code(
    asset: &crate::project::Asset,
    kind: &str,
    request: &ReadAssetRequest,
    revision: &str,
    dependency_revision: &str,
) -> Result<Value> {
    if request.limit == 0 || request.limit > 1024 * 1024 {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Asset chunk limit must be 1..1048576 bytes",
        ));
    }
    let text = std::str::from_utf8(&asset.bytes)
        .map_err(|_| Error::new("INVALID_DOCUMENT", "Code asset is not UTF-8"))?;
    if request.offset > text.len() || !text.is_char_boundary(request.offset) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Asset offset must be a UTF-8 byte boundary",
        ));
    }
    let mut end = request.offset.saturating_add(request.limit).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == request.offset && end < text.len() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Chunk limit is too small for the next UTF-8 character",
        ));
    }
    let mut result = json!({"revision":revision,"dependency_revision":dependency_revision,
        "hash":request.hash,"kind":asset.kind,"content":&text[request.offset..end],"offset":request.offset,
        "total_bytes":text.len(),"next_offset":(end<text.len()).then_some(end)});
    if kind == "script" {
        match crate::script::header::Header::parse(text) {
            Ok(header) => {
                result["script_header"] = serde_json::to_value(header).expect("script header")
            }
            Err(error) => result["script_header_error"] = json!(error),
        }
    }
    Ok(result)
}
