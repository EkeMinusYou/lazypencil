use super::*;

#[test]
fn maintenance_waits_for_every_shared_owner_and_cloned_lease() {
    let temp = tempfile::tempdir().unwrap();
    let first = AssetActivity::in_directory(temp.path(), false, None).unwrap();
    let cloned = first.clone();
    let second = AssetActivity::in_directory(temp.path(), false, None).unwrap();
    for owner in [first, cloned, second] {
        assert_eq!(
            AssetActivity::in_directory(temp.path(), true, None)
                .unwrap_err()
                .code,
            "LOCK_UNAVAILABLE"
        );
        drop(owner);
    }
    let exclusive = AssetActivity::in_directory(temp.path(), true, None).unwrap();
    assert_eq!(
        AssetActivity::in_directory(temp.path(), true, None)
            .unwrap_err()
            .code,
        "LOCK_UNAVAILABLE"
    );
    drop(exclusive);
    assert!(AssetActivity::in_directory(temp.path(), false, None).is_ok());
}

#[test]
fn cancellation_prevents_acquiring_an_activity_lease() {
    let temp = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        AssetActivity::in_directory(temp.path(), false, Some(&cancel))
            .unwrap_err()
            .code,
        "CANCELLED"
    );
    assert!(std::fs::read_dir(temp.path()).unwrap().next().is_none());
}

#[test]
fn a_captured_project_keeps_its_lease_after_the_store_and_other_clones_close() {
    let temp = tempfile::tempdir().unwrap();
    let root = crate::paths::canonical(temp.path()).unwrap();
    let file = root.join("document.pen");
    fs::write(&file, Document::empty().text()).unwrap();
    let directory = root.join("locks");
    let activity = AssetActivity::in_directory(&directory, false, None).unwrap();
    let store =
        Store::at_path_with_activity(file, Some(root.join("data")), directory.clone(), activity)
            .unwrap();
    let cloned = store.clone();
    let project = crate::project::Project::load(&store).unwrap();
    drop(store);
    drop(cloned);
    assert_eq!(
        AssetActivity::in_directory(&directory, true, None)
            .unwrap_err()
            .code,
        "LOCK_UNAVAILABLE"
    );
    drop(project);
    let maintenance = AssetActivity::in_directory(&directory, true, None).unwrap();
    let store = Store::at_path_with_activity(
        root.join("document.pen"),
        Some(root.join("data")),
        directory,
        maintenance,
    )
    .unwrap();
    // Loading inside maintenance must reuse the exclusive lease instead
    // of trying to acquire a conflicting shared lock from the same process.
    let project = crate::project::Project::load(&store).unwrap();
    assert!(
        project.root_document().value()["children"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
