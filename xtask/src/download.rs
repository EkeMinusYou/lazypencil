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
    retry(
        || {
            let mut response = agent
                .get(url)
                .header("User-Agent", "lazypencil-ci")
                .call()?;
            validate_redirect(&uri, response.get_uri())?;
            publish(response.body_mut().as_reader(), sha, path, maximum)
        },
        std::thread::sleep,
    )
}

fn transient(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(error) = error.downcast_ref::<ureq::Error>() {
        return match error {
            ureq::Error::Timeout(_)
            | ureq::Error::ConnectionFailed
            | ureq::Error::HostNotFound
            | ureq::Error::StatusCode(408 | 429 | 500 | 502 | 503 | 504) => true,
            ureq::Error::Io(error) => transient(error),
            _ => false,
        };
    }
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::BrokenPipe
        ) || error.get_ref().is_some_and(|inner| transient(inner));
    }
    false
}

fn retry<T>(mut attempt: impl FnMut() -> Result<T>, mut wait: impl FnMut(Duration)) -> Result<T> {
    for number in 1..=3 {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) if number < 3 && error.chain().any(transient) => {
                eprintln!("Transient upstream download failure (attempt {number}/3): {error:#}");
                wait(Duration::from_secs(number));
            }
            Err(error) => {
                return Err(
                    error.context(format!("Upstream download failed after {number} attempts"))
                );
            }
        }
    }
    unreachable!()
}

pub fn validate_redirect(original: &ureq::http::Uri, actual: &ureq::http::Uri) -> Result<()> {
    ensure!(
        original.scheme_str() == Some("https")
            && actual.scheme_str() == Some("https")
            && original.host().is_some()
            && actual.host() == original.host(),
        "Unexpected upstream redirect"
    );
    Ok(())
}

pub fn publish(mut reader: impl Read, sha: &str, path: &Path, maximum: u64) -> Result<u64> {
    ensure!(
        !path.exists() && !path.is_symlink(),
        "Download destination already exists"
    );
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0; 65536];
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
        util::hex(hash.finalize()) == sha,
        "Upstream checksum mismatch"
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    struct Timeout;
    impl Read for Timeout {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            // Response-body timeouts arrive wrapped in io::Error by ureq.
            Err(ureq::Error::Timeout(ureq::Timeout::PerRead).into_io())
        }
    }

    #[test]
    fn partial_body_timeout_restarts_from_clean_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("font");
        let bytes = b"pinned original font";
        let sha = util::digest(bytes);
        let mut attempts = 0;
        let mut waits = Vec::new();
        let size = retry(
            || {
                assert_eq!(fs::read_dir(directory.path())?.count(), 0);
                attempts += 1;
                if attempts == 1 {
                    publish(Cursor::new(b"partial").chain(Timeout), &sha, &path, 64)
                } else {
                    publish(Cursor::new(bytes), &sha, &path, 64)
                }
            },
            |delay| waits.push(delay),
        )
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(waits, [Duration::from_secs(1)]);
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn repeated_timeouts_stop_after_three_attempts_without_publishing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("font");
        let mut attempts = 0;
        let mut waits = Vec::new();
        let error = retry(
            || {
                attempts += 1;
                publish(
                    Cursor::new(b"partial").chain(Timeout),
                    &"0".repeat(64),
                    &path,
                    64,
                )
            },
            |delay| waits.push(delay),
        )
        .unwrap_err();
        assert_eq!(attempts, 3);
        assert_eq!(waits, [Duration::from_secs(1), Duration::from_secs(2)]);
        assert!(format!("{error:#}").contains("timeout: per read"));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn checksum_size_and_redirect_failures_are_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("font");
        for failure in ["checksum", "size", "redirect"] {
            let mut attempts = 0;
            let result = retry(
                || {
                    attempts += 1;
                    match failure {
                        "checksum" => publish(Cursor::new(b"bad"), &"0".repeat(64), &path, 64),
                        "size" => publish(Cursor::new(b"oversized"), &"0".repeat(64), &path, 1),
                        _ => validate_redirect(
                            &"https://example.com/font".parse()?,
                            &"https://other.example/font".parse()?,
                        )
                        .map(|()| 0),
                    }
                },
                |_| panic!("Invalid upstream bytes or redirects must fail immediately"),
            );
            assert!(result.is_err(), "{failure}");
            assert_eq!(attempts, 1);
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn transient_http_errors_retry_but_missing_sources_and_tls_fail_immediately() {
        for (status, expected) in [(408, 3), (429, 3), (503, 3), (404, 1)] {
            let mut attempts = 0;
            assert!(
                retry::<()>(
                    || {
                        attempts += 1;
                        Err(ureq::Error::StatusCode(status).into())
                    },
                    |_| {},
                )
                .is_err()
            );
            assert_eq!(attempts, expected, "{status}");
        }
        assert!(!transient(&ureq::Error::Tls("untrusted certificate")));
    }
}
