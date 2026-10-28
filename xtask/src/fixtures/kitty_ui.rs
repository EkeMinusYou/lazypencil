//! Pinned external Kitty for owned macOS GUI tests, never application distribution.

use crate::{download, process, util};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::Duration};

pub(crate) const VERSION: &str = "0.49.2";
const URL: &str = "https://github.com/kovidgoyal/kitty/releases/download/v0.49.2/kitty-0.49.2.dmg";
pub(crate) const SHA256: &str = "e524b894145d89cb76b584e3a9691196ae56bfa31d51384a44aa774252b7cef7";
const BYTES: u64 = 77_825_600;

fn release_host(uri: &ureq::http::Uri) -> Result<()> {
    ensure!(
        uri.scheme_str() == Some("https")
            && matches!(
                uri.host(),
                Some("github.com" | "release-assets.githubusercontent.com")
            ),
        "Unexpected Kitty release download destination"
    );
    Ok(())
}

struct Mounted<'a> {
    path: &'a Path,
    active: bool,
}

impl Mounted<'_> {
    fn detach(&mut self) -> Result<()> {
        process::checked(
            Command::new("/usr/bin/hdiutil")
                .arg("detach")
                .arg(self.path),
            Duration::from_secs(30),
        )?;
        self.active = false;
        Ok(())
    }
}

impl Drop for Mounted<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.detach();
        }
    }
}

pub fn prepare(output: &Path) -> Result<()> {
    ensure!(
        cfg!(all(target_os = "macos", target_arch = "aarch64"))
            && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_OS").as_deref() == Ok("macOS"),
        "Kitty GUI fixtures require an owned Apple Silicon macOS GHA runner"
    );
    let output = std::path::absolute(output)?;
    ensure!(
        !output.exists() && !output.is_symlink(),
        "Kitty fixture output must be new"
    );
    let parent = output.parent().context("Kitty fixture parent required")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir_in(parent)?;
    let archive = temporary.path().join("kitty.dmg");
    // GitHub publishes this digest in the versioned release asset metadata:
    // https://api.github.com/repos/kovidgoyal/kitty/releases/assets/602305400
    // Only GitHub's release-asset redirect is permitted. The regular pinned
    // downloader retains its stricter same-host policy for all other fixtures.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(300)))
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent.get(URL).call()?;
    use ureq::ResponseExt;
    release_host(response.get_uri())?;
    let bytes = download::publish(response.body_mut().as_reader(), SHA256, &archive, BYTES)?;
    ensure!(bytes == BYTES, "Unexpected Kitty archive length");

    let mountpoint = temporary.path().join("mount");
    fs::create_dir(&mountpoint)?;
    let mut mounted = Mounted {
        path: &mountpoint,
        active: true,
    };
    let installed = (|| -> Result<()> {
        process::checked(
            Command::new("/usr/bin/hdiutil")
                .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
                .arg(&mountpoint)
                .arg(&archive),
            Duration::from_secs(60),
        )?;
        let app = mountpoint.join("kitty.app");
        ensure!(
            app.is_dir() && !app.is_symlink(),
            "Kitty app bundle missing"
        );
        process::checked(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&app),
            Duration::from_secs(60),
        )?;
        process::checked(
            Command::new("/usr/bin/ditto")
                .arg(&app)
                .arg(temporary.path().join("kitty.app")),
            Duration::from_secs(60),
        )?;
        Ok(())
    })();
    let detached = mounted.detach();
    installed?;
    detached?;
    let package = temporary.path().join("kitty.app");
    for name in ["kitty", "kitten"] {
        let executable = package.join("Contents/MacOS").join(name);
        let version = process::checked(
            Command::new(&executable).arg("--version"),
            Duration::from_secs(30),
        )?;
        let version = String::from_utf8(version)?;
        ensure!(
            version.split_whitespace().take(2).eq([name, VERSION]),
            "Kitty fixture executable version differs from pin"
        );
    }
    fs::rename(package, &output)?;
    let canonical = output.canonicalize()?;
    let kitty = canonical.join("Contents/MacOS/kitty").canonicalize()?;
    let kitten = canonical.join("Contents/MacOS/kitten").canonicalize()?;
    ensure!(
        kitty.starts_with(&canonical) && kitten.starts_with(&canonical),
        "Kitty executable escaped the owned app bundle"
    );
    let report_path = output.with_extension("setup.json");
    let report = json!({"source_commit":std::env::var("GITHUB_SHA").ok(),
        "version":VERSION,"archive_sha256":SHA256,"archive_bytes":bytes,
        "kitty":kitty,"kitten":kitten,"code_signature_verified":true,
        "kitty_binary_sha256":util::file_digest(&kitty)?,"kitten_binary_sha256":util::file_digest(&kitten)?,
        "scope":"Owned GHA real-terminal tests; no local GUI access or application distribution"});
    util::publish(&report_path, &util::encode(&report)?, false)?;
    util::github_output(
        std::env::var_os("GITHUB_ENV").as_deref().map(Path::new),
        &BTreeMap::from([
            (
                "LAZYPENCIL_TEST_KITTY_INSTALLATION".into(),
                report_path.to_string_lossy().into_owned(),
            ),
            (
                "LAZYPENCIL_TEST_KITTY".into(),
                kitty.to_string_lossy().into_owned(),
            ),
            (
                "LAZYPENCIL_TEST_KITTEN".into(),
                kitten.to_string_lossy().into_owned(),
            ),
        ]),
    )?;
    util::print(&report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_release_redirects_accept_only_https_github_asset_hosts() {
        for uri in [
            URL,
            "https://release-assets.githubusercontent.com/owned/kitty.dmg?signature=temporary",
        ] {
            release_host(&uri.parse().unwrap()).unwrap();
        }
        for uri in [
            "http://github.com/kitty.dmg",
            "https://github.com.attacker.invalid/kitty.dmg",
            "https://attacker.invalid/kitty.dmg",
        ] {
            assert!(release_host(&uri.parse().unwrap()).is_err());
        }
    }
}
