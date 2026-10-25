use crate::{
    dist,
    notices::{self, assets, quickjs, skia, toolchain},
    process, shader, util,
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const MAX_EVENTS: u64 = 64 * 1024 * 1024;
const MAX_LINE: usize = 4 * 1024 * 1024;
const GAPS: &[&str] = &[
    "Cargo declarations and Skia graph inputs do not establish final binary linkage or used archive members.",
    "System-library attribution and final Rust source-to-binary/linkage verification remain required.",
    "QuickJS source comment candidates and the skia-bindings wrapper still require final native attribution review.",
    "Font/icon source declarations and original notices are collected; final applicability, other assets, and optional runtimes still require review.",
    "Complete legal review, signed build provenance, packaging, signing, and publication remain required.",
];

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    target: PathBuf,
    #[arg(long)]
    cargo_json: PathBuf,
    #[arg(long)]
    revision: String,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "rustc")]
    rustc: String,
    #[arg(long, default_value = "ninja")]
    ninja: String,
    #[arg(long)]
    cargo_home: Option<PathBuf>,
    #[arg(long)]
    toolchain_evidence: Option<PathBuf>,
}
impl Arguments {
    pub fn run(self) -> Result<()> {
        util::print(&collect(&self)?)
    }
}

fn string(v: &Value) -> Result<&str> {
    v.as_str().context("Missing build evidence string")
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("Missing build evidence array")
}

pub fn inside(path: &Path, root: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "Build evidence requires absolute paths");
    let actual = path.canonicalize()?;
    let mut lexical = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            std::path::Component::CurDir => {}
            c => lexical.push(c.as_os_str()),
        }
    }
    ensure!(
        actual.starts_with(root) && actual == lexical,
        "Build input escapes or aliases selected root: {}",
        path.display()
    );
    Ok(actual)
}

pub fn fingerprint(path: &Path, name: &str, maximum: u64) -> Result<Value> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.is_symlink() && metadata.len() <= maximum,
        "Expected bounded regular evidence input"
    );
    Ok(json!({"path":name,"bytes":metadata.len(),"sha256":util::file_digest(path)?}))
}

pub fn source_identity(root: &Path, revision: &str) -> Result<Value> {
    let manifest = dist::verify_tree(root, Some(revision))?;
    Ok(
        json!({"revision":revision,"manifest_sha256":util::file_digest(&root.join(dist::MANIFEST))?,"files":manifest.files.len(),"cargo_lock":fingerprint(&root.join("Cargo.lock"),"Cargo.lock",512*1024*1024)?,"license":fingerprint(&root.join("LICENSE"),"LICENSE",512*1024*1024)?,"package":manifest.package}),
    )
}

pub fn cargo_observations(
    events: &Path,
    root: &Path,
    target: &Path,
) -> Result<(PathBuf, BTreeMap<String, Value>, u64)> {
    ensure!(
        events.is_file() && !events.is_symlink() && fs::metadata(events)?.len() <= MAX_EVENTS,
        "Expected bounded Cargo JSON output"
    );
    let mut stream = BufReader::new(fs::File::open(events)?);
    let mut artifacts = Vec::new();
    let mut builds = BTreeMap::<String, Vec<Value>>::new();
    let mut finished = Vec::new();
    let mut ignored = 0;
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let count = stream
            .by_ref()
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(
            count <= MAX_LINE && total <= MAX_EVENTS,
            "Cargo event byte limit exceeded"
        );
        let item = match serde_json::from_slice::<Value>(&line) {
            Ok(item) if item.is_object() => item,
            _ => {
                ignored += 1;
                continue;
            }
        };
        ensure!(
            finished.is_empty(),
            "Cargo records follow completion record"
        );
        match item["reason"].as_str().unwrap_or_default() {
            "compiler-artifact" => artifacts.push(item),
            "build-script-executed" => {
                builds
                    .entry(string(&item["package_id"])?.into())
                    .or_default()
                    .push(item);
            }
            "build-finished" => finished.push(item["success"].clone()),
            _ => {}
        }
    }
    ensure!(
        finished == vec![json!(true)],
        "Cargo release build did not finish successfully"
    );
    let binaries = artifacts
        .iter()
        .filter(|a| {
            a["target"]["name"] == "lazypencil"
                && a["target"]["kind"] == json!(["bin"])
                && !a["executable"].is_null()
        })
        .collect::<Vec<_>>();
    ensure!(
        binaries.len() == 1,
        "Expected exactly one lazypencil executable"
    );
    let binary = binaries[0];
    ensure!(
        Path::new(string(&binary["manifest_path"])?).canonicalize()? == root.join("Cargo.toml")
            && binary["profile"]["test"] == false
            && binary["profile"]["opt_level"] == "3"
            && binary["fresh"] == false,
        "Expected newly compiled distributed release executable"
    );
    let executable = inside(Path::new(string(&binary["executable"])?), target)?;
    ensure!(
        executable.parent() == Some(target.join("release").as_path())
            && ["lazypencil", "lazypencil.exe"]
                .contains(&executable.file_name().unwrap().to_str().unwrap_or_default()),
        "Unexpected release executable location"
    );
    let mut selected = BTreeMap::new();
    for name in ["skia_bindings", "rquickjs_sys"] {
        let matches = artifacts
            .iter()
            .filter(|a| {
                a["target"]["name"] == name
                    && a["target"]["kind"] == json!(["lib"])
                    && a["profile"]["test"] == false
            })
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "Missing or ambiguous native artifact: {name}"
        );
        let artifact = matches[0];
        let candidates = builds
            .get(string(&artifact["package_id"])?)
            .context("Missing native build-script output")?;
        ensure!(
            candidates.len() == 1,
            "Ambiguous selected native build-script output"
        );
        let build = &candidates[0];
        let out = inside(Path::new(string(&build["out_dir"])?), target)?;
        ensure!(
            out.is_dir() && out.starts_with(target.join("release/build")),
            "Native output is outside release build directory"
        );
        for (field, maximum) in [("linked_libs", 1024), ("linked_paths", 4096)] {
            ensure!(
                array(&build[field])?.iter().all(|v| v
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= maximum)),
                "Invalid native link declarations"
            );
        }
        selected.insert(name.into(),json!({"package_id":artifact["package_id"],"manifest_path":artifact["manifest_path"],"features":artifact["features"],"linked_libs":build["linked_libs"],"linked_paths":build["linked_paths"],"out_dir":out.strip_prefix(target)?.to_str().context("Invalid output path")?.replace('\\',"/")}));
    }
    Ok((executable, selected, ignored))
}

pub fn native_archives(
    target: &Path,
    selected: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Vec<Value>>> {
    let mut result = BTreeMap::new();
    for (name, record) in selected {
        let expected = match name.as_str() {
            "skia_bindings" => skia::TARGETS
                .iter()
                .copied()
                .chain(std::iter::once("skia-bindings"))
                .collect::<BTreeSet<_>>(),
            "rquickjs_sys" => ["quickjs"].into(),
            _ => anyhow::bail!("Unknown selected native package"),
        };
        let out = target.join(string(&record["out_dir"])?);
        let directory = inside(
            &if name == "skia_bindings" {
                out.join("skia")
            } else {
                out
            },
            target,
        )?;
        let declared = array(&record["linked_libs"])?
            .iter()
            .map(|v| {
                Ok(string(v)?
                    .split_once('=')
                    .map_or(string(v).unwrap(), |(_, s)| s))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        ensure!(
            expected.is_subset(&declared),
            "Missing expected native library declarations"
        );
        if name == "skia_bindings" {
            ensure!(
                !declared.contains("svg") && !declared.contains("skresources"),
                "Review new Skia graph targets"
            );
        }
        let searches = array(&record["linked_paths"])?
            .iter()
            .map(|v| {
                Ok(PathBuf::from(
                    string(v)?
                        .split_once('=')
                        .map_or(string(v).unwrap(), |(_, s)| s),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            searches
                .iter()
                .any(|p| p.canonicalize().is_ok_and(|p| p == directory)),
            "Native library directory was not declared to Cargo"
        );
        let mut files = Vec::new();
        for library in expected {
            let candidates = [
                directory.join(format!("lib{library}.a")),
                directory.join(format!("{library}.lib")),
            ]
            .into_iter()
            .filter(|p| p.exists() || p.is_symlink())
            .collect::<Vec<_>>();
            ensure!(!candidates.is_empty(), "Missing native archive: {library}");
            let mut observed = Vec::new();
            for candidate in candidates {
                let path = inside(&candidate, target)?;
                observed.push(fingerprint(
                    &path,
                    path.strip_prefix(target)?
                        .to_str()
                        .context("Invalid archive path")?,
                    512 * 1024 * 1024,
                )?);
            }
            ensure!(
                observed
                    .iter()
                    .map(|v| (v["sha256"].as_str(), v["bytes"].as_u64()))
                    .collect::<BTreeSet<_>>()
                    .len()
                    == 1,
                "Conflicting native archive aliases"
            );
            files.extend(observed);
        }
        result.insert(name.clone(), files);
    }
    Ok(result)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for path in util::walk(source)? {
        let target = destination.join(path.strip_prefix(source)?);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(path, target)?;
    }
    Ok(())
}

pub fn collect(arguments: &Arguments) -> Result<Value> {
    let Arguments {
        source: root,
        target,
        cargo_json: events,
        revision,
        output,
        rustc,
        ninja,
        cargo_home: cargo,
        toolchain_evidence: standard_evidence,
    } = arguments;
    let cargo = cargo.as_deref();
    let standard_evidence = standard_evidence.as_deref();
    let root = root.canonicalize()?;
    let target = target.canonicalize()?;
    let output = std::path::absolute(output)?;
    ensure!(
        !output.exists()
            && !output.is_symlink()
            && !output.starts_with(&root)
            && !output.starts_with(&target),
        "Evidence output must be new and outside source/build roots"
    );
    let source = source_identity(&root, revision)?;
    let cargo_output = fingerprint(events, "cargo-build.jsonl", MAX_EVENTS)?;
    let (executable, selected, ignored) = cargo_observations(events, &root, &target)?;
    let binary = fingerprint(
        &executable,
        executable
            .strip_prefix(&target)?
            .to_str()
            .context("Invalid executable path")?,
        512 * 1024 * 1024,
    )?;
    let archives = native_archives(&target, &selected)?;
    let rust = notices::rust::verify(&root, &root.join("resources/licenses/rust"), None)?;
    let shader = shader::verify(&root, None)?;
    let compiler = process::checked(Command::new(rustc).arg("-vV"), Duration::from_secs(30))?;
    ensure!(
        compiler.len() <= 65536,
        "Compiler identity exceeds byte budget"
    );
    let compiler = String::from_utf8(compiler)?;
    let host = regex::Regex::new(r"(?m)^host: ([A-Za-z0-9_.-]+)$")?
        .captures(&compiler)
        .context("Compiler does not report host target")?[1]
        .to_owned();
    let cargo = cargo
        .map(Path::to_owned)
        .unwrap_or(notices::cargo_home_default()?);
    let quick = &selected["rquickjs_sys"];
    let manifest = PathBuf::from(string(&quick["manifest_path"])?);
    let quick_out = target.join(string(&quick["out_dir"])?);
    let features = array(&quick["features"])?
        .iter()
        .map(|v| Ok(string(v)?.to_owned()))
        .collect::<Result<Vec<_>>>()?;
    let skia_source = inside(&root.join("vendor/skia-bindings/skia"), &root)?;
    let skia_build = target
        .join(string(&selected["skia_bindings"]["out_dir"])?)
        .join("skia");
    let upstream = root.join("resources/licenses/native/upstream.json");
    let parent = output.parent().context("Missing evidence output parent")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix("release-evidence-")
        .tempdir_in(parent)?;
    let stage = temporary.path().join("evidence");
    fs::create_dir(&stage)?;
    let skia = skia::collect(
        &skia_source,
        &skia_build,
        &stage.join("skia"),
        ninja,
        Some(&upstream),
    )?;
    skia::verify(
        &skia_source,
        &skia_build,
        &stage.join("skia"),
        ninja,
        Some(&upstream),
    )?;
    let quickjs = quickjs::collect(
        &root,
        &manifest,
        &quick_out,
        &features,
        &cargo,
        &host,
        &stage.join("quickjs"),
    )?;
    quickjs::verify(
        &root,
        &manifest,
        &quick_out,
        &features,
        &cargo,
        &host,
        &stage.join("quickjs"),
    )?;
    let assets = assets::collect(&root, &stage.join("assets"))?;
    assets::verify(&root, &stage.join("assets"))?;
    let standard = standard_evidence
        .map(|path| toolchain::attach(path, &stage.join("toolchain"), &compiler, rustc))
        .transpose()?;
    let mut gaps = GAPS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    if standard.is_none() {
        gaps.push("Rust standard-library evidence has not been attached to this build.".into());
    }
    ensure!(
        source_identity(&root, revision)? == source,
        "Sources changed during evidence collection"
    );
    ensure!(
        fingerprint(&executable, string(&binary["path"])?, 512 * 1024 * 1024)? == binary
            && native_archives(&target, &selected)? == archives,
        "Build outputs changed during collection"
    );
    ensure!(
        fingerprint(events, "cargo-build.jsonl", MAX_EVENTS)? == cargo_output,
        "Cargo observations changed during collection"
    );
    copy_tree(&root.join("resources/licenses"), &stage.join("licenses"))?;
    notices::rust::verify(&root, &stage.join("licenses/rust"), None)?;
    copy_tree(
        &root.join("resources/shader-preprocessor"),
        &stage.join("shader-preprocessor"),
    )?;
    ensure!(
        source_identity(&root, revision)? == source,
        "Sources changed while copying original notices"
    );
    fs::copy(root.join(dist::MANIFEST), stage.join(dist::MANIFEST))?;
    fs::copy(root.join("LICENSE"), stage.join("LICENSE"))?;
    let hosted = [
        "GITHUB_REPOSITORY",
        "GITHUB_SHA",
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "RUNNER_OS",
        "RUNNER_ARCH",
        "ImageOS",
        "ImageVersion",
    ]
    .into_iter()
    .map(|s| (s, std::env::var(s).ok()))
    .collect::<BTreeMap<_, _>>();
    let metadata = json!({"format_version":1,"release_ready":false,"source":source,"binary":binary,"compiler":compiler,"native_packages":selected,"native_archives":archives,"rust_notices":rust,"skia_notices":skia,"quickjs_notices":quickjs,"asset_notices":assets,"toolchain_notices":standard,"shader_preprocessor":shader,"cargo_output":cargo_output,"ignored_non_cargo_lines":ignored,"remaining_review":gaps,"hosted":hosted});
    util::publish(&stage.join("build.json"), &util::encode(&metadata)?, false)?;
    let files = util::walk(&stage)?
        .iter()
        .map(|p| {
            fingerprint(
                p,
                p.strip_prefix(&stage).unwrap().to_str().unwrap(),
                64 * 1024 * 1024,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    util::publish(
        &stage.join("manifest.json"),
        &util::encode(&json!({"format_version":1,"files":files}))?,
        false,
    )?;
    fs::create_dir(&output)?;
    let publication = (|| -> Result<()> {
        for child in fs::read_dir(&stage)? {
            let child = child?;
            fs::rename(child.path(), output.join(child.file_name()))?;
        }
        Ok(())
    })();
    if publication.is_err() {
        fs::remove_dir_all(&output)?;
    }
    publication?;
    Ok(
        json!({"release_ready":false,"source_revision":revision,"binary_sha256":binary["sha256"],"files":files.len()+1,"remaining_review":gaps}),
    )
}
