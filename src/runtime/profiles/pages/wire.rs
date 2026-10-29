//! Bounded typed control frames and binary image/resource bodies.
use super::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub(super) const MAX_HEADER: usize = 8 * 1024 * 1024;
pub(super) const MAX_RESOURCES: usize = 128 * 1024 * 1024;
pub(super) const MAX_PNG: usize = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hello {
    pub context: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileHeader {
    pub url: String,
    pub mime: Option<String>,
    pub bytes: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Page {
        timeout_ms: u64,
        page: Box<PageAcquisition>,
        files: Vec<FileHeader>,
    },
    Retain {
        timeout_ms: u64,
        signatures: BTreeMap<String, String>,
    },
}

impl Request {
    pub fn validate(&self) -> Result<usize> {
        let timeout = self.timeout_ms();
        if !(1..=300000).contains(&timeout) {
            return Err(failure("profile_protocol"));
        }
        match self {
            Self::Page { page, files, .. } => {
                page.settings.validate()?;
                page.dynamic.validate()?;
                if !page.settings.profile.is_empty()
                    || timeout > page.settings.timeout_ms
                    || timeout > page.dynamic.render_timeout_ms
                    || files.is_empty() != page.resource_identity.is_none()
                    || page
                        .retained
                        .as_ref()
                        .is_some_and(|(key, signature)| !key_hash(key) || !key_hash(signature))
                {
                    return Err(failure("profile_protocol"));
                }
            }
            Self::Retain { signatures, .. } => {
                if signatures.len() > crate::cdp::MAX_RETAINED_PAGES
                    || signatures
                        .iter()
                        .any(|(key, value)| !key_hash(key) || !key_hash(value))
                {
                    return Err(failure("profile_protocol"));
                }
            }
        }
        self.bytes()
    }
    pub fn timeout_ms(&self) -> u64 {
        match self {
            Self::Page { timeout_ms, .. } | Self::Retain { timeout_ms, .. } => *timeout_ms,
        }
    }
    pub fn bytes(&self) -> Result<usize> {
        let Self::Page { files, .. } = self else {
            return Ok(0);
        };
        if files.len() > 4096 {
            return Err(failure("profile_resource_limit"));
        }
        files.iter().try_fold(0usize, |total, file| {
            if file.bytes > MAX_PNG {
                return Err(failure("profile_resource_limit"));
            }
            total
                .checked_add(file.bytes)
                .filter(|total| *total <= MAX_RESOURCES)
                .ok_or_else(|| failure("profile_resource_limit"))
        })
    }
}

fn key_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Reply {
    Hello {
        context: String,
        owner: String,
    },
    Ready,
    Raster {
        metadata: Value,
        bytes: usize,
        pid: u32,
        retained: usize,
    },
    Done {
        pid: u32,
        retained: usize,
    },
    Error {
        code: String,
        message: String,
        details: Option<Value>,
    },
}

impl Reply {
    pub fn error(error: Error) -> Self {
        Self::Error {
            code: error.code.into(),
            message: error.message,
            details: error.details,
        }
    }
    pub fn checked(self) -> Result<Self> {
        match self {
            Self::Error {
                code,
                message,
                details,
            } => {
                let code = match code.as_str() {
                    "CANCELLED" => "CANCELLED",
                    "RENDER_TIMEOUT" => "RENDER_TIMEOUT",
                    "PAGE_LOAD_FAILED" => "PAGE_LOAD_FAILED",
                    "INVALID_ARGUMENT" => "INVALID_ARGUMENT",
                    "UNSUPPORTED_FEATURE" => "UNSUPPORTED_FEATURE",
                    "RUNTIME_UNAVAILABLE" => "RUNTIME_UNAVAILABLE",
                    "IO_ERROR" => "IO_ERROR",
                    _ => return Err(failure("profile_protocol")),
                };
                Err(Error {
                    code,
                    message,
                    details,
                })
            }
            value => Ok(value),
        }
    }
}

#[cfg(test)]
mod tests;

pub(super) fn write_json(stream: &mut LocalStream, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| failure("profile_protocol"))?;
    if bytes.is_empty() || bytes.len() > MAX_HEADER {
        return Err(failure("profile_protocol_limit"));
    }
    stream
        .write_all(&(bytes.len() as u64).to_be_bytes())
        .and_then(|()| stream.write_all(&bytes))
        .map_err(|_| failure("profile_channel_write"))
}

pub(super) fn read_json<T: DeserializeOwned>(
    stream: &mut LocalStream,
    first: Option<u8>,
) -> Result<T> {
    let mut length = [0; 8];
    let start = if let Some(first) = first {
        length[0] = first;
        1
    } else {
        0
    };
    stream
        .read_exact(&mut length[start..])
        .map_err(|_| failure("profile_channel_read"))?;
    let size = usize::try_from(u64::from_be_bytes(length))
        .map_err(|_| failure("profile_protocol_limit"))?;
    if size == 0 || size > MAX_HEADER {
        return Err(failure("profile_protocol_limit"));
    }
    let mut bytes = vec![0; size];
    stream
        .read_exact(&mut bytes)
        .map_err(|_| failure("profile_channel_read"))?;
    serde_json::from_slice(&bytes).map_err(|_| failure("profile_protocol"))
}
