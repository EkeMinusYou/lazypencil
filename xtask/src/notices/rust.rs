use super::{Files, blob, stored_blob};
use crate::util;
use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

pub fn identifier(package: &Value) -> Result<String> {
    let name = package["name"].as_str().context("Missing package name")?;
    let version = package["version"]
        .as_str()
        .context("Missing package version")?;
    ensure!(
        Regex::new(r"^[A-Za-z0-9_-]{1,128}$")?.is_match(name)
            && Regex::new(r"^[A-Za-z0-9.+_-]{1,128}$")?.is_match(version),
        "Invalid package identity"
    );
    Ok(format!("{name}@{version}"))
}

pub fn is_notice(name: &str) -> bool {
    let path = Path::new(name);
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let code = [
        "rs", "py", "c", "h", "hpp", "cpp", "s", "asm", "toml", "json", "lock",
    ];
    (Regex::new(r"(?i)^(?:licen[cs]e|copying|copyright|notice|authors|unlicense)(?:[._-].*)?$")
        .unwrap()
        .is_match(file)
        && !code.contains(&extension.as_str()))
        || name
            .split('/')
            .rev()
            .skip(1)
            .any(|p| p.eq_ignore_ascii_case("LICENSES"))
}

pub fn archive_path(package: &Value, cargo: &Path) -> Result<std::path::PathBuf> {
    let key = identifier(package)?;
    ensure!(
        package["source"] == "registry+https://github.com/rust-lang/crates.io-index",
        "Unreviewed package source"
    );
    let expected = package["checksum"]
        .as_str()
        .context("Missing registry checksum")?;
    let filename = format!(
        "{}-{}.crate",
        package["name"].as_str().unwrap(),
        package["version"].as_str().unwrap()
    );
    for registry in fs::read_dir(cargo.join("registry/cache"))? {
        let path = registry?.path().join(&filename);
        if path.is_file() && !path.is_symlink() && util::file_digest(&path)? == expected {
            return Ok(path);
        }
    }
    anyhow::bail!("Fetch locked crate archive before collecting: {key}")
}

pub fn archive_record(
    package: &Value,
    cargo: &Path,
    texts: &mut Files,
    source_hashes: Option<&mut BTreeMap<String, String>>,
) -> Result<(Value, Vec<u8>)> {
    let key = identifier(package)?;
    ensure!(
        package["source"] == "registry+https://github.com/rust-lang/crates.io-index",
        "Unreviewed package source: {key}"
    );
    let expected = package["checksum"]
        .as_str()
        .context("Missing registry checksum")?;
    let filename = format!(
        "{}-{}.crate",
        package["name"].as_str().unwrap(),
        package["version"].as_str().unwrap()
    );
    let archive = archive_path(package, cargo)?;
    let bytes = util::read(&archive, 256 * 1024 * 1024)?;
    let prefix = filename.strip_suffix(".crate").unwrap();
    let mut reader = tar::Archive::new(GzDecoder::new(bytes.as_slice()));
    let mut files = Vec::new();
    let mut attributions = Vec::new();
    let mut seen = BTreeSet::new();
    let mut manifest_data = None;
    let mut manifest = None;
    let mut vcs = Value::Null;
    let mut total = 0u64;
    let mut source_hashes = source_hashes;
    let attribution = Regex::new(r"(?i)copyright|©|SPDX-(?:FileCopyrightText|License-Identifier)")?;
    for entry in reader.entries()? {
        let mut entry = entry?;
        let name = String::from_utf8(entry.path_bytes().into_owned())?;
        util::safe_path(name.trim_end_matches('/'))?;
        ensure!(
            name.starts_with(&format!("{prefix}/")) && seen.insert(name.clone()),
            "Duplicate or wrong crate root: {key}"
        );
        total = total
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(
            seen.len() <= 100_000 && total <= 512 * 1024 * 1024,
            "Crate inspection budget exceeded: {key}"
        );
        let relative = name.strip_prefix(&format!("{prefix}/")).unwrap();
        if !entry.header().entry_type().is_file() {
            ensure!(
                (source_hashes.is_none() && !is_notice(relative))
                    || entry.header().entry_type().is_dir(),
                "Linked source or notice: {key}"
            );
            continue;
        }
        ensure!(entry.size() <= 128 * 1024 * 1024, "Oversized source member");
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        if let Some(hashes) = source_hashes.as_deref_mut() {
            hashes.insert(relative.to_owned(), util::digest(&data));
        }
        if relative == "Cargo.toml" {
            ensure!(data.len() as u64 <= util::MAX_FILE, "Oversized manifest");
            manifest = Some(toml::from_str::<toml::Value>(std::str::from_utf8(&data)?)?);
            manifest_data = Some(data.clone());
        }
        if relative == ".cargo_vcs_info.json" {
            ensure!(
                data.len() as u64 <= util::MAX_FILE,
                "Oversized VCS metadata"
            );
            vcs = serde_json::from_slice(&data)?;
        }
        if is_notice(relative) {
            ensure!(data.len() as u64 <= util::MAX_FILE, "Oversized notice");
            files.push(json!({"path":relative,"kind":"archive","sha256":util::digest(&data),"blob":blob(data.clone(),texts)}));
        } else {
            let extension = Path::new(relative)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if [
                "rs", "py", "c", "h", "hpp", "cpp", "s", "asm", "toml", "json", "lock", "md", "txt",
            ]
            .contains(&extension.as_str())
            {
                let body = String::from_utf8_lossy(&data[..data.len().min(32768)]);
                let lines = body
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| {
                        attribution.is_match(line)
                            && (["md", "txt"].contains(&extension.as_str())
                                || ["//", "/*", "*", "#", ";", "!"]
                                    .iter()
                                    .any(|p| line.trim_start().starts_with(p)))
                    })
                    .map(|(i, line)| json!({"line":i+1,"text":line}))
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    attributions.push(json!({"path":relative,"lines":lines}));
                }
            }
        }
    }
    let manifest = manifest.context("Missing crate manifest")?;
    let metadata = serde_json::to_value(&manifest["package"])?;
    ensure!(
        metadata["name"] == package["name"] && metadata["version"] == package["version"],
        "Crate identity mismatch: {key}"
    );
    if let Some(name) = metadata["license-file"].as_str()
        && !files.iter().any(|v| v["path"] == name)
    {
        util::safe_path(name)?;
        // Most crates use a conventional notice name. Revisit only an explicitly
        // declared custom file instead of retaining every source member in memory.
        let expected_name = format!("{prefix}/{name}");
        let mut archive = tar::Archive::new(GzDecoder::new(bytes.as_slice()));
        let mut original = None;
        for entry in archive.entries()? {
            let mut entry = entry?;
            if entry.path_bytes().as_ref() == expected_name.as_bytes() {
                ensure!(
                    entry.header().entry_type().is_file() && entry.size() <= util::MAX_FILE,
                    "Invalid declared license file"
                );
                let mut data = Vec::new();
                entry.read_to_end(&mut data)?;
                original = Some(data);
                break;
            }
        }
        let data = original.context("Missing declared license file")?;
        files.push(json!({"path":name,"kind":"archive","sha256":util::digest(&data),"blob":blob(data,texts)}));
    }
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    let manifest_data = manifest_data.unwrap();
    Ok((
        json!({"name":package["name"],"version":package["version"],"source":package["source"],"archive_sha256":expected,"manifest_sha256":util::digest(&manifest_data),
        "license":metadata["license"],"license_file":metadata["license-file"],"authors":metadata.get("authors").cloned().unwrap_or(json!([])),"repository":metadata["repository"],"vcs":vcs,"files":files,"attribution_comments":attributions}),
        manifest_data,
    ))
}

pub fn local_record(
    package: &Value,
    root: &Path,
    location: &str,
    texts: &mut Files,
) -> Result<Value> {
    if location != "." {
        util::safe_path(location)?;
    }
    let path = root.join(location).canonicalize()?;
    ensure!(
        path.starts_with(root.canonicalize()?),
        "Local package escapes workspace"
    );
    let manifest = util::read(&path.join("Cargo.toml"), util::MAX_FILE)?;
    let metadata: toml::Value = toml::from_str(std::str::from_utf8(&manifest)?)?;
    let metadata = serde_json::to_value(&metadata["package"])?;
    ensure!(
        metadata["name"] == package["name"] && metadata["version"] == package["version"],
        "Local package identity mismatch"
    );
    let mut paths = fs::read_dir(&path)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .unwrap()
            .to_str()
            .context("Invalid notice filename")?;
        if path.is_file() && is_notice(name) {
            let data = util::read(&path, util::MAX_FILE)?;
            files.push(json!({"path":name,"kind":"workspace","sha256":util::digest(&data),"blob":blob(data,texts)}));
        }
    }
    ensure!(!files.is_empty(), "Local package has no notice");
    Ok(
        json!({"name":package["name"],"version":package["version"],"source":"workspace","workspace_path":location,"license":metadata["license"],"manifest_sha256":util::digest(manifest),"files":files,"authors":metadata.get("authors").cloned().unwrap_or(json!([])),"attribution_comments":[]}),
    )
}

pub fn supplement(
    record: &mut Value,
    manifest: &[u8],
    inputs: &BTreeMap<String, Value>,
    directory: &Path,
    texts: &mut Files,
) -> Result<()> {
    let key = identifier(record)?;
    let primary = record["files"]
        .as_array()
        .context("Missing notices")?
        .iter()
        .any(|v| {
            v["path"].as_str().is_some_and(|p| {
                (!p.contains('/')
                    && Regex::new(r"(?i)^(?:licen[cs]e|copying|unlicense)")
                        .unwrap()
                        .is_match(p))
                    || v["path"] == record["license_file"]
            })
        });
    let Some(extra) = inputs.get(&key) else {
        ensure!(primary, "Review omitted upstream notices: {key}");
        return Ok(());
    };
    ensure!(
        extra["archive_sha256"] == record["archive_sha256"],
        "Upstream archive identity mismatch"
    );
    record["supplement_reason"] = extra.get("reason").cloned().unwrap_or(json!(
        "Original workspace-root notices omitted by the published archive."
    ));
    for item in extra["files"]
        .as_array()
        .context("Missing upstream files")?
    {
        let revision = record["vcs"]["git"]["sha1"]
            .as_str()
            .context("Missing published revision")?;
        ensure!(item["revision"] == revision, "Upstream revision mismatch");
        util::safe_path(item["path"].as_str().context("Missing upstream path")?)?;
        let repository = record["repository"]
            .as_str()
            .context("Missing repository")?
            .trim_end_matches('/')
            .trim_end_matches(".git");
        ensure!(
            repository.starts_with("https://github.com/")
                && item["url"].as_str().is_some_and(|s| s.starts_with(&format!(
                    "{}/{revision}/",
                    repository.replacen(
                        "https://github.com/",
                        "https://raw.githubusercontent.com/",
                        1
                    )
                ))),
            "Upstream repository mismatch"
        );
        let data = stored_blob(directory, item)?;
        use sha1::{Digest, Sha1};
        let mut hash = Sha1::new();
        hash.update(format!("blob {}\0", data.len()));
        hash.update(&data);
        ensure!(
            item["git_blob_sha1"] == format!("{:x}", hash.finalize()),
            "Upstream Git blob mismatch"
        );
        let mut item = item.clone();
        item["kind"] = json!("upstream");
        item["blob"] = json!(blob(data, texts));
        record["files"].as_array_mut().unwrap().push(item);
    }
    if let Some(reference) = extra.get("license_reference") {
        ensure!(
            reference == "MIT"
                && record["license"]
                    .as_str()
                    .is_some_and(|s| s.contains("MIT")),
            "Unreviewed reference terms"
        );
        record["license_reference"] = reference.clone();
        record["files"].as_array_mut().unwrap().push(json!({"path":"Cargo.toml","kind":"publisher_declaration","sha256":util::digest(manifest),"blob":blob(manifest.to_vec(),texts)}));
    }
    ensure!(
        !record["files"].as_array().unwrap().is_empty(),
        "No original notice/declaration"
    );
    Ok(())
}

fn value_array(value: &Value) -> Result<&Vec<Value>> {
    value.as_array().context("Expected inventory array")
}
fn display_field(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

pub fn render(index: &Value, texts: &Files) -> Result<Vec<u8>> {
    let mut lines = vec![
        "Rust package notices".into(),
        "====================".into(),
        "".into(),
        index["scope"]
            .as_str()
            .context("Missing notice scope")?
            .into(),
        "".into(),
        "Package authors below are copied from published metadata, not inferred copyright holders."
            .into(),
        "Shared license reference texts are labeled separately from original package notices."
            .into(),
        "Original notice bytes and detailed provenance are retained in index.json and texts/."
            .into(),
        "".into(),
    ];
    let mut by_blob = BTreeMap::<String, Vec<String>>::new();
    for package in value_array(&index["packages"])? {
        let key = identifier(package)?;
        lines.push(key.clone());
        lines.push(format!(
            "Declared license: {}",
            display_field(&package["license"])
        ));
        if let Some(authors) = package["authors"].as_array().filter(|a| !a.is_empty()) {
            lines.push(format!(
                "Published package authors: {}",
                authors
                    .iter()
                    .map(display_field)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        if let Some(reason) = package["supplement_reason"].as_str() {
            lines.push(reason.into());
        }
        if let Some(reference) = package["license_reference"].as_str() {
            lines.push(format!(
                "Reference terms: {reference} (not a package copyright notice)"
            ));
        }
        for item in value_array(&package["files"])? {
            by_blob
                .entry(item["blob"].as_str().context("Missing blob")?.into())
                .or_default()
                .push(format!(
                    "{key} :: {} :: {}",
                    display_field(&item["kind"]),
                    display_field(&item["path"])
                ));
        }
        let mut comments = BTreeMap::<String, Vec<String>>::new();
        for item in value_array(&package["attribution_comments"])? {
            for line in value_array(&item["lines"])? {
                comments
                    .entry(display_field(&line["text"]))
                    .or_default()
                    .push(format!("{}:{}", display_field(&item["path"]), line["line"]));
            }
        }
        if !comments.is_empty() {
            lines.push("Verbatim source attribution comments:".into());
            for (text, paths) in comments {
                lines.push(text);
                lines.push(format!("  Source: {}", paths.join(", ")));
            }
        }
        lines.push("".into());
    }
    for item in value_array(&index["license_references"])? {
        by_blob
            .entry(display_field(&item["blob"]))
            .or_default()
            .push(format!(
                "Shared reference terms: {}; {}",
                display_field(&item["id"]),
                display_field(&item["role"])
            ));
    }
    for (name, uses) in by_blob {
        let data = texts.get(&name).context("Missing notice blob")?;
        let text = String::from_utf8(data.clone())
            .unwrap_or_else(|_| data.iter().map(|b| char::from(*b)).collect());
        let text = text
            .lines()
            .map(|s| s.trim_end_matches([' ', '\t', '\r']))
            .collect::<Vec<_>>()
            .join("\n");
        lines.extend([
            "=".repeat(72),
            uses.join("\n"),
            format!(
                "SHA-256: {}",
                Path::new(&name).file_stem().unwrap().to_str().unwrap()
            ),
            "".into(),
            text,
            "".into(),
        ]);
    }
    Ok((lines.join("\n") + "\n").into_bytes())
}

pub fn locked(root: &Path) -> Result<(Vec<u8>, BTreeMap<String, Value>)> {
    let bytes = util::read(&root.join("Cargo.lock"), util::MAX_FILE)?;
    let lock: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    let packages = serde_json::to_value(&lock["package"])?;
    let mut result = BTreeMap::new();
    for package in value_array(&packages)? {
        ensure!(
            result
                .insert(identifier(package)?, package.clone())
                .is_none(),
            "Duplicate package identity"
        );
    }
    ensure!(result.len() <= 10000, "Too many locked packages");
    Ok((bytes, result))
}

pub fn collect(root: &Path, cargo: &Path, upstream: &Path, output: &Path) -> Result<Value> {
    let (lock_data, locked) = locked(root)?;
    let policy: Value = util::json(upstream)?;
    ensure!(policy["format_version"] == 1, "Unknown upstream policy");
    let mut inputs = BTreeMap::new();
    for package in value_array(&policy["packages"])? {
        ensure!(
            inputs
                .insert(identifier(package)?, package.clone())
                .is_none(),
            "Duplicate upstream package"
        );
    }
    let mut texts = Files::new();
    let mut references = Vec::new();
    for item in value_array(&policy["license_references"])? {
        let data = stored_blob(upstream.parent().unwrap(), item)?;
        let mut item = item.clone();
        item["blob"] = json!(blob(data, &mut texts));
        references.push(item);
    }
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(root.join("Cargo.toml"))?)?;
    let mut locations = BTreeMap::from([(
        manifest["package"]["name"].as_str().unwrap().to_owned(),
        ".".to_owned(),
    )]);
    if let Some(patches) = manifest
        .get("patch")
        .and_then(|v| v.get("crates-io"))
        .and_then(toml::Value::as_table)
    {
        for (name, item) in patches {
            if let Some(path) = item.get("path").and_then(toml::Value::as_str) {
                locations.insert(name.clone(), path.into());
            }
        }
    }
    let mut records = Vec::new();
    let mut ordered: Vec<_> = locked.values().collect();
    ordered.sort_by_key(|package| {
        (
            package["name"].as_str().unwrap(),
            package["version"].as_str().unwrap(),
        )
    });
    for package in ordered {
        let record = if package.get("source").is_some() {
            let (mut record, data) = archive_record(package, cargo, &mut texts, None)?;
            supplement(
                &mut record,
                &data,
                &inputs,
                upstream.parent().unwrap(),
                &mut texts,
            )?;
            record
        } else {
            local_record(
                package,
                root,
                locations
                    .get(package["name"].as_str().unwrap())
                    .context("Unmapped local package")?,
                &mut texts,
            )?
        };
        records.push(record);
    }
    ensure!(
        inputs.keys().all(|k| locked.contains_key(k)),
        "Stale upstream inputs"
    );
    let index = json!({"format_version":1,"cargo_lock_sha256":util::digest(&lock_data),"scope":"All Cargo.lock packages, including optional, build, and platform dependencies. This is not a linked-binary inventory. Native engines, the Rust toolchain, fonts, and icons require their separate notices.","packages":records,"license_references":references});
    let mut files = texts.clone();
    files.insert("index.json".into(), util::encode(&index)?);
    files.insert("upstream.json".into(), util::encode(&policy)?);
    files.insert("RUST-NOTICES.txt".into(), render(&index, &texts)?);
    ensure!(
        util::read(&root.join("Cargo.lock"), util::MAX_FILE)? == lock_data,
        "Lockfile changed during collection"
    );
    super::publish_bundle(output, &files)?;
    verify(root, output, None)
}

pub fn verify(root: &Path, bundle: &Path, cargo: Option<&Path>) -> Result<Value> {
    let blob_name = Regex::new(r"^texts/[0-9a-f]{64}\.txt$")?;
    let files = super::verify_bundle(bundle, |s| {
        ["index.json", "upstream.json", "RUST-NOTICES.txt"].contains(&s) || blob_name.is_match(s)
    })?;
    let index: Value = serde_json::from_slice(files.get("index.json").context("Missing index")?)?;
    let (lock_data, locked) = locked(root)?;
    ensure!(
        index["format_version"] == 1 && index["cargo_lock_sha256"] == util::digest(&lock_data),
        "Stale Rust notice lockfile"
    );
    let mut packages = BTreeMap::new();
    for package in value_array(&index["packages"])? {
        ensure!(
            packages.insert(identifier(package)?, package).is_none(),
            "Duplicate notice package"
        );
    }
    ensure!(
        packages.keys().eq(locked.keys()),
        "Notice inventory does not cover locked packages"
    );
    let mut texts = Files::new();
    for (name, bytes) in &files {
        if name.starts_with("texts/") {
            texts.insert(name.clone(), bytes.clone());
        }
    }
    for (key, package) in &packages {
        ensure!(
            !value_array(&package["files"])?.is_empty(),
            "No original notice: {key}"
        );
        let registry = locked[key].get("source").is_some();
        let location = package["workspace_path"].as_str().unwrap_or(".");
        if registry {
            ensure!(
                package["source"] == locked[key]["source"]
                    && package["archive_sha256"] == locked[key]["checksum"],
                "Registry package mismatch: {key}"
            );
        } else {
            ensure!(package["source"] == "workspace", "Local source mismatch");
            if location != "." {
                util::safe_path(location)?;
            }
            ensure!(
                util::file_digest(&root.join(location).join("Cargo.toml"))?
                    == package["manifest_sha256"],
                "Local manifest changed: {key}"
            );
        }
        for item in value_array(&package["files"])? {
            let kind = item["kind"].as_str().context("Missing notice origin")?;
            ensure!(
                if registry {
                    ["archive", "upstream", "publisher_declaration"].contains(&kind)
                } else {
                    kind == "workspace"
                },
                "Wrong notice origin"
            );
            let data = files
                .get(item["blob"].as_str().context("Missing blob name")?)
                .context("Missing original notice")?;
            ensure!(util::digest(data) == item["sha256"], "Notice hash mismatch");
            if kind == "workspace" {
                let name = item["path"].as_str().context("Missing workspace path")?;
                util::safe_path(name)?;
                ensure!(
                    util::file_digest(&root.join(location).join(name))? == item["sha256"],
                    "Workspace notice changed"
                );
            }
        }
    }
    for item in value_array(&index["license_references"])? {
        ensure!(
            util::digest(
                files
                    .get(item["blob"].as_str().context("Missing blob")?)
                    .context("Missing reference")?
            ) == item["sha256"],
            "Missing license reference"
        );
    }
    ensure!(
        render(&index, &texts)? == files["RUST-NOTICES.txt"],
        "Readable notices disagree with inventory"
    );
    if let Some(cargo) = cargo {
        let policy: Value = serde_json::from_slice(&files["upstream.json"])?;
        let mut inputs = BTreeMap::new();
        for item in value_array(&policy["packages"])? {
            inputs.insert(identifier(item)?, item.clone());
        }
        for (key, package) in &packages {
            let captured = if locked[key].get("source").is_some() {
                let (mut captured, data) =
                    archive_record(&locked[key], cargo, &mut Files::new(), None)?;
                supplement(&mut captured, &data, &inputs, bundle, &mut Files::new())?;
                captured
            } else {
                local_record(
                    &locked[key],
                    root,
                    package["workspace_path"]
                        .as_str()
                        .context("Missing local path")?,
                    &mut Files::new(),
                )?
            };
            ensure!(
                &captured == *package,
                "Package provenance differs from locked source: {key}"
            );
        }
    }
    Ok(
        json!({"rust_packages":packages.len(),"verified_files":files.len()+1,"bytes":files.values().map(|b|b.len()).sum::<usize>(),"cargo_lock_sha256":util::digest(lock_data),"source_archives_verified":cargo.is_some()}),
    )
}
