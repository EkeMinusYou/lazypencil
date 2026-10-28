//! Share one compiled catalog without carrying duplicate Cargo build trees.
use super::*;

const LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Receipt {
    identity: Value,
    catalog: Catalog,
    application: Binary,
    cargo_seconds: f64,
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

fn stage_record(s: &Session, receipt: &Receipt) -> Result<()> {
    receipt.catalog.validate(
        &s.target,
        receipt.identity["commit"]
            .as_str()
            .context("Commit missing")?,
        &[],
    )?;
    ensure!(
        receipt.application.path == s.executable("debug"),
        "Unexpected application path"
    );
    for binary in receipt
        .catalog
        .binaries
        .values()
        .chain([&receipt.application])
    {
        copy(
            binary,
            &binary.path,
            &directory(s).join(relative(s, binary)?),
        )?;
    }
    util::publish(
        &directory(s).join("receipt.json"),
        &util::encode(receipt)?,
        false,
    )
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
    stage_record(
        s,
        &Receipt {
            identity: identity(s)?,
            catalog,
            application: Binary {
                sha256: util::file_digest(&path)?,
                path,
            },
            cargo_seconds: elapsed.as_secs_f64(),
        },
    )
}

fn restore_record(s: &Session, expected: &Value) -> Result<Duration> {
    let receipt: Receipt = util::json(&directory(s).join("receipt.json"))?;
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

pub(super) fn restore(s: &Session) -> Result<Duration> {
    restore_record(s, &identity(s)?)
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
    for mode in ["normal", "runtime"] {
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
        ensure!(
            digest.len() == 64
                && binaries
                    .as_deref()
                    .is_none_or(|previous| previous == digest),
            "Consumers used different compiled catalogs"
        );
        binaries = Some(digest.to_owned());
        for entry in fs::read_dir(root.join("codec-reference"))? {
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
    browser::required_images(&output, &commit)
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
        let main = s.executable("debug");
        fs::write(&main, b"application executable").unwrap();
        let identity = json!({"commit":"source", "compiler":"original compiler"});
        let receipt = Receipt {
            identity: identity.clone(),
            catalog: Catalog {
                source_commit: "source".into(),
                binaries: BTreeMap::from([(
                    "probe".into(),
                    Binary {
                        sha256: util::file_digest(&path).unwrap(),
                        path: path.clone(),
                    },
                )]),
            },
            application: Binary {
                sha256: util::file_digest(&main).unwrap(),
                path: main.clone(),
            },
            cargo_seconds: 2.5,
        };
        stage_record(&s, &receipt).unwrap();
        fs::remove_dir_all(&s.target).unwrap();
        assert!(
            restore_record(
                &s,
                &json!({"commit":"source", "compiler":"different compiler"})
            )
            .is_err()
        );
        assert!(!s.target.exists());
        let staged = directory(&s).join("debug/deps/probe");
        fs::write(&staged, b"tampered executable").unwrap();
        assert!(restore_record(&s, &identity).is_err());
        assert!(!path.exists());
        fs::write(&staged, b"test executable").unwrap();
        assert_eq!(
            restore_record(&s, &identity).unwrap(),
            Duration::from_millis(2500)
        );
        receipt
            .catalog
            .validate(&s.target, "source", &["probe"])
            .unwrap();
        assert_eq!(fs::read(main).unwrap(), b"application executable");
    }
}
