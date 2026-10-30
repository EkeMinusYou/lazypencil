//! Bounded RFC 2397 image payloads, without filesystem or network resolution.

use crate::{Error, Result, image_input::MAX_SOURCE_BYTES};
use base64::Engine;
use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

const PREFIX: &str = "lazypencil-data-image://";
const MAX_HEADER_BYTES: usize = 4096;
const MAX_URI_BYTES: usize = MAX_SOURCE_BYTES * 3 + MAX_HEADER_BYTES + 6;

pub(crate) fn is_data(uri: &str) -> bool {
    uri.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

fn invalid() -> Error {
    Error::new(
        "ASSET_NOT_FOUND",
        "Invalid, unsupported or oversized embedded image",
    )
}

pub(crate) fn identity(uri: &str) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{}", crate::document::hash(uri.as_bytes())))
}

pub(crate) fn is_key(path: &std::path::Path) -> bool {
    path.to_str().is_some_and(|value| {
        value.strip_prefix(PREFIX).is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    })
}

struct Parts<'a> {
    mime: &'static str,
    base64: bool,
    body: &'a str,
}

fn parts(uri: &str) -> Result<Parts<'_>> {
    if !is_data(uri) || uri.len() > MAX_URI_BYTES {
        return Err(invalid());
    }
    let value = &uri[5..];
    let value = value.split_once('#').map_or(value, |(body, _)| body);
    let (header, body) = value.split_once(',').ok_or_else(invalid)?;
    if header.len() > MAX_HEADER_BYTES
        || !header.is_ascii()
        || header.bytes().any(|b| b.is_ascii_control())
    {
        return Err(invalid());
    }
    let (header, base64) = match header.rsplit_once(';') {
        Some((header, marker)) if marker.eq_ignore_ascii_case("base64") => (header, true),
        _ => (header, false),
    };
    let mut fields = header.split(';');
    let mime = match fields
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/webp" => "image/webp",
        _ => return Err(invalid()),
    };
    for field in fields {
        let Some((name, value)) = field.split_once('=') else {
            return Err(invalid());
        };
        if name.is_empty() || value.is_empty() || field.bytes().any(|b| b.is_ascii_whitespace()) {
            return Err(invalid());
        }
    }
    Ok(Parts { mime, base64, body })
}

pub(crate) fn validate(uri: &str) -> Result<()> {
    parts(uri).map(|_| ())
}

pub(crate) fn decode(uri: &str, cancel: Option<&CancellationToken>) -> Result<Arc<[u8]>> {
    crate::cancel::check(cancel)?;
    let parts = parts(uri)?;
    let limit = if parts.base64 {
        MAX_SOURCE_BYTES.div_ceil(3) * 4
    } else {
        MAX_SOURCE_BYTES
    };
    let encoded = percent_bytes(parts.body, limit, parts.base64, cancel)?;
    let bytes = if parts.base64 {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| invalid())?
    } else {
        encoded
    };
    crate::cancel::check(cancel)?;
    crate::image_input::validate_encoded(&bytes, Some(parts.mime)).map_err(|_| invalid())?;
    Ok(bytes.into())
}

fn percent_bytes(
    body: &str,
    limit: usize,
    base64: bool,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    let mut result = Vec::with_capacity(body.len().min(limit).min(64 * 1024));
    let bytes = body.as_bytes();
    let mut offset = 0;
    let mut next_check = 0;
    while offset < bytes.len() {
        if offset >= next_check {
            crate::cancel::check(cancel)?;
            next_check = offset + 4096;
        }
        let byte = match bytes[offset] {
            b'%' => {
                let digits = bytes.get(offset + 1..offset + 3).ok_or_else(invalid)?;
                let hex = |byte: u8| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    b'A'..=b'F' => Some(byte - b'A' + 10),
                    _ => None,
                };
                let byte = (hex(digits[0]).ok_or_else(invalid)? << 4)
                    | hex(digits[1]).ok_or_else(invalid)?;
                offset += 3;
                byte
            }
            byte if byte.is_ascii()
                && (!byte.is_ascii_control() || base64 && byte.is_ascii_whitespace()) =>
            {
                offset += 1;
                byte
            }
            _ => return Err(invalid()),
        };
        if base64 && byte.is_ascii_whitespace() {
            continue;
        }
        if result.len() >= limit {
            return Err(invalid());
        }
        result.push(byte);
    }
    crate::cancel::check(cancel)?;
    Ok(result)
}

#[cfg(test)]
mod tests;
