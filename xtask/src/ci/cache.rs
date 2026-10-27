use super::focus;
use crate::{
    dist::{self, Entry, Manifest},
    process, util,
};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
};

const INPUTS: &[&str] = &[
    "Cargo.lock",
    "Cargo.toml",
    ".gitattributes",
    "vendor/skia-bindings",
    "resources/icons/catalogue.json",
    ".cargo/config.toml",
    ".cargo/config",
    "rust-toolchain.toml",
    "rust-toolchain",
];
const FLAG_NAMES: &[&str] = &[
    "CARGO_BUILD_JOBS",
    "CARGO_BUILD_TARGET",
    "CARGO_INCREMENTAL",
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CC",
    "CXX",
    "AR",
    "CFLAGS",
    "CXXFLAGS",
    "LDFLAGS",
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    "LIBCLANG_PATH",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTFLAGS",
];
const FLAG_PREFIXES: &[&str] = &[
    "CARGO_PROFILE_",
    "CARGO_TARGET_",
    "SKIA_",
    "BINDGEN_",
    "CC_",
    "CXX_",
    "CFLAGS_",
    "CXXFLAGS_",
];

mod compatible;
pub use compatible::{restore_combined, restore_compatible, stage_focused};

#[derive(Subcommand)]
pub enum Command {
    Native {
        #[arg(long)]
        platform: String,
        #[arg(long)]
        image: String,
        #[arg(long, default_value = "full", value_parser = ["focus", "full"])]
        phase: String,
        #[arg(long, env = "GITHUB_OUTPUT")]
        github_output: Option<PathBuf>,
    },
    Application {
        #[arg(value_parser = ["identify","stage","stage-focused","restore","restore-or-prune","restore-compatible"])]
        operation: String,
        #[arg(long)]
        compatible_directory: Option<PathBuf>,
        #[arg(long, default_value = "")]
        matched_key: String,
        #[arg(long)]
        release_only: bool,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        target: PathBuf,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        native_key: String,
        #[arg(long, env = "GITHUB_OUTPUT")]
        github_output: Option<PathBuf>,
    },
    Retain {
        #[arg(long)]
        delete: bool,
        #[arg(long, value_parser = ["ubuntu-24.04", "ubuntu-24.04-arm", "macos-15"])]
        platform: Option<String>,
        #[arg(long, env = "GITHUB_REPOSITORY")]
        repository: String,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    Audit {
        #[arg(value_parser = ["identify","verify","seal","ensure"])]
        operation: String,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long, env = "GITHUB_OUTPUT")]
        github_output: Option<PathBuf>,
    },
    Prune {
        #[arg(long,default_value_os_t=util::root())]
        source: PathBuf,
        #[arg(long, env = "CARGO_TARGET_DIR")]
        target: PathBuf,
    },
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Native {
                platform,
                image,
                phase,
                github_output,
            } => {
                let root = util::root();
                let compiler = process::text(Process::new("rustc").arg("-vV"))?;
                let tree = dependency_tree(&root)?;
                let mut report = native_identity(
                    &platform,
                    &image,
                    &tree,
                    &compiler,
                    &std::env::vars().collect(),
                )?;
                let primary = report["primary"].as_str().unwrap();
                let checkpoint = if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
                    checkpoint_key(
                        primary,
                        &phase,
                        &std::env::var("GITHUB_RUN_ID")?,
                        &std::env::var("GITHUB_RUN_ATTEMPT")?,
                    )?
                } else {
                    format!("checkpoint-{primary}")
                };
                report["checkpoint"] = json!(&checkpoint);
                util::github_output(
                    github_output.as_deref(),
                    &BTreeMap::from([
                        ("primary".into(), report["primary"].as_str().unwrap().into()),
                        ("legacy".into(), report["legacy"].as_str().unwrap().into()),
                        ("checkpoint".into(), checkpoint),
                    ]),
                )?;
                util::print(&report)
            }
            Self::Application {
                operation,
                compatible_directory,
                matched_key,
                release_only,
                source,
                target,
                directory,
                native_key,
                github_output,
            } => {
                let identity =
                    application_identity_for(&source, &target, &native_key, release_only)?;
                let value = match operation.as_str() {
                    "identify" => {
                        let previous_debug = identity.previous_debug_identity()?;
                        let checkpoint = if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                        {
                            application_checkpoint(
                                &identity.checkpoint,
                                &std::env::var("GITHUB_RUN_ID")?,
                                &std::env::var("GITHUB_RUN_ATTEMPT")?,
                            )?
                        } else {
                            identity.checkpoint.clone()
                        };
                        let checkpoint_prefix = format!("{}-", identity.checkpoint);
                        util::github_output(
                            github_output.as_deref(),
                            &BTreeMap::from([
                                ("primary".into(), identity.primary.clone()),
                                ("checkpoint".into(), checkpoint.clone()),
                                ("checkpoint_prefix".into(), checkpoint_prefix.clone()),
                                ("checkpoint_legacy".into(), identity.checkpoint.clone()),
                                (
                                    "compatible_primary".into(),
                                    identity.compatible_primary.clone(),
                                ),
                                (
                                    "compatible_prefix".into(),
                                    identity.compatible_prefix.clone(),
                                ),
                                (
                                    "previous_debug_primary".into(),
                                    previous_debug.as_ref().map_or_else(String::new, |old| {
                                        old.compatible_primary.clone()
                                    }),
                                ),
                                (
                                    "previous_debug_prefix".into(),
                                    previous_debug.as_ref().map_or_else(String::new, |old| {
                                        old.compatible_prefix.clone()
                                    }),
                                ),
                            ]),
                        )?;
                        json!({"primary":identity.primary,"checkpoint":checkpoint,"checkpoint_prefix":checkpoint_prefix,"checkpoint_legacy":identity.checkpoint,"source_commit":identity.source_commit})
                    }
                    "stage" => stage(&source, &target, &directory, &identity)?,
                    "stage-focused" => stage_focused(
                        &source,
                        &target,
                        &directory,
                        compatible_directory
                            .as_deref()
                            .context("Missing compatible directory")?,
                        &matched_key,
                        &identity,
                    )?,
                    "restore" => restore(&source, &target, &directory, &identity)?,
                    "restore-compatible" => restore_combined(
                        &source,
                        &target,
                        &directory,
                        compatible_directory
                            .as_deref()
                            .context("Missing compatible directory")?,
                        &matched_key,
                        &identity,
                    )?,
                    "restore-or-prune" => match restore(&source, &target, &directory, &identity) {
                        Ok(report) => report,
                        Err(error) => {
                            eprintln!("Application cache rejected: {error:#}");
                            if release_only {
                                prune_profiles(&source, &target, &["release"])?;
                            } else {
                                prune(&source, &target)?;
                            }
                            json!({"reused":false})
                        }
                    },
                    _ => unreachable!(),
                };
                if operation.starts_with("stage") {
                    util::github_output(
                        github_output.as_deref(),
                        &BTreeMap::from([(
                            "staged".into(),
                            (value["files"].as_u64().unwrap_or(0) > 0).to_string(),
                        )]),
                    )?;
                }
                if operation.starts_with("restore") {
                    util::github_output(
                        github_output.as_deref(),
                        &BTreeMap::from([(
                            "release_reused".into(),
                            (value["reused"] == true
                                && target.join("release/lazypencil").is_file()
                                && crate::evidence::link_map::path(&target).is_file())
                            .to_string(),
                        )]),
                    )?;
                }
                util::print(&value)
            }
            Self::Retain {
                delete,
                platform,
                repository,
                manifest,
            } => {
                ensure!(
                    Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")?.is_match(&repository),
                    "Invalid repository"
                );
                let pages: Vec<Value> =
                    serde_json::from_str(&process::text(Process::new("gh").args([
                        "api",
                        "--paginate",
                        "--slurp",
                        &format!("repos/{repository}/actions/caches?per_page=100"),
                    ]))?)?;
                let entries = pages
                    .iter()
                    .flat_map(|p| {
                        p["actions_caches"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .cloned()
                    })
                    .collect::<Vec<_>>();
                let source_hash = manifest
                    .map(|p| -> Result<_> { source_fingerprint(&util::json::<Manifest>(&p)?) })
                    .transpose()?;
                let mut obsolete = obsolete(&entries, source_hash.as_deref())?;
                if let Some(platform) = platform {
                    // Each finished platform retires only its own application
                    // payloads; parallel jobs never race over dependency/tool caches.
                    let owned = Regex::new(&format!(
                        r"^(?:tests-v1|application-v2)-{}-[0-9a-f]{{64}}-",
                        regex::escape(&platform)
                    ))?;
                    obsolete.retain(|entry| {
                        entry["key"].as_str().is_some_and(|key| owned.is_match(key))
                    });
                }
                util::print(
                    &json!({"delete":delete,"bytes":obsolete.iter().filter_map(|v|v["size_in_bytes"].as_u64()).sum::<u64>(),"caches":obsolete}),
                )?;
                if delete {
                    for entry in obsolete {
                        process::checked(
                            Process::new("gh").args([
                                "api",
                                "--method",
                                "DELETE",
                                &format!("repos/{repository}/actions/caches/{}", entry["id"]),
                            ]),
                            std::time::Duration::from_secs(15),
                        )?;
                    }
                }
                Ok(())
            }
            Self::Audit {
                operation,
                directory,
                github_output,
            } => {
                if operation == "ensure" {
                    if (Self::Audit {
                        operation: "verify".into(),
                        directory: directory.clone(),
                        github_output: None,
                    })
                    .run()
                    .is_err()
                    {
                        process::checked(
                            Process::new("cargo")
                                .args([
                                    "+stable",
                                    "install",
                                    "cargo-deny",
                                    "--version",
                                    "0.20.2",
                                    "--locked",
                                    "--force",
                                    "--root",
                                ])
                                .arg(&directory)
                                .arg("--target-dir")
                                .arg(directory.with_extension("build")),
                            std::time::Duration::from_secs(600),
                        )?;
                        (Self::Audit {
                            operation: "seal".into(),
                            directory: directory.clone(),
                            github_output: None,
                        })
                        .run()?;
                    }
                    use std::io::Write;
                    let path = std::env::var_os("GITHUB_PATH").context("GITHUB_PATH required")?;
                    writeln!(
                        fs::OpenOptions::new().append(true).open(path)?,
                        "{}",
                        directory.join("bin").display()
                    )?;
                    return Ok(());
                }
                let compiler = process::text(Process::new("rustc").args(["+stable", "-vV"]))?;
                let image = std::env::var("ImageVersion")?;
                let architecture = std::env::var("RUNNER_ARCH")?;
                let fingerprint = audit_identity(&compiler, &image, &architecture)?;
                if operation == "identify" {
                    let key = format!("cargo-deny-v1-0.20.2-{fingerprint}");
                    util::github_output(
                        github_output.as_deref(),
                        &BTreeMap::from([("key".into(), key.clone())]),
                    )?;
                    println!("{key}");
                    return Ok(());
                }
                let record = audit_record(&directory, &fingerprint)?;
                let manifest = directory.join("manifest.json");
                if operation == "seal" {
                    util::publish(&manifest, &util::encode(&record)?, true)?;
                } else {
                    ensure!(
                        util::json::<Value>(&manifest)? == record,
                        "Cached audit identity or binary changed"
                    );
                }
                Ok(())
            }
            Self::Prune { source, target } => prune(&source, &target),
        }
    }
}

// Preserve the deployed cache wire format. A language migration must not force
// an otherwise identical dependency or application cache to miss.
pub fn stable_json(value: &Value) -> Vec<u8> {
    fn string(s: &str) -> String {
        let encoded = serde_json::to_string(s).unwrap();
        let mut out = String::new();
        for c in encoded.chars() {
            if c.is_ascii() {
                out.push(c);
            } else {
                for u in c.encode_utf16(&mut [0; 2]).iter() {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
        out
    }
    fn write(value: &Value) -> String {
        match value {
            Value::Array(a) => format!("[{}]", a.iter().map(write).collect::<Vec<_>>().join(", ")),
            Value::Object(o) => {
                let sorted = o.iter().collect::<BTreeMap<_, _>>();
                format!(
                    "{{{}}}",
                    sorted
                        .into_iter()
                        .map(|(k, v)| format!("{}: {}", string(k), write(v)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            Value::String(s) => string(s),
            v => v.to_string(),
        }
    }
    write(value).into_bytes()
}

pub fn compile_flags(environment: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    environment
        .iter()
        .filter(|(k, v)| {
            !v.is_empty()
                && k.as_str() != "CARGO_TARGET_DIR"
                && (FLAG_NAMES.contains(&k.as_str())
                    || FLAG_PREFIXES.iter().any(|p| k.starts_with(p)))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

pub fn dependency_tree(root: &Path) -> Result<String> {
    ensure!(
        process::checked(
            Process::new("git")
                .arg("-C")
                .arg(root)
                .args(["diff", "--quiet", "HEAD", "--"])
                .args(INPUTS),
            std::time::Duration::from_secs(15)
        )
        .is_ok(),
        "Dependency inputs must match committed source"
    );
    let untracked = process::text(
        Process::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-files", "--others", "--exclude-standard", "--"])
            .args(INPUTS),
    )?;
    ensure!(untracked.is_empty(), "Untracked dependency inputs");
    let bytes = process::checked(
        Process::new("git")
            .args(["-c", "core.quotePath=true", "-C"])
            .arg(root)
            .args(["ls-tree", "-r", "--full-tree", "HEAD", "--"])
            .args(INPUTS),
        std::time::Duration::from_secs(15),
    )?;
    ensure!(
        bytes.windows(12).any(|s| s == b"\tCargo.lock\n")
            && bytes.windows(12).any(|s| s == b"\tCargo.toml\n"),
        "Missing dependency manifests"
    );
    Ok(util::digest(bytes))
}

pub fn native_identity(
    platform: &str,
    image: &str,
    tree: &str,
    compiler: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Value> {
    ensure!(
        [
            "ubuntu-24.04",
            "ubuntu-24.04-arm",
            "macos-15",
            "macos-15-intel",
            "windows-2025"
        ]
        .contains(&platform)
            && image != "unknown"
            && Regex::new(r"^[A-Za-z0-9_.-]{1,100}$")?.is_match(image)
            && Regex::new(r"^[0-9a-f]{64}$")?.is_match(tree),
        "Unknown native identity"
    );
    let flags = compile_flags(environment);
    // The native-v2 fingerprint uses compact separators, unlike the older
    // application cache payload. Keep both formats explicit and tested.
    let hash = |tree: &str| {
        util::digest(
            serde_json::to_vec(&json!({"inputs":tree,"rustc":compiler,"flags":flags})).unwrap(),
        )
    };
    let primary = format!("native-v2-{platform}-{image}-{}", hash(tree));
    let baseline = BTreeMap::from([
        ("CARGO_BUILD_JOBS".into(), "1".into()),
        ("CARGO_INCREMENTAL".into(), "0".into()),
        ("CARGO_PROFILE_DEV_DEBUG".into(), "0".into()),
        ("CARGO_PROFILE_TEST_DEBUG".into(), "0".into()),
    ]);
    let audited = compiler.lines().any(|s| s == "release: 1.94.0")
        && compiler
            .lines()
            .any(|s| s == "commit-hash: 4a4ef493e3a1488c6e321570238084b38948f6db")
        && flags == baseline;
    let previous = match tree {
        // The tool migration adds only a terminal test dependency and a Cargo alias.
        // Production dependency declarations, native sources and build flags are unchanged.
        // Removing the retired Shell entry points does not change dependency build inputs.
        // New byte-preservation attributes apply only to xtask notice files.
        "b8192b978f7a069656ed03fe0fc8bd0f97725ce33cd9f1787360534b73e8b397" => {
            Some("b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d")
        }
        "c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7" => {
            Some("54c7d2d9cc2e1b5e2d3a0297c8648f27b1aff64c76c685a30e63864b41d60a9f")
        }
        "b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d" => {
            Some("c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7")
        }
        _ => None,
    };
    let legacy = if audited {
        previous
            .map(|t| format!("native-v2-{platform}-{image}-{}", hash(t)))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(
        json!({"primary":primary,"legacy":legacy,"dependency_fingerprint":tree,"build_fingerprint":hash(tree),"compile_flag_names":flags.keys().collect::<Vec<_>>()}),
    )
}

// Hosted run IDs and retry numbers have one canonical positive integer spelling.
fn valid_generation_id(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|n| n > 0)
}

/// Keep later completed focused outputs distinct from an older immutable cache.
pub fn application_checkpoint(base: &str, run: &str, attempt: &str) -> Result<String> {
    ensure!(
        Regex::new(r"^(?:application-v2|release-v1)-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-[0-9a-f]{64}-[0-9a-f]{64}-focus$")?.is_match(base)
            && [run, attempt].into_iter().all(valid_generation_id),
        "Invalid application checkpoint identity"
    );
    Ok(format!("{base}-{run}-{attempt}"))
}

/// Cache entries are immutable; each completed phase needs its own checkpoint.
pub fn checkpoint_key(primary: &str, phase: &str, run: &str, attempt: &str) -> Result<String> {
    ensure!(
        primary.starts_with("native-v2-")
            && primary.len() <= 300
            && Regex::new(r"^native-v2-[A-Za-z0-9_.-]+-[0-9a-f]{64}$")?.is_match(primary)
            && ["focus", "full"].contains(&phase)
            && [run, attempt].into_iter().all(valid_generation_id),
        "Invalid native checkpoint identity"
    );
    Ok(format!("checkpoint-{primary}-{phase}-{run}-{attempt}"))
}

pub fn compiled_inputs(manifest: &Manifest) -> Vec<Entry> {
    let mut entries = manifest
        .files
        .iter()
        // Native notice review data is consumed by the evidence collector,
        // not embedded in the application. Compiler dep-info still rejects
        // reuse if an excluded file ever becomes an actual source input.
        .filter(|entry| {
            !focus::ci_only(&entry.path) && !entry.path.starts_with("resources/licenses/native/")
        })
        .cloned()
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

pub fn source_fingerprint(manifest: &Manifest) -> Result<String> {
    Ok(util::digest(stable_json(&serde_json::to_value(
        compiled_inputs(manifest),
    )?)))
}

#[derive(Clone)]
pub struct ApplicationIdentity {
    pub fingerprint: String,
    pub source_commit: String,
    pub primary: String,
    pub checkpoint: String,
    pub tracked: BTreeMap<String, Entry>,
    pub compatible_primary: String,
    pub compatible_prefix: String,
    recipe: Value,
    release_only: bool,
}

impl ApplicationIdentity {
    /// One reviewed release-only transition. All native/default/test inputs
    /// remain bound to the old fingerprint; callers may restore debug only.
    pub fn previous_debug_identity(&self) -> Result<Option<Self>> {
        if self.release_only
            || self.recipe["release_recipe"] != "root-rustc-release-map-app-cgu16-v2"
        {
            return Ok(None);
        }
        let mut old = self.clone();
        old.recipe["release_recipe"] = json!("root-rustc-release-map-v1");
        old.fingerprint = util::digest(stable_json(&old.recipe));
        old.primary = self.primary.replace(&self.fingerprint, &old.fingerprint);
        old.checkpoint = self.checkpoint.replace(&self.fingerprint, &old.fingerprint);
        let suffix = format!("{}-full-", compatible_fingerprint(&self.recipe)?);
        let prefix = self
            .compatible_prefix
            .strip_suffix(&suffix)
            .context("Invalid compatible prefix")?;
        old.compatible_prefix = format!("{prefix}{}-full-", compatible_fingerprint(&old.recipe)?);
        old.compatible_primary =
            self.compatible_primary
                .replacen(&self.compatible_prefix, &old.compatible_prefix, 1);
        Ok(Some(old))
    }
}

fn compatible_fingerprint(recipe: &Value) -> Result<String> {
    let inputs: Vec<Entry> = serde_json::from_value(recipe["inputs"].clone())?;
    let mut common = recipe.clone();
    common["inputs"] = serde_json::to_value(
        inputs
            .iter()
            .filter(|entry| !compatible::test_source(&entry.path))
            .collect::<Vec<_>>(),
    )?;
    common["scope"] = json!("integration-tests-v1");
    Ok(util::digest(stable_json(&common)))
}

pub fn application_identity(
    source: &Path,
    target: &Path,
    native_key: &str,
) -> Result<ApplicationIdentity> {
    application_identity_for(source, target, native_key, false)
}

pub fn release_identity(
    source: &Path,
    target: &Path,
    native_key: &str,
) -> Result<ApplicationIdentity> {
    application_identity_for(source, target, native_key, true)
}

fn application_identity_for(
    source: &Path,
    target: &Path,
    native_key: &str,
    release_only: bool,
) -> Result<ApplicationIdentity> {
    let manifest: Manifest = util::json(&source.join(dist::MANIFEST))?;
    let package: toml::Value = toml::from_str(&fs::read_to_string(source.join("Cargo.toml"))?)?;
    ensure!(
        !source.join("build.rs").exists()
            && package["package"]
                .get("build")
                .is_none_or(|v| v.as_bool() == Some(false)),
        "Custom build scripts disable application reuse"
    );
    let re = Regex::new(r"^native-v2-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-")?;
    let platform = re
        .captures(native_key)
        .context("Unknown native build identity")?[1]
        .to_owned();
    ensure!(
        package["package"]["name"].as_str() == Some("lazypencil"),
        "Unknown application package"
    );
    let inputs = compiled_inputs(&manifest)
        .into_iter()
        .filter(|entry| {
            !release_only
                || (!entry.path.starts_with("tests/") && !entry.path.starts_with("xtask/"))
        })
        .collect::<Vec<_>>();
    let target = std::path::absolute(target)?;
    // Preserve existing all-output cache identities while separating release
    // files from test-only inputs. Actual compiler dep-info still rejects any
    // excluded file that becomes a compiled release input.
    let mut recipe = json!({"version":3,"release_recipe":crate::evidence::link_map::RECIPE,"inputs":inputs,"native":native_key,"source":source.canonicalize()?,"target":target});
    if release_only {
        recipe["scope"] = json!("release-v1");
    }
    let fingerprint = util::digest(stable_json(&recipe));
    let source_hash = util::digest(stable_json(&serde_json::to_value(&inputs)?));
    let namespace = if release_only {
        "release-v1"
    } else {
        "application-v2"
    };
    let compatible_prefix = format!(
        "tests-v1-{platform}-{}-full-",
        compatible_fingerprint(&recipe)?
    );
    Ok(ApplicationIdentity {
        fingerprint: fingerprint.clone(),
        source_commit: manifest.git["revision"]
            .as_str()
            .context("Missing committed source")?
            .into(),
        primary: format!("{namespace}-{platform}-{source_hash}-{fingerprint}-full"),
        checkpoint: format!("{namespace}-{platform}-{source_hash}-{fingerprint}-focus"),
        tracked: inputs.into_iter().map(|e| (e.path.clone(), e)).collect(),
        compatible_primary: format!("{compatible_prefix}{source_hash}"),
        compatible_prefix,
        recipe,
        release_only,
    })
}

pub fn allowed(name: &str) -> bool {
    if name == crate::evidence::link_map::CACHE_PATH {
        return true;
    }
    if util::safe_path(name).is_err() {
        return false;
    }
    let parts = name.split('/').collect::<Vec<_>>();
    if parts.len() < 2 || !["debug", "release"].contains(&parts[0]) {
        return false;
    }
    match &parts[1..] {
        [name] => [
            "lazypencil",
            "lazypencil-test-helper",
            "liblazypencil.rlib",
            "liblazypencil.rmeta",
            "liblazypencil.dylib",
            "liblazypencil.so",
        ]
        .contains(name),
        [".fingerprint", package, _] => Regex::new(r"^lazypencil-[0-9a-f]{16}$")
            .unwrap()
            .is_match(package),
        ["deps", file] => {
            Regex::new(r"^(?:lib)?[A-Za-z0-9_]+-[0-9a-f]{16}(?:\.(?:d|rlib|rmeta|dylib|so))?$")
                .unwrap()
                .is_match(file)
        }
        ["examples", file] => Regex::new(r"^[A-Za-z0-9_]+(?:-[0-9a-f]{16})?(?:\.(?:d|rmeta))?$")
            .unwrap()
            .is_match(file),
        _ => false,
    }
}

pub fn outputs(target: &Path) -> Result<Vec<PathBuf>> {
    let mut files = BTreeSet::new();
    let marker = Regex::new(
        r"^(?:test-integration-test-|test-bin-|test-lib-|bin-|lib-|example-)([A-Za-z0-9_-]+)\.json$",
    )?;
    let fingerprint = Regex::new(r"^lazypencil-([0-9a-f]{16})$")?;
    for profile in ["debug", "release"] {
        let root = target.join(profile);
        let fingerprints = root.join(".fingerprint");
        if fingerprints.is_dir() {
            for directory in fs::read_dir(&fingerprints)? {
                let directory = directory?;
                let name = directory.file_name();
                let name = name.to_string_lossy();
                let Some(captures) = fingerprint.captures(&name) else {
                    continue;
                };
                let suffix = &captures[1];
                for item in fs::read_dir(directory.path())? {
                    let item = item?;
                    let name = item.file_name();
                    let name = name.to_string_lossy();
                    if let Some(captures) = marker.captures(&name) {
                        let example = name.starts_with("example-");
                        let name = captures[1].replace('-', "_");
                        let artifacts = root.join(if example { "examples" } else { "deps" });
                        if !artifacts.is_dir() {
                            continue;
                        }
                        for item in fs::read_dir(artifacts)? {
                            let item = item?;
                            let file = item.file_name();
                            let file = file.to_string_lossy();
                            if file.starts_with(&format!("{name}-{suffix}"))
                                || file.starts_with(&format!("lib{name}-{suffix}"))
                                || example && (file == name || file == format!("{name}.d"))
                            {
                                files.insert(item.path());
                            }
                        }
                    }
                    files.insert(item.path());
                }
            }
        }
        for name in [
            "lazypencil",
            "lazypencil-test-helper",
            "liblazypencil.rlib",
            "liblazypencil.rmeta",
            "liblazypencil.dylib",
            "liblazypencil.so",
        ] {
            let path = root.join(name);
            if path.exists() {
                files.insert(path);
            }
        }
    }
    let map = crate::evidence::link_map::path(target);
    ensure!(
        map.exists() == target.join("release/lazypencil").exists(),
        "Release cache requires the executable and link map together"
    );
    if map.exists() {
        files.insert(map);
    }
    ensure!(
        !files.is_empty()
            && files.iter().all(|p| p.is_file()
                && !p.is_symlink()
                && allowed(&p.strip_prefix(target).unwrap().to_string_lossy())),
        "Missing or unsupported application outputs"
    );
    Ok(files.into_iter().collect())
}

pub fn input_environment(
    files: &[PathBuf],
    source: &Path,
    tracked: &BTreeMap<String, Entry>,
) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::new();
    let mut checked = BTreeSet::new();
    let mut count = 0;
    for path in files
        .iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("d"))
    {
        count += 1;
        for line in fs::read_to_string(path)?.replace("\\\n", "").lines() {
            if let Some(line) = line.strip_prefix("# env-dep:") {
                let name = line.split('=').next().unwrap();
                if !name.starts_with("CARGO_") {
                    environment.insert(
                        name.to_owned(),
                        util::digest(stable_json(&serde_json::to_value(
                            std::env::var(name).ok(),
                        )?)),
                    );
                }
            }
        }
        for relative in compiler_inputs(path, source)? {
            let entry = tracked.get(&relative).context("Unbound compiler input")?;
            if checked.insert(relative.clone()) {
                ensure!(
                    util::file_digest(&source.join(relative))? == entry.sha256,
                    "Compiler source input changed"
                );
            }
        }
    }
    ensure!(count > 0, "Application reuse requires compiler dep-info");
    Ok(environment)
}

fn compiler_inputs(path: &Path, source: &Path) -> Result<BTreeSet<String>> {
    let root = source.canonicalize()?;
    let mut inputs = BTreeSet::new();
    for line in fs::read_to_string(path)?.replace("\\\n", "").lines() {
        if !line.starts_with('#')
            && let Some((_, deps)) = line.split_once(": ")
        {
            for token in shell_words::split(deps)? {
                let path = PathBuf::from(token);
                let path = if path.is_absolute() {
                    path
                } else {
                    source.join(path)
                };
                ensure!(!path.is_symlink(), "Linked compiler input");
                inputs.insert(
                    path.canonicalize()?
                        .strip_prefix(&root)
                        .context("External compiler input")?
                        .to_str()
                        .context("Invalid compiler path")?
                        .to_owned(),
                );
            }
        }
    }
    Ok(inputs)
}

#[derive(Serialize, Deserialize)]
struct OutputEntry {
    path: String,
    sha256: String,
    bytes: u64,
    mode: u32,
}
#[derive(Serialize, Deserialize)]
struct ApplicationReport {
    fingerprint: String,
    source_commit: String,
    environment: BTreeMap<String, String>,
    files: Vec<OutputEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inputs: Option<Vec<Entry>>,
}

pub fn stage(
    source: &Path,
    target: &Path,
    directory: &Path,
    identity: &ApplicationIdentity,
) -> Result<Value> {
    stage_filtered(source, target, directory, identity, None)
}

fn stage_filtered(
    source: &Path,
    target: &Path,
    directory: &Path,
    identity: &ApplicationIdentity,
    selected: Option<&BTreeSet<String>>,
) -> Result<Value> {
    let files = outputs(target)?
        .into_iter()
        .filter(|path| {
            selected.is_none_or(|names| {
                names.contains(
                    path.strip_prefix(target)
                        .unwrap()
                        .to_str()
                        .unwrap_or_default(),
                )
            }) && (!identity.release_only
                || path
                    .strip_prefix(target)
                    .is_ok_and(|p| p.starts_with("release")))
        })
        .collect::<Vec<_>>();
    if files.is_empty() && selected.is_some() {
        if directory.exists() {
            fs::remove_dir_all(directory)?;
        }
        return Ok(json!({"files":0,"bytes":0}));
    }
    ensure!(
        !identity.release_only
            || files
                .iter()
                .any(|p| p == &target.join("release/lazypencil")),
        "Release cache requires a completed mapped executable"
    );
    let environment = input_environment(&files, source, &identity.tracked)?;
    if directory.exists() {
        fs::remove_dir_all(directory)?;
    }
    fs::create_dir_all(directory)?;
    let mut entries = Vec::new();
    for path in files {
        let name = path
            .strip_prefix(target)?
            .to_str()
            .context("Invalid output filename")?
            .to_owned();
        let destination = directory.join("files").join(&name);
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::hard_link(&path, &destination)?;
        entries.push(OutputEntry {
            path: name,
            sha256: util::file_digest(&path)?,
            bytes: fs::metadata(&path)?.len(),
            mode: util::raw_mode(&path)?,
        });
    }
    let result = json!({"files":entries.len(),"bytes":entries.iter().map(|e|e.bytes).sum::<u64>()});
    util::publish(
        &directory.join("manifest.json"),
        &util::encode(&ApplicationReport {
            fingerprint: identity.fingerprint.clone(),
            source_commit: identity.source_commit.clone(),
            environment,
            files: entries,
            inputs: (!identity.release_only).then(|| identity.tracked.values().cloned().collect()),
        })?,
        false,
    )?;
    Ok(result)
}

pub fn restore(
    source: &Path,
    target: &Path,
    directory: &Path,
    identity: &ApplicationIdentity,
) -> Result<Value> {
    restore_filtered(source, target, directory, identity, &BTreeSet::new())
}

fn restore_filtered(
    source: &Path,
    target: &Path,
    directory: &Path,
    identity: &ApplicationIdentity,
    excluded: &BTreeSet<String>,
) -> Result<Value> {
    let report: ApplicationReport = util::json(&directory.join("manifest.json"))?;
    ensure!(
        report.fingerprint == identity.fingerprint,
        "Application source or build identity changed"
    );
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let directory = directory.canonicalize()?;
    for entry in &report.files {
        let path = directory.join("files").join(&entry.path);
        ensure!(
            allowed(&entry.path)
                && (!identity.release_only || entry.path.starts_with("release/"))
                && names.insert(entry.path.clone())
                && !path.is_symlink()
                && path.is_file()
                && path.canonicalize()? == path,
            "Unsafe cached output"
        );
        ensure!(
            fs::metadata(&path)?.len() == entry.bytes
                && util::raw_mode(&path)? == entry.mode
                && util::file_digest(&path)? == entry.sha256,
            "Cached output integrity changed"
        );
        if !excluded.contains(&entry.path) {
            files.push(path);
        }
    }
    ensure!(
        names.contains("release/lazypencil")
            == names.contains(crate::evidence::link_map::CACHE_PATH)
            && (!identity.release_only || names.contains("release/lazypencil")),
        "Cached release executable and map are incomplete"
    );
    ensure!(
        !files.is_empty()
            && input_environment(&files, source, &identity.tracked)? == report.environment,
        "Compiler environment changed"
    );
    for name in &names {
        let destination = target.join(name);
        let mut current = target.to_path_buf();
        for component in Path::new(name).components() {
            current.push(component);
            ensure!(!current.is_symlink(), "Linked cache destination");
        }
        ensure!(!destination.is_symlink(), "Linked cache destination");
    }
    for name in excluded {
        let destination = target.join(name);
        if destination.exists() {
            fs::remove_file(destination)?;
        }
    }
    for path in &files {
        let destination = target.join(path.strip_prefix(directory.join("files"))?);
        fs::create_dir_all(destination.parent().unwrap())?;
        if destination.exists() {
            // Identical focused outputs must not invalidate compatible dependants
            // merely by replacing their library with a newer filesystem timestamp.
            if util::raw_mode(&destination)? == util::raw_mode(path)?
                && util::file_digest(&destination)? == util::file_digest(path)?
            {
                continue;
            }
            fs::remove_file(&destination)?;
        }
        fs::hard_link(path, &destination)?;
    }
    Ok(
        json!({"reused":true,"files":files.len(),"producer":report.source_commit,"source":identity.source_commit}),
    )
}

pub fn prune(source: &Path, target: &Path) -> Result<()> {
    prune_profiles(source, target, &["dev", "release"])
}

fn prune_profiles(source: &Path, target: &Path, profiles: &[&str]) -> Result<()> {
    ensure!(
        target.is_absolute() && target.parent().is_some(),
        "Explicit non-root target directory required"
    );
    let map = crate::evidence::link_map::path(target);
    if map.exists() || map.is_symlink() {
        fs::remove_file(map)?;
    }
    for name in [
        "release-smoke",
        "release-dashboard",
        "release-graphics",
        "release-benchmark",
    ] {
        for extension in ["png", "json"] {
            let path = target.join(format!("{name}.{extension}"));
            if path.exists() || path.is_symlink() {
                fs::remove_file(path)?;
            }
        }
    }
    for profile in profiles {
        process::checked(
            Process::new("cargo")
                .current_dir(source)
                .env("CARGO_TARGET_DIR", target)
                .args([
                    "clean",
                    "--locked",
                    "--package",
                    "lazypencil",
                    "--profile",
                    profile,
                ]),
            std::time::Duration::from_secs(180),
        )?;
    }
    Ok(())
}

fn older_than_latest(mut entries: Vec<&Value>, keep: usize, selected: &mut BTreeMap<u64, Value>) {
    entries.sort_by(|a, b| {
        b["created_at"]
            .as_str()
            .cmp(&a["created_at"].as_str())
            .then_with(|| b["id"].as_u64().cmp(&a["id"].as_u64()))
    });
    for entry in entries.into_iter().skip(keep) {
        selected.insert(entry["id"].as_u64().unwrap(), entry.clone());
    }
}

pub fn obsolete(entries: &[Value], source_hash: Option<&str>) -> Result<Vec<Value>> {
    let entries = entries
        .iter()
        .filter(|e| e["ref"] == "refs/heads/main")
        .collect::<Vec<_>>();
    let keys = entries
        .iter()
        .map(|e| {
            (
                e["key"].as_str().unwrap_or_default(),
                e["version"].as_str().unwrap_or_default(),
            )
        })
        .collect::<BTreeSet<_>>();
    let re = Regex::new(
        r"^application-v([12])-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-([0-9a-f]{64})(?:-([0-9a-f]{64}))?-(full|focus)(?:-[1-9][0-9]*-[1-9][0-9]*)?$",
    )?;
    let releases = Regex::new(
        r"^release-v1-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-[0-9a-f]{64}-[0-9a-f]{64}-(full|focus)(?:-[1-9][0-9]*-[1-9][0-9]*)?$",
    )?;
    let tests = Regex::new(
        r"^tests-v1-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-[0-9a-f]{64}-full-([0-9a-f]{64})$",
    )?;
    let compatible_sources = keys
        .iter()
        .filter_map(|(key, _)| tests.captures(key))
        .map(|caps| (caps[1].to_owned(), caps[2].to_owned()))
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeMap::new();
    let mut groups = BTreeMap::<(String, String), Vec<&Value>>::new();
    let tools = Regex::new(
        r"^(xtask-v1-(?:Linux|macOS|Windows)-(?:X64|ARM64)-[A-Za-z0-9_.]+-rust[0-9.]+)-[0-9a-f]{64}$",
    )?;
    let mut tool_groups = BTreeMap::<(String, String), Vec<&Value>>::new();
    let generations =
        Regex::new(r"^(native-v2-.+-[0-9a-f]{64})-(?:focus|full)-[1-9][0-9]*-[1-9][0-9]*$")?;
    let mut checkpoints = BTreeMap::<(String, String), Vec<&Value>>::new();
    for entry in entries {
        let key = entry["key"].as_str().context("Invalid cache key")?;
        let version = entry["version"].as_str().context("Invalid cache version")?;
        let id = entry["id"].as_u64().context("Invalid cache id")?;
        if let Some(caps) = tools.captures(key) {
            tool_groups
                .entry((caps[1].into(), version.into()))
                .or_default()
                .push(entry);
        }
        if let Some(suffix) = key
            .strip_prefix("checkpoint-")
            .filter(|s| s.starts_with("native-v2-"))
        {
            let primary = generations
                .captures(suffix)
                .map_or_else(|| suffix.to_owned(), |caps| caps[1].to_owned());
            if keys.contains(&(primary.as_str(), version)) {
                selected.insert(id, entry.clone());
            } else {
                checkpoints
                    .entry((primary, version.into()))
                    .or_default()
                    .push(entry);
            }
        }
        if let Some(caps) = re.captures(key) {
            let phase = &caps[5];
            // Full compatible payloads replace the old full cache and any
            // same-source focus checkpoints, rather than duplicating gigabytes
            // of identical binaries. Never delete the old fallback first.
            if &caps[1] == "2" && compatible_sources.contains(&(caps[2].into(), caps[3].into())) {
                selected.insert(id, entry.clone());
                continue;
            }
            if source_hash.is_some_and(|s| &caps[1] == "1" || phase == "focus" && s != &caps[3]) {
                selected.insert(id, entry.clone());
                continue;
            }
            if phase == "focus"
                && keys.contains(&(
                    format!("{}full", &key[..caps.get(5).unwrap().start()]).as_str(),
                    version,
                ))
            {
                selected.insert(id, entry.clone());
            } else {
                groups
                    .entry((caps[2].into(), phase.into()))
                    .or_default()
                    .push(entry);
            }
        }
        if let Some(caps) = releases.captures(key) {
            let phase = &caps[2];
            if phase == "focus"
                && keys.contains(&(
                    format!("{}full", &key[..caps.get(2).unwrap().start()]).as_str(),
                    version,
                ))
            {
                selected.insert(id, entry.clone());
            } else {
                // The current all-output source hash includes tests and must
                // not invalidate a compatible release-only checkpoint.
                groups
                    .entry((format!("release:{}", &caps[1]), phase.into()))
                    .or_default()
                    .push(entry);
            }
        }
        if let Some(caps) = tests.captures(key) {
            groups
                .entry((format!("tests:{}:{version}", &caps[1]), "full".into()))
                .or_default()
                .push(entry);
        }
    }
    for ((_, phase), entries) in groups {
        // A new push must not discard the last full build before its replacement
        // exists: a release-evidence job may still request that exact source.
        // Restore continues to require matching source and compiler identities.
        let keep = if phase == "focus" || source_hash.is_some() {
            1
        } else {
            2
        };
        older_than_latest(entries, keep, &mut selected);
    }
    // Old tool sources can rebuild against their retained dependency cache.
    // Keep one complete generation per runner/compiler/cache format so cheap
    // tool outputs do not evict expensive native dependencies.
    for entries in tool_groups.into_values() {
        older_than_latest(entries, 1, &mut selected);
    }
    for entries in checkpoints.into_values() {
        older_than_latest(entries, 1, &mut selected);
    }
    Ok(selected.into_values().collect())
}

pub fn audit_identity(compiler: &str, image: &str, architecture: &str) -> Result<String> {
    ensure!(
        !image.is_empty() && image != "unknown" && !architecture.is_empty(),
        "Unknown audit runner identity"
    );
    Ok(util::digest(stable_json(
        &json!({"compiler":compiler,"image":image,"architecture":architecture,"version":"0.20.2"}),
    )))
}
pub fn audit_record(directory: &Path, fingerprint: &str) -> Result<Value> {
    let binary = directory.join("bin/cargo-deny");
    let version = process::text(Process::new(&binary).arg("--version"))?;
    ensure!(
        version.trim() == "cargo-deny 0.20.2",
        "Unexpected cargo-deny version"
    );
    Ok(
        json!({"fingerprint":fingerprint,"sha256":util::file_digest(&binary)?,"version":version.trim()}),
    )
}
