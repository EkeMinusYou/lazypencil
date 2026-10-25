use super::{Files, blob};
use crate::util;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

pub const VERSION: &str = "0.14.0";
pub const ARCHIVE_SHA: &str = "cee271d0eeba64f0915b846cb7ae02e16faf3dfdffdca91731101d9d30fe3423";
pub const BUILD_SHA: &str = "d392d7194ae5bcaa0e0dac28a14c55656f6cf2cce3b16a8e6968c09e2ecfff85";
pub const REVIEW: &str = "resources/licenses/native/quickjs-review.json";
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

pub fn review(comments: &[Value], definition: &Value) -> Result<Value> {
    ensure!(
        definition["format_version"] == 1
            && definition["package"]["name"] == "rquickjs-sys"
            && definition["package"]["version"] == VERSION
            && definition["package"]["archive_sha256"] == ARCHIVE_SHA,
        "QuickJS notice review belongs to another package"
    );
    let expected = definition["comments"]
        .as_array()
        .context("Missing reviewed comments")?;
    ensure!(
        expected.len() == 15 && comments.len() == expected.len(),
        "QuickJS notice membership changed; review original source comments"
    );
    let mut remaining = BTreeMap::new();
    for record in expected {
        let path = record["path"]
            .as_str()
            .context("Missing reviewed notice path")?;
        let line = record["line"]
            .as_u64()
            .filter(|n| *n > 0)
            .context("Missing reviewed notice line")?;
        let license = record["license"]
            .as_str()
            .context("Missing reviewed license")?;
        ensure!(
            ["MIT", "Unicode-3.0"].contains(&license)
                && definition["licenses"][license]["reference"].is_string()
                && remaining.insert((path, line), record).is_none(),
            "Invalid or duplicate reviewed QuickJS notice"
        );
    }
    let mut reviewed = Vec::new();
    for comment in comments {
        let path = comment["path"]
            .as_str()
            .context("Missing source notice path")?;
        let line = comment["line"]
            .as_u64()
            .context("Missing source notice line")?;
        let record = remaining
            .remove(&(path, line))
            .context("Unreviewed or duplicate QuickJS notice location")?;
        let text = comment["text"]
            .as_str()
            .context("Missing original notice text")?;
        ensure!(
            record["sha256"] == util::digest(text.as_bytes()),
            "QuickJS notice changed; review original text at {path}:{line}"
        );
        reviewed.push(record.clone());
    }
    ensure!(
        remaining.is_empty(),
        "Missing reviewed QuickJS source notice"
    );
    Ok(
        json!({"format_version":1,"package":definition["package"],"scope":definition["scope"],"licenses":definition["licenses"],"comments":reviewed}),
    )
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
    let review_bytes = util::read(&root.join(REVIEW), 64 * 1024)?;
    let mut notice_review = review(&comments, &serde_json::from_slice(&review_bytes)?)?;
    notice_review["definition"] =
        json!({"path":REVIEW,"sha256":util::digest(&review_bytes),"bytes":review_bytes.len()});
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
        "package":package,"build_script_sha256":BUILD_SHA,"target":target,"features":features,"inputs":inputs,"source_comments":comments,"source_notice_review":notice_review,"notice_files":legal,
        "rust_binding":{"path":binding_path,"sha256":util::digest(&binding),"bytes":binding.len()},"generated_target_binding":{"path":"bindings.rs","sha256":util::digest(&generated),"bytes":generated.len()},
        "limits":["Copied headers may contain unused declarations; system and compiler headers are outside this inventory.","All original source comments are retained and matched to a curated review of the pinned archive; changes require review.","Archive hashes establish content integrity; build authentication and final binary linkage require separate evidence."]}),
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
        "Reviewed source notices (original locations and complete original text)".into(),
        "".into(),
    ]);
    let reviews = index["source_notice_review"]["comments"]
        .as_array()
        .context("Missing source notice review")?;
    let comments = index["source_comments"]
        .as_array()
        .context("Missing original source notices")?;
    ensure!(
        reviews.len() == comments.len(),
        "QuickJS notice review is incomplete"
    );
    for (record, reviewed) in comments.iter().zip(reviews) {
        ensure!(
            record["path"] == reviewed["path"]
                && record["line"] == reviewed["line"]
                && reviewed["sha256"] == util::digest(string(&record["text"]).as_bytes()),
            "QuickJS original notice differs from review"
        );
        lines.extend([
            format!(
                "{}:{} [{}]",
                string(&record["path"]),
                record["line"],
                string(&reviewed["license"])
            ),
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
    let result = json!({"native_inputs":index["inputs"].as_array().unwrap().len(),"compiled_sources":4,"source_comments":index["source_comments"].as_array().unwrap().len(),"reviewed_source_notices":index["source_notice_review"]["comments"].as_array().unwrap().len(),"notice_review_definition":index["source_notice_review"]["definition"],"registry_archive_verified":true,"target":target});
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
