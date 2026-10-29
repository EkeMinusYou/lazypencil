//! Keep managed assets live until their last document/preview owner releases them.

use super::*;
use tokio_util::sync::CancellationToken;

/// A per-user shared lease held through CLI output and captured project use.
/// It coordinates asset maintenance independently of a selected data directory.
/// Cloning the Arc preserves the same lease; only its last owner releases it.
#[derive(Debug)]
pub struct AssetActivity {
    _file: File,
}

impl AssetActivity {
    pub fn acquire() -> Result<Arc<Self>> {
        Self::in_directory(&default_lock_dir()?, false, Some(&crate::cancel::token()))
    }

    pub(super) fn in_directory(
        directory: &Path,
        maintenance: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<Arc<Self>> {
        crate::cancel::check(cancel)?;
        // This reserved relative identity cannot collide with a canonical
        // document path. Keep it independent of document/data namespaces.
        let resource = Path::new("lazypencil-managed-assets-activity-v1");
        let file = super::open_lock(resource, directory)?;
        super::wait_for_file_lock(
            &file,
            resource,
            !maintenance,
            cancel,
            if maintenance {
                Duration::ZERO
            } else {
                Duration::from_secs(5)
            },
        )?;
        Ok(Arc::new(Self { _file: file }))
    }
}

#[cfg(test)]
mod tests;
