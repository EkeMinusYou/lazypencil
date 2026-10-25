//! Build and inspect the actual standalone Wasm artifact, including JavaScript-engine ABI tests.
use super::native::Session;
use crate::{dist, notices, process, shader, util};
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Stage {
    Source,
    Build,
    Test,
    Notices,
    Toolchain,
    Evidence,
    Verify,
    Prune,
}
pub fn run(stage: Stage) -> Result<()> {
    let s = Session::hosted()?;
    let root = s.temporary.join("lazypencil-wasm-source");
    let target = s.temporary.join("lazypencil-wasm-target");
    let manifest = root.join("src/shader/preprocessor/Cargo.toml");
    let cargo = notices::cargo_home_default()?;
    let flags = format!(
        "--remap-path-prefix={}=/source --remap-path-prefix={}=/cargo -C link-arg=--import-memory -C link-arg=--initial-memory=16777216 -C link-arg=--max-memory=67108864 -C link-arg=-z -C link-arg=stack-size=1048576",
        root.display(),
        cargo.display()
    );
    let binary = target.join("wasm32-unknown-unknown/release/lazypencil_shader_preprocessor.wasm");
    let command = |verb: &str| {
        let mut c = Command::new("cargo");
        c.current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .env("RUSTFLAGS", &flags)
            .arg(verb)
            .arg("--manifest-path")
            .arg(&manifest);
        c
    };
    let clean = |package: &str| -> Result<()> {
        process::checked(
            command("clean").args([
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                package,
            ]),
            Duration::from_secs(120),
        )?;
        Ok(())
    };
    match stage {
        Stage::Source => {
            let archive = s.temporary.join("shader-source.tar.gz");
            dist::source(&util::root(), &archive, 0, false)?;
            dist::verify(&archive, Some(&root))?;
            process::checked(
                command("fmt").args(["--all", "--", "--check"]),
                Duration::from_secs(120),
            )?;
            process::checked(command("fetch").arg("--locked"), Duration::from_secs(300))?;
        }
        Stage::Build => {
            clean("lazypencil-shader-preprocessor")?;
            if std::env::var("EXACT_COMPONENT_CACHE").as_deref() != Ok("true") {
                clean("glsl-lang-pp")?;
            }
            process::logged(
                command("clippy").args([
                    "--locked",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--",
                    "-D",
                    "warnings",
                ]),
                Duration::from_secs(480),
                &s.temporary.join("shader-clippy.log"),
                false,
            )?;
            let output = process::checked(
                command("build").args([
                    "--locked",
                    "--release",
                    "--target",
                    "wasm32-unknown-unknown",
                    "--message-format=json",
                ]),
                Duration::from_secs(480),
            )?;
            fs::write(s.temporary.join("shader-build.jsonl"), output)?;
        }
        Stage::Test => {
            // This fixture runs in a JavaScript WebAssembly engine, just like production
            // browser shader execution. Rust owns orchestration, deadlines and evidence.
            process::logged(
                Command::new("node")
                    .arg(root.join("tests/support/shader_preprocessor.mjs"))
                    .arg(&binary)
                    .arg(s.temporary.join("shader-preprocessor-tests.json")),
                Duration::from_secs(120),
                &s.temporary.join("shader-tests.log"),
                false,
            )?;
        }
        Stage::Notices => {
            shader::collect_notices(
                &root,
                &cargo,
                &root.join("src/shader/preprocessor"),
                &[root.join("src/shader/preprocessor/notices/upstream.json")],
                &s.temporary.join("shader-notices"),
            )?;
        }
        Stage::Toolchain => {
            super::toolchain::collect(
                &root,
                &s.temporary.join("shader-toolchain"),
                Some("wasm32-unknown-unknown"),
            )?;
        }
        Stage::Evidence => {
            let output = s.temporary.join("shader-preprocessor-evidence");
            ensure!(!output.exists(), "Evidence output already exists");
            fs::create_dir(&output)?;
            fs::copy(&binary, output.join(binary.file_name().unwrap()))?;
            let component = root.join("src/shader/preprocessor");
            let mut files = serde_json::Map::new();
            for path in util::walk(&component)? {
                files.insert(
                    path.strip_prefix(&root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    util::file_digest(&path)?.into(),
                );
            }
            let report = json!({"format_version":1,"source_commit":std::env::var("GITHUB_SHA")?,"source_files":files,"wasm_sha256":util::file_digest(&binary)?,"wasm_bytes":fs::metadata(&binary)?.len(),"target":"wasm32-unknown-unknown","rustc":process::text(Command::new("rustc").arg("-vV"))?,"rustflags":flags,"memory_maximum_bytes":67108864,"release_ready":false,"scope":"Standalone bounded preprocessor evidence; full native parity requires separate tests."});
            util::publish(&output.join("build.json"), &util::encode(&report)?, false)?;
            for name in ["shader-preprocessor-tests.json", "shader-build.jsonl"] {
                fs::copy(s.temporary.join(name), output.join(name))?;
            }
            fs::copy(component.join("Cargo.lock"), output.join("Cargo.lock"))?;
            for (from, to) in [
                ("shader-notices", "notices"),
                ("shader-toolchain", "toolchain"),
            ] {
                copy(&s.temporary.join(from), &output.join(to))?;
            }
        }
        Stage::Verify => {
            shader::verify(&root, Some(&binary))?;
        }
        Stage::Prune => {
            clean("lazypencil-shader-preprocessor")?;
            ensure!(
                !binary.exists(),
                "Package output remained in dependency cache"
            );
        }
    }
    Ok(())
}
fn copy(source: &Path, output: &Path) -> Result<()> {
    for path in util::walk(source)? {
        let to = output.join(path.strip_prefix(source)?);
        fs::create_dir_all(to.parent().context("Output parent")?)?;
        fs::copy(path, to)?;
    }
    Ok(())
}
