//! Keep the raw map from the evidence build for final archive-member review.

use super::fingerprint;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const RECIPE: &str = "root-rustc-release-map-v1";
pub const CACHE_PATH: &str = "release/lazypencil.map";

pub fn path(target: &Path) -> PathBuf {
    target.join(CACHE_PATH)
}

pub fn cargo_arguments(host: &str, target: &Path, json: bool) -> Result<Vec<String>> {
    let extra = rustc_arguments(host, &path(target))?;
    let mut arguments = [
        "cargo",
        if extra.is_some() { "rustc" } else { "build" },
        "--locked",
        "--release",
        "--bin",
        "lazypencil",
    ]
    .map(str::to_owned)
    .to_vec();
    if json {
        arguments.push("--message-format=json".into());
    }
    if let Some(extra) = extra {
        arguments.push("--".into());
        arguments.extend(extra);
    }
    Ok(arguments)
}

pub fn rustc_arguments(host: &str, map: &Path) -> Result<Option<Vec<String>>> {
    let option = match host {
        "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu" => "-Wl,-Map=",
        "aarch64-apple-darwin" => "-Wl,-map,",
        _ => return Ok(None),
    };
    let path = map.to_str().context("Link map path must be UTF-8")?;
    // Compiler drivers split commas, and GNU ld expands '%' in map paths.
    ensure!(
        map.is_absolute() && !path.chars().any(|c| c.is_control() || c == ',' || c == '%'),
        "Link map path contains linker-driver metacharacters or is relative"
    );
    Ok(Some(vec!["-C".into(), format!("link-arg={option}{path}")]))
}

pub fn collect(source: &Path, destination: &Path) -> Result<Value> {
    ensure!(
        !destination.exists() && !destination.is_symlink(),
        "Link map output must be new"
    );
    let observe = |path: &Path| fingerprint(path, "link-map.map", 64 * 1024 * 1024);
    let original = observe(source)?;
    ensure!(
        original["bytes"].as_u64().unwrap_or(0) > 0,
        "Empty link map"
    );
    fs::copy(source, destination)?;
    ensure!(
        observe(source)? == original && observe(destination)? == original,
        "Link map changed during collection"
    );
    Ok(original)
}
