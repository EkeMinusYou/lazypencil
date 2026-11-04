//! Immutable camera sources can page out without losing native pixel data.
use skia_safe::Data;
use std::{ops::Deref, sync::Arc};

#[derive(Clone)]
pub(super) enum Pixels {
    Heap(Arc<Vec<u8>>),
    #[cfg(unix)]
    Mapped(Data),
}

impl Pixels {
    pub(super) fn heap(bytes: Vec<u8>) -> Self {
        Self::Heap(Arc::new(bytes))
    }

    pub(super) fn mapped(bytes: &[u8]) -> Option<Self> {
        #[cfg(unix)]
        if bytes.len() > 16 * 1024 * 1024 {
            use std::io::Write;
            let mut file = tempfile::NamedTempFile::new().ok()?;
            file.write_all(bytes).ok()?;
            // Skia's MakeFromFILE succeeds only for a complete file mapping.
            // The immutable data keeps that mapping alive after unlinking.
            return Data::from_filename(file.path()).map(Self::Mapped);
        }
        let _ = bytes;
        None
    }

    pub(super) fn data(&self) -> Data {
        match self {
            Self::Heap(bytes) => Data::new_copy(bytes),
            #[cfg(unix)]
            Self::Mapped(data) => data.clone(),
        }
    }

    pub(super) fn discard_pages(&self) {
        #[cfg(unix)]
        if let Self::Mapped(data) = self {
            // Only unchanged, whole-file mappings enter this variant.
            // Readers can safely fault in the same bytes after reclamation.
            unsafe {
                libc::madvise(
                    data.as_bytes().as_ptr().cast_mut().cast(),
                    data.size(),
                    libc::MADV_DONTNEED,
                );
            }
        }
    }
}

impl Deref for Pixels {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Heap(bytes) => bytes,
            #[cfg(unix)]
            Self::Mapped(data) => data.as_bytes(),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn large_sources_remain_exact_after_reclamation_cloning_and_cross_thread_reads() {
        let bytes = (0..17 * 1024 * 1024 + 23)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let pixels = Pixels::mapped(&bytes).unwrap();
        let retained = pixels.clone();
        drop(pixels);
        retained.discard_pages();
        std::thread::spawn(move || {
            assert_eq!(&*retained, bytes);
            retained.discard_pages();
            assert_eq!(&*retained, bytes);
        })
        .join()
        .unwrap();
    }
}
