use lazypencil::{paths, store::Store};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn case_and_unicode_aliases_share_the_same_lock_when_the_filesystem_aliases_them() {
    let temp = TempDir::new().unwrap();
    let original = temp.path().join("Case-Caf\u{00e9}-デザイン.pen");
    let store = Store::init(&original, Some(temp.path().join("data-one"))).unwrap();
    assert_eq!(store.path(), paths::canonical(&original).unwrap());
    fs::hard_link(store.path(), temp.path().join("another-link.pen")).unwrap();
    for name in [
        "case-caf\u{00e9}-デザイン.pen",
        "Case-Cafe\u{0301}-デザイン.pen",
    ] {
        let alternate = temp.path().join(name);
        if !alternate.exists() {
            continue;
        }
        let alternate = Store::open(&alternate, Some(temp.path().join("data-two"))).unwrap();
        assert_eq!(store.path(), alternate.path());
        assert_eq!(store.lock_file_path(), alternate.lock_file_path());
    }
}

#[test]
fn hardlinks_keep_distinct_document_paths_and_are_not_rewritten_as_another_link() {
    let temp = TempDir::new().unwrap();
    let first = Store::init(
        temp.path().join("first.pen"),
        Some(temp.path().join("data")),
    )
    .unwrap();
    let second_path = temp.path().join("second.pen");
    fs::hard_link(first.path(), &second_path).unwrap();
    let _first_handle = fs::File::open(first.path()).unwrap();
    let second = Store::open(&second_path, Some(temp.path().join("data"))).unwrap();
    assert_ne!(first.path(), second.path());
    assert_ne!(first.lock_file_path(), second.lock_file_path());
    assert_eq!(second.path().file_name().unwrap(), "second.pen");

    let renamed = temp.path().join("renamed.pen");
    fs::rename(&second_path, &renamed).unwrap();
    let selected = Store::open(&renamed, Some(temp.path().join("data"))).unwrap();
    assert_eq!(selected.path().file_name().unwrap(), "renamed.pen");
    assert_ne!(first.lock_file_path(), selected.lock_file_path());
    let before = first.read().unwrap();
    selected
        .transact("edit-link", &before.revision(), "edit-link", |document| {
            let mut after = document.clone();
            after.set_property("", "name", &json!("Selected link"))?;
            Ok((after, json!({})))
        })
        .unwrap();
    assert_eq!(fs::read(first.path()).unwrap(), before.text().as_bytes());
    assert_ne!(fs::read(&renamed).unwrap(), before.text().as_bytes());
    assert!(!second_path.exists());
    assert!(first.history(10, None).unwrap().is_empty());
    assert_eq!(selected.history(10, None).unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn symlinks_share_the_target_lock() {
    let temp = TempDir::new().unwrap();
    let target = Store::init(
        temp.path().join("target.pen"),
        Some(temp.path().join("data-one")),
    )
    .unwrap();
    let link = temp.path().join("link.pen");
    std::os::unix::fs::symlink(target.path(), &link).unwrap();
    let linked = Store::open(link, Some(temp.path().join("data-two"))).unwrap();
    assert_eq!(target.lock_file_path(), linked.lock_file_path());
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_report_an_error_before_json_serialization() {
    use std::os::unix::ffi::OsStringExt;
    let temp = TempDir::new().unwrap();
    let name = std::ffi::OsString::from_vec(b"invalid-\xff.pen".to_vec());
    let path = temp.path().join(name);
    assert_eq!(paths::canonical(path).unwrap_err().code, "INVALID_ARGUMENT");
    let destination = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"new-\xff.pen".to_vec()));
    assert_eq!(
        Store::init(&destination, Some(temp.path().join("data")))
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
    assert!(!destination.exists());
}
