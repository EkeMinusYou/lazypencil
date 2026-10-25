use lazypencil_xtask::{
    dist::{self, binary},
    util,
};
use serde_json::{Value, json};
use std::{fs, io::Cursor, path::Path};

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NOTICES: &[&str] = &[
    "licenses/rust/RUST-NOTICES.txt",
    "licenses/shader-preprocessor/packages/RUST-NOTICES.txt",
    "licenses/shader-preprocessor/toolchain/inventory/TOOLCHAIN-NOTICES.txt",
    "quickjs/QUICKJS-NOTICES.txt",
    "skia/SKIA-NOTICES.txt",
    "assets/ASSET-NOTICES.txt",
    "toolchain/inventory/TOOLCHAIN-NOTICES.txt",
];

fn archive(path: &Path, files: &dist::Files) {
    let compressed = flate2::write::GzEncoder::new(
        fs::File::create(path).unwrap(),
        flate2::Compression::default(),
    );
    let mut tar = tar::Builder::new(compressed);
    for (name, (bytes, mode)) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_mtime(0);
        header.set_cksum();
        tar.append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
}

fn index(root: &Path) {
    let files=util::walk(root).unwrap().into_iter().filter(|path|path.file_name().unwrap()!="manifest.json")
        .map(|path|json!({"path":path.strip_prefix(root).unwrap().to_str().unwrap().replace('\\',"/"),"bytes":fs::metadata(&path).unwrap().len(),"sha256":util::file_digest(&path).unwrap()})).collect::<Vec<_>>();
    fs::write(
        root.join("manifest.json"),
        util::encode(&json!({"format_version":1,"files":files})).unwrap(),
    )
    .unwrap();
}

fn fixture(target: &str) -> (tempfile::TempDir, binary::Arguments) {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("source");
    fs::create_dir_all(root.join("src")).unwrap();
    for (name, bytes) in [
        (
            "Cargo.toml",
            "[package]\nname='lazypencil'\nversion='0.1.0-dev'\n",
        ),
        ("Cargo.lock", "version=4\n"),
        ("README.md", "Original fixture source\n"),
        ("LICENSE", "MIT License\nCopyright Fixture\n"),
        ("src/main.rs", "fn main() {}\n"),
    ] {
        fs::write(root.join(name), bytes).unwrap();
    }
    let source_archive = temp.path().join("source.tar.gz");
    dist::source(&root, &source_archive, 0, true).unwrap();
    let mut source_files = dist::read_archive(&source_archive).unwrap();
    let manifest = "lazypencil-0.1.0-dev/SOURCE-MANIFEST.json";
    let mut source: Value = serde_json::from_slice(&source_files[manifest].0).unwrap();
    source["git"] = json!({"revision":REVISION,"dirty":false});
    source_files.get_mut(manifest).unwrap().0 = util::encode(&source).unwrap();
    archive(&source_archive, &source_files);
    let evidence = temp.path().join("evidence");
    fs::create_dir(&evidence).unwrap();
    let mut bytes = vec![0; 64];
    if target == "aarch64-apple-darwin" {
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        bytes[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
    } else {
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        bytes[18..20].copy_from_slice(
            &if target.starts_with("aarch64") {
                183u16
            } else {
                62u16
            }
            .to_le_bytes(),
        );
    }
    let executable = temp.path().join("lazypencil");
    fs::write(&executable, &bytes).unwrap();
    let linkage = json!({"host":target,"binary_sha256":util::digest(&bytes),"declarations":{"format":"fixture"}});
    let map = b"Original fixture linker map\n";
    let build = json!({"format_version":1,"release_ready":false,"remaining_review":["Fixture review remains"],
        "compiler":format!("rustc fixture\nhost: {target}\n"),
        "source":{"revision":REVISION,"manifest_sha256":util::digest(&source_files[manifest].0),"package":source["package"]},
        "binary":{"sha256":util::digest(&bytes),"bytes":bytes.len()},"dynamic_linkage":linkage,
        "static_link_map":{"sha256":util::digest(map),"bytes":map.len()}});
    for (name, bytes) in [
        ("build.json", util::encode(&build).unwrap()),
        ("SOURCE-MANIFEST.json", source_files[manifest].0.clone()),
        ("LICENSE", fs::read(root.join("LICENSE")).unwrap()),
        ("linkage/linkage.json", util::encode(&linkage).unwrap()),
        ("link-map.map", map.to_vec()),
    ] {
        let path = evidence.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    for name in NOTICES {
        let path = evidence.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(
                "Original fixture copyright and terms for {name}.\nPreserve this complete notice.\n"
            ),
        )
        .unwrap();
    }
    index(&evidence);
    let output = temp.path().join("binary.tar.gz");
    (
        temp,
        binary::Arguments {
            executable,
            source_archive,
            evidence,
            output,
        },
    )
}

#[test]
fn candidates_round_trip_all_initial_targets_keep_original_terms_source_and_permissions() {
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "aarch64-apple-darwin",
    ] {
        let (temp, mut arguments) = fixture(target);
        let first = binary::create(&arguments).unwrap();
        let output = temp.path().join("installed");
        let verified = binary::verify(&arguments.output, Some(&output)).unwrap();
        assert_eq!(verified["source_revision"], REVISION);
        assert_eq!(verified["target"], target);
        assert_eq!(verified["release_ready"], false);
        assert_eq!(
            fs::read(output.join("bin/lazypencil")).unwrap(),
            fs::read(&arguments.executable).unwrap()
        );
        assert_eq!(
            fs::read(output.join("source/lazypencil-source.tar.gz")).unwrap(),
            fs::read(&arguments.source_archive).unwrap()
        );
        for name in NOTICES {
            assert_eq!(
                fs::read(output.join("build-evidence").join(name)).unwrap(),
                fs::read(arguments.evidence.join(name)).unwrap()
            );
        }
        assert_eq!(util::mode(&output.join("bin/lazypencil")).unwrap(), 0o755);
        assert_eq!(util::mode(&output.join("LICENSE")).unwrap(), 0o644);
        assert!(binary::verify(&arguments.output, Some(&output)).is_err());
        assert!(binary::create(&arguments).is_err());
        arguments.output = temp.path().join("second.tar.gz");
        let second = binary::create(&arguments).unwrap();
        assert_eq!(
            first["verification"]["sha256"],
            second["verification"]["sha256"]
        );
    }
}

#[test]
fn candidates_reject_changed_sources_binaries_missing_notices_and_self_consistent_wrong_bindings() {
    for case in [
        "source",
        "binary",
        "target",
        "map",
        "notice",
        "missing-notice",
        "unlisted",
    ] {
        let (_temp, arguments) = fixture("x86_64-unknown-linux-gnu");
        let build_path = arguments.evidence.join("build.json");
        let mut build: Value = util::json(&build_path).unwrap();
        match case {
            "source" => build["source"]["revision"] = json!("b".repeat(40)),
            "binary" => {
                let mut bytes = fs::read(&arguments.executable).unwrap();
                bytes[40] = 1;
                fs::write(&arguments.executable, bytes).unwrap();
            }
            "target" => {
                build["compiler"] = json!("rustc fixture\nhost: aarch64-unknown-linux-gnu\n")
            }
            "map" => {
                fs::write(arguments.evidence.join("link-map.map"), "Different map").unwrap();
            }
            "notice" => {
                fs::write(arguments.evidence.join(NOTICES[0]), "Changed notice").unwrap();
            }
            "missing-notice" => {
                fs::remove_file(arguments.evidence.join(NOTICES[0])).unwrap();
            }
            "unlisted" => {
                fs::write(arguments.evidence.join("extra"), "Unlisted").unwrap();
            }
            _ => unreachable!(),
        }
        fs::write(build_path, util::encode(&build).unwrap()).unwrap();
        // Cross-file checks must reject substitutions even with a rebuilt index.
        if !["notice", "unlisted"].contains(&case) {
            index(&arguments.evidence);
        }
        assert!(binary::create(&arguments).is_err(), "{case}");
        assert!(
            !arguments.output.exists(),
            "Invalid candidate was published: {case}"
        );
    }
}

#[test]
fn damaged_archives_and_release_status_substitution_never_publish_an_extraction() {
    for case in ["binary", "unlisted", "release-ready"] {
        let (temp, arguments) = fixture("aarch64-apple-darwin");
        binary::create(&arguments).unwrap();
        let mut files = dist::read_archive(&arguments.output).unwrap();
        let prefix = "lazypencil-0.1.0-dev-aarch64-apple-darwin";
        match case {
            "binary" => {
                files
                    .get_mut(&format!("{prefix}/bin/lazypencil"))
                    .unwrap()
                    .0[40] = 1;
            }
            "unlisted" => {
                files.insert(format!("{prefix}/extra.txt"), (b"unlisted".to_vec(), 0o644));
            }
            "release-ready" => {
                let name = format!("{prefix}/PACKAGE-MANIFEST.json");
                let mut manifest: Value = serde_json::from_slice(&files[&name].0).unwrap();
                manifest["release_ready"] = json!(true);
                files.get_mut(&name).unwrap().0 = util::encode(&manifest).unwrap();
            }
            _ => unreachable!(),
        }
        archive(&arguments.output, &files);
        let output = temp.path().join("invalid");
        assert!(
            binary::verify(&arguments.output, Some(&output)).is_err(),
            "{case}"
        );
        assert!(!output.exists());
    }
}
