//! Keep the raw map from the evidence build for final archive-member review.

use super::fingerprint;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Keep an inclusive declaration inventory: dead stripping does not revoke notices.
pub fn archive_references(bytes: &[u8]) -> Result<Value> {
    ensure!(bytes.len() <= 64 * 1024 * 1024, "Link map exceeds 64 MiB");
    // ld64 writes literal string bytes in its symbol table. Only archive paths
    // and member names need UTF-8; unrelated symbols are opaque linker output.
    let lines = || {
        bytes
            .split(|byte| *byte == b'\n')
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
    };
    let mut archives = BTreeMap::<String, BTreeSet<String>>::new();
    let mut pairs = 0usize;
    let mut add = |input: &[u8], inclusion: bool| -> Result<()> {
        let positions = [b".rlib(".as_slice(), b".a(".as_slice()]
            .into_iter()
            .filter_map(|suffix| {
                let position = if inclusion {
                    input.windows(suffix.len()).position(|part| part == suffix)
                } else {
                    input.windows(suffix.len()).rposition(|part| part == suffix)
                };
                position.map(|start| (start, suffix))
            });
        // GNU can put the referring file (itself an archive member) on this line.
        let position = if inclusion {
            positions.min_by_key(|(start, _)| *start)
        } else {
            positions.max_by_key(|(start, _)| *start)
        };
        let Some((start, suffix)) = position else {
            return Ok(());
        };
        let (archive, member) = input.split_at(start + suffix.len() - 1);
        let member = &member[1..];
        let end = member
            .iter()
            .position(|byte| *byte == b')')
            .context("Malformed archive-member declaration")?;
        let tail = &member[end + 1..];
        let archive = std::str::from_utf8(archive).context("Archive path must be UTF-8")?;
        let member = std::str::from_utf8(&member[..end]).context("Archive member must be UTF-8")?;
        ensure!(
            archive.starts_with('/')
                && archive.len() <= 32768
                && !archive.chars().any(char::is_control)
                && !member.is_empty()
                && member.len() <= 8192
                && (tail.is_empty()
                    || inclusion && tail.first().is_some_and(u8::is_ascii_whitespace))
                && !member
                    .chars()
                    .any(|c| c.is_control() || c == '(' || c == ')'),
            "Unsupported archive-member path"
        );
        if archives
            .entry(archive.into())
            .or_default()
            .insert(member.into())
        {
            pairs += 1;
            ensure!(pairs <= 100_000, "Too many link-map archive members");
        }
        Ok(())
    };
    let first = lines().next().unwrap_or_default();
    let header = std::str::from_utf8(first).unwrap_or_default();
    let format = if let Some(start) = lines().position(|line| line == b"# Object files:") {
        let mut ids = BTreeSet::new();
        let mut sections = false;
        for line in lines().skip(start + 1).filter(|line| !line.is_empty()) {
            if line == b"# Sections:" {
                sections = true;
                break;
            }
            let line = line
                .strip_prefix(b"[")
                .context("Malformed ld64 object record")?;
            let end = line
                .windows(2)
                .position(|part| part == b"] ")
                .context("Malformed ld64 object record")?;
            let id: u32 = std::str::from_utf8(&line[..end])?.trim().parse()?;
            ensure!(ids.insert(id), "Duplicate ld64 object index");
            add(&line[end + 2..], false)?;
        }
        ensure!(sections, "Missing ld64 section table");
        "macos_ld64"
    } else if header
        .split_whitespace()
        .eq(["VMA", "LMA", "Size", "Align", "Out", "In", "Symbol"])
    {
        let column = header.find("In").unwrap();
        for line in lines().skip(1) {
            let Some(input) = line.get(column..) else {
                continue;
            };
            // Symbols are indented beyond In; output sections start before it.
            if !input.starts_with(b"/")
                || line[..column]
                    .split(u8::is_ascii_whitespace)
                    .filter(|part| !part.is_empty())
                    .count()
                    != 4
            {
                continue;
            }
            if let Some(end) = input.windows(2).rposition(|part| part == b":(") {
                ensure!(input.ends_with(b")"), "Malformed lld input section");
                add(&input[..end], false)?;
            }
        }
        "elf_lld"
    } else if first == b"Archive member included to satisfy reference by file (symbol)" {
        for line in lines().skip(1).take_while(|line| {
            !line.starts_with(b"As-needed library included")
                && *line != b"Discarded input sections"
                && *line != b"Memory Configuration"
                && *line != b"Linker script and memory map"
        }) {
            // The following indented record explains the referring symbol.
            if line.starts_with(b"/") {
                add(line, true)?;
            }
        }
        "elf_gnu"
    } else {
        anyhow::bail!("Unrecognized initial-release link-map format");
    };
    ensure!(pairs > 0, "Link map has no archive-member declarations");
    Ok(json!({
        "format_version":1,"format":format,"map_sha256":crate::util::digest(bytes),"map_bytes":bytes.len(),
        "archive_member_pairs":pairs,
        "archives":archives.into_iter().map(|(archive,members)|json!({"archive":archive,"members":members})).collect::<Vec<_>>(),
        "scope":"Inclusive archive-member declarations from the ld64 object table, GNU archive inclusion table or lld input sections. Loaded objects may contain discarded sections. This is not exact live-code attribution, archive byte verification or legal approval."
    }))
}

pub fn collect_inputs(map: &Path, output: &Path) -> Result<Value> {
    let report = archive_references(&crate::util::read(map, 64 * 1024 * 1024)?)?;
    crate::util::publish(output, &crate::util::encode(&report)?, false)?;
    Ok(report)
}

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
