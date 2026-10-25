use crate::util;
use anyhow::{Result, ensure};
use clap::Subcommand;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Subcommand)]
pub enum Command {
    Fonts {
        #[arg(long)]
        output: PathBuf,
    },
    Performance {
        #[arg(value_parser=["shadow","dashboard"])]
        scene: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Fonts { output } => fonts::write(&output),
            Self::Performance { scene, output_dir } => {
                util::print(&performance(&output_dir, &scene)?)
            }
        }
    }
}
pub fn performance(output_dir: &Path, scene: &str) -> Result<serde_json::Value> {
    ensure!(
        ["shadow", "dashboard"].contains(&scene),
        "Unknown performance scene"
    );
    fs::create_dir(&output_dir)?;
    let source = util::root().join("tests/fixtures/performance");
    let document = source.join(format!("{scene}.pen"));
    let bytes = util::read(&document, util::MAX_FILE)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value["version"] == "2.21",
        "Unexpected performance fixture version"
    );
    fs::write(output_dir.join(format!("{scene}.pen")), &bytes)?;
    let mut report = serde_json::json!({"document_sha256":util::digest(bytes)});
    if scene == "dashboard" {
        let asset = util::read(&source.join("avatar.png"), util::MAX_FILE)?;
        fs::write(output_dir.join("avatar.png"), &asset)?;
        report["asset_sha256"] = serde_json::json!(util::digest(asset));
    }
    Ok(report)
}
pub mod fonts;
