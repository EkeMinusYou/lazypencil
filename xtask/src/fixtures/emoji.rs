//! Immutable, unmodified OFL fonts acquired only for hosted compatibility tests.
use crate::{download, util};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Font {
    pub format: String,
    pub asset: Asset,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub repository: String,
    pub revision: String,
    pub copyright: String,
    pub license: Asset,
    pub fonts: Vec<Font>,
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl Pin {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.repository == "googlefonts/noto-emoji"
                && hex(&self.revision, 40)
                && !self.copyright.is_empty()
                && self.copyright.len() < 1024
                && self.license.path == "2D/fonts/LICENSE"
                && self.fonts.len() == 2,
            "Invalid Noto Emoji source pin"
        );
        for (font, (format, path)) in self.fonts.iter().zip([
            ("CBDT/CBLC", "2D/fonts/NotoColorEmoji.ttf"),
            ("COLRv1", "2D/fonts/Noto-COLRv1.ttf"),
        ]) {
            ensure!(
                font.format == format && font.asset.path == path,
                "Unexpected emoji format or path"
            );
        }
        for asset in std::iter::once(&self.license).chain(self.fonts.iter().map(|f| &f.asset)) {
            ensure!(
                hex(&asset.sha256, 64) && (1..=16 * 1024 * 1024).contains(&asset.bytes),
                "Invalid emoji asset checksum or byte bound"
            );
        }
        Ok(())
    }
}

pub fn prepare(catalogue: &Path, output: &Path) -> Result<()> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
        "Hosted emoji fixture acquisition required"
    );
    let pin: Pin = util::json(catalogue)?;
    pin.validate()?;
    ensure!(
        !output.is_symlink(),
        "Emoji fixture directory must not be linked"
    );
    fs::create_dir_all(output)?;
    for asset in std::iter::once(&pin.license).chain(pin.fonts.iter().map(|f| &f.asset)) {
        let filename = Path::new(&asset.path)
            .file_name()
            .expect("validated source path");
        let target = output.join(filename);
        let bytes = if target.exists() || target.is_symlink() {
            download::verify_file(&target, &asset.sha256, asset.bytes)?
        } else {
            let url = format!(
                "https://raw.githubusercontent.com/{}/{}/{}",
                pin.repository, pin.revision, asset.path
            );
            download::download(&url, &asset.sha256, &target, asset.bytes)?
        };
        ensure!(bytes == asset.bytes, "Emoji asset byte length mismatch");
    }
    // Keep the original standalone license beside the unmodified fonts. Their
    // embedded name records retain the upstream copyright and license metadata.
    fs::write(output.join("SOURCE.json"), serde_json::to_vec_pretty(&pin)?)?;
    Ok(())
}
