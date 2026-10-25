use flate2::read::GzDecoder;
use lazypencil_xtask::{
    notices::{self, quickjs},
    util,
};
use serde_json::Value;
use std::{collections::BTreeSet, fs, io::Read, path::PathBuf};

const TARGET: &str = "aarch64-apple-darwin";
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    out: PathBuf,
    cargo: PathBuf,
    archive: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let source = root.join("source");
        let out = root.join("out");
        let cargo = root.join("cargo");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir(&out).unwrap();
        let review = root.join(quickjs::REVIEW);
        fs::create_dir_all(review.parent().unwrap()).unwrap();
        fs::copy(util::root().join(quickjs::REVIEW), review).unwrap();
        let archive = cargo.join("registry/cache/pinned/rquickjs-sys-0.14.0.crate");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::copy(
            util::root().join("xtask/tests/fixtures/quickjs/rquickjs-sys-0.14.0.crate"),
            &archive,
        )
        .unwrap();
        assert_eq!(util::file_digest(&archive).unwrap(), quickjs::ARCHIVE_SHA);
        let selected = quickjs::copied_inputs()
            .map(|(a, _)| a)
            .chain([
                "Cargo.toml".into(),
                "build.rs".into(),
                format!("src/bindings/{TARGET}.rs"),
            ])
            .collect::<BTreeSet<_>>();
        let mut tar = tar::Archive::new(GzDecoder::new(fs::File::open(&archive).unwrap()));
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap();
            let name = path
                .to_str()
                .unwrap()
                .strip_prefix("rquickjs-sys-0.14.0/")
                .unwrap_or("");
            if !selected.contains(name) {
                continue;
            }
            let path = source.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            fs::write(path, bytes).unwrap();
        }
        for (original, copied) in quickjs::copied_inputs() {
            fs::copy(source.join(original), out.join(copied)).unwrap();
        }
        fs::write(
            out.join("bindings.rs"),
            format!("macro_rules! bindings_env {{ (\"TARGET\") => {{ \"{TARGET}\" }}; }}"),
        )
        .unwrap();
        fs::write(root.join("Cargo.lock"),format!("version=4\n[[package]]\nname='rquickjs-sys'\nversion='0.14.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\n",quickjs::ARCHIVE_SHA)).unwrap();
        Self {
            _temp: temp,
            root,
            source,
            out,
            cargo,
            archive,
        }
    }
    fn build(&self) -> anyhow::Result<(Value, notices::Files)> {
        quickjs::build(
            &self.root,
            &self.source.join("Cargo.toml"),
            &self.out,
            &["std".into()],
            &self.cargo,
            TARGET,
        )
    }
}

#[test]
fn original_archive_binds_every_copied_input_and_retains_notices_separately_from_code() {
    let f = Fixture::new();
    let (index, texts) = f.build().unwrap();
    assert_eq!(index["inputs"].as_array().unwrap().len(), 19);
    assert_eq!(
        index["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["role"] == "compiled_source")
            .count(),
        4
    );
    assert_eq!(index["package"]["archive_sha256"], quickjs::ARCHIVE_SHA);
    let review = &index["source_notice_review"];
    assert_eq!(review["comments"].as_array().unwrap().len(), 15);
    assert_eq!(
        review["comments"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["license"] == "Unicode-3.0")
            .count(),
        1
    );
    assert!(
        review["comments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["path"] == "quickjs/libunicode-table.h" && r["license"] == "Unicode-3.0")
    );
    let files = quickjs::files(&index, texts).unwrap();
    let text = String::from_utf8(files["QUICKJS-NOTICES.txt"].clone()).unwrap();
    assert!(text.contains("Copyright") && text.contains("Permission is hereby granted"));
    assert!(!text.contains("JS_NewRuntime(void) {"));
    assert!(
        text.contains("UNICODE LICENSE V3") && text.contains("Copyright © 1991-2026 Unicode, Inc.")
    );
    let late = format!(
        "{}\n/* Copyright Late Holder. License: retained in full. */\n",
        " ".repeat(70000)
    );
    assert!(
        notices::source_comments("quickjs/late.c", &late)
            .unwrap()
            .iter()
            .any(|v| v["text"].as_str().unwrap().contains("Late Holder"))
    );
}

#[test]
fn notice_review_rejects_changed_missing_duplicate_and_unknown_originals() {
    let f = Fixture::new();
    let (index, _) = f.build().unwrap();
    let comments = index["source_comments"].as_array().unwrap();
    let definition: Value = util::json(&f.root.join(quickjs::REVIEW)).unwrap();
    for mode in ["changed", "missing", "duplicate", "unknown", "moved"] {
        let mut changed = comments.clone();
        match mode {
            "changed" => changed[0]["text"] = serde_json::json!("substituted permission notice"),
            "missing" => {
                changed.pop();
            }
            "duplicate" => changed[1] = changed[0].clone(),
            "unknown" => changed[0]["path"] = serde_json::json!("quickjs/new-source.c"),
            "moved" => changed[0]["line"] = serde_json::json!(2),
            _ => unreachable!(),
        }
        assert!(quickjs::review(&changed, &definition).is_err(), "{mode}");
    }
    let mut other = definition;
    other["package"]["archive_sha256"] = serde_json::json!("0".repeat(64));
    assert!(quickjs::review(comments, &other).is_err());
}

#[test]
fn changed_copied_source_original_manifest_build_script_and_archive_are_rejected() {
    let f = Fixture::new();
    for path in [
        f.out.join("quickjs.h"),
        f.source.join("quickjs/dtoa.c"),
        f.source.join("Cargo.toml"),
        f.source.join("build.rs"),
        f.archive.clone(),
    ] {
        let original = fs::read(&path).unwrap();
        let mut changed = original.clone();
        changed.extend(b"modified");
        fs::write(&path, changed).unwrap();
        assert!(f.build().is_err(), "{}", path.display());
        fs::write(path, original).unwrap();
    }
}

#[test]
fn only_reviewed_target_bindings_features_and_generated_code_are_accepted() {
    let f = Fixture::new();
    for features in [
        vec!["std".into(), "bindgen".into()],
        vec!["std".into(), "update-bindings".into()],
    ] {
        assert!(
            quickjs::build(
                &f.root,
                &f.source.join("Cargo.toml"),
                &f.out,
                &features,
                &f.cargo,
                TARGET
            )
            .is_err()
        );
    }
    assert!(
        quickjs::build(
            &f.root,
            &f.source.join("Cargo.toml"),
            &f.out,
            &["std".into()],
            &f.cargo,
            "wasm32-unknown-unknown"
        )
        .is_err()
    );
    for code in [
        "macro_rules! bindings_env { (\"TARGET\") => { \"x86_64-apple-darwin\" }; }".to_owned(),
        format!(
            "{} fn unexpected() {{}}",
            fs::read_to_string(f.out.join("bindings.rs")).unwrap()
        ),
    ] {
        fs::write(f.out.join("bindings.rs"), code).unwrap();
        assert!(f.build().is_err());
    }
}

#[test]
fn reconstructed_notices_detect_substitution_and_never_overwrite_existing_output() {
    let f = Fixture::new();
    let output = f.root.join("inventory");
    let args = (
        &f.root,
        f.source.join("Cargo.toml"),
        &f.out,
        vec!["std".into()],
        &f.cargo,
        TARGET,
    );
    quickjs::collect(args.0, &args.1, args.2, &args.3, args.4, args.5, &output).unwrap();
    quickjs::verify(args.0, &args.1, args.2, &args.3, args.4, args.5, &output).unwrap();
    assert!(quickjs::collect(args.0, &args.1, args.2, &args.3, args.4, args.5, &output).is_err());
    fs::write(output.join("QUICKJS-NOTICES.txt"), b"a substituted license").unwrap();
    assert!(quickjs::verify(args.0, &args.1, args.2, &args.3, args.4, args.5, &output).is_err());
}
