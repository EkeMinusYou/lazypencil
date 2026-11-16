//! Pinned icon downloads share the bounded, cancellable worker lifecycle.
use super::*;
use std::io::Write;
use tokio_util::sync::CancellationToken;

const TIMEOUT: Duration = Duration::from_secs(60);

fn unavailable(message: impl Into<String>) -> Error {
    Error::new("ICON_UNAVAILABLE", message)
}

pub(super) fn direct(descriptor: &Descriptor) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(&descriptor.source_url)
        .call()
        .map_err(|error| unavailable(format!("Cannot retrieve pinned icon archive: {error}")))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_ARCHIVE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new("IO_ERROR", error.to_string()))?;
    if bytes.len() > MAX_ARCHIVE {
        return Err(integrity("Icon archive exceeds 16 MiB"));
    }
    Ok(bytes)
}

pub(super) fn run(
    descriptor: &Descriptor,
    executable: &Path,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    if !executable.is_absolute() {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Icon download workers require an absolute executable path",
        ));
    }
    let mut request = (descriptor.id.len() as u32).to_le_bytes().to_vec();
    request.extend(descriptor.id.as_bytes());
    request.extend(crate::runtime::download::MAGIC);
    crate::runtime::download::collect(
        executable,
        "__icon_download_worker",
        &request,
        MAX_ARCHIVE,
        TIMEOUT,
        cancel,
    )
    .map_err(|error| {
        if error.code == "CANCELLED" {
            error
        } else {
            unavailable("Cannot retrieve pinned icon archive within its transfer bounds")
        }
    })
}

/// The private worker receives only a catalog ID, never an arbitrary source URL.
pub fn serve_worker() -> Result<()> {
    if std::env::args_os().len() != 2 {
        return Err(unavailable("Icon download workers take no arguments"));
    }
    let mut input = std::io::stdin();
    let descriptor = requested_pack(&mut input)?;
    crate::runtime::download::watch_owner()?;
    let bytes = direct(descriptor)?;
    std::io::stdout()
        .lock()
        .write_all(&bytes)
        .map_err(|error| unavailable(format!("Cannot write icon download: {error}")))
}

fn requested_pack(input: &mut impl Read) -> Result<&'static Descriptor> {
    let mut length = [0; 4];
    input
        .read_exact(&mut length)
        .map_err(|_| unavailable("Missing icon request"))?;
    let length = u32::from_le_bytes(length) as usize;
    if !(1..=128).contains(&length) {
        return Err(unavailable("Invalid icon request length"));
    }
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .map_err(|_| unavailable("Incomplete icon request"))?;
    let id = std::str::from_utf8(&bytes).map_err(|_| unavailable("Invalid icon pack ID"))?;
    let descriptor = descriptor(id)?;
    if descriptor.bundled {
        return Err(unavailable("Bundled icon packs need no download"));
    }
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &[u8]) -> Vec<u8> {
        let mut bytes = (id.len() as u32).to_le_bytes().to_vec();
        bytes.extend(id);
        bytes
    }

    #[test]
    fn requests_select_only_pinned_optional_packs_and_leave_the_owner_marker_unread() {
        let mut bytes = frame(b"feather");
        bytes.extend(crate::runtime::download::MAGIC);
        let mut input = bytes.as_slice();
        let selected = requested_pack(&mut input).unwrap();
        assert_eq!(selected.id, "feather");
        assert_eq!(
            selected.source_url,
            descriptor("feather").unwrap().source_url
        );
        assert_eq!(input, crate::runtime::download::MAGIC);
        for bytes in [
            Vec::new(),
            vec![0, 0, 0],
            frame(b""),
            frame(&[b'x'; 129]),
            [10u32.to_le_bytes().to_vec(), b"short".to_vec()].concat(),
            frame(&[0xff]),
            frame(b"https://example.test/unpinned"),
            frame(b"lucide"),
        ] {
            assert!(requested_pack(&mut bytes.as_slice()).is_err());
        }
    }
}
