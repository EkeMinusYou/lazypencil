//! Watch for writes between periodic full-content revision checks.
use crate::{Result, store::Store};

#[derive(Default)]
pub(super) struct RevisionGuard {
    #[cfg(target_os = "macos")]
    watcher: Option<macos::Watcher>,
}

impl RevisionGuard {
    pub(super) fn verify(&mut self, store: &Store, expected: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            if let Some(watcher) = &mut self.watcher
                && watcher.revision.as_deref() == Some(expected)
                && watcher.verified_at.elapsed() < std::time::Duration::from_millis(500)
                && watcher.same_file(store)
                && matches!(watcher.changed(), Ok(false))
            {
                return Ok(());
            }
            // Register before hashing, so a write during verification cannot
            // silently establish a cached revision. Failure keeps full hashing.
            self.watcher = macos::Watcher::new(store).ok();
            store.verify_revision(expected)?;
            if let Some(watcher) = &mut self.watcher {
                match watcher.changed() {
                    Ok(false) => {
                        watcher.revision = Some(expected.to_owned());
                        watcher.verified_at = std::time::Instant::now();
                    }
                    Ok(true) => {
                        self.watcher = None;
                        return Err(crate::Error::new(
                            "REVISION_CONFLICT",
                            "The document changed during preview verification; retry",
                        ));
                    }
                    Err(_) => self.watcher = None,
                }
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        store.verify_revision(expected)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{
        fs::File,
        io,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::fs::MetadataExt,
        },
    };

    pub(super) struct Watcher {
        queue: OwnedFd,
        // Keep the watched inode alive; an atomic replacement must get a new watch.
        _file: File,
        identity: (u64, u64),
        pub(super) revision: Option<String>,
        pub(super) verified_at: std::time::Instant,
    }

    impl Watcher {
        pub(super) fn new(store: &Store) -> io::Result<Self> {
            let file = crate::store::open_regular_file(store.path())
                .map_err(|error| io::Error::other(error.to_string()))?;
            let metadata = file.metadata()?;
            let raw = unsafe { libc::kqueue() };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let queue = unsafe { OwnedFd::from_raw_fd(raw) };
            if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(io::Error::last_os_error());
            }
            let change = libc::kevent {
                ident: file.as_raw_fd() as usize,
                filter: libc::EVFILT_VNODE,
                flags: libc::EV_ADD | libc::EV_CLEAR,
                fflags: libc::NOTE_WRITE
                    | libc::NOTE_EXTEND
                    | libc::NOTE_ATTRIB
                    | libc::NOTE_DELETE
                    | libc::NOTE_RENAME
                    | libc::NOTE_REVOKE
                    | libc::NOTE_LINK,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            if unsafe {
                libc::kevent(
                    queue.as_raw_fd(),
                    &change,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                queue,
                _file: file,
                identity: (metadata.dev(), metadata.ino()),
                revision: None,
                verified_at: std::time::Instant::now(),
            })
        }

        pub(super) fn same_file(&self, store: &Store) -> bool {
            // Reopen the actual path, including its ancestors. A vnode watch
            // alone cannot see replacement of an enclosing directory.
            crate::store::open_regular_file(store.path())
                .ok()
                .and_then(|file| file.metadata().ok())
                .is_some_and(|metadata| (metadata.dev(), metadata.ino()) == self.identity)
        }

        pub(super) fn changed(&mut self) -> io::Result<bool> {
            let mut event = std::mem::MaybeUninit::<libc::kevent>::zeroed();
            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let count = unsafe {
                libc::kevent(
                    self.queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    event.as_mut_ptr(),
                    1,
                    &timeout,
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(count != 0)
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn kernel_guard_detects_same_length_writes_even_with_restored_mtime() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("design.pen");
        let original = r##"{"version":"2.21","children":[],"name":"a"}"##;
        fs::write(&path, original).unwrap();
        let store = Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let expected = store.read().unwrap().revision();
        let mut guard = RevisionGuard::default();
        guard.verify(&store, &expected).unwrap();
        guard.verify(&store, &expected).unwrap();
        assert!(guard.watcher.as_ref().unwrap().revision.is_some());
        for letter in ['b', 'c', 'd', 'e'] {
            fs::write(&path, original.replace("\"a\"", &format!("\"{letter}\""))).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
            assert_eq!(
                guard.verify(&store, &expected).unwrap_err().code,
                "REVISION_CONFLICT"
            );
            fs::write(&path, original).unwrap();
            guard.verify(&store, &expected).unwrap();
        }
    }

    #[test]
    fn kernel_guard_rebinds_atomic_replacements_and_replaced_ancestors() {
        let directory = tempfile::TempDir::new().unwrap();
        let parent = directory.path().join("root");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("design.pen");
        let original = r##"{"version":"2.21","children":[],"name":"a"}"##;
        let changed = original.replace("\"a\"", "\"b\"");
        fs::write(&path, original).unwrap();
        let store = Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let expected = store.read().unwrap().revision();
        let mut guard = RevisionGuard::default();
        guard.verify(&store, &expected).unwrap();
        crate::store::atomic_replace(&path, changed.as_bytes()).unwrap();
        assert_eq!(
            guard.verify(&store, &expected).unwrap_err().code,
            "REVISION_CONFLICT"
        );
        fs::rename(&parent, directory.path().join("retired")).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::write(&path, original).unwrap();
        guard.verify(&store, &expected).unwrap();
        fs::write(&path, &changed).unwrap();
        assert_eq!(
            guard.verify(&store, &expected).unwrap_err().code,
            "REVISION_CONFLICT"
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            guard.verify(&store, &expected).unwrap_err().code,
            "IO_ERROR"
        );
        fs::write(&path, original).unwrap();
        guard.verify(&store, &expected).unwrap();
    }
}

#[cfg(all(test, target_os = "macos"))]
mod mapped_write_tests {
    use super::*;
    use std::{
        fs,
        os::fd::AsRawFd,
        time::{Duration, Instant},
    };

    #[test]
    fn expired_revision_cache_checks_mapped_writes_even_without_a_new_vnode_event() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("design.pen");
        let original = r##"{"version":"2.21","children":[],"name":"a"}"##;
        fs::write(&path, original).unwrap();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let pointer = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                original.len(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        assert_ne!(pointer, libc::MAP_FAILED);
        struct Mapping(*mut libc::c_void, usize);
        impl Drop for Mapping {
            fn drop(&mut self) {
                unsafe {
                    libc::munmap(self.0, self.1);
                }
            }
        }
        let mapping = Mapping(pointer, original.len());
        let store = Store::open(&path, Some(directory.path().join("data"))).unwrap();
        let expected = store.read().unwrap().revision();
        let mut guard = RevisionGuard::default();
        let position = original.find("\"a\"").unwrap() + 1;
        for letter in *b"bcd" {
            guard.verify(&store, &expected).unwrap();
            unsafe {
                mapping.0.cast::<u8>().add(position).write(letter);
            }
            // Writable mapped pages need not emit a vnode event for every store.
            // Periodic verification must still hash their current contents.
            guard.watcher.as_mut().unwrap().verified_at = Instant::now() - Duration::from_secs(1);
            assert_eq!(
                guard.verify(&store, &expected).unwrap_err().code,
                "REVISION_CONFLICT"
            );
            unsafe {
                mapping.0.cast::<u8>().add(position).write(b'a');
            }
        }
    }
}
