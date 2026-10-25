pub mod assets;
pub mod quickjs;
pub mod rust;
pub mod skia;
pub mod toolchain;

use crate::util;
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const MAX_BUNDLE: u64 = 64 * 1024 * 1024;
pub type Files = BTreeMap<String, Vec<u8>>;

#[derive(Subcommand)]
pub enum Command {
    Toolchain {
        #[arg(value_parser=["collect","verify","verify-upstream"])]
        operation: String,
        #[arg(long, default_value = "rustc")]
        rustc: String,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        source_root: Option<PathBuf>,
        #[arg(long)]
        legal_root: Option<PathBuf>,
        #[arg(long)]
        doc_root: Option<PathBuf>,
        #[arg(long)]
        cargo_home: Option<PathBuf>,
        #[arg(long)]
        upstream: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Skia {
        #[arg(value_parser=["collect","verify","verify-upstream"])]
        operation: String,
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        build: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "ninja")]
        ninja: String,
        #[arg(long)]
        upstream: Option<PathBuf>,
    },
    Assets {
        #[arg(value_parser=["collect","verify"])]
        operation: String,
        #[arg(long,default_value_os_t=util::root())]
        root: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Rust {
        #[arg(value_parser=["collect","verify"])]
        operation: String,
        #[arg(long, default_value_os_t=util::root())]
        root: PathBuf,
        #[arg(long)]
        cargo_home: Option<PathBuf>,
        #[arg(long)]
        upstream: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        bundle: Option<PathBuf>,
    },
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Toolchain {
                operation,
                rustc,
                target,
                source_root,
                legal_root,
                doc_root,
                cargo_home,
                upstream,
                output,
            } => {
                if operation == "verify-upstream" {
                    let upstream = upstream.context("--upstream required")?;
                    let packages = toolchain::upstream_policy(Some(&upstream))?;
                    return util::print(
                        &json!({"verified_upstream_packages":packages.len(),"verified_original_files":packages.values().filter_map(|p|p["files"].as_array()).map(Vec::len).sum::<usize>(),"canonical_reference_terms":toolchain::reference_terms(Some(&upstream))?.len(),"registry_applicability_verified":false}),
                    );
                }
                let output = output.context("--output required")?;
                let (identity, sysroot, libraries) = toolchain::observe(&rustc, target.as_deref())?;
                let docs = doc_root.unwrap_or_else(|| {
                    let candidate = sysroot.join("share/doc/rustc");
                    if candidate.join("COPYRIGHT-library.html").is_file() {
                        candidate
                    } else {
                        sysroot.join("share/doc/rust")
                    }
                });
                let (index, texts) = toolchain::build(
                    &source_root.unwrap_or_else(|| sysroot.join("lib/rustlib/src/rust")),
                    &legal_root.unwrap_or(sysroot),
                    &docs,
                    &libraries,
                    &identity,
                    cargo_home.as_deref(),
                    upstream.as_deref(),
                )?;
                let files = toolchain::files(&index, texts)?;
                if operation == "collect" {
                    publish_bundle(&output, &files)?;
                } else {
                    verify_expected(&output, &files)?;
                }
                util::print(
                    &json!({"source_inputs":index["source_inputs"].as_array().unwrap().len(),"vendor_packages":index["vendor_packages"].as_array().unwrap().len(),"registry_packages":index["registry_packages"].as_array().unwrap().len(),"registry_archives_verified":index["registry_archives_verified"],"target_libraries":index["target_libraries"].as_array().unwrap().len(),"bundle_files":files.len()+1,"review_findings":index["review_findings"]}),
                )
            }
            Self::Skia {
                operation,
                source,
                build,
                output,
                ninja,
                upstream,
            } => {
                let report = if operation == "verify-upstream" {
                    skia::verify_upstream(&upstream.context("--upstream required")?)?
                } else {
                    let source = source.context("--source required")?;
                    let build = build.context("--build required")?;
                    let output = output.context("--output required")?;
                    if operation == "collect" {
                        skia::collect(&source, &build, &output, &ninja, upstream.as_deref())?
                    } else {
                        skia::verify(&source, &build, &output, &ninja, upstream.as_deref())?
                    }
                };
                util::print(&report)
            }
            Self::Assets {
                operation,
                root,
                output,
            } => util::print(&if operation == "collect" {
                assets::collect(&root, &output)?
            } else {
                assets::verify(&root, &output)?
            }),
            Self::Rust {
                operation,
                root,
                cargo_home,
                upstream,
                output,
                bundle,
            } => {
                let result = if operation == "collect" {
                    rust::collect(
                        &root,
                        &cargo_home.unwrap_or(cargo_home_default()?),
                        &upstream.context("--upstream is required")?,
                        &output.context("--output is required")?,
                    )?
                } else {
                    rust::verify(
                        &root,
                        &bundle.unwrap_or_else(|| root.join("resources/licenses/rust")),
                        cargo_home.as_deref(),
                    )?
                };
                util::print(&result)
            }
        }
    }
}

pub fn cargo_home_default() -> Result<PathBuf> {
    Ok(std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or(
        PathBuf::from(std::env::var_os("HOME").context("CARGO_HOME or HOME is required")?)
            .join(".cargo"),
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub files: Vec<Member>,
}

pub fn manifest(files: &Files) -> Result<Vec<u8>> {
    util::encode(&Manifest {
        format_version: 1,
        files: files
            .iter()
            .map(|(path, bytes)| Member {
                path: path.clone(),
                bytes: bytes.len() as u64,
                sha256: util::digest(bytes),
            })
            .collect(),
    })
}

pub fn verify_bundle(root: &Path, allowed: impl Fn(&str) -> bool) -> Result<Files> {
    ensure!(
        !root.is_symlink() && root.is_dir(),
        "Expected regular notice directory"
    );
    let manifest: Manifest = util::json(&root.join("manifest.json"))?;
    ensure!(
        manifest.format_version == 1 && manifest.files.len() <= 10_000,
        "Unsupported notice manifest"
    );
    let mut files = Files::new();
    let mut total = 0;
    for entry in manifest.files {
        util::safe_path(&entry.path)?;
        ensure!(
            entry.path != "manifest.json"
                && allowed(&entry.path)
                && !files.contains_key(&entry.path),
            "Unexpected or repeated notice member: {}",
            entry.path
        );
        let path = root.join(&entry.path);
        ensure!(
            path.canonicalize()? == root.canonicalize()?.join(&entry.path),
            "Linked notice directory"
        );
        let bytes = util::read(&path, MAX_BUNDLE - total)?;
        total += bytes.len() as u64;
        ensure!(
            bytes.len() as u64 == entry.bytes && util::digest(&bytes) == entry.sha256,
            "Notice integrity mismatch: {}",
            entry.path
        );
        if entry.path.starts_with("texts/") {
            ensure!(
                entry.path == format!("texts/{}.txt", util::digest(&bytes)),
                "Notice blob name mismatch"
            );
        }
        files.insert(entry.path, bytes);
    }
    let actual = util::walk(root)?
        .iter()
        .map(|p| {
            p.strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/")
        })
        .collect::<BTreeSet<_>>();
    let expected = files
        .keys()
        .cloned()
        .chain(std::iter::once("manifest.json".into()))
        .collect::<BTreeSet<_>>();
    ensure!(actual == expected, "Missing or unlisted notice members");
    Ok(files)
}

pub fn publish_bundle(output: &Path, files: &Files) -> Result<()> {
    ensure!(
        !output.exists() && !output.is_symlink(),
        "Notice output must not exist"
    );
    ensure!(
        files.len() <= 10_000 && files.values().map(|v| v.len() as u64).sum::<u64>() <= MAX_BUNDLE,
        "Notice bundle exceeds limits"
    );
    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = tempfile::tempdir_in(parent)?;
    for (name, bytes) in files {
        util::safe_path(name)?;
        let path = temp.path().join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    fs::write(temp.path().join("manifest.json"), manifest(files)?)?;
    fs::create_dir(output)?;
    for entry in fs::read_dir(temp.path())? {
        let entry = entry?;
        fs::rename(entry.path(), output.join(entry.file_name()))?;
    }
    Ok(())
}

pub fn stored_blob(directory: &Path, item: &Value) -> Result<Vec<u8>> {
    let sha = item["sha256"].as_str().context("Missing notice hash")?;
    ensure!(
        sha.len() == 64
            && sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Invalid notice hash"
    );
    let name = format!("texts/{sha}.txt");
    ensure!(item["blob"] == name, "Unexpected notice blob path");
    let bytes = util::read(&directory.join(name), util::MAX_FILE)?;
    ensure!(util::digest(&bytes) == sha, "Notice blob checksum mismatch");
    Ok(bytes)
}

pub fn blob(data: Vec<u8>, texts: &mut Files) -> String {
    let name = format!("texts/{}.txt", util::digest(&data));
    texts.insert(name.clone(), data);
    name
}

pub fn fingerprint(root: &Path, name: &str, limit: u64) -> Result<Value> {
    util::safe_path(name)?;
    let path = root.join(name);
    ensure!(
        path.canonicalize()? == root.canonicalize()?.join(name),
        "Linked inventory input"
    );
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file() && !metadata.is_symlink() && metadata.len() <= limit,
        "Inventory input exceeds its byte limit"
    );
    Ok(json!({"path":name,"bytes":metadata.len(),"sha256":util::file_digest(&path)?}))
}

pub fn git_blob(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hash = Sha1::new();
    hash.update(format!("blob {}\0", bytes.len()));
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

pub fn source_comments(name: &str, text: &str) -> Result<Vec<Value>> {
    static COMMENTS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?ms)^[ \t]*/\*.*?\*/|^(?:[ \t]*//[^\n]*(?:\n|$))+")
            .expect("Constant source comment pattern")
    });
    static TERMS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)copyright|SPDX-|licen[cs]e").expect("Constant attribution pattern")
    });
    Ok(COMMENTS.find_iter(text).filter(|m|TERMS.is_match(m.as_str())).map(|m|json!({"path":name,"line":text[..m.start()].bytes().filter(|b|*b==b'\n').count()+1,"text":m.as_str()})).collect())
}

pub fn verify_expected(output: &Path, expected: &Files) -> Result<()> {
    let actual = verify_bundle(output, |_| true)?;
    ensure!(
        &actual == expected,
        "Changed, missing or stale notice inventory: {}",
        output.display()
    );
    Ok(())
}
