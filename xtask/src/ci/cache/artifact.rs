//! Checkpoint transport; compiler/source verification stays in cache::restore.
use super::*;
use flate2::read::MultiGzDecoder;
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Use one Actions cache path for partial and full payloads, without copying
/// either payload. Only sibling directories owned by this job may be exchanged.
pub fn exchange(first: &Path, second: &Path) -> Result<()> {
    let parent = first
        .parent()
        .context("Checkpoint parent required")?
        .canonicalize()?;
    ensure!(
        second
            .parent()
            .context("Checkpoint parent required")?
            .canonicalize()?
            == parent,
        "Checkpoint directories must be siblings"
    );
    let resolve = |path: &Path| -> Result<PathBuf> {
        let name = path.file_name().context("Checkpoint name required")?;
        ensure!(
            path.components().all(|part| !matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )),
            "Ambiguous checkpoint directory"
        );
        Ok(parent.join(name))
    };
    let first = resolve(first)?;
    let second = resolve(second)?;
    ensure!(first != second, "Checkpoint directories must be distinct");
    let exists = |path: &Path| -> Result<bool> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir(),
                    "Checkpoint must be an unlinked directory"
                );
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    };
    match (exists(&first)?, exists(&second)?) {
        (false, false) => anyhow::bail!("No checkpoint to exchange"),
        (true, false) => fs::rename(&first, &second)?,
        (false, true) => fs::rename(&second, &first)?,
        (true, true) => {
            // Keep recovery data on a failed rollback, rather than letting a
            // temporary-directory destructor delete the only remaining copy.
            let temporary = tempfile::Builder::new()
                .prefix(".checkpoint-exchange-")
                .tempdir_in(&parent)?
                .keep();
            let held = temporary.join("held");
            let result = (|| -> Result<()> {
                fs::rename(&first, &held)?;
                if let Err(error) = fs::rename(&second, &first) {
                    fs::rename(&held, &first).context("Checkpoint exchange rollback failed")?;
                    return Err(error.into());
                }
                if let Err(error) = fs::rename(&held, &second) {
                    fs::rename(&first, &second).context("Checkpoint exchange rollback failed")?;
                    fs::rename(&held, &first).context("Checkpoint exchange rollback failed")?;
                    return Err(error.into());
                }
                Ok(())
            })();
            // remove_dir cannot remove a held payload after failed rollback.
            let cleanup = fs::remove_dir(&temporary);
            result.with_context(|| {
                format!(
                    "Checkpoint exchange failed; recovery directory: {}",
                    temporary.display()
                )
            })?;
            cleanup?;
        }
    }
    Ok(())
}

fn members(report: &ApplicationReport) -> Result<BTreeSet<String>> {
    ensure!(
        !report.files.is_empty() && report.files.len() < dist::MAX_FILES,
        "Invalid checkpoint file count"
    );
    let mut names = BTreeSet::from(["manifest.json".into()]);
    let mut folded = BTreeSet::from([dist::path_key("manifest.json")]);
    for file in &report.files {
        util::safe_path(&file.path)?;
        let name = format!("files/{}", file.path);
        ensure!(
            names.insert(name.clone()) && folded.insert(dist::path_key(&name)),
            "Duplicate checkpoint member"
        );
    }
    Ok(names)
}

fn tar(command: &mut Process) -> Result<()> {
    let output = process::output_limited(command, Duration::from_secs(600), 1024 * 1024)?;
    ensure!(
        !output.timed_out && output.status.success(),
        "Checkpoint tar failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// Preflight every path/type before handing a private temporary directory to
/// system tar. PAX preserves executable modes and nanosecond Cargo timestamps.
fn inspect(path: &Path) -> Result<usize> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BYTES,
        "Invalid checkpoint archive"
    );
    let mut tar = tar::Archive::new(MultiGzDecoder::new(fs::File::open(path)?));
    let mut seen = BTreeSet::new();
    let mut regular = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut expected = None;
    let mut total = 0u64;
    for item in tar.entries()? {
        let mut item = item?;
        let name = String::from_utf8(item.path_bytes().into_owned())?;
        util::safe_path(&name)?;
        let kind = item.header().entry_type();
        let hard_link = kind.is_hard_link();
        if hard_link {
            let target = item.link_name()?.context("Missing checkpoint hard link")?;
            let target = target.to_str().context("Invalid checkpoint hard link")?;
            util::safe_path(target)?;
            ensure!(
                name.starts_with("files/")
                    && target.starts_with("files/")
                    && regular.contains(target)
                    && item.size() == 0,
                "Checkpoint hard link must name an earlier regular payload"
            );
        } else {
            ensure!(
                kind.is_file() && item.link_name()?.is_none(),
                "Checkpoint symlinks/directories are forbidden"
            );
        }
        ensure!(
            item.header().mode()? & !0o777 == 0,
            "Privileged checkpoint permissions"
        );
        ensure!(
            seen.len() < dist::MAX_FILES
                && seen.insert(name.clone())
                && folded.insert(dist::path_key(&name)),
            "Duplicate checkpoint path"
        );
        total = total
            .checked_add(item.size())
            .context("Checkpoint byte overflow")?;
        ensure!(total <= MAX_BYTES, "Checkpoint exceeds its byte budget");
        if name == "manifest.json" {
            ensure!(
                item.size() <= dist::MAX_MANIFEST,
                "Oversized checkpoint manifest"
            );
            let mut bytes = Vec::new();
            item.read_to_end(&mut bytes)?;
            expected = Some(members(&serde_json::from_slice(&bytes)?)?);
        } else if !hard_link {
            ensure!(name.starts_with("files/"), "Unexpected checkpoint member");
            std::io::copy(&mut item, &mut std::io::sink())?;
            regular.insert(name);
        }
    }
    // Consume the gzip trailers as well; only bounded tar padding may remain.
    let mut tail = Vec::new();
    tar.into_inner().take(10241).read_to_end(&mut tail)?;
    ensure!(
        tail.len() <= 10240 && tail.iter().all(|byte| *byte == 0),
        "Trailing checkpoint data"
    );
    ensure!(
        Some(&seen) == expected.as_ref(),
        "Incomplete checkpoint members"
    );
    Ok(seen.len())
}

pub fn pack(directory: &Path, output: &Path) -> Result<Value> {
    let started = Instant::now();
    ensure!(
        fs::symlink_metadata(directory)?.is_dir(),
        "Linked checkpoint directory"
    );
    let directory = directory.canonicalize()?;
    let report: ApplicationReport = util::json(&directory.join("manifest.json"))?;
    let names = members(&report)?;
    let mut list = tempfile::NamedTempFile::new()?;
    let mut total = 0u64;
    for name in &names {
        let mut path = directory.clone();
        for component in Path::new(name).components() {
            path.push(component);
            ensure!(!path.is_symlink(), "Linked checkpoint input");
        }
        ensure!(
            fs::symlink_metadata(directory.join(name))?.is_file(),
            "Linked checkpoint input"
        );
        total = total
            .checked_add(fs::metadata(directory.join(name))?.len())
            .context("Checkpoint byte overflow")?;
        ensure!(total <= MAX_BYTES, "Checkpoint exceeds its byte budget");
        list.write_all(name.as_bytes())?;
        list.write_all(&[0])?;
    }
    list.flush()?;
    fs::create_dir_all(output.parent().context("Archive parent required")?)?;
    let temporary = tempfile::NamedTempFile::new_in(output.parent().unwrap())?;
    let mut command = Process::new("tar");
    command.env("COPYFILE_DISABLE", "1");
    if cfg!(target_os = "linux") {
        // GNU tar accepts a fixed compressor and its arguments. No input path
        // is interpolated into the compressor command.
        command.args(["--use-compress-program", "gzip --fast"]);
    } else {
        command.arg("-z");
        if cfg!(target_os = "macos") {
            command.arg("--options=gzip:compression-level=1");
        }
    }
    tar(command
        .args(["--format=pax", "--no-recursion", "-cf"])
        .arg(temporary.path().canonicalize()?)
        .arg("-C")
        .arg(directory)
        .args(["--null", "-T"])
        .arg(list.path()))?;
    let files = inspect(temporary.path())?;
    temporary
        .persist_noclobber(output)
        .map_err(|error| error.error)?;
    Ok(
        json!({"files":files,"source_commit":report.source_commit,"uncompressed_bytes":total,"archive_bytes":fs::metadata(output)?.len(),"archive_sha256":util::file_digest(output)?,"gzip_level":if cfg!(any(target_os="linux",target_os="macos")){1}else{6},"elapsed_seconds":started.elapsed().as_secs_f64()}),
    )
}

pub fn unpack(archive: &Path, directory: &Path) -> Result<Value> {
    let started = Instant::now();
    ensure!(
        !directory.exists() && !directory.is_symlink(),
        "Checkpoint destination already exists"
    );
    ensure!(
        fs::symlink_metadata(archive)?.is_file(),
        "Linked checkpoint archive"
    );
    let archive = archive.canonicalize()?;
    let files = inspect(&archive)?;
    let parent = directory.parent().context("Checkpoint parent required")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir_in(parent)?;
    tar(Process::new("tar")
        .args(["-xzf"])
        .arg(archive)
        .arg("-C")
        .arg(temporary.path())
        .args(["--no-same-owner", "--same-permissions"]))?;
    fs::rename(temporary.path(), directory)?;
    Ok(
        json!({"files":files,"transport_only":true,"compiler_verification":"required by subsequent restore-compatible","elapsed_seconds":started.elapsed().as_secs_f64()}),
    )
}
