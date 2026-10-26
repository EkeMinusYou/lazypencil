//! Original notices for the reviewed CPU/textlayout C++ wrapper inputs.
use crate::util;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

pub const REVIEW: &str = "resources/licenses/native/skia-wrapper-review.json";
const INPUTS: &[&str] = &[
    "LICENSE",
    "build.rs",
    "build_support/features.rs",
    "build_support/skia_bindgen.rs",
    "src/bindings.cpp",
    "src/bindings.h",
    "src/paragraph.cpp",
    "src/shaper.cpp",
];

pub fn build(root: &Path, features: &[String], stdout: &[u8]) -> Result<(Value, super::Files)> {
    ensure!(
        stdout.len() <= 16 * 1024 * 1024,
        "Unbounded wrapper build output"
    );
    let definition = util::read(&root.join(REVIEW), 64 * 1024)?;
    let review: Value = serde_json::from_slice(&definition)?;
    ensure!(
        review["format_version"] == 1
            && review["package"]
                == json!({"name":"skia-bindings","version":"0.153.3","license":"MIT"}),
        "Unreviewed wrapper package"
    );
    let required: BTreeSet<String> = serde_json::from_value(review["required_features"].clone())?;
    let optional: BTreeSet<String> = serde_json::from_value(review["optional_features"].clone())?;
    let selected = features.iter().cloned().collect::<BTreeSet<_>>();
    ensure!(
        selected.len() == features.len()
            && required.is_subset(&selected)
            && selected.is_subset(&required.union(&optional).cloned().collect()),
        "Review newly selected Skia wrapper features"
    );
    let package = root.join("vendor/skia-bindings");
    let manifest: toml::Value = toml::from_str(&String::from_utf8(util::read(
        &package.join("Cargo.toml"),
        util::MAX_FILE,
    )?)?)?;
    for field in ["name", "version", "license"] {
        ensure!(
            manifest["package"][field].as_str() == review["package"][field].as_str(),
            "Wrapper manifest differs from review"
        );
    }
    let units: Vec<String> = serde_json::from_value(review["compiled_sources"].clone())?;
    ensure!(
        units == ["src/bindings.cpp", "src/shaper.cpp", "src/paragraph.cpp"],
        "Unreviewed wrapper source selection"
    );
    let observed = std::str::from_utf8(stdout)?
        .lines()
        .filter_map(|line| line.strip_prefix("COMPILING BINDINGS: "))
        .map(serde_json::from_str::<Vec<String>>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        observed.len() == 1 && observed.first() == Some(&units),
        "Wrapper build output does not establish reviewed compilation inputs"
    );
    let mut inputs = Vec::new();
    let mut paths = BTreeSet::new();
    for pin in review["files"]
        .as_array()
        .context("Missing reviewed wrapper inputs")?
    {
        let path = pin["path"].as_str().context("Missing wrapper input path")?;
        ensure!(
            INPUTS.contains(&path) && paths.insert(path),
            "Unreviewed or duplicate wrapper input"
        );
        let bytes = util::read(&package.join(path), util::MAX_FILE)?;
        ensure!(
            pin["bytes"] == bytes.len() && pin["sha256"] == util::digest(&bytes),
            "Wrapper source changed; review original attribution: {path}"
        );
        if path.ends_with(".cpp") || path.ends_with(".h") {
            ensure!(
                super::source_comments(path, std::str::from_utf8(&bytes)?)?.is_empty(),
                "Review new wrapper source notices: {path}"
            );
        }
        inputs.push(pin.clone());
    }
    ensure!(
        paths == INPUTS.iter().copied().collect(),
        "Missing reviewed wrapper input"
    );
    let license = util::read(&package.join("LICENSE"), util::MAX_FILE)?;
    let index = json!({"format_version":1,"package":review["package"],"scope":review["scope"],"definition":{"path":REVIEW,"sha256":util::digest(&definition)},"features":selected,"compiled_sources":units,"inputs":inputs,"source_comments":[],"build_output_sha256":util::digest(stdout)});
    let mut files = super::Files::new();
    files.insert("LICENSE.original".into(), license.clone());
    files.insert("review.json".into(), definition);
    files.insert("build-output.log".into(), stdout.to_vec());
    files.insert("inventory.json".into(), util::encode(&index)?);
    let mut notice = format!(
        "skia-bindings 0.153.3 C++ wrapper\n\n{}\n\nOriginal package license (unmodified):\n\n",
        review["scope"]
            .as_str()
            .context("Missing wrapper review scope")?
    )
    .into_bytes();
    notice.extend(license);
    files.insert("WRAPPER-NOTICES.txt".into(), notice);
    Ok((index, files))
}

pub fn collect(root: &Path, features: &[String], stdout: &[u8], output: &Path) -> Result<Value> {
    let (index, files) = build(root, features, stdout)?;
    super::publish_bundle(output, &files)?;
    super::verify_expected(output, &files)?;
    Ok(
        json!({"compiled_sources":index["compiled_sources"],"inputs":index["inputs"],"definition":index["definition"],"features":index["features"],"build_output_sha256":index["build_output_sha256"],"review_scope":index["scope"]}),
    )
}
