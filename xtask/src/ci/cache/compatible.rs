use super::*;

pub(super) fn test_source(name: &str) -> bool {
    name.strip_prefix("tests/")
        .is_some_and(|name| !name.contains('/') && name.ends_with(".rs"))
}

/// Only edits to existing integration-test roots may reuse a full older build.
/// Compiler dep-info identifies affected units; Cargo still validates every
/// retained artifact and runs the complete test catalog after restoration.
pub fn restore_compatible(
    source: &Path,
    target: &Path,
    directory: &Path,
    matched_key: &str,
    identity: &ApplicationIdentity,
) -> Result<Value> {
    ensure!(
        !identity.release_only,
        "Test reuse requires all-output identity"
    );
    let directory = directory.canonicalize()?;
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
                "Only existing integration-test source edits are compatible"
            );
            changed.insert(name.clone());
        }
    }
    let unit = Regex::new(r"^(debug|release)/deps/([A-Za-z0-9_]+)-([0-9a-f]{16})\.d$")?;
    let mut invalid = BTreeSet::new();
    let mut covered = BTreeSet::new();
    for entry in &report.files {
        ensure!(allowed(&entry.path), "Unsafe cached output");
        if !entry.path.ends_with(".d") {
            continue;
        }
        let path = directory.join("files").join(&entry.path);
        ensure!(
            !path.is_symlink() && path.canonicalize()? == std::path::absolute(&path)?,
            "Linked cached dep-info"
        );
        let affected = compiler_inputs(&path, source)?
            .intersection(&changed)
            .cloned()
            .collect::<BTreeSet<_>>();
        if affected.is_empty() {
            continue;
        }
        let captures = unit
            .captures(&entry.path)
            .context("Unsupported affected compiler unit")?;
        let (profile, name, suffix) = (&captures[1], &captures[2], &captures[3]);
        let prefix = format!("{profile}/.fingerprint/lazypencil-{suffix}/");
        ensure!(
            report.files.iter().any(|entry| {
                entry
                    .path
                    .strip_prefix(&prefix)
                    .and_then(|name| name.strip_prefix("test-integration-test-"))
                    .and_then(|name| name.strip_suffix(".json"))
                    .is_some_and(|target| target.replace('-', "_") == name)
            }),
            "Changed test is compiled into a non-integration unit"
        );
        let artifact = format!("{profile}/deps/{name}-{suffix}");
        let library = format!("{profile}/deps/lib{name}-{suffix}");
        for entry in &report.files {
            if entry.path.starts_with(&prefix)
                || [artifact.as_str(), library.as_str()].iter().any(|prefix| {
                    entry.path == *prefix
                        || entry
                            .path
                            .strip_prefix(*prefix)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                })
            {
                invalid.insert(entry.path.clone());
            }
        }
        covered.extend(affected);
    }
    ensure!(covered == changed, "Changed tests lack compiler dep-info");
    let mut result = restore_filtered(source, target, &directory, &old, &invalid)?;
    result["invalidated_files"] = json!(invalid.len());
    result["changed_tests"] = json!(changed);
    Ok(result)
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
