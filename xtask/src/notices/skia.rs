use super::{Files, fingerprint};
use crate::{process, util};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const TARGETS: &[&str] = &[
    "skia",
    "skparagraph",
    "skshaper",
    "skunicode_core",
    "skunicode_icu",
];
const EXTERNALS: &[&str] = &[
    "expat",
    "harfbuzz",
    "icu",
    "libjpeg-turbo",
    "libpng",
    "libwebp",
    "wuffs",
    "zlib",
    "freetype",
];
const ZLIB: &str = "https://chromium.googlesource.com/chromium/src/third_party/zlib";
const PREFIX: &str = "third_party/externals/zlib/";
// LICENSE.TXT delegates to these files, whose names do not match is_notice.
// Preserve both license alternatives and the separately licensed contributions.
const FREETYPE_TERMS: &[&str] = &[
    "LICENSE.TXT",
    "docs/FTL.TXT",
    "docs/GPLv2.TXT",
    "src/bdf/README",
    "src/pcf/README",
    "src/gzip/zlib.h",
    "src/base/fthash.c",
    "include/freetype/internal/fthash.h",
    "src/autofit/ft-hb-ft.c",
    "src/autofit/ft-hb-decls.h",
    "src/autofit/ft-hb-types.h",
    "src/autofit/hb-script-list.h",
];
// The primary libjpeg-turbo notice delegates its IJG terms to README.ijg.
const JPEG_TERMS: &[&str] = &["LICENSE.md", "README.ijg"];
// WebP source headers explicitly refer to the additional IP grant in PATENTS.
const WEBP_TERMS: &[&str] = &["COPYING", "AUTHORS", "PATENTS"];

fn sha(value: &Value, width: usize) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == width
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}
fn string(value: &Value) -> Result<&str> {
    value.as_str().context("Missing native inventory string")
}

pub fn reference(upstream: &Path) -> Result<(Value, Vec<u8>)> {
    let policy: Value = util::json(upstream)?;
    ensure!(
        policy["format_version"] == 1,
        "Unsupported native reference format"
    );
    let references = policy["references"]
        .as_array()
        .context("Missing native reference declarations")?;
    ensure!(
        references.len() == 1,
        "Review native reference declarations"
    );
    let reference = &references[0];
    ensure!(
        reference["id"] == "chromium-bsd",
        "Unreviewed native reference identifier"
    );
    let applies = &reference["applies_to"];
    let sources = applies["source_sha256"]
        .as_object()
        .context("Missing native source bindings")?;
    ensure!(
        sha(&applies["skia_deps_sha256"], 64) && (1..=1000).contains(&sources.len()),
        "Invalid native source bindings"
    );
    for (path, hash) in sources {
        util::safe_path(path)?;
        ensure!(
            path.starts_with(PREFIX) && sha(hash, 64),
            "Invalid native source binding"
        );
    }
    let empty = serde_json::Map::new();
    let variants = match applies.get("working_tree_variants") {
        Some(v) => v
            .as_object()
            .context("Invalid native working-tree variants")?,
        None => &empty,
    };
    ensure!(
        variants.len() <= sources.len(),
        "Invalid native working-tree variants"
    );
    if !variants.is_empty() {
        let origin = applies["source_origin"]
            .as_object()
            .context("Missing native source origin")?;
        ensure!(
            origin.len() == 2
                && origin.get("repository") == Some(&json!(ZLIB))
                && origin.get("revision").is_some_and(|v| sha(v, 40)),
            "Invalid native variant origin"
        );
        for (path, variant) in variants {
            let fields = variant
                .as_object()
                .context("Invalid working-tree variant")?;
            ensure!(
                sources.contains_key(path)
                    && path.starts_with(PREFIX)
                    && fields.len() == 4
                    && ["transform", "sha256", "source_git_blob_sha1", "url"]
                        .iter()
                        .all(|k| fields.contains_key(*k)),
                "Unreviewed native variant"
            );
            ensure!(
                variant["transform"] == "lf_to_crlf"
                    && sha(&variant["sha256"], 64)
                    && variant["sha256"] != sources[path]
                    && sha(&variant["source_git_blob_sha1"], 40)
                    && variant["url"]
                        == format!(
                            "{ZLIB}/+/{}/{}?format=TEXT",
                            string(&origin["revision"])?,
                            path.strip_prefix(PREFIX).unwrap()
                        ),
                "Invalid native variant provenance"
            );
        }
    }
    let repository = "https://chromium.googlesource.com/chromium/src";
    ensure!(
        reference["repository"] == repository
            && sha(&reference["revision"], 40)
            && reference["path"] == "LICENSE"
            && reference["url"]
                == format!(
                    "{repository}/+/{}/LICENSE?format=TEXT",
                    string(&reference["revision"])?
                ),
        "Native repository/revision mismatch"
    );
    let bytes = super::stored_blob(
        upstream
            .parent()
            .context("Missing native reference directory")?,
        reference,
    )?;
    ensure!(
        reference["git_blob_sha1"] == super::git_blob(&bytes),
        "Native reference Git blob mismatch"
    );
    Ok((reference.clone(), bytes))
}

pub fn verify_upstream(upstream: &Path) -> Result<Value> {
    let (reference, data) = reference(upstream)?;
    Ok(
        json!({"verified_reference":reference["id"],"source_bindings":reference["applies_to"]["source_sha256"].as_object().unwrap().len(),"reviewed_working_tree_variants":reference["applies_to"]["working_tree_variants"].as_object().map_or(0,|v|v.len()),"bytes":data.len()}),
    )
}

pub fn component(name: &str) -> Result<String> {
    util::safe_path(name)?;
    let parts = name.split('/').collect::<Vec<_>>();
    if parts.starts_with(&["third_party", "externals"]) {
        ensure!(
            parts.len() > 3 && EXTERNALS.contains(&parts[2]),
            "Review newly used native component: {name}"
        );
        return Ok(parts[2].into());
    }
    if parts[0] == "third_party" {
        ensure!(
            parts.len() > 2
                && (EXTERNALS.contains(&parts[1]) || ["freetype2", "icu_bidi"].contains(&parts[1])),
            "Review newly used Skia wrapper: {name}"
        );
    }
    Ok("skia".into())
}

pub fn query(build: &Path, args: &[&str], ninja: &str) -> Result<String> {
    let out = process::output_limited(
        Command::new(ninja)
            .arg("-C")
            .arg(build)
            .arg("-t")
            .args(args),
        Duration::from_secs(60),
        256 * 1024 * 1024,
    )?;
    ensure!(
        !out.timed_out && out.status.success(),
        "Ninja inventory query failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn notice_candidate(path: &Path) -> bool {
    path.to_str().is_some_and(super::rust::is_notice)
        || path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| ["README.ijg", "README.chromium"].contains(&name))
}

pub fn build(source: &Path, directory: &Path, ninja: &str) -> Result<(Value, Files)> {
    let source = source.canonicalize()?;
    let directory = directory.canonicalize()?;
    let mut arguments = vec!["inputs"];
    arguments.extend(TARGETS);
    let graph = query(&directory, &arguments, ninja)?;
    let headers = query(&directory, &["deps"], ninja)?;
    build_from_graph(&source, &directory, &graph, &headers)
}

pub fn build_from_graph(
    source: &Path,
    directory: &Path,
    graph: &str,
    headers: &str,
) -> Result<(Value, Files)> {
    let source = source.canonicalize()?;
    let directory = directory.canonicalize()?;
    ensure!(
        source.is_dir() && directory.is_dir(),
        "Provide existing Skia source/build directories"
    );
    let graph = graph
        .lines()
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>();
    let headers = headers
        .lines()
        .filter_map(|s| s.strip_prefix("    ").map(str::trim))
        .collect::<BTreeSet<_>>();
    ensure!(!headers.is_empty(), "No recorded Skia headers");
    let mut paths = BTreeMap::<String, BTreeSet<&str>>::new();
    let mut generated = BTreeMap::new();
    let mut excluded = BTreeMap::<String, u64>::new();
    for (kind, names) in [("graph", graph), ("recorded_header", headers)] {
        for name in names {
            let original = directory.join(name);
            let lexical = lexical(&original);
            let path = if original.exists() {
                original.canonicalize()?
            } else {
                lexical.clone()
            };
            for root in [&source, &directory] {
                if lexical.starts_with(root) {
                    ensure!(
                        !original.is_symlink() && path.starts_with(root),
                        "Native input link escapes its root"
                    );
                }
            }
            if path.starts_with(&source) {
                paths
                    .entry(
                        path.strip_prefix(&source)?
                            .to_str()
                            .context("Invalid native source name")?
                            .replace('\\', "/"),
                    )
                    .or_default()
                    .insert(kind);
            } else if path.starts_with(&directory) {
                let name = path
                    .strip_prefix(&directory)?
                    .to_str()
                    .context("Invalid native build name")?
                    .replace('\\', "/");
                if path.is_file() {
                    generated.insert(
                        name.clone(),
                        fingerprint(&directory, &name, 512 * 1024 * 1024)?,
                    );
                } else {
                    let extension = path
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    ensure!(
                        kind == "graph"
                            && !["a", "lib", "o", "obj", "h", "c", "cc", "cpp", "s"]
                                .contains(&extension.as_str()),
                        "Missing generated native input"
                    );
                }
            } else {
                ensure!(
                    kind == "recorded_header",
                    "Graph input escapes source/build roots: {name}"
                );
                *excluded
                    .entry("system_or_external_headers".into())
                    .or_default() += 1;
            }
        }
    }
    ensure!(
        !paths.is_empty()
            && generated
                .keys()
                .any(|s| s.ends_with(".a") || s.ends_with(".lib")),
        "No built native libraries or source inputs"
    );
    let referenced = Regex::new(r"(?i)Chromium source repository LICENSE")?;
    let mut inputs = Vec::new();
    let mut comments = Vec::new();
    let mut used = BTreeSet::new();
    let mut findings = BTreeSet::new();
    for (relative, kinds) in paths {
        let group = component(&relative)?;
        used.insert(group.clone());
        let bytes = util::read(&source.join(&relative), 32 * 1024 * 1024)?;
        let mut record = json!({"path":relative,"sha256":util::digest(&bytes),"bytes":bytes.len(),"component":group,"observed_in":kinds});
        let extension = Path::new(&relative)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ["c", "cc", "cpp", "h", "hh", "hpp", "inc", "s"].contains(&extension.as_str()) {
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]);
            if referenced.is_match(&text) {
                let crlf = bytes.windows(2).filter(|p| *p == b"\r\n").count();
                let mut lf = Vec::new();
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i..].starts_with(b"\r\n") {
                        lf.push(b'\n');
                        i += 2;
                    } else {
                        lf.push(bytes[i]);
                        i += 1;
                    }
                }
                record["reference_line_endings"] = json!({"lf_sha256":util::digest(&lf),"lf_git_blob_sha1":super::git_blob(&lf),"crlf_pairs":crlf,"bare_lf":bytes.iter().filter(|b|**b==b'\n').count()-crlf,"bare_cr":bytes.iter().filter(|b|**b==b'\r').count()-crlf});
            }
            for comment in super::source_comments(&relative, &text)? {
                if referenced.is_match(string(&comment["text"])?) {
                    findings.insert("Chromium BSD terms referenced by source comments require a separate original license; the zlib LICENSE alone is insufficient.".to_owned());
                }
                comments.push(comment);
            }
        }
        inputs.push(record);
    }
    let mut texts = Files::new();
    let mut notices = Vec::new();
    let mut candidates = [
        source.join("LICENSE"),
        source.join("AUTHORS"),
        source.join("DEPS"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    for name in used.iter().filter(|s| s.as_str() != "skia") {
        let root = source.join("third_party/externals").join(name);
        let (label, required): (&str, &[&str]) = match name.as_str() {
            "freetype" => ("FreeType", FREETYPE_TERMS),
            "libjpeg-turbo" => ("libjpeg-turbo", JPEG_TERMS),
            "libwebp" => ("libwebp", WEBP_TERMS),
            _ => ("", &[]),
        };
        for relative in required {
            let path = root.join(relative);
            ensure!(
                path.is_file() && !path.is_symlink(),
                "Missing original {label} terms: {relative}"
            );
            candidates.insert(path);
        }
        let mut primary = false;
        for path in util::walk_filtered(&root, |p| {
            p.components().any(|c| c.as_os_str() == ".git")
                // Upstream trees contain unrelated aliases, such as Expat's README.md.
                // Do not follow them; selected notices and observed inputs still reject links.
                || (!notice_candidate(p) && root.join(p).is_symlink())
        })? {
            let relative = path
                .strip_prefix(&root)?
                .to_str()
                .context("Invalid native notice filename")?;
            if relative.split('/').any(|s| s == ".git") {
                continue;
            }
            if notice_candidate(Path::new(relative)) {
                if !relative.contains('/') && super::rust::is_notice(relative) {
                    primary = true;
                }
                candidates.insert(path);
            }
        }
        ensure!(
            primary,
            "No original primary notice for native component: {name}"
        );
    }
    for path in candidates {
        ensure!(
            path.canonicalize()?.starts_with(&source),
            "Native notice escapes Skia source"
        );
        let relative = path
            .strip_prefix(&source)?
            .to_str()
            .context("Invalid notice name")?;
        let bytes = util::read(&path, util::MAX_FILE)?;
        notices.push(json!({"path":relative,"sha256":util::digest(&bytes),"blob":super::blob(bytes,&mut texts)}));
    }
    let build_files = ["args.gn", "build.ninja", "toolchain.ninja"]
        .iter()
        .map(|name| fingerprint(&directory, name, 512 * 1024 * 1024))
        .collect::<Result<Vec<_>>>()?;
    Ok((
        json!({"format_version":1,"scope":"Observed Skia build inputs and original notice candidates; binary linkage and complete attribution review are not established.",
        "targets":TARGETS,"components":used,"inputs":inputs,"generated_inputs":generated.into_values().collect::<Vec<_>>(),"build_files":build_files,"excluded_inputs":excluded,"source_comments":comments,"notice_files":notices,"review_findings":findings,"license_references":[],
        "limits":{"source_comments":"License/copyright comment candidates in the first 65536 bytes of each observed C/C++/assembly source or header.","recorded_headers":"All recorded objects in this build directory, which can include unused or stale targets; missing files are rejected.","excluded":"System/external headers, Rust toolchain, QuickJS, fonts, icons, and optional runtimes need separate inventories."}}),
        texts,
    ))
}

pub fn apply_reference(index: &mut Value, texts: &mut Files, upstream: &Path) -> Result<()> {
    let (mut reference, data) = reference(upstream)?;
    let re = Regex::new(r"(?i)Chromium source repository LICENSE")?;
    let expected = index["source_comments"]
        .as_array()
        .context("Missing source comments")?
        .iter()
        .filter(|c| re.is_match(c["text"].as_str().unwrap_or_default()))
        .map(|c| c["path"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    if expected.is_empty() {
        return Ok(());
    }
    let deps = index["notice_files"]
        .as_array()
        .context("Missing native notices")?
        .iter()
        .find(|v| v["path"] == "DEPS")
        .context("Missing Skia DEPS")?;
    let applies = &reference["applies_to"];
    ensure!(
        applies["skia_deps_sha256"] == deps["sha256"],
        "Native reference belongs to another Skia DEPS"
    );
    let empty = serde_json::Map::new();
    let variants = applies["working_tree_variants"]
        .as_object()
        .unwrap_or(&empty);
    if !variants.is_empty() {
        let origin = &applies["source_origin"];
        let declaration = regex::escape(&format!(
            "{}@{}",
            string(&origin["repository"])?,
            string(&origin["revision"])?
        ));
        let pattern = Regex::new(&format!(
            r#"(?m)^\s*["']third_party/externals/zlib["']\s*:\s*["']{declaration}["']"#
        ))?;
        ensure!(
            pattern.is_match(std::str::from_utf8(
                texts
                    .get(string(&deps["blob"])?)
                    .context("Missing DEPS bytes")?
            )?),
            "Native variant origin differs from Skia dependency"
        );
    }
    let inputs = index["inputs"]
        .as_array()
        .context("Missing native inputs")?
        .iter()
        .map(|v| Ok((string(&v["path"])?, v)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut forms = json!({});
    for path in &expected {
        let item = inputs
            .get(path.as_str())
            .context("Missing referenced native input")?;
        let canonical = &applies["source_sha256"][path];
        if item["sha256"] == *canonical {
            forms[path] = json!({"form":"original_bytes","sha256":item["sha256"]});
            continue;
        }
        let variant = variants
            .get(path)
            .context("Review changed source referring to Chromium terms")?;
        let endings = &item["reference_line_endings"];
        ensure!(
            item["sha256"] == variant["sha256"]
                && endings["crlf_pairs"].as_u64().is_some_and(|n| n > 0)
                && endings["bare_cr"] == 0
                && endings["bare_lf"] == 0
                && endings["lf_sha256"] == *canonical
                && endings["lf_git_blob_sha1"] == variant["source_git_blob_sha1"],
            "Review changed source referring to Chromium license terms: {path}"
        );
        forms[path] = json!({"form":"reviewed_lf_to_crlf","sha256":item["sha256"]});
    }
    reference["observed_source_paths"] = json!(expected);
    reference["observed_source_forms"] = forms;
    reference["blob"] = json!(super::blob(data, texts));
    index["license_references"] = json!([reference]);
    index["review_findings"]
        .as_array_mut()
        .context("Missing findings")?
        .retain(|v| {
            !v.as_str()
                .unwrap_or_default()
                .starts_with("Chromium BSD terms referenced by source comments")
        });
    Ok(())
}

pub fn files(index: &Value, mut texts: Files) -> Result<Files> {
    let mut lines = vec![
        "Skia build notice inventory".into(),
        "".into(),
        string(&index["scope"])?.into(),
        "".into(),
        format!(
            "Observed components: {}",
            index["components"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "".into(),
        "Required review:".into(),
    ];
    lines.extend(
        index["review_findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned()),
    );
    if index["components"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "freetype")
    {
        lines.extend([
            "".into(),
            "FreeType is used under the FreeType License (FTL).".into(),
            "This software is based in part on the work of the FreeType Team (https://freetype.org).".into(),
            "The alternative GPLv2 text and separately licensed contributions are retained below.".into(),
        ]);
    }
    if index["components"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "libjpeg-turbo")
    {
        lines.extend([
            "".into(),
            "This software is based in part on the work of the Independent JPEG Group.".into(),
        ]);
    }
    lines.push("".into());
    for record in index["notice_files"].as_array().unwrap() {
        lines.extend([
            "=".repeat(72),
            string(&record["path"])?.into(),
            "=".repeat(72),
            String::from_utf8_lossy(
                texts
                    .get(string(&record["blob"])?)
                    .context("Missing native notice bytes")?,
            )
            .into_owned(),
            "".into(),
        ]);
    }
    for record in index["license_references"].as_array().unwrap() {
        lines.extend([
            "=".repeat(72),
            format!("Referenced upstream terms: {}", string(&record["id"])?),
            string(&record["url"])?.into(),
            "Source copyright notices are retained separately below.".into(),
            "=".repeat(72),
            String::from_utf8_lossy(
                texts
                    .get(string(&record["blob"])?)
                    .context("Missing native reference bytes")?,
            )
            .into_owned(),
            "".into(),
        ]);
    }
    lines.extend([
        "Source comment candidates (original locations; not license classifications)".into(),
        "".into(),
    ]);
    for record in index["source_comments"].as_array().unwrap() {
        lines.extend([
            format!("{}:{}", string(&record["path"])?, record["line"]),
            string(&record["text"])?.into(),
            "".into(),
        ]);
    }
    texts.insert("inventory.json".into(), util::encode(index)?);
    texts.insert(
        "SKIA-NOTICES.txt".into(),
        (lines.join("\n").replace("\r\n", "\n") + "\n").into_bytes(),
    );
    Ok(texts)
}

pub fn collect(
    source: &Path,
    directory: &Path,
    output: &Path,
    ninja: &str,
    upstream: Option<&Path>,
) -> Result<Value> {
    let (mut index, mut texts) = build(source, directory, ninja)?;
    if let Some(upstream) = upstream {
        apply_reference(&mut index, &mut texts, upstream)?;
    }
    super::publish_bundle(output, &files(&index, texts)?)?;
    Ok(
        json!({"components":index["components"],"source_inputs":index["inputs"].as_array().unwrap().len(),"notice_files":index["notice_files"].as_array().unwrap().len(),"review_findings":index["review_findings"]}),
    )
}

pub fn verify(
    source: &Path,
    directory: &Path,
    output: &Path,
    ninja: &str,
    upstream: Option<&Path>,
) -> Result<Value> {
    let (mut index, mut texts) = build(source, directory, ninja)?;
    if let Some(upstream) = upstream {
        apply_reference(&mut index, &mut texts, upstream)?;
    }
    let expected = files(&index, texts)?;
    super::verify_expected(output, &expected)?;
    Ok(
        json!({"verified_files":expected.len()+1,"source_inputs":index["inputs"].as_array().unwrap().len(),"review_findings":index["review_findings"]}),
    )
}
