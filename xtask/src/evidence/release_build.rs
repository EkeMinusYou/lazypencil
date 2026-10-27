//! Shared release compiler policy, distinct from link-map parsing.

use anyhow::{Context, Result, ensure};
use std::path::Path;

pub const RECIPE: &str = "root-rustc-release-map-app-cgu16-v2";

pub fn cargo_arguments(host: &str, target: &Path, json: bool) -> Result<Vec<String>> {
    let extra = rustc_arguments(host, &super::link_map::path(target))?;
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
    if extra.is_some() {
        // Partition only our application package. Dependency, build-script
        // and native compiler settings retain their verified cache identity.
        // Cargo still decides which application units need recompilation.
        arguments.extend([
            "--verbose".into(),
            "--config".into(),
            "profile.release.package.lazypencil.codegen-units=16".into(),
        ]);
    }
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
