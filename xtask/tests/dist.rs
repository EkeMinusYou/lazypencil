use lazypencil_xtask::{dist, util};
use serde_json::Value;
use std::{
    fs,
    io::{Cursor, Read, Write},
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
    let (temp, root, archive) = fixture();
    dist::source(&root, &archive, 0, true).unwrap();
    let output = temp.path().join("extracted");
    dist::verify(&archive, Some(&output)).unwrap();
    let mut manifest: Value = util::json(&output.join(dist::MANIFEST)).unwrap();
    let mut members = dist::read_archive(&archive).unwrap();
    // Case-insensitive hosts merge these files before the source builder sees
    // them. Archive and manifest inputs retain both names on every platform.
    for name in ["src/straße.rs", "src/STRASSE.rs"] {
        fs::write(output.join(name), b"a").unwrap();
        manifest["files"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "path":name,"sha256":util::digest(b"a"),"bytes":1,"mode":0o644,
            }));
        members.insert(format!("fixture-0.1.0/{name}"), (b"a".to_vec(), 0o644));
    }
    fs::write(
        output.join(dist::MANIFEST),
        util::encode(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        dist::verify_tree(&output, None)
            .unwrap_err()
            .to_string()
            .contains("Duplicate/reserved")
    );
    replace(&archive, &members);
    assert!(
        dist::read_archive(&archive)
            .unwrap_err()
            .to_string()
            .contains("colliding archive path")
    );
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

#[test]
fn compressed_bombs_are_bounded_before_tar_parsing_and_links_or_declared_payloads_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("bomb.tar.gz");
    let mut encoder = flate2::write::GzEncoder::new(
        fs::File::create(&path).unwrap(),
        flate2::Compression::default(),
    );
    for _ in 0..128 {
        encoder.write_all(&[0; 4096]).unwrap();
    }
    encoder.finish().unwrap();
    let error = dist::read_archive_with_limit(&path, 4096).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Expanded distribution exceeds its byte budget")
    );
    for linked in [true, false] {
        let mut header = tar::Header::new_gnu();
        header.set_path("fixture-0.1.0/payload").unwrap();
        header.set_mode(0o644);
        if linked {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("../../private").unwrap();
            header.set_size(0);
        } else {
            header.set_size(dist::MAX_BYTES + dist::MAX_MANIFEST + 1);
        }
        header.set_cksum();
        let mut raw = header.as_bytes().to_vec();
        raw.extend([0; 1024]);
        let mut encoder = flate2::write::GzEncoder::new(
            fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        encoder.write_all(&raw).unwrap();
        encoder.finish().unwrap();
        let output = temporary.path().join("must-not-exist");
        let error = dist::verify(&path, Some(&output)).unwrap_err();
        assert!(
            error.to_string().contains(if linked {
                "regular files"
            } else {
                "byte budget"
            }),
            "{error:#}"
        );
        assert!(!output.exists());
    }
}
#[test]
fn bundled_mpl_corresponding_source_is_the_complete_exact_locked_registry_archive() {
    let root = util::root();
    let provenance: Value =
        util::json(&root.join("resources/licenses/source-provenance.json")).unwrap();
    let lock: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("Cargo.lock")).unwrap()).unwrap();
    let packages = serde_json::to_value(&lock["package"]).unwrap();
    for item in provenance["sources"].as_array().unwrap() {
        let matches = packages
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| {
                p["name"] == item["name"]
                    && p["version"] == item["version"]
                    && p["source"] == item["registry"]
            })
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1);
        let archive = root
            .join("resources/licenses")
            .join(util::safe_path(item["archive"].as_str().unwrap()).unwrap());
        assert_eq!(util::file_digest(&archive).unwrap(), matches[0]["checksum"]);
        assert_eq!(item["sha256"], matches[0]["checksum"]);
        assert_eq!(item["modified"], false);
        let mut files = std::collections::BTreeMap::new();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(
            fs::File::open(archive).unwrap(),
        ));
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            if entry.header().entry_type().is_file() {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                files.insert(name, bytes);
            }
        }
        let prefix = format!(
            "{}-{}/",
            item["name"].as_str().unwrap(),
            item["version"].as_str().unwrap()
        );
        for member in ["src/lib.rs", "Cargo.toml", "LICENSE.txt"] {
            assert!(files.contains_key(&format!("{prefix}{member}")));
        }
        let manifest: toml::Value =
            toml::from_str(std::str::from_utf8(&files[&format!("{prefix}Cargo.toml")]).unwrap())
                .unwrap();
        assert_eq!(
            serde_json::to_value(&manifest["package"]["license"]).unwrap(),
            item["license"]
        );
        let original = &files[&format!("{prefix}LICENSE.txt")];
        let license = root
            .join("resources/licenses")
            .join(util::safe_path(item["license_file"].as_str().unwrap()).unwrap());
        assert_eq!(fs::read(license).unwrap(), *original);
        assert_eq!(util::digest(original), item["license_sha256"]);
    }
}
