use crate::{download, process, util};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const PACKAGES: &[&str] = &[
    "clang",
    "libclang-dev",
    "cmake",
    "ninja-build",
    "pkg-config",
    "libfontconfig1-dev",
    "libfreetype6-dev",
    "libwayland-dev",
    "libx11-dev",
    "xvfb",
    "xauth",
];
const SOURCES: &str = "Types: deb\nURIs: https://archive.ubuntu.com/ubuntu\nSuites: noble noble-updates noble-backports\nComponents: main restricted universe multiverse\nArchitectures: amd64\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n\nTypes: deb\nURIs: https://security.ubuntu.com/ubuntu\nSuites: noble-security\nComponents: main restricted universe multiverse\nArchitectures: amd64\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n";

pub fn linux_dependencies() -> Result<()> {
    hosted_linux_packages(PACKAGES)
}

pub fn linux_profile_dependencies() -> Result<()> {
    hosted_linux_packages(&["binutils", "libfontconfig1", "libfreetype6"])
}

pub fn linux_terminal_dependencies() -> Result<()> {
    hosted_linux_packages(&[
        "kitty", "xterm", "xvfb", "xauth", "xdotool", "x11-apps", "strace",
    ])
}

fn hosted_linux_packages(packages: &[&str]) -> Result<()> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_OS").as_deref() == Ok("Linux")
            && cfg!(target_os = "linux"),
        "Native package installation requires a disposable Linux GHA runner"
    );
    let release = fs::read_to_string("/etc/os-release")?;
    let value = |name: &str| {
        release.lines().find_map(|s| {
            s.strip_prefix(&format!("{name}="))
                .map(|s| s.trim_matches('"'))
        })
    };
    ensure!(
        value("ID") == Some("ubuntu") && value("VERSION_ID") == Some("24.04"),
        "Expected Ubuntu 24.04"
    );
    let architecture = std::env::var("RUNNER_ARCH")?;
    ensure!(
        ["X64", "ARM64"].contains(&architecture.as_str()),
        "Unsupported hosted Linux architecture"
    );
    let temporary = PathBuf::from(std::env::var_os("RUNNER_TEMP").context("Missing RUNNER_TEMP")?);
    ensure!(
        temporary.is_absolute() && temporary.is_dir(),
        "Expected hosted temporary directory"
    );
    install_linux_packages(&architecture, &temporary, packages, |command| {
        let output = process::checked(command, Duration::from_secs(20 * 60))?;
        print!("{}", String::from_utf8_lossy(&output));
        Ok(())
    })
}

pub fn install_linux(
    architecture: &str,
    temporary: &Path,
    run: impl FnMut(&mut Command) -> Result<()>,
) -> Result<()> {
    install_linux_packages(architecture, temporary, PACKAGES, run)
}

fn install_linux_packages(
    architecture: &str,
    temporary: &Path,
    packages: &[&str],
    mut run: impl FnMut(&mut Command) -> Result<()>,
) -> Result<()> {
    ensure!(
        ["X64", "ARM64"].contains(&architecture),
        "Unsupported hosted Linux architecture"
    );
    let temp = tempfile::tempdir_in(temporary)?;
    let mut options = Vec::<String>::new();
    if architecture == "X64" {
        let source = temp.path().join("ubuntu.sources");
        fs::write(&source, SOURCES)?;
        let parts = temp.path().join("empty-parts");
        fs::create_dir(&parts)?;
        options.extend([
            "-o".into(),
            format!("Dir::Etc::sourcelist={}", source.display()),
            "-o".into(),
            format!("Dir::Etc::sourceparts={}", parts.display()),
            "-o".into(),
            "Acquire::Retries=2".into(),
            "-o".into(),
            "Acquire::https::Timeout=30".into(),
        ]);
        println!("Using official Ubuntu HTTPS archives for native CI packages");
    }
    for args in [
        vec!["update"],
        std::iter::once("install")
            .chain(std::iter::once("--yes"))
            .chain(packages.iter().copied())
            .collect(),
    ] {
        run(Command::new("sudo")
            .arg("apt-get")
            .args(&options)
            .args(args))?;
    }
    Ok(())
}

pub fn material_fonts(catalogue: &Path, output: &Path) -> Result<Value> {
    let records: Vec<Value> = util::json(catalogue)?;
    ensure!(!output.is_symlink(), "Font output must not be a symlink");
    fs::create_dir_all(output)?;
    let mut fonts = Vec::new();
    for style in ["Outlined", "Rounded", "Sharp"] {
        let name = format!("Material Symbols {style}");
        let records = records
            .iter()
            .filter(|r| r["id"] == name)
            .collect::<Vec<_>>();
        ensure!(
            records.len() == 1,
            "Expected exactly one Material font record"
        );
        let record = records[0];
        let version = record["version"]
            .as_str()
            .context("Missing font revision")?;
        let sha = record["source_sha256"]
            .as_str()
            .context("Missing font checksum")?;
        let filename = format!("MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf");
        let encoded = filename
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>();
        let source = format!(
            "https://raw.githubusercontent.com/google/material-design-icons/{version}/variablefont/{encoded}"
        );
        ensure!(
            record["source_format"] == "ttf"
                && version.len() == 40
                && version
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                && sha.len() == 64
                && sha
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                && record["source_url"] == source,
            "Invalid immutable Material source pin"
        );
        let target = output.join(&filename);
        ensure!(!target.is_symlink(), "Cached font must not be linked");
        let reused = target.exists();
        let size = if reused {
            download::verify_file(&target, sha, 16 * 1024 * 1024)?
        } else {
            download::download(&source, sha, &target, 16 * 1024 * 1024)?
        };
        fonts.push(json!({"pack":name,"version":version,"filename":filename,"sha256":sha,"bytes":size,"reused":reused}));
    }
    Ok(json!({"fonts":fonts}))
}
