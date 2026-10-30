//! Source-bound binary candidates, including every original collected notice.

use super::*;
use clap::Args;

pub const MANIFEST: &str = "PACKAGE-MANIFEST.json";
const SOURCE: &str = "source/lazypencil-source.tar.gz";
const EXECUTABLE: &str = "bin/lazypencil";
const NOTICE_FILES: &[&str] = &[
    "licenses/rust/RUST-NOTICES.txt",
    "licenses/native/skcms/LICENSE",
    "licenses/native/skcms/NOTICE.txt",
    "licenses/native/clang-neon/ORIGINAL-NOTICE.txt",
    "licenses/native/clang-neon/NOTICE.txt",
    "licenses/shader-preprocessor/packages/RUST-NOTICES.txt",
    "licenses/shader-preprocessor/toolchain/inventory/TOOLCHAIN-NOTICES.txt",
    "quickjs/QUICKJS-NOTICES.txt",
    "skia/SKIA-NOTICES.txt",
    "skia-wrapper/WRAPPER-NOTICES.txt",
    "assets/ASSET-NOTICES.txt",
    "toolchain/inventory/TOOLCHAIN-NOTICES.txt",
];

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    pub executable: PathBuf,
    #[arg(long)]
    pub source_archive: PathBuf,
    #[arg(long)]
    pub evidence: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceEntry {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceManifest {
    format_version: u32,
    files: Vec<EvidenceEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    format_version: u32,
    kind: String,
    release_ready: bool,
    package: Value,
    target: String,
    source_revision: String,
    files: Vec<Entry>,
}

fn evidence_files(root: &Path) -> Result<Files> {
    let paths = util::walk(root)?;
    ensure!(paths.len() < MAX_FILES, "Too many evidence inputs");
    let mut files = Files::new();
    let mut total = 0;
    for path in paths {
        let name = path
            .strip_prefix(root)?
            .components()
            .map(|part| part.as_os_str().to_str().context("Non-UTF-8 evidence path"))
            .collect::<Result<Vec<_>>>()?
            .join("/");
        util::safe_path(&name)?;
        let bytes = util::read(&path, MAX_BYTES - total)?;
        total += bytes.len() as u64;
        ensure!(
            files.insert(name, (bytes, 0o644)).is_none(),
            "Duplicate evidence filename"
        );
    }
    validate_evidence(&files)?;
    Ok(files)
}

fn validate_evidence(files: &Files) -> Result<()> {
    let bytes = &files
        .get("manifest.json")
        .context("Missing evidence index")?
        .0;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "Oversized evidence manifest"
    );
    let index: EvidenceManifest = serde_json::from_slice(bytes)?;
    ensure!(
        index.format_version == 1 && index.files.len() + 1 == files.len(),
        "Evidence membership mismatch"
    );
    let mut names = BTreeSet::from(["manifest.json".to_owned()]);
    for entry in index.files {
        util::safe_path(&entry.path)?;
        ensure!(names.insert(entry.path.clone()), "Duplicate evidence input");
        let bytes = &files.get(&entry.path).context("Missing evidence input")?.0;
        ensure!(
            bytes.len() as u64 == entry.bytes && util::digest(bytes) == entry.sha256,
            "Evidence input changed: {}",
            entry.path
        );
    }
    for name in NOTICE_FILES {
        ensure!(
            files.get(*name).is_some_and(|(bytes, _)| !bytes.is_empty()),
            "Missing original notices: {name}"
        );
    }
    Ok(())
}

fn json_file(files: &Files, name: &str) -> Result<Value> {
    let bytes = &files
        .get(name)
        .with_context(|| format!("Missing {name}"))?
        .0;
    ensure!(
        bytes.len() as u64 <= util::MAX_FILE,
        "Oversized JSON evidence: {name}"
    );
    Ok(serde_json::from_slice(bytes)?)
}

fn executable_target(bytes: &[u8], target: &str) -> Result<()> {
    let valid = match target {
        "aarch64-apple-darwin" => {
            bytes.len() >= 32
                && bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe]
                && u32::from_le_bytes(bytes[4..8].try_into()?) == 0x0100000c
                && u32::from_le_bytes(bytes[12..16].try_into()?) == 2
        }
        "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu" => {
            bytes.len() >= 64
                && bytes[..4] == *b"\x7fELF"
                && bytes[4] == 2
                && bytes[5] == 1
                && [2, 3].contains(&u16::from_le_bytes(bytes[16..18].try_into()?))
                && u16::from_le_bytes(bytes[18..20].try_into()?)
                    == if target.starts_with("aarch64") {
                        183
                    } else {
                        62
                    }
        }
        _ => false,
    };
    ensure!(
        valid,
        "Executable format or architecture differs from the candidate target"
    );
    Ok(())
}

fn validate(files: &Files, package: &Package) -> Result<()> {
    ensure!(
        package.format_version == 1
            && package.kind == "release-candidate"
            && !package.release_ready,
        "Unsupported binary candidate manifest"
    );
    ensure!(
        package.source_revision.len() == 40
            && package
                .source_revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Invalid source revision"
    );
    ensure!(
        [
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
        ]
        .contains(&package.target.as_str()),
        "Unsupported initial-release target"
    );
    let source = &files
        .get(SOURCE)
        .context("Missing matching source archive")?
        .0;
    let temporary = tempfile::NamedTempFile::new()?;
    fs::write(temporary.path(), source)?;
    let verified = super::verify(temporary.path(), None)?;
    ensure!(
        verified["git"] == json!({"revision":package.source_revision,"dirty":false})
            && verified["package"] == package.package
            && package.package["name"] == "lazypencil",
        "Binary/source package identity mismatch"
    );
    let source_members = read_archive(temporary.path())?;
    let source_prefix = format!(
        "{}-{}",
        package.package["name"]
            .as_str()
            .context("Missing package name")?,
        package.package["version"]
            .as_str()
            .context("Missing package version")?
    );
    let source_manifest = &source_members
        .get(&format!("{source_prefix}/{}", super::MANIFEST))
        .context("Missing source manifest")?
        .0;
    let evidence = files
        .iter()
        .filter_map(|(name, value)| {
            name.strip_prefix("build-evidence/")
                .map(|name| (name.into(), value.clone()))
        })
        .collect::<Files>();
    validate_evidence(&evidence)?;
    ensure!(
        &evidence
            .get(super::MANIFEST)
            .context("Missing build source manifest")?
            .0
            == source_manifest,
        "Build evidence belongs to different source bytes"
    );
    let build = json_file(&evidence, "build.json")?;
    ensure!(
        build["format_version"] == 1
            && build["release_ready"].is_boolean()
            && build["remaining_review"]
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
        "Missing build review status"
    );
    ensure!(
        build["source"]["revision"] == package.source_revision
            && build["source"]["manifest_sha256"] == util::digest(source_manifest)
            && build["source"]["package"] == package.package,
        "Build source identity mismatch"
    );
    let binary = &files
        .get(EXECUTABLE)
        .context("Missing packaged executable")?
        .0;
    executable_target(binary, &package.target)?;
    ensure!(
        !binary.is_empty()
            && build["binary"]["sha256"] == util::digest(binary)
            && build["binary"]["bytes"] == binary.len(),
        "Build evidence belongs to another executable"
    );
    let compiler = build["compiler"]
        .as_str()
        .context("Missing compiler identity")?;
    let hosts = compiler
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect::<Vec<_>>();
    ensure!(
        hosts == [package.target.as_str()],
        "Compiler target mismatch"
    );
    let linkage = json_file(&evidence, "linkage/linkage.json")?;
    ensure!(
        linkage == build["dynamic_linkage"]
            && linkage["host"] == package.target
            && linkage["binary_sha256"] == build["binary"]["sha256"],
        "Binary linkage identity mismatch"
    );
    let map = &evidence
        .get("link-map.map")
        .context("Missing raw linker map")?
        .0;
    ensure!(
        !map.is_empty()
            && build["static_link_map"]["sha256"] == util::digest(map)
            && build["static_link_map"]["bytes"] == map.len(),
        "Linker map identity mismatch"
    );
    let inputs = build
        .get("static_link_inputs")
        .filter(|value| !value.is_null());
    ensure!(
        inputs.is_some() == evidence.contains_key("link-map-inputs.json"),
        "Missing link-map archive inventory binding"
    );
    if let Some(inputs) = inputs {
        ensure!(
            json_file(&evidence, "link-map-inputs.json")? == *inputs
                && crate::evidence::link_map::archive_references(map)? == *inputs,
            "Link-map archive inventory mismatch"
        );
    }
    let license = &source_members
        .get(&format!("{source_prefix}/LICENSE"))
        .context("Missing original project license")?
        .0;
    ensure!(
        &files.get("LICENSE").context("Missing package license")?.0 == license
            && &evidence.get("LICENSE").context("Missing build license")?.0 == license,
        "Project license bytes changed"
    );
    ensure!(
        files[EXECUTABLE].1 == 0o755
            && files
                .iter()
                .all(|(name, (_, mode))| name == EXECUTABLE || *mode == 0o644),
        "Invalid binary package permissions"
    );
    Ok(())
}

pub fn create(arguments: &Arguments) -> Result<Value> {
    let source = util::read(&arguments.source_archive, MAX_BYTES)?;
    let verified = super::verify(&arguments.source_archive, None)?;
    let evidence = evidence_files(&arguments.evidence)?;
    let build = json_file(&evidence, "build.json")?;
    let compiler = build["compiler"]
        .as_str()
        .context("Missing compiler identity")?;
    let target = compiler
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .context("Missing compiler host")?
        .to_owned();
    let revision = verified["git"]["revision"]
        .as_str()
        .context("Binary candidates require clean source provenance")?
        .to_owned();
    let binary = util::read(&arguments.executable, MAX_BYTES)?;
    let license = evidence
        .get("LICENSE")
        .context("Missing project license")?
        .clone();
    let mut files = evidence
        .into_iter()
        .map(|(name, value)| (format!("build-evidence/{name}"), value))
        .collect::<Files>();
    files.insert(EXECUTABLE.into(), (binary, 0o755));
    files.insert(SOURCE.into(), (source, 0o644));
    files.insert("LICENSE".into(), license);
    files.insert("README.txt".into(),(format!("lazypencil binary candidate\nTarget: {target}\nSource: {revision}\n\nExtract this directory and run bin/lazypencil --help. Place bin/lazypencil on PATH to install. Chromium is obtained separately with lazypencil runtime setup chromium.\n\nThis is a verification candidate. Review status is retained in build-evidence/build.json. No signing, minimum OS acceptance or final release approval is implied. The matching project source archive and all original collected third-party notices are included.\n").into_bytes(),0o644));
    ensure!(
        files.len() < MAX_FILES
            && files
                .values()
                .map(|(bytes, _)| bytes.len() as u64)
                .sum::<u64>()
                <= MAX_BYTES,
        "Binary candidate exceeds its budget"
    );
    let package = Package {
        format_version: 1,
        kind: "release-candidate".into(),
        release_ready: false,
        package: verified["package"].clone(),
        target,
        source_revision: revision,
        files: files
            .iter()
            .map(|(path, (bytes, mode))| Entry {
                path: path.clone(),
                sha256: util::digest(bytes),
                bytes: bytes.len() as u64,
                mode: *mode,
            })
            .collect(),
    };
    validate(&files, &package)?;
    let encoded = util::encode(&package)?;
    ensure!(
        encoded.len() as u64 <= MAX_MANIFEST,
        "Binary manifest exceeds its budget"
    );
    let prefix = prefix(&package)?;
    let parent = arguments
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    write_archive(
        temporary.as_file(),
        &prefix,
        0,
        std::iter::once((MANIFEST, encoded.as_slice(), 0o644)).chain(
            files
                .iter()
                .map(|(name, (bytes, mode))| (name.as_str(), bytes.as_slice(), *mode)),
        ),
    )?;
    let result = verify(temporary.path(), None)?;
    temporary
        .persist_noclobber(&arguments.output)
        .map_err(|e| e.error)?;
    Ok(json!({"archive":arguments.output.canonicalize()?,"verification":result}))
}

fn prefix(package: &Package) -> Result<String> {
    let name = format!(
        "{}-{}-{}",
        package.package["name"]
            .as_str()
            .context("Missing package name")?,
        package.package["version"]
            .as_str()
            .context("Missing package version")?,
        package.target
    );
    util::safe_path(&name)?;
    Ok(name)
}

pub fn verify(archive: &Path, extract: Option<&Path>) -> Result<Value> {
    let members = read_archive(archive)?;
    let manifests = members
        .iter()
        .filter(|(name, _)| {
            name.matches('/').count() == 1 && name.ends_with(&format!("/{MANIFEST}"))
        })
        .collect::<Vec<_>>();
    ensure!(
        manifests.len() == 1,
        "Expected exactly one binary package manifest"
    );
    let (name, (bytes, _)) = manifests[0];
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST,
        "Oversized binary manifest"
    );
    let package: Package = serde_json::from_slice(bytes)?;
    let prefix = prefix(&package)?;
    ensure!(
        *name == format!("{prefix}/{MANIFEST}")
            && members.len() == package.files.len() + 1
            && package.files.len() < MAX_FILES,
        "Binary package membership mismatch"
    );
    let mut files = Files::new();
    for entry in &package.files {
        util::safe_path(&entry.path)?;
        ensure!(
            entry.path != MANIFEST && !files.contains_key(&entry.path),
            "Duplicate binary package entry"
        );
        let (bytes, mode) = members
            .get(&format!("{prefix}/{}", entry.path))
            .context("Missing binary package input")?;
        ensure!(
            bytes.len() as u64 == entry.bytes
                && util::digest(bytes) == entry.sha256
                && *mode == entry.mode,
            "Binary package input changed: {}",
            entry.path
        );
        files.insert(entry.path.clone(), (bytes.clone(), *mode));
    }
    validate(&files, &package)?;
    if let Some(extract) = extract {
        extract_files(&files, MANIFEST, bytes, extract)?;
    }
    Ok(
        json!({"release_ready":false,"kind":package.kind,"package":package.package,"target":package.target,"source_revision":package.source_revision,"binary_sha256":util::digest(&files[EXECUTABLE].0),"files":files.len(),"sha256":util::file_digest(archive)?}),
    )
}
