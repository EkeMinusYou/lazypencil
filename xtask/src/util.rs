use anyhow::{Context, Result, bail, ensure};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_FILE: u64 = 4 * 1024 * 1024;

pub fn root() -> PathBuf {
    if let Ok(current) = std::env::current_dir() {
        for candidate in current.ancestors() {
            let manifest = candidate.join("Cargo.toml");
            if let Ok(body) = fs::read_to_string(&manifest) {
                if let Ok(value) = toml::from_str::<toml::Value>(&body) {
                    if value
                        .get("package")
                        .and_then(|p| p.get("name"))
                        .and_then(toml::Value::as_str)
                        == Some("lazypencil")
                    {
                        return candidate.to_owned();
                    }
                }
            }
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

pub fn digest(data: impl AsRef<[u8]>) -> String {
    hex(Sha256::digest(data.as_ref()))
}
pub fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn file_digest(path: &Path) -> Result<String> {
    let (file, before) = open_regular(path, u64::MAX)?;
    let mut file = file.take(before.len().saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    let mut total = 0;
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        total += size as u64;
        hasher.update(&buffer[..size]);
    }
    unchanged(path, &before, total)?;
    Ok(hex(hasher.finalize()))
}

fn open_regular(path: &Path, limit: u64) -> Result<(fs::File, fs::Metadata)> {
    let before = fs::symlink_metadata(path)?;
    ensure!(
        before.is_file() && !before.is_symlink() && before.len() <= limit,
        "Expected a bounded regular file: {}",
        path.display()
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .with_context(|| format!("Reading {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "Input is no longer a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata()?;
        ensure!(
            before.dev() == opened.dev() && before.ino() == opened.ino(),
            "Input changed before opening"
        );
    }
    Ok((file, before))
}

fn unchanged(path: &Path, before: &fs::Metadata, size: u64) -> Result<()> {
    let after = fs::symlink_metadata(path)?;
    ensure!(
        size == before.len()
            && before.len() == after.len()
            && before.modified()? == after.modified()?
            && after.is_file()
            && !after.is_symlink(),
        "Input changed or exceeded its limit: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            before.dev() == after.dev() && before.ino() == after.ino(),
            "Input identity changed while reading"
        );
    }
    Ok(())
}

pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let (file, before) = open_regular(path, limit)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    unchanged(path, &before, bytes.len() as u64)?;
    Ok(bytes)
}

pub fn json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read(path, MAX_FILE)?)?)
}

pub fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn publish(path: &Path, data: &[u8], replace: bool) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(data)?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(path).map_err(|e| e.error)?;
    } else {
        temporary.persist_noclobber(path).map_err(|e| e.error)?;
    }
    Ok(())
}

pub fn print(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

pub fn github_output(path: Option<&Path>, values: &BTreeMap<String, String>) -> Result<()> {
    if let Some(path) = path {
        let mut stream = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for (key, value) in values {
            ensure!(
                !key.contains(['\n', '\r', '=']) && !value.contains(['\n', '\r']),
                "Invalid Actions output"
            );
            writeln!(stream, "{key}={value}")?;
        }
    }
    Ok(())
}

pub fn safe_path(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty() && value.len() <= 4096 && !value.contains(['\\', '\0']),
        "Invalid archive path"
    );
    for part in value.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 255
                && !part.contains(':')
                && !part.ends_with(['.', ' '])
                && !part.chars().any(char::is_control),
            "Unsafe archive path: {value}"
        );
        let stem = part.split('.').next().unwrap().to_ascii_lowercase();
        if ["con", "prn", "aux", "nul"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            bail!("Reserved archive path: {value}");
        }
    }
    Ok(value)
}

pub fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    walk_filtered(root, |_| false)
}

pub fn walk_filtered(root: &Path, skip: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    fn visit(
        base: &Path,
        root: &Path,
        out: &mut Vec<PathBuf>,
        skip: &impl Fn(&Path) -> bool,
        count: &mut usize,
    ) -> Result<()> {
        if skip(root.strip_prefix(base)?) {
            return Ok(());
        }
        ensure!(
            root.strip_prefix(base)?.components().count() <= 128,
            "File inventory exceeds its depth limit"
        );
        *count += 1;
        ensure!(*count <= 100_000, "File inventory exceeds its entry limit");
        let metadata = fs::symlink_metadata(root)?;
        ensure!(!metadata.is_symlink(), "Linked input: {}", root.display());
        if metadata.is_file() {
            out.push(root.to_owned());
        } else if metadata.is_dir() {
            let mut children = fs::read_dir(root)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            children.sort();
            for child in children {
                visit(base, &child, out, skip, count)?;
            }
        } else {
            bail!("Unsupported file: {}", root.display());
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(root, root, &mut out, &skip, &mut 0)?;
    Ok(out)
}

pub fn mode(path: &Path) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(if fs::metadata(path)?.permissions().mode() & 0o111 != 0 {
            0o755
        } else {
            0o644
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(0o644)
    }
}

pub fn raw_mode(path: &Path) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::metadata(path)?.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        mode(path)
    }
}

pub fn chmod(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}
