use std::{io, path::Path};

pub fn unpack<R: io::Read>(
    archive: &mut tar::Archive<R>,
    destination: &Path,
    root: &Path,
    keep: fn(&Path) -> bool,
) -> io::Result<()> {
    let mut directories = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let mut components = path.components();
        if components.next().map(|component| component.as_os_str()) != Some(root.as_os_str())
            || !keep(components.as_path())
        {
            continue;
        }
        // Apply directory permissions after descendants, as Archive::unpack
        // does, and restore archived Unix mtimes below. tar preserves file
        // mtimes but leaves directory mtimes at extraction time.
        if entry.header().entry_type().is_dir() {
            directories.push(entry);
        } else {
            entry.unpack_in(destination)?;
        }
    }
    directories.sort_by(|a, b| b.path_bytes().cmp(&a.path_bytes()));
    for mut directory in directories {
        if !directory.unpack_in(destination)? {
            continue;
        }
        #[cfg(unix)]
        {
            let mtime = std::time::SystemTime::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(
                    directory.header().mtime()?.max(1),
                ))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Invalid directory mtime")
                })?;
            std::fs::File::open(destination.join(directory.path()?))?
                .set_times(std::fs::FileTimes::new().set_modified(mtime))?;
        }
    }
    Ok(())
}
