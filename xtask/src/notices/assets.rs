use super::{Files, fingerprint};
use crate::util;
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const PACKS: &[&str] = &[
    "lucide",
    "feather",
    "phosphor",
    "Material Symbols Outlined",
    "Material Symbols Rounded",
    "Material Symbols Sharp",
];

fn expected_declarations() -> BTreeSet<(String, String)> {
    let mut out = [
        ("src/text.rs", "resources/fonts/NotoSansJP.ttf"),
        ("src/cli.rs", "resources/fonts/OFL.txt"),
        ("src/cli.rs", "resources/fonts/manifest.json"),
        ("src/icons/mod.rs", "resources/icons/catalogue.json"),
        ("src/icons/mod.rs", "resources/icons/lucide/icons.json"),
        (
            "src/icons/optional.rs",
            "resources/icons/phosphor/index.json",
        ),
    ]
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .into_iter()
    .collect::<BTreeSet<_>>();
    for name in PACKS {
        out.insert((
            if ["lucide", "feather"].contains(name) {
                "src/icons/mod.rs"
            } else {
                "src/icons/optional.rs"
            }
            .into(),
            format!("resources/icons/{name}/LICENSE"),
        ));
    }
    for style in ["Outlined", "Rounded", "Sharp"] {
        out.insert((
            "src/icons/optional.rs".into(),
            format!("resources/icons/Material Symbols {style}/codepoints.txt"),
        ));
    }
    out
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            c => result.push(c.as_os_str()),
        }
    }
    result
}

pub fn declarations(root: &Path) -> Result<BTreeSet<(String, String)>> {
    let root = root.canonicalize()?;
    let source = root.join("src");
    let call = Regex::new(r"include_(?:str|bytes)!\s*\(")?;
    let literal = Regex::new(r#"include_(?:str|bytes)!\s*\(\s*"([^"\\]+)"\s*\)"#)?;
    let paths = util::walk(&source)?;
    ensure!(
        paths.len() <= 20_000,
        "Declaration inventory exceeds entry limit"
    );
    let mut total = 0;
    let mut result = BTreeSet::new();
    for path in paths {
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let bytes = util::read(&path, util::MAX_FILE)?;
        total += bytes.len();
        ensure!(
            total <= 32 * 1024 * 1024,
            "Declaration inventory exceeds byte limit"
        );
        let body = std::str::from_utf8(&bytes)?;
        let matches = literal.captures_iter(body).collect::<Vec<_>>();
        ensure!(
            call.find_iter(body)
                .map(|m| m.start())
                .eq(matches.iter().map(|m| m.get(0).unwrap().start())),
            "Review computed/nonliteral include: {}",
            path.display()
        );
        for m in matches {
            let target = path.parent().unwrap().join(&m[1]);
            let actual = target.canonicalize()?;
            ensure!(
                actual.starts_with(&root) && actual == lexical(&target),
                "Embedded input aliases or escapes its source root"
            );
            let name = actual
                .strip_prefix(&root)?
                .to_str()
                .context("Invalid asset path")?;
            if name.starts_with("resources/fonts/") || name.starts_with("resources/icons/") {
                ensure!(
                    result.insert((
                        path.strip_prefix(&root)?
                            .to_str()
                            .context("Invalid source path")?
                            .into(),
                        name.into()
                    )),
                    "Repeated embedded asset declaration"
                );
            }
        }
    }
    ensure!(
        result == expected_declarations(),
        "Review changed embedded font/icon declarations"
    );
    Ok(result)
}

pub fn build(root: &Path) -> Result<(Value, Files)> {
    let declared = declarations(root)?;
    let embedded = declared
        .iter()
        .map(|(_, p)| p.as_str())
        .collect::<BTreeSet<_>>();
    let mut originals = Files::new();
    let mut inputs = BTreeMap::new();
    fn observe(
        root: &Path,
        name: &str,
        embedded: &BTreeSet<&str>,
        inputs: &mut BTreeMap<String, Value>,
    ) -> Result<Value> {
        let mut value = fingerprint(root, name, MAX_ASSET)?;
        value["embedded_declaration"] = json!(embedded.contains(name));
        inputs.insert(name.into(), value.clone());
        Ok(value)
    }
    fn retain(
        root: &Path,
        name: &str,
        embedded: &BTreeSet<&str>,
        inputs: &mut BTreeMap<String, Value>,
        originals: &mut Files,
    ) -> Result<Value> {
        let value = observe(root, name, embedded, inputs)?;
        originals.insert(name.into(), util::read(&root.join(name), util::MAX_FILE)?);
        Ok(value)
    }
    let font: Value = util::json(&root.join("resources/fonts/manifest.json"))?;
    let files = font["files"].as_array().context("Missing font members")?;
    ensure!(
        font["family"] == "Noto Sans JP"
            && font["license"] == "OFL-1.1"
            && files.len() == 3
            && files
                .iter()
                .filter_map(|v| v["file"].as_str())
                .collect::<BTreeSet<_>>()
                == ["NotoSansJP.ttf", "OFL.txt", "METADATA.pb"].into(),
        "Review bundled font identity"
    );
    for entry in files {
        let file = entry["file"].as_str().context("Missing font filename")?;
        let name = format!("resources/fonts/{file}");
        let value = observe(root, &name, &embedded, &mut inputs)?;
        ensure!(
            value["sha256"] == entry["sha256"] && value["bytes"] == entry["bytes"],
            "Font differs from its source manifest"
        );
        if file != "NotoSansJP.ttf" {
            retain(root, &name, &embedded, &mut inputs, &mut originals)?;
        }
    }
    for name in [
        "resources/fonts/manifest.json",
        "resources/icons/catalogue.json",
    ] {
        retain(root, name, &embedded, &mut inputs, &mut originals)?;
    }
    let catalogue: Value = serde_json::from_slice(&originals["resources/icons/catalogue.json"])?;
    let descriptors = catalogue.as_array().context("Missing icon catalogue")?;
    ensure!(
        descriptors.len() == PACKS.len()
            && descriptors
                .iter()
                .filter_map(|v| v["id"].as_str())
                .collect::<BTreeSet<_>>()
                == PACKS.iter().copied().collect()
            && descriptors
                .iter()
                .filter(|v| v["bundled"] == true)
                .filter_map(|v| v["id"].as_str())
                .collect::<BTreeSet<_>>()
                == ["lucide"].into(),
        "Review icon pack/bundled geometry coverage"
    );
    for descriptor in descriptors {
        let id = descriptor["id"].as_str().unwrap();
        let directory = format!("resources/icons/{id}/");
        ensure!(
            retain(
                root,
                &format!("{directory}LICENSE"),
                &embedded,
                &mut inputs,
                &mut originals
            )?["sha256"]
                == descriptor["license_sha256"],
            "Icon license differs from pin"
        );
        if id == "lucide" {
            ensure!(
                observe(
                    root,
                    &format!("{directory}icons.json"),
                    &embedded,
                    &mut inputs
                )?["sha256"]
                    == descriptor["data_sha256"],
                "Lucide geometry differs from pin"
            );
        } else if id != "feather" {
            let name = if id == "phosphor" {
                "index.json"
            } else {
                "codepoints.txt"
            };
            ensure!(
                observe(root, &format!("{directory}{name}"), &embedded, &mut inputs)?["sha256"]
                    == descriptor["index_sha256"],
                "Optional icon index differs from pin"
            );
            let name = format!("{directory}provenance.json");
            retain(root, &name, &embedded, &mut inputs, &mut originals)?;
            ensure!(
                serde_json::from_slice::<Value>(&originals[&name])?["descriptor"] == *descriptor,
                "Icon provenance descriptor differs from catalogue"
            );
        }
    }
    ensure!(
        embedded.iter().all(|name| inputs.contains_key(*name)),
        "Embedded asset lacks attribution coverage"
    );
    let index = json!({"format_version":1,"scope":"Reviewed source declarations and original font/icon notices; final binary linkage and complete legal review are not established.",
        "declarations":declared.iter().map(|(source,input)|json!({"source":source,"input":input})).collect::<Vec<_>>(),"inputs":inputs.into_values().collect::<Vec<_>>(),"font":font,"icon_packs":catalogue,
        "limits":["All reviewed include declarations are inventoried, including optional metadata whose final linker inclusion is not established.","Only Noto Sans JP and Lucide geometry are bundled; other icon geometry/fonts are acquired separately.","User/project/system assets, optional runtime distributions, and other embedded project code need separate applicability review."]});
    Ok((index, originals))
}

const MAX_ASSET: u64 = 64 * 1024 * 1024;

pub fn files(index: &Value, originals: Files) -> Result<Files> {
    let mut lines = vec![
        "Embedded font and icon notice inventory".to_owned(),
        "".into(),
        index["scope"].as_str().unwrap().into(),
        "".into(),
    ];
    lines.extend(
        index["limits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_owned()),
    );
    lines.extend([
        "".into(),
        format!(
            "Font: {} ({})",
            index["font"]["family"].as_str().unwrap(),
            index["font"]["license"].as_str().unwrap()
        ),
        "".into(),
    ]);
    for pack in index["icon_packs"].as_array().unwrap() {
        lines.extend([
            format!(
                "{} {} ({})",
                pack["id"].as_str().unwrap(),
                pack["version"].as_str().unwrap(),
                pack["license"].as_str().unwrap()
            ),
            format!("Source: {}", pack["source_url"].as_str().unwrap()),
            pack["copyright"].as_str().unwrap_or_default().into(),
            "".into(),
        ]);
    }
    for (name, data) in &originals {
        if name.ends_with("/LICENSE") || name.ends_with("/OFL.txt") {
            lines.extend([
                "=".repeat(72),
                name.clone(),
                "=".repeat(72),
                std::str::from_utf8(data)?.into(),
                "".into(),
            ]);
        }
    }
    let mut files = originals;
    files.insert("inventory.json".into(), util::encode(index)?);
    files.insert(
        "ASSET-NOTICES.txt".into(),
        (lines.join("\n") + "\n").into_bytes(),
    );
    Ok(files)
}

pub fn collect(root: &Path, output: &Path) -> Result<Value> {
    let (index, originals) = build(root)?;
    let result = json!({"embedded_declarations":index["declarations"].as_array().unwrap().len(),"asset_inputs":index["inputs"].as_array().unwrap().len(),"icon_packs":6,"original_files":originals.len()});
    super::publish_bundle(output, &files(&index, originals)?)?;
    Ok(result)
}

pub fn verify(root: &Path, output: &Path) -> Result<Value> {
    let (index, originals) = build(root)?;
    let expected = files(&index, originals)?;
    let actual = super::verify_bundle(output, |_| true)?;
    ensure!(actual == expected, "Stale or changed asset notice inputs");
    Ok(
        json!({"verified_files":expected.len()+1,"asset_inputs":index["inputs"].as_array().unwrap().len()}),
    )
}
