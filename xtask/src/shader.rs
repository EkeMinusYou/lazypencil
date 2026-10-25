use crate::{notices, util};
use anyhow::{Context, Result, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Args)]
pub struct Arguments {
    #[arg(long,default_value_os_t=util::root())]
    root: PathBuf,
    #[arg(long)]
    rebuilt: Option<PathBuf>,
}
impl Arguments {
    pub fn run(self) -> Result<()> {
        util::print(&verify(&self.root, self.rebuilt.as_deref())?)
    }
}

pub fn memory_import(binary: &[u8]) -> Result<Value> {
    ensure!(
        binary.starts_with(b"\0asm\x01\0\0\0"),
        "Invalid Wasm module header"
    );
    fn integer(binary: &[u8], cursor: &mut usize) -> Result<u32> {
        let mut value = 0u32;
        for shift in [0, 7, 14, 21, 28] {
            let byte = *binary.get(*cursor).context("Truncated Wasm integer")?;
            *cursor += 1;
            ensure!(shift != 28 || byte & 0xf0 == 0, "Oversized Wasm integer");
            value |= ((byte & 127) as u32) << shift;
            if byte < 128 {
                return Ok(value);
            }
        }
        anyhow::bail!("Oversized Wasm integer")
    }
    fn string<'a>(binary: &'a [u8], cursor: &mut usize) -> Result<&'a str> {
        let length = integer(binary, cursor)? as usize;
        let end = cursor
            .checked_add(length)
            .context("Wasm string length overflow")?;
        let text = std::str::from_utf8(binary.get(*cursor..end).context("Truncated Wasm string")?)?;
        *cursor = end;
        Ok(text)
    }
    let mut cursor = 8;
    let mut found = false;
    while cursor < binary.len() {
        let section = binary[cursor];
        cursor += 1;
        let size = integer(binary, &mut cursor)? as usize;
        let end = cursor
            .checked_add(size)
            .context("Wasm section length overflow")?;
        ensure!(end <= binary.len(), "Truncated Wasm section");
        if section == 2 {
            ensure!(
                !found && integer(binary, &mut cursor)? == 1,
                "Unexpected Wasm import count"
            );
            let module = string(binary, &mut cursor)?;
            let name = string(binary, &mut cursor)?;
            ensure!(
                cursor < end && binary[cursor] == 2,
                "Expected only a memory import"
            );
            cursor += 1;
            let flags = integer(binary, &mut cursor)?;
            let minimum = integer(binary, &mut cursor)?;
            let maximum = integer(binary, &mut cursor)?;
            ensure!(
                (module, name, flags, minimum, maximum) == ("env", "memory", 1, 256, 1024)
                    && cursor == end,
                "Unexpected Wasm memory boundary or unlisted import bytes"
            );
            found = true;
        }
        cursor = end;
    }
    ensure!(found, "Missing bounded Wasm memory import");
    Ok(json!({"module":"env","name":"memory","initial_pages":256,"maximum_pages":1024}))
}

pub fn verify(root: &Path, rebuilt: Option<&Path>) -> Result<Value> {
    let directory = root.join("resources/shader-preprocessor");
    let report: Value = util::json(&directory.join("build.json"))?;
    let binary = util::read(&directory.join("module.wasm"), 16 * 1024 * 1024)?;
    ensure!(
        report["format_version"] == 1 && report["target"] == "wasm32-unknown-unknown",
        "Invalid shader build identity"
    );
    ensure!(
        report["wasm_bytes"] == binary.len() && report["wasm_sha256"] == util::digest(&binary),
        "Shader module differs from verified GHA build"
    );
    ensure!(
        report["memory_maximum_bytes"] == 64 * 1024 * 1024,
        "Unexpected shader memory policy"
    );
    let memory = memory_import(&binary)?;
    let component = root.join("src/shader/preprocessor");
    let mut files = BTreeMap::new();
    for path in util::walk_filtered(&component, |p| {
        p.components()
            .any(|c| c.as_os_str() == "target" || c.as_os_str() == "__pycache__")
    })? {
        let name = path
            .strip_prefix(&component)?
            .to_str()
            .context("Invalid shader source path")?;
        if name
            .split('/')
            .any(|s| ["target", "__pycache__"].contains(&s))
        {
            continue;
        }
        files.insert(
            path.strip_prefix(root)?
                .to_str()
                .unwrap()
                .replace('\\', "/"),
            util::file_digest(&path)?,
        );
    }
    ensure!(
        serde_json::to_value(&files)? == report["source_files"],
        "Shader component source changed; regenerate module on GHA"
    );
    let evidence: Value = util::json(&directory.join("verification.json"))?;
    ensure!(
        evidence["source"] == report["source_commit"]
            && evidence["wasm_sha256"] == report["wasm_sha256"]
            && evidence["source_files_verified"] == files.len(),
        "Shader producer evidence does not bind these inputs"
    );
    let legal = root.join("resources/licenses/shader-preprocessor");
    let packages = notices::rust::verify(&component, &legal.join("packages"), None)?;
    let directory = legal.join("toolchain/inventory");
    let entries = notices::verify_bundle(&directory, |_| true)?;
    let std: Value = serde_json::from_slice(
        entries
            .get("inventory.json")
            .context("Missing shader toolchain inventory")?,
    )?;
    let identity = &std["observed_toolchain"];
    ensure!(
        identity["target"] == "wasm32-unknown-unknown"
            && identity["release"] == "1.94.0"
            && identity["commit_hash"] == "4a4ef493e3a1488c6e321570238084b38948f6db",
        "Wrong shader compiler/target notices"
    );
    ensure!(
        std["target_libraries"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "Missing Wasm standard-library fingerprints"
    );
    if let Some(path) = rebuilt {
        ensure!(
            util::read(path, 16 * 1024 * 1024)? == binary,
            "Rebuilt module differs; import GHA module with its notices"
        );
    }
    Ok(
        json!({"wasm_sha256":report["wasm_sha256"],"wasm_bytes":binary.len(),"source_files":files.len(),"memory_import":memory,"original_package_notices":packages["rust_packages"],"target_library_notices":std["target_libraries"].as_array().unwrap().len(),"rebuilt_bytes_equal":rebuilt.is_some()}),
    )
}

pub fn collect_notices(
    root: &Path,
    cargo: &Path,
    component: &Path,
    extra_policies: &[PathBuf],
    output: &Path,
) -> Result<Value> {
    let (_, packages) = notices::rust::locked(component)?;
    let selected = packages.keys().cloned().collect::<BTreeSet<_>>();
    let temp = tempfile::tempdir_in(output.parent().context("Missing notice output parent")?)?;
    let mut references = BTreeMap::new();
    let mut records = BTreeMap::new();
    let policies = std::iter::once(root.join("resources/licenses/rust/upstream.json"))
        .chain(extra_policies.iter().cloned());
    for path in policies {
        let policy: Value = util::json(&path)?;
        ensure!(
            policy["format_version"] == 1,
            "Unsupported package notice policy"
        );
        let directory = path.parent().unwrap();
        for item in policy["license_references"]
            .as_array()
            .context("Missing reference terms")?
        {
            let id = item["id"]
                .as_str()
                .context("Missing reference identifier")?;
            if let Some(previous) = references.insert(id.to_owned(), item.clone()) {
                ensure!(previous == *item, "Conflicting reference license");
            }
            let data = notices::stored_blob(directory, item)?;
            let destination = temp.path().join(item["blob"].as_str().unwrap());
            std::fs::create_dir_all(destination.parent().unwrap())?;
            std::fs::write(destination, data)?;
        }
        for package in policy["packages"]
            .as_array()
            .context("Missing policy packages")?
        {
            let key = notices::rust::identifier(package)?;
            if !selected.contains(&key) {
                continue;
            }
            ensure!(
                records.insert(key, package.clone()).is_none(),
                "Duplicate upstream package input"
            );
            for item in package["files"]
                .as_array()
                .context("Missing original package notices")?
            {
                let data = notices::stored_blob(directory, item)?;
                let path = temp.path().join(item["blob"].as_str().unwrap());
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(path, data)?;
            }
        }
    }
    let policy = json!({"format_version":1,"packages":records.into_values().collect::<Vec<_>>(),"license_references":references.into_values().collect::<Vec<_>>()});
    let upstream = temp.path().join("upstream.json");
    std::fs::write(&upstream, util::encode(&policy)?)?;
    notices::rust::collect(component, cargo, &upstream, output)?;
    notices::rust::verify(component, output, Some(cargo))
}
