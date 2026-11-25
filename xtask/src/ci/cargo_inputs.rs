use anyhow::{Context, Result, ensure};
use serde::Serialize;
use toml::{Table, Value};

#[derive(PartialEq, Serialize)]
pub(super) struct Dependencies {
    manifest: Table,
    lock: Table,
}

pub(super) fn dependencies(manifest: &str, lock: &str) -> Result<(String, Dependencies)> {
    let mut manifest: Table = toml::from_str(manifest)?;
    let package = manifest
        .get_mut("package")
        .and_then(Value::as_table_mut)
        .context("Missing application package")?;
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .context("Missing application package name")?
        .to_owned();
    let Some(Value::String(version)) = package.remove("version") else {
        anyhow::bail!("Missing application package version");
    };
    let mut lock: Table = toml::from_str(lock)?;
    let packages = lock
        .get_mut("package")
        .and_then(Value::as_array_mut)
        .context("Missing locked packages")?;
    let mut application = packages
        .iter_mut()
        .filter_map(Value::as_table_mut)
        .filter(|p| {
            !p.contains_key("source") && p.get("name").and_then(Value::as_str) == Some(&name)
        });
    let package = application.next().context("Missing locked application")?;
    ensure!(application.next().is_none(), "Ambiguous locked application");
    ensure!(
        package.remove("version") == Some(Value::String(version.clone())),
        "Locked application version differs from the manifest"
    );
    Ok((version, Dependencies { manifest, lock }))
}
