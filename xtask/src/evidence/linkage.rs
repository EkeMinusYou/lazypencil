//! Inspect the selected executable without loading it or invoking its loader.
//!
//! These are direct dynamic-load and ABI declarations, not transitive library
//! resolution, static archive attribution, or minimum-OS runtime acceptance.

use crate::{process, util};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::Path, process::Command, time::Duration};

const MAX_OUTPUT: u64 = 4 * 1024 * 1024;

pub fn native_host() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") if cfg!(target_env = "gnu") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") if cfg!(target_env = "gnu") => Ok("aarch64-unknown-linux-gnu"),
        _ => anyhow::bail!("Native dynamic-load inspection is limited to initial-release hosts"),
    }
}

pub fn format(header: &[u8], host: &str) -> Result<&'static str> {
    ensure!(header.len() >= 32, "Truncated executable header");
    match host {
        "aarch64-apple-darwin" => {
            ensure!(
                header[..4] == [0xcf, 0xfa, 0xed, 0xfe]
                    && u32::from_le_bytes(header[4..8].try_into()?) == 0x0100_000c
                    && u32::from_le_bytes(header[12..16].try_into()?) == 2,
                "Expected a thin arm64 Mach-O executable"
            );
            Ok("mach-o")
        }
        "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu" => {
            let machine = if host.starts_with("x86_64") { 62 } else { 183 };
            ensure!(
                header[..7] == [0x7f, b'E', b'L', b'F', 2, 1, 1]
                    && [2, 3].contains(&u16::from_le_bytes(header[16..18].try_into()?))
                    && u16::from_le_bytes(header[18..20].try_into()?) == machine,
                "Expected a little-endian ELF64 executable for {host}"
            );
            Ok("elf")
        }
        _ => anyhow::bail!("Dynamic-load evidence is not implemented for deferred target {host}"),
    }
}

pub fn elf(text: &str) -> Result<Value> {
    let dependencies = Regex::new(r"(?m)^.*\(NEEDED\).*Shared library: \[([^\]\r\n]+)\]\s*$")?
        .captures_iter(text)
        .map(|m| m[1].to_owned())
        .collect::<Vec<_>>();
    ensure!(!dependencies.is_empty(), "Missing ELF dynamic dependencies");
    let interpreters = Regex::new(r"\[Requesting program interpreter: ([^\]\r\n]+)\]")?
        .captures_iter(text)
        .map(|m| m[1].to_owned())
        .collect::<Vec<_>>();
    ensure!(
        interpreters.len() == 1 && interpreters[0].starts_with('/'),
        "Missing or ambiguous ELF interpreter"
    );
    let paths =
        Regex::new(r"(?m)^.*\((RPATH|RUNPATH)\).*Library (?:rpath|runpath): \[([^\]\r\n]*)\]\s*$")?
            .captures_iter(text)
            .map(|m| json!({"kind":&m[1],"value":&m[2]}))
            .collect::<Vec<_>>();
    let file = Regex::new(r"\bFile: (\S+)\s+Cnt: (\d+)\s*$")?;
    let name = Regex::new(r"\bName: (\S+)\s+Flags:.*\bVersion: \d+\s*$")?;
    let mut requirements = BTreeMap::<String, Vec<String>>::new();
    let mut current = None;
    let mut counts = BTreeMap::new();
    let mut needs = false;
    for line in text.lines() {
        if line.starts_with("Version ") {
            needs = line.starts_with("Version needs section ");
            current = None;
        }
        if !needs {
            continue;
        }
        if let Some(capture) = file.captures(line) {
            let library = capture[1].to_owned();
            ensure!(
                dependencies.contains(&library),
                "Version requirement lacks DT_NEEDED"
            );
            ensure!(
                counts
                    .insert(library.clone(), capture[2].parse::<usize>()?)
                    .is_none(),
                "Duplicate ELF version-need library"
            );
            requirements.insert(library.clone(), Vec::new());
            current = Some(library);
        } else if let Some(capture) = name.captures(line) {
            let library = current
                .as_ref()
                .context("Version name precedes its library")?;
            requirements
                .get_mut(library)
                .unwrap()
                .push(capture[1].to_owned());
        }
    }
    ensure!(
        !requirements.is_empty(),
        "Missing ELF symbol-version requirements"
    );
    for (library, versions) in &requirements {
        ensure!(
            versions.len() == counts[library] && !versions.is_empty(),
            "Incomplete ELF version needs"
        );
    }
    let glibc = requirements
        .values()
        .flatten()
        .filter_map(|v| v.strip_prefix("GLIBC_"))
        .collect::<Vec<_>>();
    let numeric = |v: &str| -> Option<Vec<u32>> {
        let parts = v
            .split('.')
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()?;
        (parts.len() >= 2).then_some(parts)
    };
    let maximum = glibc
        .iter()
        .filter_map(|v| numeric(v).map(|n| (n, *v)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v);
    let non_numeric = glibc
        .iter()
        .filter(|v| numeric(v).is_none())
        .copied()
        .collect::<Vec<_>>();
    ensure!(
        maximum.is_some(),
        "Missing numeric glibc symbol requirement"
    );
    Ok(
        json!({"format":"elf","needed":dependencies,"interpreter":interpreters[0],"search_paths":paths,
        "required_symbol_versions":requirements,"maximum_numeric_glibc_requirement":maximum,
        "non_numeric_glibc_requirements":non_numeric}),
    )
}

fn field<'a>(lines: &[&'a str], name: &str) -> Result<&'a str> {
    let values = lines
        .iter()
        .filter_map(|s| s.strip_prefix(name))
        .collect::<Vec<_>>();
    ensure!(values.len() == 1, "Missing or ambiguous Mach-O {name}");
    Ok(values[0])
}

fn load_path<'a>(lines: &[&'a str], name: &str) -> Result<&'a str> {
    let value = field(lines, name)?;
    let (value, offset) = value
        .rsplit_once(" (offset ")
        .context("Missing Mach-O string offset")?;
    ensure!(
        !value.is_empty()
            && offset
                .strip_suffix(')')
                .is_some_and(|s| s.parse::<u32>().is_ok()),
        "Malformed Mach-O load path"
    );
    Ok(value)
}

pub fn mach_o(text: &str) -> Result<Value> {
    let mut dependencies = Vec::new();
    let mut paths = Vec::new();
    let mut interpreters = Vec::new();
    let mut builds = Vec::new();
    let version = Regex::new(r"^\d+\.\d+(?:\.\d+)?$")?;
    for block in text.split("\nLoad command ").skip(1) {
        let lines = block.lines().map(str::trim).collect::<Vec<_>>();
        match field(&lines, "cmd ")? {
            kind @ ("LC_LOAD_DYLIB"
            | "LC_LOAD_WEAK_DYLIB"
            | "LC_REEXPORT_DYLIB"
            | "LC_LOAD_UPWARD_DYLIB"
            | "LC_LAZY_LOAD_DYLIB") => {
                let current = field(&lines, "current version ")?;
                let compatibility = field(&lines, "compatibility version ")?;
                ensure!(
                    version.is_match(current) && version.is_match(compatibility),
                    "Invalid dylib version"
                );
                dependencies.push(json!({"kind":kind,"name":load_path(&lines, "name ")?,"current_version":current,"compatibility_version":compatibility}));
            }
            "LC_RPATH" => paths.push(load_path(&lines, "path ")?.to_owned()),
            "LC_LOAD_DYLINKER" => interpreters.push(load_path(&lines, "name ")?.to_owned()),
            "LC_BUILD_VERSION" => {
                let platform = field(&lines, "platform ")?;
                let minimum = field(&lines, "minos ")?;
                let sdk = field(&lines, "sdk ")?;
                ensure!(
                    ["1", "MACOS", "macos"].contains(&platform)
                        && version.is_match(minimum)
                        && version.is_match(sdk),
                    "Expected macOS build version"
                );
                builds.push(json!({"platform":"macos","minimum_os":minimum,"sdk":sdk}));
            }
            _ => {}
        }
    }
    ensure!(
        !dependencies.is_empty() && builds.len() == 1 && interpreters.len() == 1,
        "Incomplete Mach-O dynamic-load declarations"
    );
    Ok(
        json!({"format":"mach-o","needed":dependencies,"interpreter":interpreters[0],"search_paths":paths,"build_version":builds[0]}),
    )
}

fn capture(command: &mut Command, output: &Path) -> Result<String> {
    let result = process::output_limited(
        command.env("LC_ALL", "C"),
        Duration::from_secs(30),
        MAX_OUTPUT,
    )?;
    ensure!(
        !result.timed_out && result.status.success(),
        "Executable inspection failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    ensure!(
        result.stderr.is_empty(),
        "Executable inspection emitted diagnostics"
    );
    let text = String::from_utf8(result.stdout)?;
    fs::write(output, &text)?;
    Ok(text)
}

pub fn collect(executable: &Path, host: &str, output: &Path) -> Result<Value> {
    let before = util::file_digest(executable)?;
    let mut header = [0; 64];
    fs::File::open(executable)?.read_exact(&mut header)?;
    let format = format(&header, host)?;
    fs::create_dir(output)?;
    let (declarations, inspector) = if format == "elf" {
        let version = capture(
            Command::new("readelf").arg("--version"),
            &output.join("inspector-version.txt"),
        )?;
        let text = capture(
            Command::new("readelf")
                .args([
                    "--file-header",
                    "--program-headers",
                    "--dynamic",
                    "--version-info",
                    "--wide",
                    "--",
                ])
                .arg(executable),
            &output.join("readelf.txt"),
        )?;
        (elf(&text)?, json!({"program":"readelf","version":version}))
    } else {
        let location = process::checked(
            Command::new("xcrun").args(["--find", "otool"]),
            Duration::from_secs(30),
        )?;
        ensure!(location.len() <= 4096, "Unexpected otool path length");
        let path = std::path::PathBuf::from(String::from_utf8(location)?.trim());
        ensure!(
            path.is_absolute() && path.is_file(),
            "Expected an installed otool executable"
        );
        let path = path.canonicalize()?;
        let hash = util::file_digest(&path)?;
        let text = capture(
            Command::new(&path).arg("-l").arg(executable),
            &output.join("otool.txt"),
        )?;
        ensure!(
            util::file_digest(&path)? == hash,
            "Inspector changed during collection"
        );
        (
            mach_o(&text)?,
            json!({"program":"otool","path":path,"sha256":hash}),
        )
    };
    ensure!(
        util::file_digest(executable)? == before,
        "Executable changed during inspection"
    );
    let report = json!({"format_version":1,"binary_sha256":before,"host":host,"inspector":inspector,
        "declarations":declarations,"scope":"Direct binary declarations only; no loader execution, transitive resolution, static attribution or minimum-OS acceptance."});
    util::publish(&output.join("linkage.json"), &util::encode(&report)?, false)?;
    Ok(report)
}
