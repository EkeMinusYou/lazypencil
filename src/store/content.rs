//! Bounded streaming revisions without allocating another copy of an asset.

use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
use tokio_util::sync::CancellationToken;

pub(crate) fn file_hash(
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<String> {
    hash_reader(super::open_regular_file(path)?, path, limit, cancel)
}

pub(crate) fn hash_reader(
    reader: impl Read,
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
) -> Result<String> {
    let mut digest = Sha256::new();
    read_chunks(
        reader,
        path,
        limit,
        cancel,
        Error::new("INVALID_DOCUMENT", "An asset exceeds its size limit"),
        |bytes| digest.update(bytes),
    )?;
    Ok(crate::document::digest_hex(digest.finalize()))
}

pub(crate) fn read_bounded(
    reader: impl Read,
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
    limit_error: Error,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    read_chunks(reader, path, limit, cancel, limit_error, |bytes| {
        output.extend_from_slice(bytes);
    })?;
    Ok(output)
}

fn read_chunks(
    reader: impl Read,
    path: &Path,
    limit: usize,
    cancel: Option<&CancellationToken>,
    limit_error: Error,
    mut consume: impl FnMut(&[u8]),
) -> Result<()> {
    let mut reader = reader.take((limit as u64).saturating_add(1));
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0usize;
    loop {
        crate::cancel::check(cancel)?;
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::io(path, error)),
        };
        if count == 0 {
            return crate::cancel::check(cancel);
        }
        total = total
            .checked_add(count)
            .filter(|total| *total <= limit)
            .ok_or_else(|| limit_error.clone())?;
        consume(&buffer[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    #[test]
    fn revisions_cover_empty_exact_limit_partial_chunks_and_the_final_byte() {
        for length in [0, 1, 65535, 65536, 65537, 131079] {
            let mut bytes = (0..length).map(|n| (n % 251) as u8).collect::<Vec<_>>();
            let revision =
                hash_reader(Cursor::new(&bytes), Path::new("input"), length, None).unwrap();
            assert_eq!(revision, crate::document::hash(&bytes));
            if let Some(last) = bytes.last_mut() {
                *last ^= 1;
                assert_ne!(
                    hash_reader(Cursor::new(&bytes), Path::new("input"), length, None).unwrap(),
                    revision
                );
            }
            bytes.push(0);
            assert_eq!(
                hash_reader(Cursor::new(bytes), Path::new("input"), length, None)
                    .unwrap_err()
                    .code,
                "INVALID_DOCUMENT"
            );
        }
    }

    #[test]
    fn bounded_reads_share_hash_limits_and_preserve_the_callers_error() {
        for length in [0, 1, 65535, 65536, 65537] {
            let bytes = vec![7; length];
            assert_eq!(
                read_bounded(
                    Cursor::new(&bytes),
                    Path::new("input"),
                    length,
                    None,
                    Error::new("ATTACHMENT_INVALID", "custom limit")
                )
                .unwrap(),
                bytes
            );
            let error = read_bounded(
                Cursor::new(&bytes),
                Path::new("input"),
                length.saturating_sub(1),
                None,
                Error::new("ATTACHMENT_INVALID", "custom limit"),
            );
            if length != 0 {
                assert_eq!(error.unwrap_err().message, "custom limit");
            }
        }
    }

    struct Interruptible {
        cancel: CancellationToken,
        reads: usize,
    }
    impl Read for Interruptible {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if self.reads == 1 {
                return Err(io::ErrorKind::Interrupted.into());
            }
            assert_eq!(self.reads, 2, "Cancellation must stop the next read");
            buffer.fill(7);
            self.cancel.cancel();
            Ok(buffer.len())
        }
    }

    #[test]
    fn interrupted_reads_retry_and_cancellation_stops_between_chunks() {
        let cancel = CancellationToken::new();
        let reader = Interruptible {
            cancel: cancel.clone(),
            reads: 0,
        };
        assert_eq!(
            hash_reader(reader, Path::new("input"), usize::MAX, Some(&cancel))
                .unwrap_err()
                .code,
            "CANCELLED"
        );
    }

    #[test]
    fn a_read_failure_never_becomes_a_partial_revision() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        assert_eq!(
            hash_reader(Failing, Path::new("input"), 1024, None)
                .unwrap_err()
                .code,
            "IO_ERROR"
        );
    }
}
