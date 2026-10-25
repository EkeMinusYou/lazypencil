use lazypencil_xtask::{dist, util};
use serde_json::Value;
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    for (name, bytes) in [
        (
            "Cargo.toml",
            "[package]\nname='fixture'\nversion='0.1.0'\nrust-version='1.94'\n[patch.crates-io]\ndep={path='vendor/skia-bindings'}\n",
        ),
        ("Cargo.lock", "version=4\n"),
        ("LICENSE", "MIT License\nCopyright (c) Fixture\n"),
        ("README.md", "Fixture 日本語\n"),
        ("src/main.rs", "fn main() {}\n"),
        (
            "vendor/skia-bindings/Cargo.toml",
            "[package]\nname='dep'\nversion='0.1.0'\n",
        ),
        (
            "vendor/skia-bindings/src/lib.rs",
            "// Patched implementation\n",
        ),
    ] {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    let archive = temp.path().join("source.tar.gz");
    (temp, root, archive)
}

fn replace(path: &Path, members: &dist::Files) {
    let compressed = flate2::write::GzEncoder::new(
        fs::File::create(path).unwrap(),
        flate2::Compression::default(),
    );
    let mut builder = tar::Builder::new(compressed);
    for (name, (bytes, mode)) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

#[test]
fn complete_source_is_deterministic_keeps_local_patches_and_excludes_build_caches() {
    let (temp, root, archive) = fixture();
    for name in [
        "vendor/skia-bindings/skia/large-cache",
        "xtask/target/debug/secret",
        "target/debug/secret",
        "scripts/__pycache__/cache.pyc",
    ] {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "not distributable").unwrap();
    }
    let first = dist::source(&root, &archive, 0, true).unwrap();
    let second = dist::source(&root, &temp.path().join("second.tar.gz"), 0, true).unwrap();
    assert_eq!(first["sha256"], second["sha256"]);
    let output = temp.path().join("extracted");
    let verified = dist::verify(&archive, Some(&output)).unwrap();
    assert_eq!(verified["sha256"], first["sha256"]);
    assert_eq!(
        fs::read(output.join("vendor/skia-bindings/src/lib.rs")).unwrap(),
        fs::read(root.join("vendor/skia-bindings/src/lib.rs")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(output.join("README.md")).unwrap(),
        "Fixture 日本語\n"
    );
    for name in [
        "vendor/skia-bindings/skia",
        "target",
        "xtask/target",
        "scripts/__pycache__",
    ] {
        assert!(!output.join(name).exists(), "{name}");
    }
}

#[test]
fn missing_patches_corrupt_content_and_unlisted_files_are_rejected_before_publication() {
    let (_temp, root, archive) = fixture();
    fs::remove_file(root.join("vendor/skia-bindings/Cargo.toml")).unwrap();
    assert!(dist::source(&root, &archive, 0, true).is_err());
    assert!(!archive.exists());
    let (temp, root, archive) = fixture();
    dist::source(&root, &archive, 0, true).unwrap();
    let original = dist::read_archive(&archive).unwrap();
    let mut members = original.clone();
    members.get_mut("fixture-0.1.0/src/main.rs").unwrap().0 = b"tampered".to_vec();
    replace(&archive, &members);
    let output = temp.path().join("extracted");
    assert!(dist::verify(&archive, Some(&output)).is_err());
    assert!(!output.exists());
    let mut members = original;
    members.insert(
        "fixture-0.1.0/extra.txt".into(),
        (b"unlisted".to_vec(), 0o644),
    );
    replace(&archive, &members);
    assert!(dist::verify(&archive, None).is_err());
}

#[test]
fn portable_paths_reject_escape_drives_streams_reserved_names_and_aliases() {
    for name in [
        "",
        "/absolute",
        "../escape",
        "a/../b",
        "a//b",
        "a/./b",
        "C:/escape",
        "a:stream",
        "NUL.txt",
        "aux",
        "COM1.json",
        "lpt9",
        "trailing.",
        "trailing ",
        "a\\b",
        "a\0b",
        "control\nfile",
    ] {
        assert!(util::safe_path(name).is_err(), "{name:?}");
    }
    for name in [
        "普通の名前.rs",
        "data/valid-name.json",
        "COM10",
        "text/notice.txt",
    ] {
        assert!(util::safe_path(name).is_ok(), "{name:?}");
    }
    let (_temp, root, archive) = fixture();
    fs::write(root.join("src/straße.rs"), b"a").unwrap();
    fs::write(root.join("src/STRASSE.rs"), b"b").unwrap();
    assert!(dist::source(&root, &archive, 0, true).is_err());
}

#[test]
fn archive_and_extraction_never_replace_existing_outputs() {
    let (temp, root, archive) = fixture();
    dist::source(&root, &archive, 0, true).unwrap();
    let original = fs::read(&archive).unwrap();
    assert!(dist::source(&root, &archive, 0, true).is_err());
    assert_eq!(fs::read(&archive).unwrap(), original);
    let output = temp.path().join("existing");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep.txt"), b"keep").unwrap();
    assert!(dist::verify(&archive, Some(&output)).is_err());
    assert_eq!(fs::read(output.join("keep.txt")).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn symlinks_never_read_private_files_or_alias_source_directories() {
    use std::os::unix::fs::symlink;
    let (temp, root, archive) = fixture();
    let private = temp.path().join("private.txt");
    fs::write(&private, b"private").unwrap();
    symlink(&private, root.join("src/linked")).unwrap();
    assert!(dist::source(&root, &archive, 0, true).is_err());
    assert!(!archive.exists());
}

#[test]
fn clean_git_identity_is_required_for_release_and_ignored_files_stay_private() {
    let (temp, root, archive) = fixture();
    let hooks = temp.path().join("empty-hooks");
    fs::create_dir(&hooks).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
            ])
            .arg(format!("core.hooksPath={}", hooks.display()))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--quiet"]);
    git(&["add", "."]);
    git(&["commit", "-m", "Create distribution fixture"]);
    fs::write(root.join(".git/info/exclude"), "src/ignored-local.rs\n").unwrap();
    fs::write(root.join("src/ignored-local.rs"), "private").unwrap();
    let report = dist::source(&root, &archive, 0, false).unwrap();
    assert_eq!(
        report["git"],
        serde_json::json!({"revision":git(&["rev-parse","HEAD"]),"dirty":false})
    );
    assert!(
        !dist::read_archive(&archive)
            .unwrap()
            .keys()
            .any(|n| n.ends_with("ignored-local.rs"))
    );
    fs::write(root.join("README.md"), "changed").unwrap();
    let refused = temp.path().join("dirty.tar.gz");
    assert!(dist::source(&root, &refused, 0, false).is_err());
    assert!(!refused.exists());
    assert_eq!(
        dist::source(&root, &refused, 0, true).unwrap()["git"]["dirty"],
        true
    );
}

#[test]
fn source_manifest_types_duplicates_modes_and_package_identity_are_strict() {
    let (_temp, root, archive) = fixture();
    dist::source(&root, &archive, 0, true).unwrap();
    let original = dist::read_archive(&archive).unwrap();
    let manifest_name = "fixture-0.1.0/SOURCE-MANIFEST.json";
    for variant in 0..7 {
        let mut members = original.clone();
        let mut manifest: Value = serde_json::from_slice(&members[manifest_name].0).unwrap();
        match variant {
            0 => manifest["format_version"] = serde_json::json!(2),
            1 => manifest["files"][0]["bytes"] = serde_json::json!(true),
            2 => manifest["files"][0]["mode"] = serde_json::json!(true),
            3 => manifest["files"][0]["extra"] = serde_json::json!(1),
            4 => {
                let duplicate = manifest["files"][0].clone();
                manifest["files"].as_array_mut().unwrap().push(duplicate);
            }
            5 => manifest["package"]["version"] = serde_json::json!("different"),
            _ => manifest["files"][0]["path"] = serde_json::json!("../escape"),
        }
        members.get_mut(manifest_name).unwrap().0 = serde_json::to_vec(&manifest).unwrap();
        replace(&archive, &members);
        assert!(dist::verify(&archive, None).is_err(), "variant={variant}");
    }
}
