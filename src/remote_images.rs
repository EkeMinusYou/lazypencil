//! Bounded HTTP image acquisition without cookies, persistent caches or URL logs.

use crate::{Error, Result, document::hash};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const PREFIX: &str = "lazypencil-http-image://";
pub(crate) const MAX_SECONDS: u64 = 30;
static CLI_WORKER: OnceLock<PathBuf> = OnceLock::new();

/// Called by the CLI entry point, which implements the private worker command.
/// Library hosts select their worker explicitly through Project construction.
#[doc(hidden)]
pub fn initialize_worker() {
    if let Ok(executable) = std::env::current_exe() {
        let _ = CLI_WORKER.set(executable);
    }
}

fn failure(message: &str) -> Error {
    Error::new("ASSET_NOT_FOUND", message)
}

pub(crate) fn is_http(uri: &str) -> bool {
    uri.get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http:"))
        || uri
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https:"))
}

pub(crate) fn validate(uri: &str) -> Result<url::Url> {
    if uri.len() > 4096 || uri.chars().any(char::is_control) || uri.contains('\\') {
        return Err(failure("Invalid or oversized HTTP image URL"));
    }
    let url = url::Url::parse(uri).map_err(|_| failure("Invalid HTTP image URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || uri.split_once("://").is_none_or(|(_, authority)| {
            authority
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("")
                .contains('@')
        })
        || url.as_str().len() > 4096
    {
        return Err(failure(
            "HTTP images require a bounded URL without embedded credentials",
        ));
    }
    Ok(url)
}

pub(crate) fn key(uri: &str) -> Result<PathBuf> {
    validate(uri)?;
    Ok(identity(uri))
}

pub(crate) fn identity(uri: &str) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{}", hash(uri.as_bytes())))
}

pub(crate) fn is_key(path: &Path) -> bool {
    path.to_str().is_some_and(|path| {
        path.strip_prefix(PREFIX).is_some_and(|digest| {
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    url: String,
    timeout_ms: u64,
}

pub(crate) fn fetch(
    uri: &str,
    executable: Option<&Path>,
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<Arc<[u8]>> {
    let cli_cancel = crate::cancel::token();
    let cancel = cancel.or_else(|| crate::cancel::cli_handler_installed().then_some(&cli_cancel));
    crate::cancel::check(cancel)?;
    let mut url = validate(uri)?;
    url.set_fragment(None);
    let executable = match executable {
        Some(executable) => executable.to_owned(),
        None => CLI_WORKER.get().cloned().ok_or_else(|| {
            failure("Embedded HTTP image capture requires an explicit lazypencil worker")
        })?,
    };
    let timeout = timeout.min(Duration::from_secs(MAX_SECONDS));
    if timeout < Duration::from_millis(1) {
        return Err(failure("HTTP image acquisition deadline exceeded"));
    }
    let request = serde_json::to_vec(&Request {
        url: url.into(),
        timeout_ms: timeout.as_millis() as u64,
    })
    .map_err(|_| failure("Cannot encode HTTP image acquisition"))?;
    let mut input = (request.len() as u32).to_le_bytes().to_vec();
    input.extend(request);
    input.extend(crate::runtime::download::MAGIC);
    let bytes = crate::runtime::download::collect(
        &executable,
        "__http_image_worker",
        &input,
        crate::image_input::MAX_SOURCE_BYTES,
        timeout,
        cancel,
    )
    .map_err(|error| {
        if error.code == "CANCELLED" {
            error
        } else {
            failure("HTTP image acquisition failed or exceeded its bounds")
        }
    })?;
    crate::cancel::check(cancel)?;
    crate::image_input::validate_encoded(&bytes, None)?;
    Ok(bytes.into())
}

/// Private worker entry point. URLs are read over the owner pipe and never
/// appear in argv or diagnostics. EOF terminates blocked DNS/TLS/HTTP work.
pub fn serve_worker() -> Result<()> {
    if std::env::args_os().len() != 2 {
        return Err(failure("HTTP image workers take no arguments"));
    }
    let mut input = std::io::stdin();
    let mut length = [0; 4];
    input
        .read_exact(&mut length)
        .map_err(|_| failure("Missing image request"))?;
    let length = u32::from_le_bytes(length) as usize;
    if !(1..=8192).contains(&length) {
        return Err(failure("Invalid image request length"));
    }
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .map_err(|_| failure("Incomplete image request"))?;
    let request: Request =
        serde_json::from_slice(&bytes).map_err(|_| failure("Invalid image request"))?;
    if !(1..=MAX_SECONDS * 1000).contains(&request.timeout_ms) {
        return Err(failure("Invalid image request deadline"));
    }
    crate::runtime::download::watch_owner()?;
    let mut url = validate(&request.url)?;
    let deadline = Instant::now() + Duration::from_millis(request.timeout_ms);
    for redirect in 0..=5 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| failure("Image request deadline exceeded"))?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(remaining))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = agent
            .get(url.as_str())
            .header("accept", "image/png,image/jpeg,image/webp")
            .call()
            .map_err(|_| failure("Image request failed"))?;
        match response.status().as_u16() {
            301 | 302 | 303 | 307 | 308 if redirect < 5 => {
                let target = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|location| url.join(location).ok())
                    .ok_or_else(|| failure("Invalid image redirect"))?;
                let target = validate(target.as_str())?;
                if url.scheme() == "https" && target.scheme() != "https" {
                    return Err(failure("Image redirects cannot downgrade HTTPS"));
                }
                url = target;
            }
            200 => {
                if response
                    .headers()
                    .get("content-length")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|length| length.parse::<u64>().ok())
                    .is_some_and(|length| length > crate::image_input::MAX_SOURCE_BYTES as u64)
                {
                    return Err(failure("Image response exceeds its byte limit"));
                }
                let mut bytes = Vec::new();
                response
                    .body_mut()
                    .as_reader()
                    .take(crate::image_input::MAX_SOURCE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| failure("Cannot read image response"))?;
                crate::image_input::validate_encoded(&bytes, None)?;
                if Instant::now() >= deadline {
                    return Err(failure("Image request deadline exceeded"));
                }
                return std::io::stdout()
                    .lock()
                    .write_all(&bytes)
                    .map_err(|_| failure("Cannot return image response"));
            }
            _ => return Err(failure("Image response was not successful")),
        }
    }
    Err(failure("Too many image redirects"))
}

#[cfg(test)]
mod tests;
