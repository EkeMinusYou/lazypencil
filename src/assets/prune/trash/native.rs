use super::*;
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
};

struct Directory {
    path: PathBuf,
    file: File,
}

fn name(value: &std::ffi::OsStr) -> Result<CString> {
    if value.is_empty()
        || value.as_bytes().contains(&b'/')
        || matches!(value.to_str(), Some("." | ".."))
    {
        return Err(changed());
    }
    CString::new(value.as_bytes()).map_err(|_| changed())
}
fn owned(fd: libc::c_int, path: &Path) -> Result<File> {
    if fd < 0 {
        Err(Error::io(path, std::io::Error::last_os_error()))
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

impl Directory {
    fn open(path: &Path, budget: &Budget<'_>) -> Result<Self> {
        budget.check()?;
        Ok(Self {
            path: path.into(),
            file: filesystem::open_directory(path, budget.cancel)?,
        })
    }
    fn child(&self, child: &str, budget: &Budget<'_>) -> Result<Self> {
        budget.check()?;
        let key = name(child.as_ref())?;
        let path = self.path.join(child);
        let file = owned(
            unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    key.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            },
            &path,
        )?;
        Ok(Self { path, file })
    }
    fn verify(&self, budget: &Budget<'_>) -> Result<()> {
        let current = Self::open(&self.path, budget)?;
        let current = current
            .file
            .metadata()
            .map_err(|error| Error::io(&self.path, error))?;
        let held = self
            .file
            .metadata()
            .map_err(|error| Error::io(&self.path, error))?;
        if (current.dev(), current.ino()) != (held.dev(), held.ino()) {
            return Err(changed());
        }
        Ok(())
    }
    fn sync(&self) -> Result<()> {
        self.file
            .sync_all()
            .map_err(|error| Error::io(&self.path, error))
    }
    fn content(&self, child: &str, budget: &mut Budget<'_>) -> Result<Option<Content>> {
        budget.check()?;
        let key = name(child.as_ref())?;
        let path = self.path.join(child);
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                key.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let mut file = owned(fd, &path)?;
        let metadata = file.metadata().map_err(|error| Error::io(&path, error))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE as u64 {
            return Err(changed());
        }
        let limit = MAX_FILE.min(MAX_BYTES.saturating_sub(budget.bytes));
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            budget.check()?;
            let count = match file.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result.map_err(|error| Error::io(&path, error))?,
            };
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > limit {
                return Err(Error::new(
                    "RESOURCE_LIMIT",
                    "Asset cleanup exceeds its byte budget",
                ));
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        budget.bytes += bytes.len();
        Ok(Some(Content::of(&bytes)))
    }
    fn unlink(&self, child: &str, directory: bool) -> Result<()> {
        let key = name(child.as_ref())?;
        if unsafe {
            libc::unlinkat(
                self.file.as_raw_fd(),
                key.as_ptr(),
                if directory { libc::AT_REMOVEDIR } else { 0 },
            )
        } != 0
        {
            return Err(Error::io(
                &self.path.join(child),
                std::io::Error::last_os_error(),
            ));
        }
        self.sync()
    }
}

fn rename(
    source: &Directory,
    child: &std::ffi::OsStr,
    destination: &Directory,
    target: &std::ffi::OsStr,
    budget: &Budget<'_>,
) -> Result<()> {
    source.verify(budget)?;
    destination.verify(budget)?;
    let child = name(child)?;
    let target = name(target)?;
    #[cfg(target_os = "macos")]
    let status = unsafe {
        libc::renameatx_np(
            source.file.as_raw_fd(),
            child.as_ptr(),
            destination.file.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let status = unsafe {
        libc::renameat2(
            source.file.as_raw_fd(),
            child.as_ptr(),
            destination.file.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if status != 0 {
        return Err(Error::io(
            &destination.path,
            std::io::Error::last_os_error(),
        ));
    }
    // The rename is committed even if a later sync/check fails. Its journal
    // must remain intact so a subsequent reviewed run can recover it.
    source.sync()?;
    destination.sync()?;
    source.verify(budget)?;
    destination.verify(budget)
}

pub(super) fn quarantine(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    let parent = Directory::open(journal.asset.path.parent().unwrap(), budget)?;
    let directory = journal.directory();
    let key = name(directory.file_name().unwrap())?;
    if unsafe { libc::mkdirat(parent.file.as_raw_fd(), key.as_ptr(), 0o700) } != 0 {
        return Err(Error::io(&directory, std::io::Error::last_os_error()));
    }
    let trash = parent.child(directory.file_name().unwrap().to_str().unwrap(), budget)?;
    let mut file = owned(
        unsafe {
            libc::openat(
                trash.file.as_raw_fd(),
                c"journal.json".as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        },
        &directory.join("journal.json"),
    )?;
    let bytes = serde_json::to_vec(journal).map_err(|_| changed())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| Error::io(&directory, error))?;
    trash.sync()?;
    parent.sync()?;
    scan::verify_candidate(&journal.asset, false, budget)?;
    rename(
        &parent,
        journal.asset.path.file_name().unwrap(),
        &trash,
        "payload".as_ref(),
        budget,
    )?;
    // A writer ignoring the activity lease can still change a path. Verify
    // what was actually moved before permitting any irreversible removal.
    scan::verify_candidate(&journal.payload(), false, budget)?;
    Ok(())
}

fn verify_journal(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    let directory = journal.directory();
    let observed = load(&directory, budget)?;
    if serde_json::to_value(&observed).unwrap() != serde_json::to_value(journal).unwrap() {
        return Err(changed());
    }
    Ok(())
}

fn finish_journal(journal: &Journal, trash: &Directory, budget: &mut Budget<'_>) -> Result<()> {
    trash.verify(budget)?;
    let entries = filesystem::entries(&trash.path, 2, budget.cancel)?;
    if entries.len() != 1 || entries[0].name != "journal.json" {
        return Err(changed());
    }
    let parent = Directory::open(journal.asset.path.parent().unwrap(), budget)?;
    // Keep a durable owner outside the final directory too. A crash between
    // deleting its journal and removing an empty directory must not strand an
    // unidentifiable trash directory or lose the final cleanup operation.
    rename(
        trash,
        "journal.json".as_ref(),
        &parent,
        journal.marker().file_name().unwrap(),
        budget,
    )?;
    finish_marker(journal, budget)
}

fn finish_marker(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    verify_journal(journal, budget)?;
    if !journal.finalizing()? {
        return Err(changed());
    }
    let parent = Directory::open(journal.asset.path.parent().unwrap(), budget)?;
    if crate::paths::MissingPath::capture(&journal.directory())?.is_none() {
        let trash = parent.child(
            journal.directory().file_name().unwrap().to_str().unwrap(),
            budget,
        )?;
        if !filesystem::entries(&trash.path, 1, budget.cancel)?.is_empty() {
            return Err(changed());
        }
        trash.verify(budget)?;
        parent.unlink(trash.path.file_name().unwrap().to_str().unwrap(), true)?;
    }
    verify_journal(journal, budget)?;
    parent.verify(budget)?;
    parent.unlink(
        journal.marker().file_name().unwrap().to_str().unwrap(),
        false,
    )
}

fn remove_tree(
    asset: &Candidate,
    directory: &Directory,
    relative: &str,
    budget: &mut Budget<'_>,
) -> Result<()> {
    directory.verify(budget)?;
    let mut entries = filesystem::entries(&directory.path, 8194, budget.cancel)?;
    budget.entries(entries.len())?;
    // Keep the manifest until every other member has been removed. Ownership
    // after that point is still proven by the journal outside the payload.
    entries.sort_by_key(|entry| (entry.name == "bundle.json", entry.name.clone()));
    for entry in entries {
        budget.check()?;
        let member = if relative.is_empty() {
            entry.name.clone()
        } else {
            format!("{relative}/{}", entry.name)
        };
        match entry.kind {
            filesystem::Kind::Directory
                if asset
                    .files
                    .keys()
                    .any(|name| name.starts_with(&format!("{member}/"))) =>
            {
                let child = directory.child(&entry.name, budget)?;
                remove_tree(asset, &child, &member, budget)?;
                directory.verify(budget)?;
                directory.unlink(&entry.name, true)?;
            }
            filesystem::Kind::File => {
                let expected = asset.files.get(&member).ok_or_else(changed)?;
                if directory.content(&entry.name, budget)?.as_ref() != Some(expected) {
                    return Err(changed());
                }
                directory.verify(budget)?;
                directory.unlink(&entry.name, false)?;
            }
            _ => return Err(changed()),
        }
    }
    Ok(())
}

pub(super) fn finish(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    verify_journal(journal, budget)?;
    if journal.finalizing()? {
        return finish_marker(journal, budget);
    }
    let trash = Directory::open(&journal.directory(), budget)?;
    let payload = journal.payload();
    let entries = filesystem::entries(&trash.path, 2, budget.cancel)?;
    if let Some(entry) = entries.iter().find(|entry| entry.name == "payload") {
        if payload.bundle && entry.kind == filesystem::Kind::Directory {
            // Validate every remaining member before starting a new cleanup
            // pass. Unknown or changed contents are retained, never removed.
            scan::verify_candidate(&payload, true, budget)?;
            let child = trash.child("payload", budget)?;
            remove_tree(&payload, &child, "", budget)?;
            trash.verify(budget)?;
            trash.unlink("payload", true)?;
        } else if !payload.bundle && entry.kind == filesystem::Kind::File {
            if trash.content("payload", budget)?.as_ref() != payload.files.get("") {
                return Err(changed());
            }
            trash.verify(budget)?;
            trash.unlink("payload", false)?;
        } else {
            return Err(changed());
        }
    }
    finish_journal(journal, &trash, budget)
}

pub(super) fn restore(journal: &Journal, budget: &mut Budget<'_>) -> Result<()> {
    verify_journal(journal, budget)?;
    if journal.finalizing()? {
        scan::verify_candidate(&journal.asset, false, budget).map_err(|error| {
            Error::new(
                "RECOVERY_REQUIRED",
                "Referenced asset was already removed before final journal cleanup",
            )
            .details(json!({"cause":error,"journal":journal.marker()}))
        })?;
        return finish_marker(journal, budget);
    }
    let trash = Directory::open(&journal.directory(), budget)?;
    let parent = Directory::open(journal.asset.path.parent().unwrap(), budget)?;
    if filesystem::entries(&trash.path, 2, budget.cancel)?
        .iter()
        .any(|entry| entry.name == "payload")
    {
        scan::verify_candidate(&journal.payload(), false, budget).map_err(|error| {
            Error::new(
                "RECOVERY_REQUIRED",
                "Referenced quarantined asset is incomplete or changed",
            )
            .details(json!({"cause":error,"journal":trash.path}))
        })?;
        rename(
            &trash,
            "payload".as_ref(),
            &parent,
            journal.asset.path.file_name().unwrap(),
            budget,
        )?;
    }
    scan::verify_candidate(&journal.asset, false, budget)?;
    finish_journal(journal, &trash, budget)
}
