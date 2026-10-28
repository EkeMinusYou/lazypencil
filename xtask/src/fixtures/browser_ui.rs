//! A full browser for owned headed-login tests, separate from managed runtimes.

use crate::{download, process, util};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalogue {
    version: String,
    source: String,
    usage: String,
    hash_provenance: String,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    platform: String,
    url: String,
    sha256: String,
    bytes: u64,
    executable: String,
}

impl Catalogue {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == "156.0.8078.47"
                && self.source == "https://googlechromelabs.github.io/chrome-for-testing/"
                && !self.usage.is_empty()
                && !self.hash_provenance.is_empty()
                && self.artifacts.len() == 3,
            "Unexpected headed-browser catalogue"
        );
        for (artifact, platform) in
            self.artifacts
                .iter()
                .zip(["mac-arm64", "linux64", "linux-arm64"])
        {
            let root = format!("chrome-{platform}");
            let executable = if platform == "mac-arm64" {
                format!(
                    "{root}/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
                )
            } else {
                format!("{root}/chrome")
            };
            ensure!(
                artifact.platform == platform
                    && artifact.executable == executable
                    && artifact.url
                        == format!(
                            "https://storage.googleapis.com/chrome-for-testing-public/{}/{platform}/{root}.zip",
                            self.version
                        )
                    && (1..=512 * 1024 * 1024).contains(&artifact.bytes)
                    && artifact.sha256.len() == 64
                    && artifact
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "Invalid headed-browser artifact pin"
            );
        }
        Ok(())
    }
}

pub fn prepare(output: &Path) -> Result<()> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
        "Headed-browser fixtures belong on an owned GHA runner"
    );
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "mac-arm64",
        ("linux", "x86_64") => "linux64",
        ("linux", "aarch64") => "linux-arm64",
        _ => anyhow::bail!("Unsupported headed-browser fixture platform"),
    };
    let catalogue: Catalogue =
        util::json(&util::root().join("tests/fixtures/browser-ui/chromium.json"))?;
    catalogue.validate()?;
    let artifact = catalogue
        .artifacts
        .iter()
        .find(|a| a.platform == platform)
        .unwrap();
    let output = std::path::absolute(output)?;
    ensure!(
        !output.exists() && !output.is_symlink(),
        "Browser fixture output must be new"
    );
    let parent = output.parent().context("Browser fixture parent required")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir_in(parent)?;
    let archive = temporary.path().join("browser.zip");
    let bytes = download::download(&artifact.url, &artifact.sha256, &archive, artifact.bytes)?;
    ensure!(bytes == artifact.bytes, "Unexpected browser archive length");
    let package = temporary.path().join("package");
    fs::create_dir(&package)?;
    let inventory = extract(&archive, &package, &format!("chrome-{platform}"))?;
    let executable = package.join(&artifact.executable);
    ensure!(
        executable.is_file() && !executable.is_symlink(),
        "Browser executable missing"
    );
    let version = process::output_limited(
        Command::new(&executable)
            .arg(format!(
                "--user-data-dir={}",
                temporary.path().join("version-profile").display()
            ))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--version",
            ]),
        Duration::from_secs(30),
        65536,
    )?;
    ensure!(
        !version.timed_out,
        "Owned full-browser version probe timed out"
    );
    ensure!(
        version.status.success(),
        "Owned full-browser version probe failed: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    let version = String::from_utf8(version.stdout)?;
    ensure!(
        version.trim().ends_with(&format!(" {}", catalogue.version)),
        "Full browser version differs from the pin: {}",
        version.trim()
    );
    // Keep downloads temporary instead of evicting native dependency caches.
    // Publish only a completely extracted, version-checked owned installation.
    fs::rename(&package, &output)?;
    let executable = output.join(&artifact.executable).canonicalize()?;
    ensure!(
        executable.starts_with(output.canonicalize()?),
        "Browser executable escaped output"
    );
    util::github_output(
        std::env::var_os("GITHUB_ENV").as_deref().map(Path::new),
        &BTreeMap::from([
            (
                "LAZYPENCIL_TEST_FULL_CHROMIUM".into(),
                executable.to_string_lossy().into_owned(),
            ),
            (
                "LAZYPENCIL_TEST_FULL_CHROMIUM_VERSION".into(),
                catalogue.version.clone(),
            ),
            (
                "LAZYPENCIL_TEST_FULL_CHROMIUM_SHA256".into(),
                artifact.sha256.clone(),
            ),
        ]),
    )?;
    util::print(
        &json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"platform":platform,"version":catalogue.version,
        "archive_sha256":artifact.sha256,"archive_bytes":bytes,"executable":executable,"inventory":inventory,
        "scope":"Owned headed-login tests; not a managed or distributed application runtime"}),
    )
}

fn extract(archive: &Path, output: &Path, root: &str) -> Result<serde_json::Value> {
    ensure!(
        output.is_dir() && !output.is_symlink() && fs::read_dir(output)?.next().is_none(),
        "Browser extraction requires a new empty directory"
    );
    let mut archive = zip::ZipArchive::new(fs::File::open(archive)?)?;
    ensure!(
        (1..=4096).contains(&archive.len()),
        "Unexpected browser ZIP entry count"
    );
    let mut names = BTreeSet::new();
    let mut links = 0usize;
    let mut bytes = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name()?.trim_end_matches('/').to_owned();
        util::safe_path(&name)?;
        ensure!(
            name == root || name.starts_with(&format!("{root}/")),
            "Browser ZIP root changed"
        );
        ensure!(names.insert(name), "Duplicate browser ZIP path");
        let mode = entry
            .unix_mode()
            .context("Browser ZIP lacks Unix permissions")?;
        ensure!(
            matches!(mode & 0o170000, 0o040000 | 0o100000 | 0o120000),
            "Unsupported browser ZIP entry type"
        );
        if mode & 0o170000 == 0o120000 {
            ensure!(entry.size() <= 4096, "Oversized browser framework link");
            links += 1;
        }
        bytes = bytes
            .checked_add(entry.size())
            .context("Browser ZIP size overflow")?;
        ensure!(
            bytes <= 2 * 1024 * 1024 * 1024 && entry.size() <= 512 * 1024 * 1024,
            "Browser ZIP exceeds extraction bound"
        );
    }
    // zip 9 defaults to ExtractRecursiveInFolder on supported Unix hosts,
    // preserving app-bundle links while rejecting recursive target escapes.
    archive.extract(output)?;
    Ok(json!({"entries":names.len(),"uncompressed_bytes":bytes,"internal_symlinks":links}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;

    #[test]
    fn catalogue_pins_exactly_the_three_release_platforms_and_upstream_paths() {
        let mut catalogue: Catalogue =
            util::json(&util::root().join("tests/fixtures/browser-ui/chromium.json")).unwrap();
        catalogue.validate().unwrap();
        catalogue.artifacts[0].url.push_str("?untrusted=1");
        assert!(catalogue.validate().is_err());
    }

    #[cfg(unix)]
    fn framework_zip(path: &Path, target: &str) {
        use zip::{ZipWriter, write::SimpleFileOptions};
        let mut writer = ZipWriter::new(fs::File::create(path).unwrap());
        writer
            .start_file(
                "chrome-test/App.framework/Versions/156/resource",
                SimpleFileOptions::default().unix_permissions(0o644),
            )
            .unwrap();
        writer.write_all(b"owned browser fixture").unwrap();
        writer
            .add_symlink(
                "chrome-test/App.framework/Versions/Current",
                target,
                SimpleFileOptions::default(),
            )
            .unwrap();
        writer.finish().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn browser_extraction_preserves_internal_framework_links_and_rejects_escapes() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("browser.zip");
        let output = temporary.path().join("package");
        fs::create_dir(&output).unwrap();
        framework_zip(&archive, "156");
        let report = extract(&archive, &output, "chrome-test").unwrap();
        assert_eq!(report["internal_symlinks"], 1);
        assert_eq!(
            fs::read(output.join("chrome-test/App.framework/Versions/Current/resource")).unwrap(),
            b"owned browser fixture"
        );
        assert!(
            output
                .join("chrome-test/App.framework/Versions/Current")
                .is_symlink()
        );
        // Do not permit an existing installation to be overwritten.
        assert!(extract(&archive, &output, "chrome-test").is_err());
        for target in ["../../../../outside", "/tmp/lazypencil-unowned-browser"] {
            let trial = tempfile::tempdir_in(temporary.path()).unwrap();
            let output = trial.path().join("package");
            fs::create_dir(&output).unwrap();
            framework_zip(&archive, target);
            assert!(
                extract(&archive, &output, "chrome-test").is_err(),
                "{target}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn browser_extraction_rejects_a_different_package_root_before_writing_files() {
        let temporary = tempfile::tempdir().unwrap();
        let archive = temporary.path().join("browser.zip");
        let output = temporary.path().join("package");
        fs::create_dir(&output).unwrap();
        framework_zip(&archive, "156");
        assert!(extract(&archive, &output, "different-root").is_err());
        assert!(fs::read_dir(output).unwrap().next().is_none());
    }
}
