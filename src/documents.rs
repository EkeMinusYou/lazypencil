//! Bounded document discovery shared by the launcher and MCP.
use crate::{Error, Result};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs::ReadDir,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub struct Limits {
    pub documents: usize,
    pub entries: usize,
    pub time: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            documents: 10_000,
            entries: 100_000,
            time: Duration::from_secs(5),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Listing {
    pub root: PathBuf,
    pub documents: Vec<PathBuf>,
    pub complete: bool,
    pub stop_reason: Option<&'static str>,
    pub entries_scanned: usize,
    pub unreadable_directories: usize,
}
pub struct Scan {
    pub listing: Listing,
    directory: Option<(ReadDir, usize)>,
    pending: VecDeque<(PathBuf, usize)>,
    limits: Limits,
    started: Instant,
    recursive: bool,
    finished: bool,
}
impl Scan {
    pub fn new(root: &Path, recursive: bool, limits: Limits) -> Result<Self> {
        let root = crate::paths::canonical(root)?;
        let entries = std::fs::read_dir(&root).map_err(|e| Error::io(&root, e))?;
        Ok(Self {
            listing: Listing {
                root,
                documents: vec![],
                complete: false,
                stop_reason: None,
                entries_scanned: 0,
                unreadable_directories: 0,
            },
            directory: Some((entries, 0)),
            pending: VecDeque::new(),
            limits,
            started: Instant::now(),
            recursive,
            finished: false,
        })
    }
    fn stop(&mut self, reason: &'static str) {
        self.listing.stop_reason = Some(reason);
        self.directory = None;
        self.pending.clear();
        self.finished = true;
    }
    /// Process a small batch so callers can publish results and handle input.
    pub fn step(&mut self, cancel: &CancellationToken) -> bool {
        if self.finished {
            return true;
        }
        for _ in 0..256 {
            if cancel.is_cancelled() {
                self.stop("cancelled");
                break;
            }
            if self.listing.documents.len() >= self.limits.documents {
                self.stop("document_limit");
                break;
            }
            if self.listing.entries_scanned >= self.limits.entries {
                self.stop("entry_limit");
                break;
            }
            if self.started.elapsed() >= self.limits.time {
                self.stop("time_limit");
                break;
            }
            if self.directory.is_none()
                && let Some((path, depth)) = self.pending.pop_front()
            {
                match std::fs::read_dir(path) {
                    Ok(entries) => self.directory = Some((entries, depth)),
                    Err(_) => self.listing.unreadable_directories += 1,
                }
                continue;
            }
            let Some((directory, depth)) = &mut self.directory else {
                self.finished = true;
                self.listing.complete = self.listing.unreadable_directories == 0;
                if !self.listing.complete {
                    self.listing.stop_reason = Some("unreadable_entries");
                }
                break;
            };
            let Some(entry) = directory.next() else {
                self.directory = None;
                continue;
            };
            self.listing.entries_scanned += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    self.listing.unreadable_directories += 1;
                    continue;
                }
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => {
                    self.listing.unreadable_directories += 1;
                    continue;
                }
            };
            if kind.is_dir() && self.recursive {
                if *depth >= 127 {
                    self.listing.unreadable_directories += 1;
                    continue;
                }
                // Finish this level before descending, without holding queued directories open.
                self.pending.push_back((entry.path(), *depth + 1));
            } else if kind.is_file()
                && is_pen(&entry.path())
                && let Ok(path) = entry.path().strip_prefix(&self.listing.root)
            {
                if path.to_str().is_some() {
                    self.listing.documents.push(path.to_owned());
                } else {
                    self.listing.unreadable_directories += 1;
                }
            }
        }
        self.finished
    }
    pub fn finish(mut self, cancel: &CancellationToken) -> Listing {
        while !self.step(cancel) {}
        self.listing.documents.sort();
        self.listing
    }
}
pub fn is_pen(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pen"))
}
pub fn require_pen(path: &Path) -> Result<()> {
    if !is_pen(path) {
        return Err(Error::new(
            "INVALID_ARGUMENT",
            "Expected a .pen document path",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_discovery_finds_shallow_documents_before_deeper_entries() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("design.pen"), "").unwrap();
        let mut shallow = vec![PathBuf::from("design.pen")];
        for name in ["a", "b", "c"] {
            let directory = temp.path().join(name);
            std::fs::create_dir_all(directory.join("deep")).unwrap();
            std::fs::write(directory.join("child.pen"), "").unwrap();
            shallow.push(PathBuf::from(name).join("child.pen"));
            for index in 0..8 {
                std::fs::write(directory.join("deep").join(format!("{index}.txt")), "").unwrap();
            }
        }
        shallow.sort();
        let cancel = CancellationToken::new();
        // Each budget covers one more level, regardless of filesystem entry order.
        for (entries, expected) in [(4, vec![PathBuf::from("design.pen")]), (10, shallow)] {
            let listing = Scan::new(
                temp.path(),
                true,
                Limits {
                    entries,
                    ..Limits::default()
                },
            )
            .unwrap()
            .finish(&cancel);
            assert_eq!(listing.documents, expected);
            assert_eq!(listing.entries_scanned, entries);
            assert_eq!(listing.stop_reason, Some("entry_limit"));
            assert!(!listing.complete);
        }
    }
    #[test]
    fn recursive_discovery_includes_ignored_files_and_bounds_work() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        for file in [
            "a.pen",
            "nested/b.pen",
            "nested/c.PEN",
            ".hidden.pen",
            ".gitignore",
        ] {
            std::fs::write(temp.path().join(file), "ignored").unwrap();
        }
        let cancel = CancellationToken::new();
        let all = Scan::new(temp.path(), true, Limits::default())
            .unwrap()
            .finish(&cancel);
        assert!(all.complete);
        assert_eq!(all.documents.len(), 4);
        assert_eq!(
            Scan::new(temp.path(), false, Limits::default())
                .unwrap()
                .finish(&cancel)
                .documents
                .len(),
            2
        );
        for (limits, reason) in [
            (
                Limits {
                    documents: 1,
                    ..Limits::default()
                },
                "document_limit",
            ),
            (
                Limits {
                    entries: 1,
                    ..Limits::default()
                },
                "entry_limit",
            ),
            (
                Limits {
                    time: Duration::ZERO,
                    ..Limits::default()
                },
                "time_limit",
            ),
        ] {
            let stopped = Scan::new(temp.path(), true, limits)
                .unwrap()
                .finish(&cancel);
            assert!(!stopped.complete);
            assert_eq!(stopped.stop_reason, Some(reason));
        }
        cancel.cancel();
        assert_eq!(
            Scan::new(temp.path(), true, Limits::default())
                .unwrap()
                .finish(&cancel)
                .stop_reason,
            Some("cancelled")
        );
    }
    #[cfg(unix)]
    #[test]
    fn recursive_discovery_does_not_follow_symbolic_link_cycles() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a.pen"), "").unwrap();
        std::os::unix::fs::symlink(temp.path(), temp.path().join("cycle")).unwrap();
        let listing = Scan::new(temp.path(), true, Limits::default())
            .unwrap()
            .finish(&CancellationToken::new());
        assert!(listing.complete);
        assert_eq!(listing.documents, vec![PathBuf::from("a.pen")]);
    }
}
