use super::*;

pub(super) fn test_source(name: &str) -> bool {
    name.ends_with(".rs")
        && !name.contains('\\')
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
        && (name
            .strip_prefix("tests/")
            .is_some_and(|name| !name.contains('/'))
            || name.starts_with("tests/support/")
            || name.starts_with("xtask/src/test_support/"))
}

/// Existing integration roots and shared Rust test helpers are candidates.
/// Every changed file must be covered by real compiler dep-info, and every
/// affected unit must be an integration, library-test or binary-test unit.
/// A helper used by a production library, example or executable rejects reuse,
/// regardless of its directory name.
/// Cargo still validates retained artifacts and runs the complete test catalog.
pub fn restore_compatible(
    source: &Path,
    target: &Path,
    directory: &Path,
    matched_key: &str,
    identity: &ApplicationIdentity,
) -> Result<Value> {
    let directory = directory.canonicalize()?;
    let (report, old, changed) = compatible_inputs(&directory, matched_key, identity)?;
    let names = report
        .files
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    // A shared input can occur in several units. Coverage by one unit must
    // never conceal a missing dep-info file for another retained unit.
    let fingerprint = Regex::new(
        r"^(debug|release)/\.fingerprint/lazypencil-([0-9a-f]{16})/(?:test-integration-test-|(?:test-)?(?:lib|bin|example)-).+\.json$",
    )?;
    for name in &names {
        if let Some(captures) = fingerprint.captures(name) {
            let profile = format!("{}/", &captures[1]);
            let suffix = format!("-{}.d", &captures[2]);
            ensure!(
                names
                    .iter()
                    .any(|path| path.starts_with(&profile) && path.ends_with(&suffix)),
                "Cached compiler unit lacks dep-info"
            );
        }
    }
    let (invalid, covered) = affected_outputs(source, &directory.join("files"), &names, &changed)?;
    ensure!(covered == changed, "Changed tests lack compiler dep-info");
    let mut result = restore_filtered(source, target, &directory, &old, &invalid)?;
    result["invalidated_files"] = json!(invalid.len());
    result["changed_tests"] = json!(changed);
    Ok(result)
}

fn compatible_inputs(
    directory: &Path,
    matched_key: &str,
    identity: &ApplicationIdentity,
) -> Result<(ApplicationReport, ApplicationIdentity, BTreeSet<String>)> {
    ensure!(
        !identity.release_only,
        "Test reuse requires all-output identity"
    );
    let report: ApplicationReport = util::json(&directory.join("manifest.json"))?;
    let inputs = report
        .inputs
        .as_ref()
        .context("Missing cached source inputs")?;
    let expected_key = format!(
        "{}{}",
        identity.compatible_prefix,
        util::digest(stable_json(&serde_json::to_value(inputs)?))
    );
    ensure!(matched_key == expected_key, "Compatible cache key changed");
    let cached = inputs
        .iter()
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        cached.values().cloned().collect::<Vec<_>>() == *inputs
            && cached.keys().eq(identity.tracked.keys()),
        "Cached source set changed"
    );
    let mut old = identity.clone();
    old.recipe["inputs"] = serde_json::to_value(inputs)?;
    old.fingerprint = util::digest(stable_json(&old.recipe));
    ensure!(
        old.fingerprint == report.fingerprint,
        "Compatible build recipe changed"
    );
    let mut changed = BTreeSet::new();
    for (name, entry) in &cached {
        util::safe_path(name)?;
        let current = &identity.tracked[name];
        if entry != current {
            ensure!(
                test_source(name) && entry.mode == current.mode,
                "Only existing integration-test or Rust helper source edits are compatible"
            );
            changed.insert(name.clone());
        }
    }
    Ok((report, old, changed))
}

fn affected_outputs(
    source: &Path,
    root: &Path,
    names: &BTreeSet<String>,
    changed: &BTreeSet<String>,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let unit = Regex::new(r"^(debug|release)/deps/([A-Za-z0-9_]+)-([0-9a-f]{16})\.d$")?;
    let mut invalid = BTreeSet::new();
    let mut covered = BTreeSet::new();
    for name in names {
        ensure!(allowed(name), "Unsafe cached output");
        if !name.ends_with(".d") {
            continue;
        }
        let path = root.join(name);
        ensure!(
            !path.is_symlink() && path.canonicalize()? == std::path::absolute(&path)?,
            "Linked cached dep-info"
        );
        let affected = compiler_inputs(&path, source)?
            .intersection(changed)
            .cloned()
            .collect::<BTreeSet<_>>();
        if affected.is_empty() {
            continue;
        }
        let captures = unit
            .captures(name)
            .context("Unsupported affected compiler unit")?;
        let (profile, name, suffix) = (&captures[1], &captures[2], &captures[3]);
        let prefix = format!("{profile}/.fingerprint/lazypencil-{suffix}/");
        ensure!(
            names.iter().any(|entry| {
                entry
                    .strip_prefix(&prefix)
                    .and_then(|name| {
                        ["test-integration-test-", "test-lib-", "test-bin-"]
                            .into_iter()
                            .find_map(|prefix| name.strip_prefix(prefix))
                    })
                    .and_then(|name| name.strip_suffix(".json"))
                    .is_some_and(|target| target.replace('-', "_") == name)
            }),
            "Changed test is compiled into a production or unsupported unit"
        );
        let artifact = format!("{profile}/deps/{name}-{suffix}");
        let library = format!("{profile}/deps/lib{name}-{suffix}");
        for entry in names {
            if entry.starts_with(&prefix)
                || [artifact.as_str(), library.as_str()].iter().any(|prefix| {
                    entry.as_str() == *prefix
                        || entry
                            .strip_prefix(*prefix)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                })
            {
                invalid.insert(entry.clone());
            }
        }
        covered.extend(affected);
    }
    Ok((invalid, covered))
}

/// Focus checkpoints contain complete changed units, never a second copy of
/// the unchanged full payload. Shared fingerprint metadata belongs to its unit
/// even when its bytes happen to match the previous build.
pub fn stage_focused(
    source: &Path,
    target: &Path,
    directory: &Path,
    compatible: &Path,
    matched_key: &str,
    identity: &ApplicationIdentity,
) -> Result<Value> {
    let (_, _, changed) = match compatible_inputs(compatible, matched_key, identity) {
        Ok(inputs) => inputs,
        Err(_) => return stage(source, target, directory, identity),
    };
    let names = outputs(target)?
        .into_iter()
        .map(|path| {
            Ok(path
                .strip_prefix(target)?
                .to_str()
                .context("Invalid output filename")?
                .to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let selected = match affected_outputs(source, &target.canonicalize()?, &names, &changed) {
        Ok((selected, _)) => selected,
        // A nominal test helper can also be a production compiler input.
        // Keep the verified same-source build without offering cross-source reuse.
        Err(_) => return stage(source, target, directory, identity),
    };
    stage_filtered(source, target, directory, identity, Some(&selected))
}

/// A valid same-source focused build overlays a compatible full build last.
/// Any partial failed overlay is removed before accepting another payload.
pub fn restore_combined(
    source: &Path,
    target: &Path,
    exact: &Path,
    compatible: &Path,
    matched_key: &str,
    identity: &ApplicationIdentity,
) -> Result<Value> {
    let retained = match restore_compatible(source, target, compatible, matched_key, identity) {
        Ok(report) => Some(report),
        Err(error) => {
            eprintln!("Compatible test cache unavailable: {error:#}");
            prune(source, target)?;
            None
        }
    };
    if !exact.join("manifest.json").exists()
        && let Some(report) = &retained
    {
        return Ok(report.clone());
    }
    match restore(source, target, exact, identity) {
        Ok(mut report) => {
            report["compatible"] = json!(retained);
            Ok(report)
        }
        Err(error) => {
            eprintln!("Exact application cache unavailable: {error:#}");
            // Validation occurs before overlay. If an I/O error interrupted an
            // exact overlay, discard it and restore the validated full payload.
            prune(source, target)?;
            if retained.is_some() {
                restore_compatible(source, target, compatible, matched_key, identity)
            } else {
                Ok(json!({"reused":false}))
            }
        }
    }
}
