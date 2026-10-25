use crate::util;
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};
use ureq::ResponseExt;

pub fn download(url: &str, sha: &str, path: &Path, maximum: u64) -> Result<u64> {
    ensure!(
        !path.exists() && !path.is_symlink(),
        "Download destination already exists"
    );
    ensure!(
        sha.len() == 64
            && sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Invalid upstream checksum"
    );
    let uri: ureq::http::Uri = url.parse()?;
    ensure!(
        uri.scheme_str() == Some("https") && uri.host().is_some(),
        "Expected immutable HTTPS source"
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(300)))
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", "lazypencil-ci")
        .call()?;
    ensure!(
        response.get_uri().scheme_str() == Some("https") && response.get_uri().host() == uri.host(),
        "Unexpected upstream redirect"
    );
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 65536];
    let mut reader = response.body_mut().as_reader();
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        ensure!(size <= maximum, "Upstream download exceeds byte limit");
        hash.update(&buffer[..count]);
        temporary.write_all(&buffer[..count])?;
    }
    ensure!(
        format!("{:x}", hash.finalize()) == sha,
        "Upstream checksum mismatch: {url}"
    );
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path).map_err(|e| e.error)?;
    Ok(size)
}

pub fn verify_file(path: &Path, sha: &str, maximum: u64) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.is_symlink() && metadata.len() <= maximum,
        "Expected bounded cached input"
    );
    ensure!(
        util::file_digest(path)? == sha,
        "Cached input checksum mismatch"
    );
    Ok(metadata.len())
}
