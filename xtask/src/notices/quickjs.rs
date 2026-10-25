use super::{Files, blob};
use crate::util;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

pub const VERSION: &str = "0.14.0";
pub const ARCHIVE_SHA: &str = "cee271d0eeba64f0915b846cb7ae02e16faf3dfdffdca91731101d9d30fe3423";
pub const BUILD_SHA: &str = "d392d7194ae5bcaa0e0dac28a14c55656f6cf2cce3b16a8e6968c09e2ecfff85";
const SOURCES: &[&str] = &["libregexp.c", "libunicode.c", "quickjs.c", "dtoa.c"];
const HEADERS: &[&str] = &[
    "builtin-array-fromasync.h",
    "builtin-iterator-zip-keyed.h",
    "builtin-iterator-zip.h",
    "cutils.h",
    "dtoa.h",
    "libregexp-opcode.h",
    "libregexp.h",
    "libunicode-table.h",
    "libunicode.h",
    "list.h",
    "quickjs-atom.h",
    "quickjs-opcode.h",
    "quickjs-c-atomics.h",
    "quickjs.h",
];

pub fn copied_inputs() -> impl Iterator<Item = (String, String)> {
    SOURCES
        .iter()
        .chain(HEADERS)
        .map(|name| (format!("quickjs/{name}"), (*name).to_owned()))
        .chain(std::iter::once((
            "quickjs.bind.h".into(),
            "quickjs.bind.h".into(),
        )))
}

fn original(source: &Path, name: &str, hashes: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    util::safe_path(name)?;
    let path = source.join(name);
    ensure!(
        path.canonicalize()?.starts_with(source.canonicalize()?),
        "QuickJS input escapes selected root"
    );
    let bytes = util::read(&path, 32 * 1024 * 1024)?;
    ensure!(
        hashes.get(name) == Some(&util::digest(&bytes)),
        "QuickJS input differs from registry archive: {name}"
    );
    Ok(bytes)
}

pub fn build(
    root: &Path,
    manifest: &Path,
    out: &Path,
    features: &[String],
    cargo: &Path,
    target: &str,
) -> Result<(Value, Files)> {
    ensure!(
        [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc"
        ]
        .contains(&target),
        "Review new QuickJS native target"
    );
    ensure!(
        !features
            .iter()
            .any(|s| ["bindgen", "update-bindings"].contains(&s.as_str())),
        "Review generated QuickJS bindings for these features"
    );
    let source = manifest
        .parent()
        .context("Missing QuickJS source directory")?
        .canonicalize()?;
    let out = out.canonicalize()?;
    ensure!(
        out.is_dir() && !manifest.is_symlink(),
        "Expected regular QuickJS build inputs"
    );
    let (_, packages) = super::rust::locked(root)?;
    let packages = packages
        .values()
        .filter(|p| p["name"] == "rquickjs-sys")
        .collect::<Vec<_>>();
    ensure!(
        packages.len() == 1
            && packages[0]["version"] == VERSION
            && packages[0]["checksum"] == ARCHIVE_SHA,
        "Review new QuickJS package/build script"
    );
    let mut texts = Files::new();
    let mut hashes = BTreeMap::new();
    let (record, manifest_data) =
        super::rust::archive_record(packages[0], cargo, &mut texts, Some(&mut hashes))?;
    ensure!(
        hashes.get("build.rs").map(String::as_str) == Some(BUILD_SHA)
            && util::read(manifest, util::MAX_FILE)? == manifest_data,
        "Selected QuickJS manifest/build differs from reviewed archive"
    );
    original(&source, "build.rs", &hashes)?;
    let binding_path = format!("src/bindings/{target}.rs");
    let binding = original(&source, &binding_path, &hashes)?;
    let generated = util::read(&out.join("bindings.rs"), util::MAX_FILE)?;
    let pattern = regex::Regex::new(
        r#"^\s*macro_rules!\s+bindings_env\s*\{\s*\("TARGET"\)\s*=>\s*\{\s*"([A-Za-z0-9_.-]+)"\s*\};\s*\}\s*$"#,
    )?;
    ensure!(
        pattern
            .captures(std::str::from_utf8(&generated)?)
            .is_some_and(|m| &m[1] == target),
        "Generated target binding disagrees with observed compiler host"
    );
    let mut inputs = Vec::new();
    let mut comments = Vec::new();
    for (original_name, name) in copied_inputs() {
        let bytes = original(&source, &original_name, &hashes)?;
        ensure!(
            util::read(&out.join(&name), 32 * 1024 * 1024)? == bytes,
            "Copied QuickJS input changed: {name}"
        );
        inputs.push(json!({"source":original_name,"output":name,"sha256":util::digest(&bytes),"bytes":bytes.len(),"role":if SOURCES.contains(&name.as_str()){"compiled_source"}else{"copied_header"}}));
        comments.extend(super::source_comments(
            &original_name,
            std::str::from_utf8(&bytes)?,
        )?);
    }
    let legal = record["files"]
        .as_array()
        .context("Missing QuickJS notices")?
        .iter()
        .filter(|v| v["path"] == "quickjs/LICENSE")
        .cloned()
        .collect::<Vec<_>>();
    ensure!(legal.len() == 1, "Missing original QuickJS license");
    let name = legal[0]["blob"]
        .as_str()
        .context("Missing QuickJS license blob")?;
    let mut used = Files::new();
    blob(
        texts
            .get(name)
            .context("Missing original license bytes")?
            .clone(),
        &mut used,
    );
    let mut package = json!({});
    for key in [
        "name",
        "version",
        "source",
        "archive_sha256",
        "manifest_sha256",
        "repository",
        "vcs",
    ] {
        package[key] = record[key].clone();
    }
    let mut features = features.to_vec();
    features.sort();
    Ok((
        json!({"format_version":1,"scope":"Copied QuickJS native inputs selected by a reviewed build script; final linkage and complete attribution review are not established.",
        "package":package,"build_script_sha256":BUILD_SHA,"target":target,"features":features,"inputs":inputs,"source_comments":comments,"notice_files":legal,
        "rust_binding":{"path":binding_path,"sha256":util::digest(&binding),"bytes":binding.len()},"generated_target_binding":{"path":"bindings.rs","sha256":util::digest(&generated),"bytes":generated.len()},
        "limits":["Copied headers may contain unused declarations; system and compiler headers are outside this inventory.","Comment candidates cover the complete selected C/header files; they are not automatic license classifications.","Archive hashes establish content integrity; build authentication and final binary linkage require separate evidence."]}),
        used,
    ))
}

pub fn files(index: &Value, mut texts: Files) -> Result<Files> {
    let string = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let mut lines = vec![
        "QuickJS native build notice inventory".into(),
        "".into(),
        string(&index["scope"]),
        "".into(),
        format!(
            "Registry archive SHA-256: {}",
            string(&index["package"]["archive_sha256"])
        ),
        format!("Observed target: {}", string(&index["target"])),
        "".into(),
    ];
    for record in index["notice_files"].as_array().unwrap() {
        lines.extend([
            "=".repeat(72),
            string(&record["path"]),
            "=".repeat(72),
            std::str::from_utf8(
                texts
                    .get(record["blob"].as_str().unwrap())
                    .context("Missing native notice")?,
            )?
            .into(),
            "".into(),
        ]);
    }
    lines.extend([
        "Source comment candidates (original locations; not license classifications)".into(),
        "".into(),
    ]);
    for record in index["source_comments"].as_array().unwrap() {
        lines.extend([
            format!("{}:{}", string(&record["path"]), record["line"]),
            string(&record["text"]),
            "".into(),
        ]);
    }
    texts.insert("inventory.json".into(), util::encode(index)?);
    texts.insert(
        "QUICKJS-NOTICES.txt".into(),
        (lines.join("\n") + "\n").into_bytes(),
    );
    Ok(texts)
}

pub fn collect(
    root: &Path,
    manifest: &Path,
    out: &Path,
    features: &[String],
    cargo: &Path,
    target: &str,
    output: &Path,
) -> Result<Value> {
    let (index, texts) = build(root, manifest, out, features, cargo, target)?;
    let result = json!({"native_inputs":index["inputs"].as_array().unwrap().len(),"compiled_sources":4,"source_comments":index["source_comments"].as_array().unwrap().len(),"registry_archive_verified":true,"target":target});
    super::publish_bundle(output, &files(&index, texts)?)?;
    Ok(result)
}

pub fn verify(
    root: &Path,
    manifest: &Path,
    out: &Path,
    features: &[String],
    cargo: &Path,
    target: &str,
    output: &Path,
) -> Result<Value> {
    let (index, texts) = build(root, manifest, out, features, cargo, target)?;
    let expected = files(&index, texts)?;
    super::verify_expected(output, &expected)?;
    Ok(
        json!({"verified_files":expected.len()+1,"native_inputs":index["inputs"].as_array().unwrap().len()}),
    )
}
