//! Bounded document discovery shared by the launcher and MCP.
use crate::{Error, Result};
use ignore::gitignore::Gitignore;
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    fs::ReadDir,
    path::{Path, PathBuf},
    sync::Arc,
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
    directory: Option<Directory>,
    pending: VecDeque<PendingDirectory>,
    deferred: BTreeMap<usize, VecDeque<Deferred>>,
    limits: Limits,
    started: Instant,
    recursive: bool,
    finished: bool,
}
struct IgnoreRules {
    matcher: Gitignore,
    parent: Option<Arc<Self>>,
}
impl IgnoreRules {
    fn load(path: &Path, parent: Option<Arc<Self>>) -> Arc<Self> {
        let (matcher, _) = Gitignore::new(path.join(".gitignore"));
        Arc::new(Self { matcher, parent })
    }
    fn at_root(root: &Path) -> Arc<Self> {
        let mut ancestors = Vec::new();
        for path in root.ancestors() {
            ancestors.push(path);
            if path.join(".git").exists() {
                break;
            }
        }
        let mut rules = None;
        for path in ancestors.into_iter().rev() {
            rules = Some(Self::load(path, rules));
        }
        rules.expect("The discovery root has at least one ancestor")
    }
    fn ignored(&self, path: &Path, is_dir: bool) -> bool {
        let mut rules = Some(self);
        while let Some(current) = rules {
            let matched = current.matcher.matched(path, is_dir);
            if !matched.is_none() {
                return matched.is_ignore();
            }
            rules = current.parent.as_deref();
        }
        false
    }
}
struct Directory {
    entries: ReadDir,
    depth: usize,
    rules: Arc<IgnoreRules>,
    ignored: bool,
}
struct PendingDirectory {
    path: PathBuf,
    depth: usize,
    rules: Arc<IgnoreRules>,
    ignored: bool,
}
enum Deferred {
    Directory(PendingDirectory),
    Document(PathBuf),
}
impl Scan {
    pub fn new(root: &Path, recursive: bool, limits: Limits) -> Result<Self> {
        let root = crate::paths::canonical(root)?;
        let entries = std::fs::read_dir(&root).map_err(|e| Error::io(&root, e))?;
        let rules = IgnoreRules::at_root(&root);
        Ok(Self {
            listing: Listing {
                root,
                documents: vec![],
                complete: false,
                stop_reason: None,
                entries_scanned: 0,
                unreadable_directories: 0,
            },
            directory: Some(Directory {
                entries,
                depth: 0,
                rules,
                ignored: false,
            }),
            pending: VecDeque::new(),
            deferred: BTreeMap::new(),
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
        self.deferred.clear();
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
            if self.directory.is_none() {
                let next = if let Some(next) = self.pending.pop_front() {
                    Some(Deferred::Directory(next))
                } else if let Some(mut level) = self.deferred.first_entry() {
                    let next = level.get_mut().pop_front();
                    if level.get().is_empty() {
                        level.remove();
                    }
                    next
                } else {
                    None
                };
                if let Some(Deferred::Document(path)) = next {
                    self.listing.documents.push(path);
                    continue;
                } else if let Some(Deferred::Directory(next)) = next {
                    match std::fs::read_dir(&next.path) {
                        Ok(entries) => {
                            self.directory = Some(Directory {
                                entries,
                                depth: next.depth,
                                rules: IgnoreRules::load(&next.path, Some(next.rules)),
                                ignored: next.ignored,
                            });
                        }
                        Err(_) => self.listing.unreadable_directories += 1,
                    }
                    continue;
                }
            }
            let Some(directory) = &mut self.directory else {
                self.finished = true;
                self.listing.complete = self.listing.unreadable_directories == 0;
                if !self.listing.complete {
                    self.listing.stop_reason = Some("unreadable_entries");
                }
                break;
            };
            let Some(entry) = directory.entries.next() else {
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
            let path = entry.path();
            let ignored = directory.ignored || directory.rules.ignored(&path, kind.is_dir());
            if kind.is_dir() && self.recursive {
                if directory.depth >= 127 {
                    self.listing.unreadable_directories += 1;
                    continue;
                }
                // Finish this level before descending, without holding queued directories open.
                let pending = PendingDirectory {
                    path,
                    depth: directory.depth + 1,
                    rules: directory.rules.clone(),
                    ignored,
                };
                if ignored {
                    self.deferred
                        .entry(pending.depth)
                        .or_default()
                        .push_back(Deferred::Directory(pending));
                } else {
                    self.pending.push_back(pending);
                }
            } else if kind.is_file()
                && is_pen(&path)
                && let Ok(path) = path.strip_prefix(&self.listing.root)
            {
                if path.to_str().is_some() {
                    if ignored && !directory.ignored {
                        self.deferred
                            .entry(directory.depth)
                            .or_default()
                            .push_back(Deferred::Document(path.to_owned()));
                    } else {
                        self.listing.documents.push(path.to_owned());
                    }
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
    fn ignored_subtrees_do_not_consume_the_budget_before_normal_documents() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".gitignore"), "ignored/\n").unwrap();
        std::fs::create_dir(temp.path().join("ignored")).unwrap();
        std::fs::write(temp.path().join("ignored/.gitignore"), "!*.pen\n").unwrap();
        std::fs::write(temp.path().join("ignored/hidden.pen"), "").unwrap();
        for index in 0..32 {
            std::fs::write(temp.path().join(format!("ignored/{index}.txt")), "").unwrap();
        }
        std::fs::create_dir_all(temp.path().join("normal/deep")).unwrap();
        std::fs::write(temp.path().join("normal/deep/design.pen"), "").unwrap();
        let cancel = CancellationToken::new();
        let bounded = Scan::new(
            temp.path(),
            true,
            Limits {
                entries: 5,
                ..Limits::default()
            },
        )
        .unwrap()
        .finish(&cancel);
        assert_eq!(
            bounded.documents,
            vec![PathBuf::from("normal/deep/design.pen")]
        );
        assert_eq!(bounded.stop_reason, Some("entry_limit"));
        let all = Scan::new(temp.path(), true, Limits::default())
            .unwrap()
            .finish(&cancel);
        assert!(all.complete);
        assert_eq!(all.documents.len(), 2);
    }
    #[test]
    fn ignored_documents_wait_for_nested_rules_and_negations() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".gitignore"), "*.pen\n!keep.pen\n").unwrap();
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        std::fs::write(
            temp.path().join("nested/.gitignore"),
            "!nested.pen\nkeep.pen\n",
        )
        .unwrap();
        for path in [
            "keep.pen",
            "drop.pen",
            "nested/keep.pen",
            "nested/nested.pen",
            "nested/drop.pen",
        ] {
            std::fs::write(temp.path().join(path), "").unwrap();
        }
        let cancel = CancellationToken::new();
        let listing = Scan::new(
            temp.path(),
            true,
            Limits {
                documents: 2,
                ..Limits::default()
            },
        )
        .unwrap()
        .finish(&cancel);
        assert_eq!(
            listing.documents,
            vec![
                PathBuf::from("keep.pen"),
                PathBuf::from("nested/nested.pen")
            ]
        );
        assert_eq!(listing.stop_reason, Some("document_limit"));
        let nested = Scan::new(
            &temp.path().join("nested"),
            false,
            Limits {
                documents: 1,
                ..Limits::default()
            },
        )
        .unwrap()
        .finish(&cancel);
        assert_eq!(nested.documents, vec![PathBuf::from("nested.pen")]);
        let all = Scan::new(temp.path(), true, Limits::default())
            .unwrap()
            .finish(&cancel);
        assert!(all.complete);
        assert_eq!(all.documents.len(), 5);
    }
    #[test]
    fn deferred_discovery_keeps_shallow_documents_ahead_of_deeper_ones() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".gitignore"), "ignored/\n*.pen\n").unwrap();
        std::fs::create_dir_all(temp.path().join("ignored/shallow")).unwrap();
        std::fs::create_dir_all(temp.path().join("normal/deep/deeper")).unwrap();
        std::fs::write(temp.path().join("ignored/shallow/design.pen"), "").unwrap();
        std::fs::write(temp.path().join("normal/deep/deeper/design.pen"), "").unwrap();
        // A normal branch discovers its ignored file before any ignored subtree is opened.
        let listing = Scan::new(
            temp.path(),
            true,
            Limits {
                documents: 1,
                ..Limits::default()
            },
        )
        .unwrap()
        .finish(&CancellationToken::new());
        assert_eq!(
            listing.documents,
            vec![PathBuf::from("ignored/shallow/design.pen")]
        );
        assert_eq!(listing.stop_reason, Some("document_limit"));
    }
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
