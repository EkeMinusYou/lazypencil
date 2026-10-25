use crate::{
    download,
    notices::{self, toolchain},
    util,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const PREFIX: &str = "rustc-1.94.0-src";
const LEGAL: &[&str] = &[
    "COPYRIGHT",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "src/llvm-project/libunwind/LICENSE.TXT",
];

pub fn prepare_source(archive: &Path, destination: &Path) -> Result<(PathBuf, Value)> {
    let source = fs::File::open(archive)?;
    let decoder = lzma_rust2::XzReader::new(source, true);
    let mut archive = tar::Archive::new(decoder);
    let (mut files, mut size) = (0u64, 0u64);
    let mut seen = std::collections::BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = String::from_utf8(entry.path_bytes().into_owned())?;
        let name = name.trim_end_matches('/');
        ensure!(
            name == PREFIX || name.starts_with(&format!("{PREFIX}/")),
            "Unexpected Rust source archive root"
        );
        let relative = name.strip_prefix(&format!("{PREFIX}/")).unwrap_or("");
        if !LEGAL.contains(&relative) && relative != "library" && !relative.starts_with("library/")
        {
            continue;
        }
        util::safe_path(name)?;
        ensure!(
            seen.insert(name.to_owned()),
            "Duplicate standard-library source member"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Linked standard-library source member"
        );
        let path = destination.join(name);
        if kind.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        files += 1;
        size = size
            .checked_add(entry.size())
            .context("Source size overflow")?;
        ensure!(
            files <= 20_000 && entry.size() <= 32 * 1024 * 1024 && size <= 256 * 1024 * 1024,
            "Selected Rust sources exceed byte/file budget"
        );
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        fs::create_dir_all(path.parent().unwrap())?;
        util::publish(&path, &bytes, false)?;
    }
    let source = destination.join(PREFIX);
    for name in [
        "COPYRIGHT",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "library/Cargo.toml",
        "library/Cargo.lock",
    ] {
        let path = source.join(name);
        ensure!(
            path.is_file() && !path.is_symlink(),
            "Missing Rust source input: {name}"
        );
    }
    Ok((
        source,
        json!({"selected_files":files,"selected_bytes":size}),
    ))
}

pub fn collect(root: &Path, output: &Path, target: Option<&str>) -> Result<Value> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
        "Run large-input preparation on GHA"
    );
    ensure!(
        !output.exists() && !output.is_symlink(),
        "Toolchain evidence output must not exist"
    );
    let (identity, sysroot, libraries) = toolchain::observe("rustc", target)?;
    ensure!(
        identity["release"] == toolchain::RELEASE && identity["commit_hash"] == toolchain::COMMIT,
        "Review selected compiler source provenance"
    );
    let work = tempfile::Builder::new()
        .prefix("stdlib-notice-inputs-")
        .tempdir_in(
            std::env::var_os("RUNNER_TEMP").context("Missing hosted temporary directory")?,
        )?;
    let archive = work.path().join("source.tar.xz");
    download::download(
        toolchain::SOURCE_URL,
        toolchain::SOURCE_SHA,
        &archive,
        512 * 1024 * 1024,
    )?;
    let (source, selection) = prepare_source(&archive, work.path())?;
    fs::remove_file(&archive)?;
    let cargo = work.path().join("cargo");
    let cache = cargo.join("registry/cache/stdlib");
    fs::create_dir_all(&cache)?;
    let (_, packages) = notices::rust::locked(&source.join("library"))?;
    let packages = packages
        .values()
        .filter(|p| p.get("source").is_some())
        .collect::<Vec<_>>();
    ensure!(
        packages.len() <= 1000,
        "Too many standard-library registry packages"
    );
    let mut total = 0;
    for package in &packages {
        notices::rust::identifier(package)?;
        ensure!(
            package["source"] == "registry+https://github.com/rust-lang/crates.io-index",
            "Unreviewed standard-library registry"
        );
        let name = package["name"].as_str().unwrap();
        let version = package["version"].as_str().unwrap();
        let filename = format!("{name}-{version}.crate");
        total += download::download(
            &format!("https://static.crates.io/crates/{name}/{filename}"),
            package["checksum"]
                .as_str()
                .context("Missing source checksum")?,
            &cache.join(&filename),
            64 * 1024 * 1024,
        )?;
        ensure!(
            total <= 512 * 1024 * 1024,
            "Standard-library archives exceed total budget"
        );
    }
    let docs = ["rust", "rust/html", "rustc", "rustc/html"]
        .iter()
        .map(|p| sysroot.join("share/doc").join(p))
        .find(|p| p.join("COPYRIGHT-library.html").is_file())
        .context("Install matching rust-docs; COPYRIGHT-library.html missing")?;
    let upstream = root.join("resources/licenses/toolchain/upstream.json");
    let (index, texts) = toolchain::build(
        &source,
        &source,
        &docs,
        &libraries,
        &identity,
        Some(&cargo),
        Some(&upstream),
    )?;
    fs::create_dir(output)?;
    let inventory = output.join("inventory");
    notices::publish_bundle(&inventory, &toolchain::files(&index, texts)?)?;
    let (again, texts) = toolchain::build(
        &source,
        &source,
        &docs,
        &libraries,
        &identity,
        Some(&cargo),
        Some(&upstream),
    )?;
    notices::verify_expected(&inventory, &toolchain::files(&again, texts)?)?;
    let distribution = json!({"format_version":1,"release_ready":false,"observed_toolchain":identity,"source_url":toolchain::SOURCE_URL,"source_sha256":toolchain::SOURCE_SHA,"source_selection":selection,"registry_packages":packages.len(),"registry_download_bytes":total,"scope":"This runner's installed standard-library input evidence; no application binary or final linkage is established.","collector_revision":std::env::var("GITHUB_SHA").ok(),"run_id":std::env::var("GITHUB_RUN_ID").ok()});
    util::publish(
        &output.join("source-distribution.json"),
        &util::encode(&distribution)?,
        false,
    )?;
    Ok(
        json!({"source_inputs":index["source_inputs"].as_array().unwrap().len(),"registry_packages":index["registry_packages"].as_array().unwrap().len(),"target_libraries":index["target_libraries"].as_array().unwrap().len(),"review_findings":index["review_findings"]}),
    )
}
