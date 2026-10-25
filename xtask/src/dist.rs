use crate::{process, util};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use flate2::{Compression, GzBuilder, read::MultiGzDecoder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command as Process,
};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

pub const MAX_FILES: usize = 10_000;
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_MANIFEST: u64 = 4 * 1024 * 1024;
pub const MANIFEST: &str = "SOURCE-MANIFEST.json";

#[derive(Subcommand)]
pub enum Command {
    Source {
        #[arg(long, default_value_os_t = util::root())]
        root: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, env = "SOURCE_DATE_EPOCH", default_value_t = 0)]
        epoch: u32,
        #[arg(long)]
        allow_dirty: bool,
    },
    VerifySource {
        archive: PathBuf,
        #[arg(long)]
        extract: Option<PathBuf>,
    },
}

impl Command {
    pub fn run(self) -> Result<()> {
        match self {
            Self::Source {
                root,
                output,
                epoch,
                allow_dirty,
            } => util::print(&source(&root, &output, epoch, allow_dirty)?),
            Self::VerifySource { archive, extract } => {
                util::print(&verify(&archive, extract.as_deref())?)
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub mode: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub format_version: u32,
    pub package: Value,
    pub source_date_epoch: u32,
    pub git: Value,
    pub files: Vec<Entry>,
}

pub type Files = BTreeMap<String, (Vec<u8>, u32)>;

fn path_key(name: &str) -> String {
    name.nfc()
        .flat_map(char::to_lowercase)
        .case_fold()
        .collect()
}

pub fn verify_tree(root: &Path, revision: Option<&str>) -> Result<Manifest> {
    ensure!(
        !root.is_symlink() && root.is_dir() && !root.join(".git").exists(),
        "Expected an extracted source distribution"
    );
    let manifest: Manifest = util::json(&root.join(MANIFEST))?;
    ensure!(
        manifest.format_version == 1
            && !manifest.files.is_empty()
            && manifest.files.len() <= MAX_FILES,
        "Invalid source manifest"
    );
    if let Some(revision) = revision {
        ensure!(
            revision.len() == 40
                && revision
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                && manifest.git == json!({"revision":revision,"dirty":false}),
            "Expected clean source revision"
        );
    }
    let mut names = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut total = 0u64;
    for entry in &manifest.files {
        util::safe_path(&entry.path)?;
        ensure!(
            entry.path != MANIFEST
                && names.insert(entry.path.clone())
                && folded.insert(path_key(&entry.path)),
            "Duplicate/reserved source input"
        );
        total = total
            .checked_add(entry.bytes)
            .context("Source size overflow")?;
        ensure!(
            total <= MAX_BYTES && [0o644, 0o755].contains(&entry.mode),
            "Invalid source byte/mode metadata"
        );
        let path = root.join(&entry.path);
        ensure!(
            path.canonicalize()? == root.canonicalize()?.join(&entry.path),
            "Linked source input"
        );
        ensure!(
            fs::metadata(&path)?.len() == entry.bytes
                && util::file_digest(&path)? == entry.sha256
                && util::mode(&path)? == entry.mode,
            "Source input changed: {}",
            entry.path
        );
    }
    let actual = util::walk_filtered(root, |p| {
        p.to_str()
            .is_some_and(|p| excluded(&p.replace('\\', "/")) || p == MANIFEST)
    })?
    .into_iter()
    .map(|p| {
        p.strip_prefix(root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/")
    })
    .collect::<BTreeSet<_>>();
    ensure!(
        actual == names,
        "Missing or unlisted distributed source inputs"
    );
    ensure!(
        ["Cargo.toml", "Cargo.lock", "LICENSE", "src/main.rs"]
            .iter()
            .all(|s| names.contains(*s)),
        "Missing required distributed source bindings"
    );
    Ok(manifest)
}

pub fn excluded(name: &str) -> bool {
    name == "vendor/skia-bindings/skia"
        || name.starts_with("vendor/skia-bindings/skia/")
        || name
            .split('/')
            .any(|p| [".git", "target", "__pycache__", ".DS_Store"].contains(&p))
        || [".pyc", ".pyo", ".tmp"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

pub fn source_files(root: &Path, selected: Option<&BTreeSet<String>>) -> Result<Files> {
    fn visit(
        root: &Path,
        path: &Path,
        selected: Option<&BTreeSet<String>>,
        files: &mut Files,
        folded: &mut BTreeSet<String>,
        total: &mut u64,
    ) -> Result<()> {
        let name = path
            .strip_prefix(root)?
            .to_str()
            .context("Source filenames must be UTF-8")?
            .replace('\\', "/");
        ensure!(
            name.split('/').count() <= 128,
            "Source directory exceeds its depth limit"
        );
        if excluded(&name)
            || selected.is_some_and(|s| {
                !s.contains(&name) && !s.iter().any(|p| p.starts_with(&(name.clone() + "/")))
            })
        {
            return Ok(());
        }
        let info = fs::symlink_metadata(path)?;
        ensure!(
            !info.is_symlink(),
            "Source distributions do not follow symlinks: {name}"
        );
        if info.is_dir() {
            let mut paths = fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            paths.sort();
            for path in paths {
                visit(root, &path, selected, files, folded, total)?;
            }
        } else {
            util::safe_path(&name)?;
            ensure!(
                info.is_file() && files.len() < MAX_FILES && info.len() <= MAX_BYTES - *total,
                "Source exceeds its file/byte budget"
            );
            ensure!(
                folded.insert(path_key(&name)),
                "Source paths collide: {name}"
            );
            let bytes = util::read(path, MAX_BYTES - *total)?;
            *total += bytes.len() as u64;
            files.insert(name, (bytes, util::mode(path)?));
        }
        Ok(())
    }
    let mut files = Files::new();
    let mut folded = BTreeSet::new();
    let mut total = 0;
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "LICENSE",
        "README.md",
        ".gitignore",
        ".gitattributes",
        "deny.toml",
        ".cargo",
        "xtask",
        "src",
        "resources",
        "examples",
        "tests",
        "scripts",
        "docs",
        ".github",
        "vendor/skia-bindings",
    ] {
        let path = root.join(name);
        if path.exists() || path.is_symlink() {
            visit(root, &path, selected, &mut files, &mut folded, &mut total)?;
        } else {
            ensure!(
                !["Cargo.toml", "Cargo.lock", "LICENSE", "README.md"].contains(&name),
                "Required source file is missing: {name}"
            );
        }
    }
    Ok(files)
}

pub fn package_identity(files: &Files) -> Result<Value> {
    let manifest: toml::Value =
        std::str::from_utf8(&files.get("Cargo.toml").context("Missing Cargo.toml")?.0)?.parse()?;
    let package = &manifest["package"];
    let name = package["name"].as_str().context("Invalid package name")?;
    let version = package["version"]
        .as_str()
        .context("Invalid package version")?;
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            && !version.is_empty()
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".+-".contains(&c)),
        "Unsafe package identity"
    );
    if let Some(patch) = manifest.get("patch").and_then(toml::Value::as_table) {
        for table in patch.values().filter_map(toml::Value::as_table) {
            for dependency in table.values() {
                if let Some(path) = dependency.get("path").and_then(toml::Value::as_str) {
                    util::safe_path(path)?;
                    ensure!(
                        files.contains_key(&format!("{path}/Cargo.toml")),
                        "Source omits patched dependency: {path}"
                    );
                }
            }
        }
    }
    for required in ["Cargo.lock", "LICENSE", "README.md", "src/main.rs"] {
        ensure!(
            files.contains_key(required),
            "Missing required source input: {required}"
        );
    }
    // A source distribution must carry the complete, independently buildable tools.
    if files.contains_key(".cargo/config.toml") {
        ensure!(
            files.contains_key("xtask/Cargo.toml") && files.contains_key("xtask/Cargo.lock"),
            "Source omits xtask or its lockfile"
        );
    }
    Ok(
        json!({"name": name, "version": version, "rust_version": package.get("rust-version").and_then(toml::Value::as_str)}),
    )
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    process::text(Process::new("git").arg("-C").arg(root).args(args))
}

fn vcs(root: &Path, allow_dirty: bool) -> Result<Value> {
    let revision = git(root, &["rev-parse", "HEAD"]);
    if revision.is_err() && allow_dirty {
        return Ok(Value::Null);
    }
    let revision = revision.context("Source provenance requires Git")?;
    let dirty = !git(root, &["status", "--porcelain=v1", "--untracked-files=all"])?
        .trim()
        .is_empty();
    ensure!(
        !dirty || allow_dirty,
        "Refusing dirty release source; use --allow-dirty for development"
    );
    Ok(json!({"revision": revision.trim(), "dirty": dirty}))
}

pub fn source(root: &Path, output: &Path, epoch: u32, allow_dirty: bool) -> Result<Value> {
    let root = root.canonicalize()?;
    let provenance = vcs(&root, allow_dirty)?;
    let mut args = vec!["ls-files", "--cached", "-z"];
    if allow_dirty {
        args.extend(["--others", "--exclude-standard"]);
    }
    args.extend(["--", "."]);
    let selected = git(&root, &args).map(|s| {
        s.split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    });
    let selected = match selected {
        Ok(s) => Some(s),
        Err(e) if !provenance.is_null() => return Err(e),
        Err(_) => None,
    };
    let files = source_files(&root, selected.as_ref())?;
    let package = package_identity(&files)?;
    ensure!(
        vcs(&root, allow_dirty)? == provenance,
        "Source provenance changed while collecting files"
    );
    let manifest = Manifest {
        format_version: 1,
        package: package.clone(),
        source_date_epoch: epoch,
        git: provenance.clone(),
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
    let encoded = util::encode(&manifest)?;
    ensure!(
        encoded.len() as u64 <= MAX_MANIFEST,
        "Source manifest exceeds its byte budget"
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let compressed = GzBuilder::new()
        .mtime(epoch)
        .write(temporary.as_file(), Compression::default());
    let mut archive = tar::Builder::new(compressed);
    let prefix = format!(
        "{}-{}",
        package["name"].as_str().unwrap(),
        package["version"].as_str().unwrap()
    );
    for (name, bytes, mode) in std::iter::once((MANIFEST, encoded.as_slice(), 0o644)).chain(
        files
            .iter()
            .map(|(name, (bytes, mode))| (name.as_str(), bytes.as_slice(), *mode)),
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(mode);
        header.set_mtime(epoch as u64);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        archive.append_data(&mut header, format!("{prefix}/{name}"), Cursor::new(bytes))?;
    }
    archive.into_inner()?.finish()?;
    verify(temporary.path(), None)?;
    temporary.persist_noclobber(output).map_err(|e| e.error)?;
    Ok(
        json!({"archive": output.canonicalize()?, "sha256": util::file_digest(output)?, "files": files.len(), "package": package, "git": provenance}),
    )
}

pub fn read_archive(path: &Path) -> Result<Files> {
    let compressed = util::read(path, MAX_BYTES)?;
    let limit = MAX_BYTES + MAX_MANIFEST + MAX_FILES as u64 * 4096;
    let mut bytes = Vec::new();
    MultiGzDecoder::new(compressed.as_slice())
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Expanded distribution exceeds its byte budget"
    );
    let mut archive = tar::Archive::new(bytes.as_slice());
    let mut files = Files::new();
    let mut folded = BTreeSet::new();
    let mut total = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = String::from_utf8(entry.path_bytes().into_owned())?;
        util::safe_path(&name)?;
        ensure!(
            entry.header().entry_type().is_file() && entry.link_name()?.is_none(),
            "Source archives contain only regular files"
        );
        ensure!(
            folded.insert(path_key(&name)),
            "Duplicate or colliding archive path: {name}"
        );
        ensure!(
            files.len() <= MAX_FILES && entry.size() <= MAX_BYTES + MAX_MANIFEST - total,
            "Archive exceeds its file/byte budget"
        );
        let mode = entry.header().mode()?;
        total += entry.size();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        files.insert(name, (data, mode));
    }
    Ok(files)
}

pub fn verify(path: &Path, extract: Option<&Path>) -> Result<Value> {
    let members = read_archive(path)?;
    let manifests = members
        .keys()
        .filter(|s| s.matches('/').count() == 1 && s.ends_with(&format!("/{MANIFEST}")))
        .collect::<Vec<_>>();
    ensure!(
        manifests.len() == 1,
        "Expected exactly one root source manifest"
    );
    let manifest_name = manifests[0];
    let prefix = manifest_name.split('/').next().unwrap();
    ensure!(
        members[manifest_name].0.len() as u64 <= MAX_MANIFEST,
        "Oversized source manifest"
    );
    let manifest: Manifest = serde_json::from_slice(&members[manifest_name].0)?;
    ensure!(
        manifest.format_version == 1 && manifest.files.len() <= MAX_FILES,
        "Unsupported source manifest"
    );
    let mut files = Files::new();
    for entry in &manifest.files {
        util::safe_path(&entry.path)?;
        ensure!(
            entry.path != MANIFEST && !files.contains_key(&entry.path),
            "Duplicate or reserved manifest path"
        );
        let (data, mode) = members
            .get(&format!("{prefix}/{}", entry.path))
            .context("Missing archive member")?;
        ensure!(
            data.len() as u64 == entry.bytes
                && util::digest(data) == entry.sha256
                && *mode == entry.mode
                && [0o644, 0o755].contains(mode),
            "Distribution integrity mismatch: {}",
            entry.path
        );
        files.insert(entry.path.clone(), (data.clone(), *mode));
    }
    ensure!(
        members.len() == files.len() + 1,
        "Distribution contains unlisted files"
    );
    let package = package_identity(&files)?;
    ensure!(
        manifest.package == package
            && prefix
                == format!(
                    "{}-{}",
                    package["name"].as_str().unwrap(),
                    package["version"].as_str().unwrap()
                ),
        "Distribution package identity mismatch"
    );
    if let Some(extract) = extract {
        let parent = extract
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = tempfile::tempdir_in(parent)?;
        for (name, (data, mode)) in &files {
            let destination = temporary.path().join(name);
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::write(&destination, data)?;
            util::chmod(&destination, *mode)?;
        }
        fs::write(temporary.path().join(MANIFEST), &members[manifest_name].0)?;
        fs::create_dir(extract).context("Extraction destination must not exist")?;
        for entry in fs::read_dir(temporary.path())? {
            let entry = entry?;
            fs::rename(entry.path(), extract.join(entry.file_name()))?;
        }
    }
    Ok(
        json!({"package":package,"files":files.len(),"sha256":util::file_digest(path)?,"git":manifest.git}),
    )
}
