//! Source-bound release declarations, independent of the application crate.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SOURCE_PATH: &str = "resources/release.json";
pub const PACKAGE_PATH: &str = "RELEASE.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Limitation {
    pub id: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub format_version: u32,
    pub version: String,
    pub channel: String,
    pub target: String,
    pub minimum_os: String,
    pub limitations: Vec<Limitation>,
}

impl Release {
    pub fn parse(bytes: &[u8], version: &str) -> Result<Self> {
        ensure!(bytes.len() <= 16 * 1024, "Release declaration is too large");
        let declaration: Self = serde_json::from_slice(bytes)?;
        ensure!(
            declaration.format_version == 1
                && declaration.version == version
                && ["stable", "prerelease"].contains(&declaration.channel.as_str())
                && (declaration.channel != "stable" || !version.contains('-'))
                && declaration.target == "aarch64-apple-darwin",
            "Release declaration differs from the supported package identity"
        );
        let components = declaration.minimum_os.split('.').collect::<Vec<_>>();
        ensure!(
            components.len() == 3 && components.iter().all(|part| part.parse::<u32>().is_ok()),
            "Invalid minimum macOS version"
        );
        let mut ids = BTreeSet::new();
        ensure!(
            declaration.limitations.len() <= 32
                && declaration.limitations.iter().all(|item| {
                    !item.id.is_empty()
                        && item.id.len() <= 128
                        && item.id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                        && ids.insert(item.id.clone())
                        && !item.description.trim().is_empty()
                        && item.description.len() <= 4096
                }),
            "Invalid or duplicate release limitation"
        );
        Ok(declaration)
    }

    pub fn ready_for(&self, target: &str) -> bool {
        self.channel == "stable" && self.target == target
    }
}

pub fn from_source(files: &super::Files, package: &serde_json::Value) -> Result<Option<Release>> {
    files
        .get(SOURCE_PATH)
        .map(|(bytes, _)| Release::parse(bytes, package["version"].as_str().unwrap_or_default()))
        .transpose()
}
