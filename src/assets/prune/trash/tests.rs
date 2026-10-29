use super::*;
use std::fs;

fn budget() -> Budget<'static> {
    Budget {
        started: Instant::now(),
        cancel: None,
        entries: 0,
        bytes: 0,
    }
}

fn bundle(root: &Path) -> Candidate {
    // Generate the fixture through the real publisher's manifest encoder.
    let files = crate::managed::File::bundle(
        &root.join("main.pen"),
        BTreeMap::from([
            (
                "nested/a.js".into(),
                std::sync::Arc::<[u8]>::from(b"return [];".as_slice()),
            ),
            (
                "b.png".into(),
                std::sync::Arc::<[u8]>::from(b"image".as_slice()),
            ),
        ]),
        None,
    )
    .unwrap();
    for file in &files {
        fs::create_dir_all(file.path.parent().unwrap()).unwrap();
        fs::write(&file.path, &file.bytes).unwrap();
    }
    let path = files[0]
        .path
        .ancestors()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "bundle")
        })
        .unwrap();
    scan::candidate(path, &mut budget()).unwrap()
}

#[test]
fn interrupted_bundle_cleanup_recovers_even_after_its_manifest_was_removed() {
    for remove_manifest in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let asset = bundle(&root);
        let journal = quarantine(&asset, &root, &root.join("main.pen"), &mut budget()).unwrap();
        fs::remove_file(journal.payload().path.join("nested/a.js")).unwrap();
        if remove_manifest {
            fs::remove_file(journal.payload().path.join("bundle.json")).unwrap();
        }
        let loaded = load(&journal.directory(), &mut budget()).unwrap();
        finish(&loaded, &mut budget()).unwrap();
        assert!(!journal.directory().exists());
        assert!(!asset.path.exists());
    }
}

#[test]
fn final_cleanup_remains_identifiable_before_and_after_the_empty_directory_is_removed() {
    for remove_directory in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let asset = bundle(&root);
        let journal = quarantine(&asset, &root, &root.join("main.pen"), &mut budget()).unwrap();
        fs::remove_dir_all(journal.payload().path).unwrap();
        fs::rename(journal.directory().join("journal.json"), journal.marker()).unwrap();
        if remove_directory {
            fs::remove_dir(journal.directory()).unwrap();
        }
        let loaded = load(&journal.marker(), &mut budget()).unwrap();
        finish(&loaded, &mut budget()).unwrap();
        assert!(!journal.marker().exists());
        assert!(!journal.directory().exists());
        assert!(!asset.path.exists());
    }
}

#[test]
fn quarantine_restores_complete_referenced_bundles_without_clobbering_a_new_owner() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let asset = bundle(&root);
    let journal = quarantine(&asset, &root, &root.join("main.pen"), &mut budget()).unwrap();
    fs::create_dir(&asset.path).unwrap();
    fs::write(asset.path.join("foreign"), b"new owner").unwrap();
    assert!(restore(&journal, &mut budget()).is_err());
    assert_eq!(fs::read(asset.path.join("foreign")).unwrap(), b"new owner");
    assert!(journal.payload().path.join("bundle.json").exists());
    fs::remove_file(asset.path.join("foreign")).unwrap();
    fs::remove_dir(&asset.path).unwrap();
    restore(&journal, &mut budget()).unwrap();
    scan::verify_candidate(&asset, false, &mut budget()).unwrap();
    assert!(!journal.directory().exists());
}

#[test]
fn partial_referenced_bundles_foreign_files_links_and_cancellation_remain_recoverable() {
    for condition in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let asset = bundle(&root);
        let journal = quarantine(&asset, &root, &root.join("main.pen"), &mut budget()).unwrap();
        let payload = journal.payload().path;
        match condition {
            0 => {
                fs::remove_file(payload.join("nested/a.js")).unwrap();
                assert_eq!(
                    restore(&journal, &mut budget()).unwrap_err().code,
                    "RECOVERY_REQUIRED"
                );
            }
            1 => {
                fs::write(payload.join("foreign"), b"unrelated").unwrap();
                assert!(finish(&journal, &mut budget()).is_err());
                assert_eq!(fs::read(payload.join("foreign")).unwrap(), b"unrelated");
            }
            2 => {
                fs::remove_file(payload.join("b.png")).unwrap();
                std::os::unix::fs::symlink(root.join("outside"), payload.join("b.png")).unwrap();
                fs::write(root.join("outside"), b"private").unwrap();
                assert!(finish(&journal, &mut budget()).is_err());
                assert_eq!(fs::read(root.join("outside")).unwrap(), b"private");
            }
            _ => {
                let token = CancellationToken::new();
                token.cancel();
                let mut cancelled = Budget {
                    cancel: Some(&token),
                    ..budget()
                };
                assert_eq!(
                    finish(&journal, &mut cancelled).unwrap_err().code,
                    "CANCELLED"
                );
            }
        }
        assert!(journal.directory().join("journal.json").exists());
        assert!(payload.join("bundle.json").exists());
    }
}
