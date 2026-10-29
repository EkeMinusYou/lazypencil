//! Share one compiled catalog without carrying duplicate Cargo build trees.
use super::*;
use std::io::Read;

const LIMIT: u64 = 256 * 1024 * 1024;
const ARCHIVE: &str = "catalog.tar.zst";
const ARCHIVE_LIMIT: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Receipt {
    identity: Value,
    catalog: Catalog,
    application: Binary,
    cargo_seconds: f64,
}

pub(super) enum Subset {
    All,
    Runtime,
    Terminal,
    Material,
}

impl Subset {
    fn targets(&self) -> Option<std::collections::BTreeSet<&'static str>> {
        match self {
            Self::All => None,
            Self::Runtime => Some(runtime_targets()),
            Self::Terminal => Some(normal::QUIET.into_iter().collect()),
            Self::Material => Some(["mcp", "optional_icons"].into_iter().collect()),
        }
    }
}

fn identity(s: &Session) -> Result<Value> {
    Ok(json!({
        "schema":1, "commit":std::env::var("GITHUB_SHA")?,
        "run":std::env::var("GITHUB_RUN_ID")?,
        "os":std::env::consts::OS, "arch":std::env::consts::ARCH,
        "image":std::env::var("LAZYPENCIL_RUNNER_IMAGE")?,
        "compiler":process::text(Command::new("rustc").arg("-vV"))?,
        "flags":super::super::cache::compile_flags(&std::env::vars().collect()),
        "test_profile":commands::TEST_PROFILE_CONFIG,
        "optimized_tests":commands::optimized_tests()?,
        "source_root":s.root, "target_root":s.target
    }))
}

fn directory(s: &Session) -> PathBuf {
    s.temporary.join("native-binaries")
}

fn runtime_targets() -> std::collections::BTreeSet<&'static str> {
    browser::TEST_LANES
        .into_iter()
        .flatten()
        .copied()
        .chain(["mcp", "agent", "tui", "lib"])
        .collect()
}

fn copy(binary: &Binary, source: &Path, target: &Path) -> Result<()> {
    let bytes = util::read(source, LIMIT)?;
    ensure!(
        util::digest(&bytes) == binary.sha256,
        "Compiled binary changed"
    );
    util::publish(target, &bytes, false)?;
    util::chmod(target, 0o700)
}

fn relative<'a>(s: &Session, binary: &'a Binary) -> Result<&'a Path> {
    let relative = binary.path.strip_prefix(&s.target)?;
    util::safe_path(
        &relative
            .to_str()
            .context("Non-UTF8 binary path")?
            .replace('\\', "/"),
    )?;
    Ok(relative)
}

fn pack(
    s: &Session,
    receipt: &Receipt,
    destination: &Path,
    targets: Option<&std::collections::BTreeSet<&str>>,
) -> Result<()> {
    ensure!(
        receipt.identity["commit"] == receipt.catalog.source_commit,
        "Catalog source differs from the receipt"
    );
    ensure!(
        receipt.application.path == s.executable("debug"),
        "Unexpected application path"
    );
    fs::create_dir_all(destination)?;
    let encoded = util::encode(receipt)?;
    util::publish(&destination.join("receipt.json"), &encoded, false)?;
    let work = tempfile::tempdir_in(&s.temporary)?;
    let path = work.path().join("catalog.tar");
    let mut tar = tar::Builder::new(fs::File::create_new(&path)?);
    append(&mut tar, Path::new("receipt.json"), &encoded, 0o600)?;
    let mut total = encoded.len() as u64;
    // Read and verify each immutable compiler output directly into the tar.
    // Avoid publishing and syncing another complete executable tree first.
    for binary in receipt
        .catalog
        .binaries
        .iter()
        .filter(|(name, _)| targets.is_none_or(|targets| targets.contains(name.as_str())))
        .map(|(_, binary)| binary)
        .chain([&receipt.application])
    {
        let bytes = util::read(&binary.path, LIMIT)?;
        ensure!(
            util::digest(&bytes) == binary.sha256,
            "Compiled binary changed"
        );
        total = total
            .checked_add(bytes.len() as u64)
            .context("Executable size overflow")?;
        ensure!(total <= ARCHIVE_LIMIT, "Executable archive too large");
        append(&mut tar, relative(s, binary)?, &bytes, 0o700)?;
    }
    tar.finish()?;
    drop(tar);
    s.execute_command(
        Command::new("zstd")
            .args(["-3", "--long=27", "--single-thread", "--no-progress", "-o"])
            .arg(destination.join(ARCHIVE))
            .arg(&path),
        300,
        &format!(
            "{}-transport.log",
            destination
                .file_name()
                .context("Archive directory missing")?
                .to_string_lossy()
        ),
        true,
    )?;
    println!(
        "Executable transport: {total} bytes -> {} bytes",
        fs::metadata(destination.join(ARCHIVE))?.len()
    );
    Ok(())
}

fn append(tar: &mut tar::Builder<fs::File>, name: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_cksum();
    tar.append_data(&mut header, name, bytes)?;
    Ok(())
}

pub(super) fn stage(s: &Session, elapsed: Duration) -> Result<()> {
    let catalog = s.verified_catalog(&[])?;
    let binaries = catalog
        .binaries
        .iter()
        .map(|(name, binary)| (name.clone(), binary.path.clone()))
        .collect();
    let _ = commands::default_tests(&s.root, &binaries)?;
    let path = s.executable("debug");
    let receipt = Receipt {
        identity: identity(s)?,
        catalog,
        application: Binary {
            sha256: util::file_digest(&path)?,
            path,
        },
        cargo_seconds: elapsed.as_secs_f64(),
    };
    parallel::run(vec![
        (
            "complete catalog",
            Box::new(|| pack(s, &receipt, &directory(s), None)),
        ),
        (
            "consumer subsets",
            Box::new(|| {
                for (name, subset) in [
                    ("runtime", Subset::Runtime),
                    ("terminal", Subset::Terminal),
                    ("material", Subset::Material),
                ] {
                    pack(
                        s,
                        &receipt,
                        &s.temporary.join(format!("native-{name}-binaries")),
                        subset.targets().as_ref(),
                    )?;
                }
                Ok(())
            }),
        ),
    ])
}

fn unpack(s: &Session, targets: Option<&std::collections::BTreeSet<&str>>) -> Result<()> {
    let work = tempfile::tempdir_in(&s.temporary)?;
    let path = work.path().join("catalog.tar");
    s.execute_command(
        Command::new("zstd")
            .args(["-d", "-M256MB", "--no-progress", "-o"])
            .arg(&path)
            .arg(directory(s).join(ARCHIVE)),
        300,
        "executable-transport.log",
        true,
    )?;
    ensure!(
        fs::metadata(&path)?.len() <= ARCHIVE_LIMIT,
        "Executable archive too large"
    );
    unpack_tar(s, &path, targets)
}

fn unpack_tar(
    s: &Session,
    path: &Path,
    targets: Option<&std::collections::BTreeSet<&str>>,
) -> Result<()> {
    let mut tar = tar::Archive::new(fs::File::open(path)?);
    let mut entries = tar.entries()?;
    let mut first = entries.next().context("Executable receipt missing")??;
    ensure!(
        first.path_bytes().as_ref() == b"receipt.json"
            && first.header().entry_type().is_file()
            && first.size() <= util::MAX_FILE,
        "Invalid executable receipt"
    );
    let mut bytes = Vec::new();
    first.read_to_end(&mut bytes)?;
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    util::publish(&directory(s).join("receipt.json"), &bytes, false)?;
    drop(first);
    let mut expected = std::collections::BTreeSet::new();
    for binary in receipt
        .catalog
        .binaries
        .iter()
        .filter(|(name, _)| targets.is_none_or(|targets| targets.contains(name.as_str())))
        .map(|(_, binary)| binary)
        .chain([&receipt.application])
    {
        let name_in_tar = relative(s, binary)?.to_string_lossy().replace('\\', "/");
        ensure!(expected.insert(name_in_tar), "Duplicate executable member");
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let mut entry = entry?;
        let name = String::from_utf8(entry.path_bytes().into_owned())?;
        util::safe_path(&name)?;
        ensure!(
            entry.header().entry_type().is_file()
                && entry.size() <= LIMIT
                && seen.insert(name.clone()),
            "Invalid executable member"
        );
        ensure!(expected.contains(&name), "Unexpected executable member");
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        util::publish(&directory(s).join(&name), &bytes, false)?;
    }
    ensure!(
        seen.len() == expected.len(),
        "Incomplete executable archive"
    );
    let mut tail = Vec::new();
    tar.into_inner().take(10241).read_to_end(&mut tail)?;
    ensure!(
        tail.len() <= 10240 && tail.iter().all(|byte| *byte == 0),
        "Trailing executable data"
    );
    Ok(())
}

fn restore_record(
    s: &Session,
    expected: &Value,
    targets: Option<&std::collections::BTreeSet<&str>>,
) -> Result<Duration> {
    let mut receipt: Receipt = util::json(&directory(s).join("receipt.json"))?;
    ensure!(
        &receipt.identity == expected,
        "Compiled catalog source or compiler conditions differ"
    );
    ensure!(
        receipt.cargo_seconds.is_finite() && (0.0..900.0).contains(&receipt.cargo_seconds),
        "Invalid completed test budget"
    );
    ensure!(
        receipt.application.path == s.executable("debug"),
        "Unexpected application path"
    );
    if let Some(targets) = targets {
        ensure!(
            targets
                .iter()
                .all(|name| receipt.catalog.binaries.contains_key(*name)),
            "Required runtime executable missing from the shared catalog"
        );
        receipt
            .catalog
            .binaries
            .retain(|name, _| targets.contains(name.as_str()));
    }
    for binary in receipt
        .catalog
        .binaries
        .values()
        .chain([&receipt.application])
    {
        copy(
            binary,
            &directory(s).join(relative(s, binary)?),
            &binary.path,
        )?;
    }
    receipt.catalog.validate(
        &s.target,
        expected["commit"].as_str().context("Commit missing")?,
        &[],
    )?;
    util::publish(
        &s.temporary.join("test-executables.json"),
        &util::encode(&receipt.catalog)?,
        false,
    )?;
    util::publish(
        &s.temporary.join("binary-receipt.json"),
        &util::read(&directory(s).join("receipt.json"), LIMIT)?,
        false,
    )?;
    util::publish(
        &s.temporary.join("lazypencil-tests.log"),
        receipt.identity["compiler"]
            .as_str()
            .unwrap_or_default()
            .as_bytes(),
        false,
    )?;
    Ok(Duration::from_secs_f64(receipt.cargo_seconds))
}

pub(super) fn restore(s: &Session, subset: Subset) -> Result<Duration> {
    let targets = subset.targets();
    unpack(s, targets.as_ref())?;
    restore_record(s, &identity(s)?, targets.as_ref())
}

pub(super) fn completed(s: &Session, mode: &str) -> Result<()> {
    util::publish(
        &s.temporary.join("consumer-result.json"),
        &util::encode(&json!({
            "commit":std::env::var("GITHUB_SHA")?, "run":std::env::var("GITHUB_RUN_ID")?,
            "mode":mode, "binaries":util::file_digest(&directory(s).join("receipt.json"))?
        }))?,
        false,
    )
}

pub(super) fn gate(s: &Session) -> Result<()> {
    let commit = std::env::var("GITHUB_SHA")?;
    let run = std::env::var("GITHUB_RUN_ID")?;
    let output = s.temporary.join("codec-reference");
    fs::create_dir(&output)?;
    let mut binaries: Option<String> = None;
    for mode in ["normal", "pixels", "runtime", "terminal", "material"] {
        let root = s
            .temporary
            .join("consumer-evidence")
            .join(format!("native-test-{mode}-{commit}"));
        let result: Value = util::json(&root.join("consumer-result.json"))?;
        ensure!(
            result["commit"] == commit && result["run"] == run && result["mode"] == mode,
            "Wrong consumer evidence"
        );
        let digest = result["binaries"]
            .as_str()
            .context("Binary receipt missing")?;
        let receipt_path = root.join("binary-receipt.json");
        ensure!(
            util::file_digest(&receipt_path)? == digest,
            "Consumer receipt changed"
        );
        let receipt: Receipt = util::json(&receipt_path)?;
        ensure!(
            receipt.identity["commit"] == commit
                && receipt.identity["run"] == run
                && receipt.identity["os"] == "macos"
                && receipt.identity["arch"] == "aarch64",
            "Expected evidence from the initial Mac Apple Silicon release target"
        );
        ensure!(
            digest.len() == 64
                && binaries
                    .as_deref()
                    .is_none_or(|previous| previous == digest),
            "Consumers used different compiled catalogs"
        );
        binaries = Some(digest.to_owned());
        // The pixel lane owns native render diagnostics; codec-script images
        // remain mandatory outputs of the normal and runtime producers.
        let codec = root.join("codec-reference");
        if !["normal", "runtime"].contains(&mode) && !codec.exists() {
            continue;
        }
        for entry in fs::read_dir(codec)? {
            let entry = entry?;
            let name = entry.file_name();
            ensure!(
                name.to_str().is_some_and(|n| n.starts_with("codec-"))
                    && entry.file_type()?.is_file(),
                "Unexpected shared evidence entry"
            );
            // No overwrite: independent producers must retain separate names.
            util::publish(
                &output.join(name),
                &util::read(&entry.path(), LIMIT)?,
                false,
            )?;
        }
    }
    // Validate the producer's pinned browser, independent of this light host.
    browser::required_images_for(&output, &commit, "mac-arm64")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_rejects_changed_bytes_and_compiler_conditions_before_execution() {
        let temp = tempfile::tempdir().unwrap();
        let s = Session {
            root: temp.path().join("source"),
            target: temp.path().join("target"),
            temporary: temp.path().to_owned(),
            timing_lock: Mutex::new(()),
        };
        let path = s.target.join("debug/deps/probe");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"test executable").unwrap();
        let unused = s.target.join("debug/deps/unused");
        fs::write(&unused, b"another test executable").unwrap();
        let main = s.executable("debug");
        fs::write(&main, b"application executable").unwrap();
        let identity = json!({"commit":"source", "compiler":"original compiler"});
        let receipt = Receipt {
            identity: identity.clone(),
            catalog: Catalog {
                source_commit: "source".into(),
                binaries: BTreeMap::from([
                    (
                        "probe".into(),
                        Binary {
                            sha256: util::file_digest(&path).unwrap(),
                            path: path.clone(),
                        },
                    ),
                    (
                        "unused".into(),
                        Binary {
                            sha256: util::file_digest(&unused).unwrap(),
                            path: unused.clone(),
                        },
                    ),
                ]),
            },
            application: Binary {
                sha256: util::file_digest(&main).unwrap(),
                path: main.clone(),
            },
            cargo_seconds: 2.5,
        };
        pack(&s, &receipt, &directory(&s), None).unwrap();
        assert!(!directory(&s).join("debug").exists());
        fs::write(&path, b"changed compiler output").unwrap();
        assert!(pack(&s, &receipt, &s.temporary.join("changed"), None).is_err());
        fs::write(&path, b"test executable").unwrap();
        let packed = s.temporary.join("packed");
        fs::rename(directory(&s), &packed).unwrap();
        fs::create_dir(directory(&s)).unwrap();
        fs::copy(packed.join(ARCHIVE), directory(&s).join(ARCHIVE)).unwrap();
        unpack(&s, None).unwrap();
        fs::remove_dir_all(&s.target).unwrap();
        assert!(
            restore_record(
                &s,
                &json!({"commit":"source", "compiler":"different compiler"}),
                None
            )
            .is_err()
        );
        assert!(!s.target.exists());
        let staged = directory(&s).join("debug/deps/probe");
        fs::write(&staged, b"tampered executable").unwrap();
        assert!(restore_record(&s, &identity, None).is_err());
        assert!(!path.exists());
        fs::write(&staged, b"test executable").unwrap();
        assert_eq!(
            restore_record(&s, &identity, None).unwrap(),
            Duration::from_millis(2500)
        );
        receipt
            .catalog
            .validate(&s.target, "source", &["probe"])
            .unwrap();
        assert_eq!(fs::read(main).unwrap(), b"application executable");
        assert!(
            s.capture(
                Command::new("cargo").arg("--version"),
                1,
                "forbidden-cargo.log"
            )
            .unwrap_err()
            .to_string()
            .contains("cannot invoke Cargo")
        );
        let partial = s.temporary.join("partial");
        let targets = std::collections::BTreeSet::from(["probe"]);
        pack(&s, &receipt, &partial, Some(&targets)).unwrap();
        assert_eq!(
            util::file_digest(&partial.join("receipt.json")).unwrap(),
            util::file_digest(&directory(&s).join("receipt.json")).unwrap()
        );
        fs::remove_dir_all(&s.target).unwrap();
        fs::remove_dir_all(directory(&s)).unwrap();
        fs::create_dir(directory(&s)).unwrap();
        fs::copy(partial.join(ARCHIVE), directory(&s).join(ARCHIVE)).unwrap();
        unpack(&s, Some(&targets)).unwrap();
        for name in [
            "test-executables.json",
            "binary-receipt.json",
            "lazypencil-tests.log",
        ] {
            fs::remove_file(s.temporary.join(name)).unwrap();
        }
        restore_record(&s, &identity, Some(&targets)).unwrap();
        assert!(path.exists());
        assert!(!unused.exists());
        let partial_catalog: Catalog =
            util::json(&s.temporary.join("test-executables.json")).unwrap();
        assert_eq!(partial_catalog.binaries.len(), 1);
        partial_catalog
            .validate(&s.target, "source", &["probe"])
            .unwrap();

        let bad = s.temporary.join("unexpected.tar");
        let mut tar = tar::Builder::new(fs::File::create_new(&bad).unwrap());
        tar.append_path_with_name(partial.join("receipt.json"), "receipt.json")
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, "unexpected", std::io::Cursor::new(b"x"))
            .unwrap();
        tar.finish().unwrap();
        drop(tar);
        let invalid = Session {
            temporary: s.temporary.join("invalid"),
            ..s
        };
        fs::create_dir_all(directory(&invalid)).unwrap();
        assert!(
            unpack_tar(&invalid, &bad, Some(&targets))
                .unwrap_err()
                .to_string()
                .contains("Unexpected executable member")
        );
    }
}
