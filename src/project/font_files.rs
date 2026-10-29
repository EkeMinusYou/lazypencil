//! Bounded font snapshots retain a revision, not every configured file's bytes.

use crate::{Error, Result, document::hash};
use std::{
    fs::File,
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

pub(crate) const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;

#[cfg(feature = "rendering")]
use std::io::{Cursor, SeekFrom};

#[cfg(feature = "rendering")]
pub(crate) trait FontReader: Read + Seek {}
#[cfg(feature = "rendering")]
impl<T: Read + Seek> FontReader for T {}

#[derive(Clone)]
pub(crate) struct FontFile {
    pub path: PathBuf,
    pub revision: String,
    pub length: usize,
    bytes: Option<Arc<[u8]>>,
}

impl FontFile {
    pub fn capture(path: &Path, cancel: Option<&CancellationToken>) -> Result<Self> {
        let mut file = crate::store::open_regular_file(path)?;
        let length = file
            .metadata()
            .map_err(|error| Error::io(path, error))?
            .len();
        if length > MAX_FONT_BYTES as u64 {
            return Err(Error::new("INVALID_ARGUMENT", "A font file exceeds 64 MiB"));
        }
        let revision = crate::store::content::hash_reader(&mut file, path, MAX_FONT_BYTES, cancel)?;
        if file
            .stream_position()
            .map_err(|error| Error::io(path, error))?
            != length
        {
            return Err(changed(path));
        }
        Ok(Self {
            path: path.into(),
            revision,
            length: length as usize,
            bytes: None,
        })
    }

    pub fn from_bytes(path: PathBuf, bytes: Arc<[u8]>) -> Self {
        Self {
            path,
            revision: hash(&bytes),
            length: bytes.len(),
            bytes: Some(bytes),
        }
    }

    fn open(&self) -> Result<File> {
        let file = crate::store::open_regular_file(&self.path)?;
        if file
            .metadata()
            .map_err(|error| Error::io(&self.path, error))?
            .len()
            != self.length as u64
        {
            return Err(changed(&self.path));
        }
        Ok(file)
    }

    #[cfg(feature = "rendering")]
    pub fn reader(&self) -> Result<Box<dyn FontReader>> {
        match &self.bytes {
            Some(bytes) => Ok(Box::new(Cursor::new(bytes.clone()))),
            None => Ok(Box::new(self.open()?)),
        }
    }

    pub fn verify(&self, cancel: Option<&CancellationToken>) -> Result<()> {
        crate::cancel::check(cancel)?;
        if self.bytes.is_none()
            && crate::store::content::hash_reader(self.open()?, &self.path, MAX_FONT_BYTES, cancel)?
                != self.revision
        {
            return Err(changed(&self.path));
        }
        Ok(())
    }

    pub fn read(&self, cancel: Option<&CancellationToken>) -> Result<Arc<[u8]>> {
        crate::cancel::check(cancel)?;
        if let Some(bytes) = &self.bytes {
            return Ok(bytes.clone());
        }
        let mut reader = self.open()?;
        let mut bytes = Vec::with_capacity(self.length);
        let mut buffer = [0; 64 * 1024];
        loop {
            crate::cancel::check(cancel)?;
            let count = match reader.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(Error::io(&self.path, error)),
            };
            if count == 0 {
                break;
            }
            if bytes.len() + count > MAX_FONT_BYTES {
                return Err(changed(&self.path));
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        if bytes.len() != self.length || hash(&bytes) != self.revision {
            return Err(changed(&self.path));
        }
        Ok(bytes.into())
    }

    #[cfg(feature = "rendering")]
    pub fn read_range(
        &self,
        reader: &mut dyn FontReader,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>> {
        if offset
            .checked_add(length)
            .is_none_or(|end| end > self.length)
        {
            return Err(Error::new(
                "INVALID_ARGUMENT",
                "Font table lies outside the file",
            ));
        }
        let mut bytes = vec![0; length];
        reader
            .seek(SeekFrom::Start(offset as u64))
            .and_then(|_| reader.read_exact(&mut bytes))
            .map_err(|error| Error::io(&self.path, error))?;
        Ok(bytes)
    }
}

fn changed(path: &Path) -> Error {
    Error::new(
        "REVISION_CONFLICT",
        format!("Font changed during the operation: {}", path.display()),
    )
}
