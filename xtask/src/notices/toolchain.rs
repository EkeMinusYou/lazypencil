use super::{Files, fingerprint};
use crate::{process, util};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const RELEASE: &str = "1.94.0";
pub const COMMIT: &str = "4a4ef493e3a1488c6e321570238084b38948f6db";
pub const SOURCE_SHA: &str = "0b53ae34f5c0c3612cfe1de139f9167a018cd5737bc2205664fd69ba9b25f600";
pub const SOURCE_URL: &str = "https://static.rust-lang.org/dist/rustc-1.94.0-src.tar.xz";

fn string(v: &Value) -> Result<&str> {
    v.as_str().context("Missing toolchain inventory string")
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("Missing toolchain inventory array")
}
fn sha(v: &Value, n: usize) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

pub fn tree(root: &Path) -> Result<Files> {
    ensure!(
        root.is_dir() && !root.is_symlink(),
        "Expected regular standard-library source directory"
    );
    let mut result = Files::new();
    let mut total = 0;
    for path in util::walk_filtered(root, |p| p.components().any(|c| c.as_os_str() == ".git"))? {
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .context("Invalid library source path")?
            .replace('\\', "/");
        if relative.split('/').any(|s| s == ".git") {
            continue;
        }
        let bytes = util::read(&path, 32 * 1024 * 1024)?;
        total += bytes.len();
        ensure!(
            result.len() < 20_000 && total <= 256 * 1024 * 1024,
            "Standard-library source exceeds inspection limits"
        );
        result.insert(relative, bytes);
    }
    Ok(result)
}

fn source_lock(files: &Files) -> Result<Vec<Value>> {
    let lock: toml::Value = toml::from_str(std::str::from_utf8(
        files
            .get("Cargo.lock")
            .context("Missing library lockfile")?,
    )?)?;
    let packages = serde_json::to_value(
        lock.get("package")
            .context("Missing library locked packages")?,
    )?;
    Ok(array(&packages)?.clone())
}

pub fn vendor_packages(files: &Files) -> Result<(Vec<Value>, Vec<String>)> {
    let mut packages = Vec::new();
    let mut findings = Vec::new();
    let mut expected = BTreeSet::new();
    for package in source_lock(files)? {
        if package.get("source").is_none() {
            continue;
        }
        ensure!(
            package["source"] == "registry+https://github.com/rust-lang/crates.io-index"
                && sha(&package["checksum"], 64),
            "Unreviewed standard-library package source/checksum"
        );
        let key = super::rust::identifier(&package)?;
        let prefix = format!(
            "vendor/{}-{}/",
            string(&package["name"])?,
            string(&package["version"])?
        );
        let manifest_path = format!("{prefix}Cargo.toml");
        expected.insert(manifest_path.clone());
        let metadata = files
            .get(&manifest_path)
            .context("Missing standard-library vendor manifest")?;
        let manifest: toml::Value = toml::from_str(std::str::from_utf8(metadata)?)?;
        let manifest = serde_json::to_value(
            manifest
                .get("package")
                .context("Missing vendored package")?,
        )?;
        ensure!(
            manifest["name"] == package["name"] && manifest["version"] == package["version"],
            "Vendor package identity mismatch"
        );
        let checksums: Value = serde_json::from_slice(
            files
                .get(&format!("{prefix}.cargo-checksum.json"))
                .context("Missing vendor checksum metadata")?,
        )?;
        ensure!(
            checksums["package"] == package["checksum"],
            "Vendor checksum differs from library lock"
        );
        let actual = files
            .iter()
            .filter_map(|(p, b)| {
                p.strip_prefix(&prefix)
                    .filter(|s| *s != ".cargo-checksum.json")
                    .map(|p| (p, b))
            })
            .collect::<BTreeMap<_, _>>();
        let declared = checksums["files"]
            .as_object()
            .context("Invalid vendor checksum map")?;
        ensure!(
            actual.keys().copied().collect::<BTreeSet<_>>()
                == declared.keys().map(String::as_str).collect(),
            "Missing or extra vendor files"
        );
        for (path, hash) in declared {
            util::safe_path(path)?;
            ensure!(
                util::digest(actual[path.as_str()]) == *hash,
                "Changed standard-library vendor source"
            );
        }
        let legal = actual
            .keys()
            .filter(|p| super::rust::is_notice(p))
            .map(|p| format!("{prefix}{p}"))
            .collect::<Vec<_>>();
        if legal.is_empty() {
            findings.push(format!("No standalone notice in vendored package {key}; retain and review its publisher declaration and source references."));
        }
        packages.push(json!({"name":package["name"],"version":package["version"],"archive_sha256_declared":package["checksum"],"manifest_sha256":util::digest(metadata),"license":manifest["license"],"authors":manifest.get("authors").cloned().unwrap_or(json!([])),"repository":manifest["repository"],"notice_paths":legal,"verified_vendor_files":actual.len()}));
    }
    let re = Regex::new(r"^vendor/[^/]+/Cargo.toml$")?;
    let actual = files
        .keys()
        .filter(|p| re.is_match(p))
        .cloned()
        .collect::<BTreeSet<_>>();
    ensure!(
        actual == expected,
        "Unreviewed vendor packages outside library lock"
    );
    packages.sort_by(|a, b| {
        string(&a["name"])
            .unwrap()
            .cmp(string(&b["name"]).unwrap())
            .then_with(|| {
                string(&a["version"])
                    .unwrap()
                    .cmp(string(&b["version"]).unwrap())
            })
    });
    findings.sort();
    Ok((packages, findings))
}

pub fn reference_terms(upstream: Option<&Path>) -> Result<BTreeMap<String, (Value, Vec<u8>)>> {
    let Some(upstream) = upstream else {
        return Ok(BTreeMap::new());
    };
    let policy: Value = util::json(upstream)?;
    let mut result = BTreeMap::new();
    let empty = vec![];
    let references = match policy.get("license_references") {
        Some(v) => array(v)?,
        None => &empty,
    };
    ensure!(references.len() <= 100, "Too many reference terms");
    for item in references {
        let id = string(&item["id"])?;
        ensure!(
            Regex::new(r"^[A-Za-z0-9][A-Za-z0-9.+-]{0,127}$")?.is_match(id)
                && !result.contains_key(id),
            "Invalid or duplicate reference identifier"
        );
        ensure!(
            item["kind"] == "reference_terms_only"
                && item["repository"] == "https://github.com/spdx/license-list-data"
                && sha(&item["revision"], 40)
                && item["path"] == format!("text/{id}.txt")
                && item["url"]
                    == format!(
                        "https://raw.githubusercontent.com/spdx/license-list-data/{}/{}",
                        string(&item["revision"])?,
                        string(&item["path"])?
                    ),
            "Reference term provenance mismatch"
        );
        let data = super::stored_blob(upstream.parent().unwrap(), item)?;
        ensure!(
            item["git_blob_sha1"] == super::git_blob(&data),
            "Reference Git blob mismatch"
        );
        result.insert(id.into(), (item.clone(), data));
    }
    Ok(result)
}

pub fn upstream_policy(upstream: Option<&Path>) -> Result<BTreeMap<String, Value>> {
    let policy = if let Some(p) = upstream {
        util::json::<Value>(p)?
    } else {
        json!({"format_version":1,"packages":[],"license_references":[]})
    };
    ensure!(
        policy["format_version"] == 1,
        "Unsupported toolchain policy"
    );
    reference_terms(upstream)?;
    let packages = array(&policy["packages"])?;
    ensure!(packages.len() <= 1000, "Too many upstream packages");
    let mut result = BTreeMap::new();
    for package in packages {
        let key = super::rust::identifier(package)?;
        ensure!(
            result.insert(key, package.clone()).is_none() && sha(&package["archive_sha256"], 64),
            "Duplicate/invalid upstream package"
        );
        let files = array(&package["files"])?;
        ensure!(
            (1..=100).contains(&files.len()),
            "Missing upstream notice files"
        );
        for item in files {
            let path = string(&item["path"])?;
            util::safe_path(path)?;
            let revision = string(&item["revision"])?;
            ensure!(sha(&item["revision"],40)&&Regex::new(&format!(r"^https://raw\.githubusercontent\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/{revision}/{}$",regex::escape(path)))?.is_match(string(&item["url"])?),"Upstream URL/revision mismatch");
            let data = super::stored_blob(
                upstream
                    .context("Missing policy directory")?
                    .parent()
                    .unwrap(),
                item,
            )?;
            ensure!(
                item["git_blob_sha1"] == super::git_blob(&data),
                "Upstream Git blob mismatch"
            );
        }
    }
    Ok(result)
}

pub fn registry_packages(
    files: &Files,
    cargo: &Path,
    upstream: Option<&Path>,
    texts: &mut Files,
) -> Result<(Vec<Value>, Vec<String>, Vec<Value>)> {
    let supplements = upstream_policy(upstream)?;
    let mut packages = Vec::new();
    let mut findings = Vec::new();
    let mut sources = Vec::new();
    let mut locked = source_lock(files)?;
    locked.sort_by(|a, b| {
        string(&a["name"])
            .unwrap()
            .cmp(string(&b["name"]).unwrap())
            .then_with(|| {
                string(&a["version"])
                    .unwrap()
                    .cmp(string(&b["version"]).unwrap())
            })
    });
    for package in locked {
        if package.get("source").is_none() {
            continue;
        }
        let key = super::rust::identifier(&package)?;
        let mut hashes = BTreeMap::new();
        let (mut record, manifest) =
            super::rust::archive_record(&package, cargo, texts, Some(&mut hashes))?;
        if supplements.contains_key(&key) {
            super::rust::supplement(
                &mut record,
                &manifest,
                &supplements,
                upstream.unwrap().parent().unwrap(),
                texts,
            )?;
        }
        record["files"].as_array_mut().unwrap().push(json!({"path":"Cargo.toml","kind":"publisher_declaration","sha256":util::digest(&manifest),"blob":super::blob(manifest,texts)}));
        if !array(&record["files"])?
            .iter()
            .any(|v| v["kind"] != "publisher_declaration")
        {
            findings.push(format!("No standalone notice in registry package {key}; review its publisher declaration and source references."));
        }
        if record["vcs"]["git"]["dirty"] == true {
            findings.push(format!("Published VCS metadata declares dirty=true for {key}; pinned upstream terms do not attest a pristine source checkout."));
        }
        let prefix = format!(
            "vendor/{}-{}/",
            string(&package["name"])?,
            string(&package["version"])?
        );
        let actual = files
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(&prefix)
                    .filter(|s| *s != ".cargo-checksum.json")
                    .map(|p| (p.to_owned(), util::digest(bytes)))
            })
            .collect::<BTreeMap<_, _>>();
        let mut comparison = json!({"present":!actual.is_empty(),"matches":null});
        if !actual.is_empty() {
            let missing = hashes
                .keys()
                .filter(|p| !actual.contains_key(*p))
                .cloned()
                .collect::<Vec<_>>();
            let extra = actual
                .keys()
                .filter(|p| !hashes.contains_key(*p))
                .cloned()
                .collect::<Vec<_>>();
            let changed = hashes
                .iter()
                .filter(|(p, h)| actual.get(*p).is_some_and(|v| v != *h))
                .map(|(p, _)| p.clone())
                .collect::<Vec<_>>();
            let matches = missing.is_empty() && extra.is_empty() && changed.is_empty();
            comparison = json!({"present":true,"matches":matches,"missing_files":missing,"extra_files":extra,"changed_files":changed});
            if !matches {
                findings.push(format!("Installed vendor sources differ from the original registry archive for {key}; inspect vendor_registry_comparison."));
            }
        }
        record["vendor_registry_comparison"] = comparison;
        record["original_source_hashes"] = serde_json::to_value(hashes)?;
        if Regex::new(r"[A-Za-z0-9][A-Za-z0-9.+-]*")?
            .find_iter(record["license"].as_str().unwrap_or_default())
            .any(|m| m.as_str() == "MPL-2.0")
        {
            let data = util::read(
                &super::rust::archive_path(&package, cargo)?,
                256 * 1024 * 1024,
            )?;
            let path = format!(
                "sources/{}-{}.crate",
                string(&package["name"])?,
                string(&package["version"])?
            );
            sources.push(json!({"package":key,"path":path,"sha256":util::digest(&data),"bytes":data.len(),"license":"MPL-2.0","unmodified_registry_archive":true,"url":format!("https://static.crates.io/crates/{}/{}-{}.crate",string(&package["name"])?,string(&package["name"])?,string(&package["version"])?)}));
            texts.insert(path, data);
        }
        packages.push(record);
    }
    findings.sort();
    Ok((packages, findings, sources))
}

fn retain(path: &str, data: Vec<u8>, kind: &str, originals: &mut Vec<Value>, texts: &mut Files) {
    originals.push(json!({"path":path,"kind":kind,"sha256":util::digest(&data),"blob":super::blob(data,texts)}));
}

pub fn build(
    source: &Path,
    legal: &Path,
    docs: &Path,
    libraries: &Path,
    identity: &Value,
    cargo: Option<&Path>,
    upstream: Option<&Path>,
) -> Result<(Value, Files)> {
    ensure!(
        upstream.is_none() || cargo.is_some(),
        "Upstream terms require exact original registry archives"
    );
    let files = tree(&source.join("library"))?;
    ensure!(
        files.contains_key("Cargo.lock") && files.contains_key("Cargo.toml"),
        "Missing library Cargo manifest/lock"
    );
    let vendor = Regex::new(r"^vendor/[^/]+/Cargo.toml$")?;
    let has_vendor = files.keys().any(|p| vendor.is_match(p));
    ensure!(
        has_vendor || cargo.is_some(),
        "Provide original archives or vendored sources"
    );
    let (packages, mut findings) = if has_vendor {
        vendor_packages(&files)?
    } else {
        (vec![], vec![])
    };
    let mut texts = Files::new();
    let mut originals = Vec::new();
    let (registry, corresponding) = if let Some(cargo) = cargo {
        let (registry, registry_findings, sources) =
            registry_packages(&files, cargo, upstream, &mut texts)?;
        findings = registry_findings;
        (registry, sources)
    } else {
        (vec![], vec![])
    };
    for name in ["COPYRIGHT", "LICENSE-MIT", "LICENSE-APACHE"] {
        retain(
            &format!("distribution/{name}"),
            util::read(&legal.join(name), util::MAX_FILE)?,
            "distribution",
            &mut originals,
            &mut texts,
        );
    }
    let copyright = util::read(&docs.join("COPYRIGHT-library.html"), util::MAX_FILE)?;
    retain(
        "documentation/COPYRIGHT-library.html",
        copyright.clone(),
        "standard_library_copyright",
        &mut originals,
        &mut texts,
    );
    let re = Regex::new(r"<b>License:</b>\s*([^<]+)")?;
    let body = std::str::from_utf8(&copyright)?;
    let expressions = re
        .captures_iter(body)
        .map(|m| m[1].to_owned())
        .collect::<Vec<_>>();
    ensure!(
        !expressions.is_empty(),
        "Missing standard-library copyright license declarations"
    );
    let tokens = Regex::new(r"[A-Za-z0-9][A-Za-z0-9.+-]*")?;
    let mut terms = BTreeSet::new();
    for expression in expressions {
        for token in tokens.find_iter(&html_escape::decode_html_entities(&expression)) {
            if !["OR", "AND", "WITH"].contains(&token.as_str()) {
                terms.insert(token.as_str().to_owned());
            }
        }
    }
    let references = reference_terms(upstream)?;
    for name in terms {
        let path = docs.join("licenses").join(format!("{name}.txt"));
        let retained = format!("documentation/licenses/{name}.txt");
        if path.exists() || path.is_symlink() || !references.contains_key(&name) {
            retain(
                &retained,
                util::read(&path, util::MAX_FILE)?,
                "referenced_terms",
                &mut originals,
                &mut texts,
            );
        } else {
            let (reference, data) = &references[&name];
            retain(
                &retained,
                data.clone(),
                "referenced_terms_upstream",
                &mut originals,
                &mut texts,
            );
            originals.last_mut().unwrap()["provenance"] = reference.clone();
        }
    }
    let mut comments = Vec::new();
    let mut inputs = Vec::new();
    for (relative, bytes) in &files {
        let name = format!("library/{relative}");
        inputs.push(json!({"path":name,"sha256":util::digest(bytes),"bytes":bytes.len()}));
        if super::rust::is_notice(relative)
            || ["Cargo.toml", "Cargo.lock"].contains(&relative.as_str())
            || vendor.is_match(relative)
        {
            retain(&name, bytes.clone(), "source", &mut originals, &mut texts);
        }
        let extension = Path::new(relative)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ["rs", "c", "cc", "cpp", "h", "hpp", "s"].contains(&extension.as_str()) {
            comments.extend(super::source_comments(
                &name,
                &String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]),
            )?);
        }
    }
    for package in &registry {
        let prefix = format!("registry/{}/", super::rust::identifier(package)?);
        for item in array(&package["files"])? {
            let bytes = texts
                .get(string(&item["blob"])?)
                .context("Missing registry original notice")?
                .clone();
            retain(
                &format!("{prefix}{}", string(&item["path"])?),
                bytes,
                string(&item["kind"])?,
                &mut originals,
                &mut texts,
            );
        }
        for item in array(&package["attribution_comments"])? {
            for line in array(&item["lines"])? {
                comments.push(json!({"path":format!("{prefix}{}",string(&item["path"])?),"line":line["line"],"text":line["text"]}));
            }
        }
    }
    let unwind = source.join("src/llvm-project/libunwind/LICENSE.TXT");
    if unwind.exists() {
        retain(
            "src/llvm-project/libunwind/LICENSE.TXT",
            util::read(&unwind, util::MAX_FILE)?,
            "native_unwind_reference",
            &mut originals,
            &mut texts,
        );
    }
    Ok((
        json!({"format_version":1,"scope":"Installed Rust standard-library source/notice and target-library inventory; not exact application linkage.","observed_toolchain":identity,"source_inputs":inputs,"vendor_packages":packages,"registry_packages":registry,"corresponding_sources":corresponding,"notice_files":originals,"source_comments":comments,"target_libraries":library_fingerprints(libraries)?,"review_findings":findings,"vendor_file_checksums_verified":has_vendor,"registry_archives_verified":cargo.is_some(),
        "limits":{"source_provenance":"Local source/target hashes and observed compiler identity; upstream authenticity and source-to-binary provenance require separate verification.","vendor_checksums":"Present vendor files are checked against vendor metadata and the library lock. With --cargo-home, original archive checksums and all source hashes are also verified; vendor/archive differences remain explicit review findings. Collection never fetches inputs.","coverage":"All installed library sources and library lock registry packages, including unused, optional and test code; compiler executables, system libraries and exact app linkage are excluded.","comments":"Comment candidates in the first 65536 bytes of each observed Rust/C/C++/assembly source, not automatic license classifications."}}),
        texts,
    ))
}

pub fn library_fingerprints(libraries: &Path) -> Result<Vec<Value>> {
    let mut paths = fs::read_dir(libraries)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    let mut result = Vec::new();
    for path in paths {
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if path.is_file()
            && ["rlib", "rmeta", "so", "dylib", "dll", "lib"].contains(&extension.as_str())
        {
            result.push(fingerprint(
                libraries,
                path.file_name()
                    .unwrap()
                    .to_str()
                    .context("Invalid library filename")?,
                512 * 1024 * 1024,
            )?);
        }
    }
    ensure!(
        result.iter().any(|p| p["path"]
            .as_str()
            .is_some_and(|s| s.starts_with("libstd-") && s.ends_with(".rlib"))),
        "No installed target standard library"
    );
    Ok(result)
}

pub fn files(index: &Value, mut texts: Files) -> Result<Files> {
    let mut lines = vec![
        "Rust standard-library notice inventory".into(),
        "".into(),
        string(&index["scope"])?.into(),
        "".into(),
        format!(
            "Compiler: {}",
            string(&index["observed_toolchain"]["release"])?
        ),
        format!(
            "Target: {}",
            string(&index["observed_toolchain"]["target"])?
        ),
        "".into(),
        "Review findings:".into(),
    ];
    lines.extend(
        array(&index["review_findings"])?
            .iter()
            .map(|v| v.as_str().unwrap().to_owned()),
    );
    lines.push("".into());
    for item in array(&index["notice_files"])? {
        let kind = string(&item["kind"])?;
        lines.extend([
            "=".repeat(72),
            format!("{} ({kind})", string(&item["path"])?),
            "=".repeat(72),
        ]);
        if kind.starts_with("referenced_terms") {
            lines.push("Reference terms only; original package/source copyright notices are retained separately.".into());
        }
        lines.extend([
            String::from_utf8_lossy(
                texts
                    .get(string(&item["blob"])?)
                    .context("Missing original toolchain notice")?,
            )
            .into_owned(),
            "".into(),
        ]);
    }
    lines.extend([
        "Source comment candidates; source copyright notices are retained verbatim".into(),
        "".into(),
    ]);
    for item in array(&index["source_comments"])? {
        lines.extend([
            format!("{}:{}", string(&item["path"])?, item["line"]),
            string(&item["text"])?.into(),
            "".into(),
        ]);
    }
    for item in array(&index["corresponding_sources"])? {
        lines.extend([
            format!(
                "Corresponding source for {} ({}):",
                string(&item["package"])?,
                string(&item["license"])?
            ),
            string(&item["path"])?.into(),
            "Complete, unmodified registry archive. Original notices are retained above.".into(),
            format!("SHA-256: {}", string(&item["sha256"])?),
            "".into(),
        ]);
    }
    texts.insert("inventory.json".into(), util::encode(index)?);
    texts.insert(
        "TOOLCHAIN-NOTICES.txt".into(),
        (lines.join("\n").replace("\r\n", "\n") + "\n").into_bytes(),
    );
    Ok(texts)
}

pub fn observe(rustc: &str, target: Option<&str>) -> Result<(Value, PathBuf, PathBuf)> {
    let run = |args: &[&str]| -> Result<String> {
        let output = process::checked(
            Command::new(rustc).args(args),
            std::time::Duration::from_secs(20),
        )?;
        ensure!(output.len() < 65536, "Oversized rustc metadata response");
        Ok(String::from_utf8(output)?.trim().to_owned())
    };
    let verbose = run(&["--version", "--verbose"])?;
    let fields = verbose
        .lines()
        .filter_map(|s| s.split_once(": "))
        .collect::<BTreeMap<_, _>>();
    let target = target
        .or_else(|| fields.get("host").copied())
        .context("Missing compiler host")?;
    ensure!(
        Regex::new(r"^[A-Za-z0-9_.-]{1,128}$")?.is_match(target),
        "Invalid target triple"
    );
    let identity = json!({"verbose":verbose,"release":fields.get("release").context("Missing compiler release")?,"commit_hash":fields.get("commit-hash").context("Missing compiler commit")?,"target":target});
    Ok((
        identity,
        PathBuf::from(run(&["--print", "sysroot"])?),
        PathBuf::from(run(&["--print", "target-libdir", "--target", target])?),
    ))
}

pub fn validate_evidence(root: &Path, compiler: &str, rustc: &str) -> Result<Value> {
    ensure!(
        root.is_dir() && !root.is_symlink(),
        "Expected regular toolchain evidence directory"
    );
    let root = root.canonicalize()?;
    let names = fs::read_dir(&root)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().to_string()))
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    ensure!(
        names == ["inventory".into(), "source-distribution.json".into()].into(),
        "Unexpected toolchain evidence members"
    );
    let distribution_bytes = util::read(&root.join("source-distribution.json"), util::MAX_FILE)?;
    let distribution: Value = serde_json::from_slice(&distribution_bytes)?;
    ensure!(
        distribution["format_version"] == 1
            && distribution["release_ready"] == false
            && distribution["source_url"] == SOURCE_URL
            && distribution["source_sha256"] == SOURCE_SHA,
        "Unreviewed standard-library distribution"
    );
    let allowed = Regex::new(
        r"^(?:inventory\.json|TOOLCHAIN-NOTICES\.txt|texts/[0-9a-f]{64}\.txt|sources/[A-Za-z0-9_.+-]+\.crate)$",
    )?;
    let members = super::verify_bundle(&root.join("inventory"), |s| allowed.is_match(s))?;
    ensure!(
        members.contains_key("TOOLCHAIN-NOTICES.txt"),
        "Missing readable standard-library notice document"
    );
    let index_bytes = members
        .get("inventory.json")
        .context("Missing standard-library inventory")?;
    let index: Value = serde_json::from_slice(index_bytes)?;
    let (identity, _, libraries) = observe(rustc, None)?;
    ensure!(
        index["format_version"] == 1
            && index["registry_archives_verified"] == true
            && identity["release"] == RELEASE
            && identity["commit_hash"] == COMMIT
            && index["observed_toolchain"] == identity
            && distribution["observed_toolchain"] == identity
            && identity["verbose"] == compiler.replace("\r\n", "\n").trim(),
        "Toolchain evidence belongs to another compiler/target"
    );
    ensure!(
        index["target_libraries"] == serde_json::to_value(library_fingerprints(&libraries)?)?,
        "Installed target libraries differ from prepared evidence"
    );
    Ok(
        json!({"source_distribution_sha256":util::digest(distribution_bytes),"manifest_sha256":util::file_digest(&root.join("inventory/manifest.json"))?,"inventory_sha256":util::digest(index_bytes),"target":identity["target"],"target_libraries":array(&index["target_libraries"])?.len(),"registry_packages":array(&index["registry_packages"])?.len(),"review_findings":index["review_findings"],"scope":"Source inventory bound to observed compiler and installed library bytes; final binary linkage and source-to-binary reproducibility are not established."}),
    )
}

pub fn attach(root: &Path, output: &Path, compiler: &str, rustc: &str) -> Result<Value> {
    ensure!(
        !output.exists() && !output.is_symlink(),
        "Toolchain attachment output must not exist"
    );
    let result = validate_evidence(root, compiler, rustc)?;
    fs::create_dir(output)?;
    for path in util::walk(root)? {
        let destination = output.join(path.strip_prefix(root)?);
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::copy(path, destination)?;
    }
    ensure!(
        validate_evidence(output, compiler, rustc)? == result,
        "Toolchain evidence changed during attachment"
    );
    Ok(result)
}
