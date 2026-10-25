//! Original tiny source archives exercise provenance and publication without native builds.
use base64::{Engine, engine::general_purpose::STANDARD};
use lazypencil_xtask::{
    notices::{self, assets, rust, skia, toolchain},
    util,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: Value,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("fixtures/notices.json")).unwrap();
        let data = cases.into_iter().find(|v| v["name"] == name).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        for (name, bytes) in data["inputs"].as_object().unwrap() {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, STANDARD.decode(bytes.as_str().unwrap()).unwrap()).unwrap();
        }
        Self {
            _temp: temp,
            root,
            data,
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn json(&self, name: &str) -> Value {
        util::json(&self.path(name)).unwrap()
    }
    fn set(&self, name: &str, value: &Value) {
        self.write(name, util::encode(value).unwrap());
    }
    fn expected(&self) -> notices::Files {
        self.data["bundle"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(n, b)| (n.clone(), STANDARD.decode(b.as_str().unwrap()).unwrap()))
            .collect()
    }
    fn skia(&self) -> anyhow::Result<(Value, notices::Files)> {
        let (mut index, mut texts) = skia::build_from_graph(
            &self.path("source"),
            &self.path("build"),
            self.data["graph"].as_str().unwrap(),
            self.data["headers"].as_str().unwrap(),
        )?;
        if self.path("reference/upstream.json").exists() {
            skia::apply_reference(
                &mut index,
                &mut texts,
                &self.path("reference/upstream.json"),
            )?;
        }
        Ok((index, texts))
    }
    fn toolchain(&self) -> anyhow::Result<(Value, notices::Files)> {
        toolchain::build(
            &self.path("source"),
            &self.path("legal"),
            &self.path("docs"),
            &self.path("libs"),
            &self.data["identity"],
            self.path("cargo")
                .exists()
                .then(|| self.path("cargo"))
                .as_deref(),
            self.path("canonical/upstream.json")
                .exists()
                .then(|| self.path("canonical/upstream.json"))
                .as_deref(),
        )
    }
    fn checksum(&self) {
        let directory = self.path("source/library/vendor/dependency-1.0.0");
        let mut value = self.json("source/library/vendor/dependency-1.0.0/.cargo-checksum.json");
        value["files"] = json!(
            util::walk(&directory)
                .unwrap()
                .into_iter()
                .filter(|p| p.file_name().unwrap() != ".cargo-checksum.json")
                .map(|p| (
                    p.strip_prefix(&directory)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    util::file_digest(&p).unwrap()
                ))
                .collect::<BTreeMap<_, _>>()
        );
        self.set(
            "source/library/vendor/dependency-1.0.0/.cargo-checksum.json",
            &value,
        );
    }
}
#[test]
fn original_inventories_retain_every_notice_attribution_and_reference_byte() {
    for name in [
        "rust",
        "rust-crlf",
        "rust-custom",
        "assets",
        "skia",
        "skia-reference",
        "skia-crlf",
        "toolchain",
        "toolchain-registry",
        "toolchain-reference",
    ] {
        let f = Fixture::new(name);
        let (index, files) = if name.starts_with("rust") {
            let output = f.path("output");
            rust::collect(
                &f.root,
                &f.path("cargo"),
                &f.path("inputs/upstream.json"),
                &output,
            )
            .unwrap();
            rust::verify(&f.root, &output, Some(&f.path("cargo"))).unwrap();
            (
                util::json(&output.join("index.json")).unwrap(),
                notices::verify_bundle(&output, |_| true).unwrap(),
            )
        } else if name == "assets" {
            let (i, t) = assets::build(&f.root).unwrap();
            let files = assets::files(&i, t).unwrap();
            (i, files)
        } else if name.starts_with("skia") {
            let (i, t) = f.skia().unwrap();
            let files = skia::files(&i, t).unwrap();
            (i, files)
        } else {
            let (i, t) = f.toolchain().unwrap();
            let files = toolchain::files(&i, t).unwrap();
            (i, files)
        };
        assert_eq!(index, f.data["index"], "{name}");
        assert_eq!(
            files,
            f.expected(),
            "{name}: source notices and readable output must remain reproducible"
        );
        let output = f.path("published");
        notices::publish_bundle(&output, &files).unwrap();
        notices::verify_expected(&output, &files).unwrap();
        assert!(notices::publish_bundle(&output, &files).is_err());
    }
}
#[test]
fn every_bundle_rejects_changed_missing_and_unlisted_members() {
    for family in ["rust", "assets", "skia", "toolchain"] {
        for mutation in ["changed", "missing", "unlisted", "forged"] {
            let f = Fixture::new(family);
            let files = f.expected();
            let output = f.path("output");
            notices::publish_bundle(&output, &files).unwrap();
            let member = output.join(files.keys().next().unwrap());
            match mutation {
                "changed" => fs::write(member, b"changed original notice").unwrap(),
                "missing" => fs::remove_file(member).unwrap(),
                "unlisted" => fs::write(output.join("unlisted"), b"extra").unwrap(),
                _ => {
                    let mut index = f.json("output/manifest.json");
                    index["files"][0]["sha256"] = "0".repeat(64).into();
                    f.set("output/manifest.json", &index);
                }
            }
            assert!(
                notices::verify_expected(&output, &files).is_err(),
                "{family}/{mutation}"
            );
        }
    }
}
#[test]
fn rust_inventory_rejects_changed_archive_lock_local_notice_and_forged_metadata() {
    for mutation in ["archive", "lock", "manifest", "notice", "authors"] {
        let f = Fixture::new("rust");
        let output = f.path("output");
        rust::collect(
            &f.root,
            &f.path("cargo"),
            &f.path("inputs/upstream.json"),
            &output,
        )
        .unwrap();
        match mutation {
            "archive" => f.write(
                "cargo/registry/cache/index.crates.io-test/dependency-1.0.0.crate",
                b"corrupt",
            ),
            "lock" => {
                let mut bytes = fs::read(f.path("Cargo.lock")).unwrap();
                bytes.extend(b"# changed\n");
                f.write("Cargo.lock", bytes);
            }
            "manifest" => f.write(
                "Cargo.toml",
                b"[package]\nname='fixture'\nversion='0.1.0'\nlicense='changed'\n",
            ),
            "notice" => f.write("LICENSE", b"Different local notice\n"),
            _ => {
                let mut files = notices::verify_bundle(&output, |_| true).unwrap();
                let mut index: Value = serde_json::from_slice(&files["index.json"]).unwrap();
                index["packages"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|v| v["name"] == "dependency")
                    .unwrap()["authors"] = json!(["Not the published author"]);
                let texts = files
                    .iter()
                    .filter(|(k, _)| k.starts_with("texts/"))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                files.insert("index.json".into(), util::encode(&index).unwrap());
                files.insert(
                    "RUST-NOTICES.txt".into(),
                    rust::render(&index, &texts).unwrap(),
                );
                fs::remove_dir_all(&output).unwrap();
                notices::publish_bundle(&output, &files).unwrap();
                rust::verify(&f.root, &output, None).unwrap();
            }
        }
        assert!(
            rust::verify(&f.root, &output, Some(&f.path("cargo"))).is_err(),
            "{mutation}"
        );
    }
}
#[test]
fn asset_inclusions_payloads_provenance_and_derived_geometry_require_review() {
    for name in [
        "resources/fonts/NotoSansJP.ttf",
        "resources/fonts/OFL.txt",
        "resources/icons/lucide/icons.json",
        "resources/icons/Material Symbols Sharp/codepoints.txt",
        "resources/icons/phosphor/LICENSE",
        "resources/icons/phosphor/provenance.json",
    ] {
        let f = Fixture::new("assets");
        f.write(name, b"changed");
        assert!(
            assets::collect(&f.root, &f.path("output")).is_err(),
            "{name}"
        );
        assert!(!f.path("output").exists());
    }
    for mutation in ["missing", "duplicate", "extra", "computed", "bundled"] {
        let f = Fixture::new("assets");
        let path = "src/text.rs";
        let original = fs::read(f.path(path)).unwrap();
        match mutation {
            "missing" => f.write(path, b""),
            "duplicate" => f.write(path, [original.clone(), original].concat()),
            "extra" => f.write(
                path,
                [
                    original,
                    b"const EXTRA: &[u8]=include_bytes!(\"../resources/fonts/OFL.txt\");".to_vec(),
                ]
                .concat(),
            ),
            "computed" => f.write(
                path,
                [
                    original,
                    b"const OTHER: &str=include_str!(concat!(\"a\",\"b\"));".to_vec(),
                ]
                .concat(),
            ),
            _ => {
                let mut c = f.json("resources/icons/catalogue.json");
                c[0]["bundled"] = true.into();
                f.set("resources/icons/catalogue.json", &c);
            }
        }
        assert!(assets::build(&f.root).is_err(), "{mutation}");
    }
}
#[test]
fn skia_reference_binds_git_repository_revision_blob_deps_and_source_variants() {
    for pointer in [
        "/references/0/repository",
        "/references/0/revision",
        "/references/0/url",
        "/references/0/sha256",
        "/references/0/git_blob_sha1",
        "/references/0/path",
        "/references/0/applies_to/skia_deps_sha256",
        "/references/0/applies_to/source_sha256/third_party~1externals~1zlib~1native.h",
    ] {
        let f = Fixture::new("skia-reference");
        let mut policy = f.json("reference/upstream.json");
        *policy.pointer_mut(pointer).unwrap() = "invalid".into();
        f.set("reference/upstream.json", &policy);
        assert!(f.skia().is_err(), "{pointer}");
    }
    for field in ["transform", "sha256", "source_git_blob_sha1", "url"] {
        let f = Fixture::new("skia-crlf");
        let mut policy = f.json("reference/upstream.json");
        policy["references"][0]["applies_to"]["working_tree_variants"]["third_party/externals/zlib/native.h"]
            [field] = "invalid".into();
        f.set("reference/upstream.json", &policy);
        assert!(f.skia().is_err(), "{field}");
    }
    for mutation in [
        "content",
        "mixed",
        "bare-cr",
        "variant-origin",
        "extra-variant",
        "source",
        "deps",
        "terms",
        "malformed",
    ] {
        let f = Fixture::new("skia-crlf");
        let path = "source/third_party/externals/zlib/native.h";
        let original = fs::read_to_string(f.path(path)).unwrap();
        match mutation {
            "content" => f.write(path, original.replace("Header Holder", "Substitute Holder")),
            "mixed" => f.write(path, original.replacen("\r\n", "\n", 1)),
            "bare-cr" => f.write(path, original.replace("\r\n", "\r\r\n")),
            "source" => f.write(path, format!("{original}// Changed source\r\n")),
            "deps" => f.write("source/DEPS", b"Changed Skia revision"),
            "terms" => {
                let policy = f.json("reference/upstream.json");
                f.write(
                    &format!(
                        "reference/{}",
                        policy["references"][0]["blob"].as_str().unwrap()
                    ),
                    b"changed terms",
                );
            }
            "malformed" => f.write("reference/upstream.json", b"[]"),
            _ => {
                let mut policy = f.json("reference/upstream.json");
                let applies = &mut policy["references"][0]["applies_to"];
                if mutation == "variant-origin" {
                    applies["source_origin"]["revision"] = "b".repeat(40).into();
                } else {
                    let variants = applies["working_tree_variants"].as_object_mut().unwrap();
                    let value = variants
                        .remove("third_party/externals/zlib/native.h")
                        .unwrap();
                    variants.insert("third_party/externals/zlib/unreviewed.h".into(), value);
                }
                f.set("reference/upstream.json", &policy);
            }
        }
        assert!(f.skia().is_err(), "{mutation}");
    }
}
#[test]
fn native_graph_never_silently_drops_unknown_external_or_missing_recorded_inputs() {
    for mutation in [
        "unknown",
        "external",
        "missing-header",
        "no-headers",
        "no-library",
        "missing-build",
        "incomplete",
    ] {
        let mut f = Fixture::new("skia");
        match mutation {
            "unknown" => {
                f.data["headers"] = format!(
                    "{}    ../source/third_party/externals/unreviewed/header.h\n",
                    f.data["headers"].as_str().unwrap()
                )
                .into()
            }
            "external" => {
                f.data["graph"] = format!(
                    "{}../unreviewed/native.c\n",
                    f.data["graph"].as_str().unwrap()
                )
                .into()
            }
            "missing-header" => {
                f.data["headers"] = format!(
                    "{}    ../source/src/deleted.h\n",
                    f.data["headers"].as_str().unwrap()
                )
                .into()
            }
            "no-headers" => f.data["headers"] = "".into(),
            "no-library" => fs::remove_file(f.path("build/libnative.a")).unwrap(),
            "missing-build" => fs::remove_file(f.path("build/args.gn")).unwrap(),
            _ => {
                f.data["graph"] = "".into();
            }
        }
        assert!(f.skia().is_err(), "{mutation}");
    }
}
#[test]
fn toolchain_vendor_checksums_manifest_identity_and_library_evidence_are_required() {
    for mutation in [
        "changed-file",
        "missing-file",
        "manifest",
        "package-checksum",
        "malformed-files",
        "partial-vendor",
        "missing-lib",
        "missing-reference",
    ] {
        let f = Fixture::new("toolchain");
        let prefix = "source/library/vendor/dependency-1.0.0";
        match mutation {
            "changed-file" => f.write(&format!("{prefix}/LICENSE"), b"Changed source license"),
            "missing-file" => fs::remove_file(f.path(&format!("{prefix}/LICENSE"))).unwrap(),
            "manifest" => {
                let text = fs::read_to_string(f.path(&format!("{prefix}/Cargo.toml")))
                    .unwrap()
                    .replace("version=\"1.0.0\"", "version=\"2.0.0\"");
                f.write(&format!("{prefix}/Cargo.toml"), text);
                f.checksum();
            }
            "package-checksum" | "malformed-files" => {
                let name = format!("{prefix}/.cargo-checksum.json");
                let mut v = f.json(&name);
                if mutation == "package-checksum" {
                    v["package"] = "0".repeat(64).into();
                } else {
                    v["files"] = json!(["LICENSE"]);
                }
                f.set(&name, &v);
            }
            "partial-vendor" => {
                fs::remove_file(f.path(&format!("{prefix}/.cargo-checksum.json"))).unwrap()
            }
            "missing-lib" => fs::remove_file(f.path("libs/libstd-target.rlib")).unwrap(),
            _ => fs::remove_file(f.path("docs/licenses/MIT.txt")).unwrap(),
        }
        assert!(f.toolchain().is_err(), "{mutation}");
    }
}
#[test]
fn registry_originals_and_changed_installed_vendor_notices_are_retained_separately() {
    let f = Fixture::new("toolchain-registry");
    f.write(
        "source/library/vendor/dependency-1.0.0/LICENSE",
        b"Changed installed vendor notice",
    );
    f.checksum();
    let (index, texts) = f.toolchain().unwrap();
    assert_eq!(
        index["registry_packages"][0]["vendor_registry_comparison"]["matches"],
        false
    );
    assert_eq!(
        index["registry_packages"][0]["vendor_registry_comparison"]["changed_files"],
        json!(["LICENSE"])
    );
    assert_eq!(index["registry_archives_verified"], true);
    let files = toolchain::files(&index, texts).unwrap();
    let text = String::from_utf8_lossy(&files["TOOLCHAIN-NOTICES.txt"]);
    assert!(text.contains("Original Holder") && text.contains("Changed installed vendor notice"));
    fs::remove_dir_all(f.path("source/library/vendor")).unwrap();
    let (index, _) = f.toolchain().unwrap();
    assert_eq!(index["vendor_file_checksums_verified"], false);
    assert_eq!(index["registry_archives_verified"], true);
    let archive = f.path("cargo/registry/cache/test/dependency-1.0.0.crate");
    fs::write(&archive, b"corrupt").unwrap();
    assert!(f.toolchain().is_err());
    fs::remove_file(archive).unwrap();
    assert!(f.toolchain().is_err());
}
#[test]
fn missing_vendor_notices_report_findings_without_inventing_a_copyright_holder() {
    let f = Fixture::new("toolchain");
    fs::remove_file(f.path("source/library/vendor/dependency-1.0.0/LICENSE")).unwrap();
    f.checksum();
    let (index, texts) = f.toolchain().unwrap();
    assert!(
        index["review_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("No standalone notice"))
    );
    assert!(
        String::from_utf8_lossy(&toolchain::files(&index, texts).unwrap()["TOOLCHAIN-NOTICES.txt"])
            .contains("license=\"MIT\"")
    );
}
#[cfg(unix)]
#[test]
fn source_notice_and_bundle_symlinks_are_rejected_even_when_the_bytes_match() {
    use std::os::unix::fs::symlink;
    for (family, name) in [
        ("assets", "resources/fonts/NotoSansJP.ttf"),
        ("rust", "LICENSE"),
        ("skia", "source/third_party/externals/zlib/native.h"),
        ("skia", "source/LICENSE"),
        ("toolchain", "source/library/core.rs"),
    ] {
        let f = Fixture::new(family);
        let original = f.path(name);
        let target = f.path("external");
        fs::rename(&original, &target).unwrap();
        symlink(target, original).unwrap();
        let result = match family {
            "assets" => assets::build(&f.root).map(|_| ()),
            "rust" => rust::collect(
                &f.root,
                &f.path("cargo"),
                &f.path("inputs/upstream.json"),
                &f.path("output"),
            )
            .map(|_| ()),
            "skia" => f.skia().map(|_| ()),
            _ => f.toolchain().map(|_| ()),
        };
        assert!(result.is_err(), "{family}/{name}");
    }
    let f = Fixture::new("rust");
    let output = f.path("output");
    notices::publish_bundle(&output, &f.expected()).unwrap();
    fs::rename(output.join("texts"), f.path("external-texts")).unwrap();
    symlink(f.path("external-texts"), output.join("texts")).unwrap();
    assert!(rust::verify(&f.root, &output, None).is_err());
}
