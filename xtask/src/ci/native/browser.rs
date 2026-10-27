use super::*;
use crate::performance;

pub fn setup(s: &Session) -> Result<()> {
    let binary = s.executable("debug");
    let data = s.temporary.join("chromium-test-data");
    s.capture(
        Command::new(&binary)
            .arg("--json")
            .arg("--data-dir")
            .arg(&data)
            .args(["runtime", "setup", "chromium"]),
        600,
        "chromium-setup.json",
    )?;
    let raw = s.capture(
        Command::new(&binary)
            .arg("--json")
            .arg("--data-dir")
            .arg(&data)
            .args(["runtime", "status"]),
        300,
        "chromium-status.json",
    )?;
    let status: Value = serde_json::from_slice(&raw)?;
    ensure!(
        status["available"] == true
            && status["managed"] == true
            && status["integrity_verified"] == true,
        "Chromium runtime integrity failed"
    );
    let executable = PathBuf::from(
        status["executable"]
            .as_str()
            .context("Chromium executable missing")?,
    )
    .canonicalize()?;
    ensure!(
        executable.starts_with(data.join("runtimes/chromium").canonicalize()?),
        "Chromium executable outside verified managed runtime"
    );
    util::github_output(
        std::env::var_os("GITHUB_ENV").as_deref().map(Path::new),
        &BTreeMap::from([(
            "LAZYPENCIL_TEST_CHROMIUM".into(),
            executable.to_string_lossy().into_owned(),
        )]),
    )?;
    if cfg!(target_os = "linux") {
        sandbox(s, &executable)?;
        let profile = tempfile::tempdir_in(&s.temporary)?;
        let mut environment = String::new();
        for (program, args) in [
            ("id", vec![]),
            (
                "ldd",
                vec![executable.to_str().context("Non-UTF8 executable")?],
            ),
        ] {
            let out = process::output(Command::new(program).args(args), Duration::from_secs(30))?;
            environment.push_str(&String::from_utf8_lossy(&out.stdout));
            environment.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        for path in [
            "/proc/sys/kernel/unprivileged_userns_clone",
            "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
            "/proc/sys/user/max_user_namespaces",
        ] {
            if let Ok(value) = fs::read_to_string(path) {
                environment.push_str(&format!("{path}: {value}"));
            }
        }
        fs::write(
            s.temporary.join("chromium-linux-environment.log"),
            &environment,
        )?;
        println!("{environment}");
        let result = process::output(
            Command::new(executable)
                .args([
                    "--headless",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-background-networking",
                ])
                .arg(format!("--user-data-dir={}", profile.path().display()))
                .args(["--dump-dom", "about:blank"]),
            Duration::from_secs(15),
        )?;
        fs::write(s.temporary.join("chromium-startup-dom.log"), result.stdout)?;
        fs::write(
            s.temporary.join("chromium-startup-stderr.log"),
            &result.stderr,
        )?;
        println!("{}", String::from_utf8_lossy(&result.stderr));
        ensure!(
            !result.timed_out && result.status.success(),
            "Sandboxed browser startup failed"
        );
    }
    Ok(())
}
fn sandbox(s: &Session, executable: &Path) -> Result<()> {
    let restriction = Path::new("/proc/sys/kernel/apparmor_restrict_unprivileged_userns");
    if !restriction.exists() || fs::read_to_string(restriction)?.trim() != "1" {
        return Ok(());
    }
    let text = executable.to_str().context("Non-UTF8 browser path")?;
    ensure!(
        regex::Regex::new(r"^/[A-Za-z0-9_./-]+$")?.is_match(text),
        "Unsafe AppArmor executable path"
    );
    let policy = format!(
        "abi <abi/4.0>,\ninclude <tunables/global>\nprofile lazypencil_ci_chromium \"{text}\" flags=(unconfined) {{\n  userns,\n}}\n"
    );
    let path = s.temporary.join("chromium-userns.apparmor");
    fs::write(&path, &policy)?;
    util::chmod(&path, 0o600)?;
    process::checked(
        Command::new("sudo")
            .args(["-n", "/usr/sbin/apparmor_parser", "-r"])
            .arg(path),
        Duration::from_secs(300),
    )?;
    ensure!(
        fs::read_to_string(restriction)?.trim() == "1",
        "Global AppArmor restriction changed"
    );
    util::publish(
        &s.temporary.join("chromium-userns-policy.json"),
        &util::encode(
            &json!({"executable":text,"profile_sha256":util::digest(policy),"kernel_restriction_before":1,"kernel_restriction_after":1,"loaded":true}),
        )?,
        false,
    )
}
// Each target owns its temporary documents, browser profiles and process groups.
// Their retained filenames occupy separate namespaces. Keep tests within each
// target serial, and keep the ownership/destruction test after both lanes join.
pub const TEST_LANES: [&[&str]; 2] = [
    &["browser_readiness"],
    &[
        "dynamic_render",
        "shader_uniforms",
        "sampler_sources",
        "imports",
        "windows_agent",
        "script_scope",
    ],
];

pub fn tests(s: &Session) -> Result<()> {
    let deadline = playback(s)?;
    lanes(s, deadline)?;
    ownership(s)
}

/// The full job already ran these normal tests. A focused runtime-only catalog
/// must generate their required comparison inputs before the shared PNG gate.
pub fn focus_inputs(s: &Session) -> Result<()> {
    let catalog = s.verified_catalog(&["native_layout", "script_scope"])?;
    let commit = std::env::var("GITHUB_SHA")?;
    let mut failures = Vec::new();
    for (target, filter, reports) in [
        (
            "native_layout",
            "native_cancellation_static_runtime_computation_stops_and_recovers_without_a_worker",
            &[
                "codec-native-layout-cancellation-2.19.json",
                "codec-native-layout-cancellation-2.21.json",
            ][..],
        ),
        (
            "script_scope",
            "script_bindings::overrides::inherited_overrides_preserve_live_namespaces_metadata_and_pixels_after_two_independent_copies",
            &["codec-script-overrides.json"][..],
        ),
    ] {
        // Earlier focused checks may have already produced these inputs in the
        // same job. The final shared gate still checks every PNG and its hash.
        if reports.iter().all(|name| {
            util::json::<Value>(&s.temporary.join("codec-reference").join(name))
                .is_ok_and(|report| report["source_commit"] == commit)
        }) {
            continue;
        }
        let args = vec![
            catalog.binaries[target].path.to_string_lossy().into_owned(),
            filter.into(),
            "--exact".into(),
            "--test-threads=2".into(),
        ];
        let mut command = s.command(&args);
        command
            .env("CARGO_MANIFEST_DIR", &s.root)
            .env("CARGO_MANIFEST_PATH", s.root.join("Cargo.toml"));
        if let Err(error) =
            s.execute_command(&args, &mut command, 300, "focus-runtime-inputs.log", true)
        {
            failures.push(format!("{target}: {error:#}"));
        }
    }
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// The PTY playback assertion has a real interactive deadline. Keep it clear
/// of concurrent Chromium/MCP/lifecycle load, with its original 15-second waits.
pub fn playback(s: &Session) -> Result<Instant> {
    let deadline = Instant::now() + Duration::from_secs(1200);
    let catalog = s.verified_catalog(&["dynamic_render"])?;
    let args = playback_args(&catalog.binaries["dynamic_render"].path);
    s.execute(&args, 1200, "chromium-page-playback.log", false)?;
    Ok(deadline)
}

pub fn playback_args(executable: &Path) -> commands::Invocation {
    vec![
        executable.to_string_lossy().into_owned(),
        focus::RETAINED_PAGE.into(),
        "--exact".into(),
        "--ignored".into(),
        "--test-threads=1".into(),
    ]
}

pub fn lane_args(target: &str, executable: &Path) -> commands::Invocation {
    let mut args = vec![
        executable.to_string_lossy().into_owned(),
        "--ignored".into(),
        "--test-threads=1".into(),
    ];
    if target == "dynamic_render" {
        args.extend(["--skip".into(), focus::RETAINED_PAGE.into()]);
    }
    args
}

pub fn lanes(s: &Session, deadline: Instant) -> Result<()> {
    let mut required: Vec<_> = TEST_LANES.into_iter().flatten().copied().collect();
    required.push("agent");
    let catalog = s.verified_catalog(&required)?;
    let result = parallel::run(
        TEST_LANES
            .into_iter()
            .enumerate()
            .map(|(index, targets)| -> parallel::Task<'_> {
                let catalog = &catalog;
                (
                    ["browser readiness", "dynamic rendering and shaders"][index],
                    Box::new(move || {
                        let mut failures = Vec::new();
                        for target in targets {
                            let remaining = deadline.saturating_duration_since(Instant::now());
                            let outcome = if remaining.is_zero() {
                                Err(anyhow::anyhow!("Browser suite deadline exceeded"))
                            } else {
                                let args = lane_args(target, &catalog.binaries[*target].path);
                                s.execute(
                                    &args,
                                    remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0),
                                    &format!("chromium-suite-{target}.log"),
                                    false,
                                )
                            };
                            if let Err(error) = outcome {
                                failures.push(format!("{target}: {error:#}"));
                            }
                        }
                        ensure!(failures.is_empty(), "{}", failures.join("\n"));
                        Ok(())
                    }),
                )
            })
            .collect(),
    );
    // Preserve the existing combined evidence even when a lane fails. Timing
    // records and individual logs retain each target's independent duration.
    let mut combined = fs::File::create(s.temporary.join("chromium-render.log"))?;
    for target in required {
        let path = s.temporary.join(format!("chromium-suite-{target}.log"));
        if path.exists() {
            writeln!(combined, "\nBrowser test target: {target}")?;
            combined.write_all(&fs::read(path)?)?;
        }
    }
    result
}

pub fn ownership(s: &Session) -> Result<()> {
    let catalog = s.verified_catalog(&["agent"])?;
    s.execute(
        &[
            catalog.binaries["agent"].path.to_string_lossy().into_owned(),
            "browser_ownership::real_browser_drop_and_owner_death_reap_the_tree_and_preserve_other_owners".into(),
            "--exact".into(),
            "--ignored".into(),
            "--test-threads=1".into(),
        ],
        300,
        "chromium-render.log",
        true,
    )?;
    required_images(
        &s.temporary.join("codec-reference"),
        &std::env::var("GITHUB_SHA")?,
    )
}
pub fn required_images(root: &Path, commit: &str) -> Result<()> {
    let report = |name: &str| -> Result<Value> {
        let v: Value = util::json(&root.join(name))
            .with_context(|| format!("Browser evidence report: {name}"))?;
        ensure!(v["source_commit"] == commit, "Wrong source: {name}");
        Ok(v)
    };
    let png = |name: &str, digest: &Value, w: u32, h: u32| -> Result<Vec<u8>> {
        ensure!(
            Path::new(name).file_name().and_then(|s| s.to_str()) == Some(name)
                && name.starts_with("codec-"),
            "Invalid PNG path"
        );
        let bytes = performance::png(&root.join(name), &json!({"png_hash":digest}), w, h)?;
        ensure!(
            bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82"),
            "Truncated PNG: {name}"
        );
        Ok(bytes)
    };
    let dimensions = |row: &Value| -> Result<(u32, u32)> {
        let width = u32::try_from(row["width"].as_u64().context("Missing image width")?)?;
        let height = u32::try_from(row["height"].as_u64().context("Missing image height")?)?;
        ensure!(
            width > 0 && height > 0 && width <= 16384 && height <= 16384,
            "Invalid image dimensions"
        );
        Ok((width, height))
    };
    let structures = report("codec-shader-structures.json")?;
    let structures = structures["cases"]
        .as_array()
        .context("Missing shader structures")?;
    ensure!(structures.len() == 16, "Expected 16 shader structures");
    let expanded = report("codec-shader-preprocessed.json")?;
    let expanded = expanded["cases"]
        .as_array()
        .context("Missing preprocessed shaders")?;
    ensure!(expanded.len() == 64, "Expected 64 preprocessed shaders");
    for version in ["2.19", "2.21"] {
        for case in 0..8 {
            let rows: Vec<_> = structures
                .iter()
                .filter(|r| r["document_version"] == version && r["case"] == case)
                .collect();
            ensure!(rows.len() == 1, "Missing or duplicate shader structure");
            png(
                &format!("codec-shader-structure-{version}-{case}.png"),
                &rows[0]["png_hash"],
                32,
                16,
            )?;
            for effect in [false, true] {
                for scale in [1, 2] {
                    let rows: Vec<_> = expanded
                        .iter()
                        .filter(|r| {
                            r["document_version"] == version
                                && r["case"] == case
                                && r["effect"] == effect
                                && r["scale"] == scale
                        })
                        .collect();
                    ensure!(rows.len() == 1, "Missing or duplicate preprocessed shader");
                    let name =
                        format!("codec-shader-preprocessed-{version}-{case}-{effect}-{scale}.png");
                    ensure!(rows[0]["png"] == name, "Wrong preprocessed PNG name");
                    png(&name, &rows[0]["png_hash"], 32 * scale, 16 * scale)?;
                }
            }
        }
        let prefix = format!("codec-native-layout-cancellation-{version}");
        let r = report(&format!("{prefix}.json"))?;
        ensure!(
            r["document_version"] == version
                && r["worker_process_id"].is_null()
                && r["before_snapshot_id"] == r["recovered_snapshot_id"],
            "Native cancellation did not recover"
        );
        ensure!(
            png(
                &format!("{prefix}-before.png"),
                &r["before_png_hash"],
                320,
                80
            )? == png(
                &format!("{prefix}-recovered.png"),
                &r["recovered_png_hash"],
                320,
                80
            )?,
            "Recovered cancellation pixels differ"
        );
    }
    let sampler = report("codec-sampler-copy.json")?;
    let cases = sampler["cases"]
        .as_array()
        .context("Missing sampler copy images")?;
    ensure!(cases.len() == 4, "Expected four sampler copy conditions");
    for absolute in [false, true] {
        for theme in ["light", "dark"] {
            let rows: Vec<_> = cases
                .iter()
                .filter(|row| row["absolute"] == absolute && row["theme"] == theme)
                .collect();
            ensure!(
                rows.len() == 1,
                "Missing or duplicate sampler copy condition"
            );
            let row = rows[0];
            let (width, height) = dimensions(row)?;
            let before = format!("codec-sampler-copy-{absolute}-{theme}-before.png");
            let after = format!("codec-sampler-copy-{absolute}-{theme}-after.png");
            ensure!(
                row["before"] == before && row["after"] == after,
                "Wrong sampler image names"
            );
            ensure!(
                png(&before, &row["png_hash"], width, height)?
                    == png(&after, &row["png_hash"], width, height)?,
                "Copied sampler pixels differ"
            );
        }
    }
    let projection = report("codec-sampler-projection.json")?;
    let cases = projection["cases"]
        .as_array()
        .context("Missing sampler projection images")?;
    ensure!(
        cases.len() == 12,
        "Expected twelve sampler projection conditions"
    );
    for version in ["2.19", "2.21"] {
        for case in ["aliases", "file_directory", "case"] {
            for theme in ["light", "dark"] {
                let rows: Vec<_> = cases
                    .iter()
                    .filter(|row| {
                        row["document_version"] == version
                            && row["case"] == case
                            && row["theme"] == theme
                    })
                    .collect();
                ensure!(
                    rows.len() == 1,
                    "Missing or duplicate sampler projection condition"
                );
                let row = rows[0];
                let (width, height) = dimensions(row)?;
                let prefix = format!("codec-sampler-projection-{version}-{case}-{theme}");
                let before = format!("{prefix}-before.png");
                ensure!(
                    row["before"] == before,
                    "Wrong sampler projection image name"
                );
                let original = png(&before, &row["png_hash"], width, height)?;
                for stage in ["copied", "recopied"] {
                    let name = format!("{prefix}-{stage}.png");
                    ensure!(row[stage] == name, "Wrong sampler projection image name");
                    ensure!(
                        original == png(&name, &row["png_hash"], width, height)?,
                        "Sampler projection pixels differ after {stage}"
                    );
                }
            }
        }
    }
    let overrides = report("codec-script-overrides.json")?;
    let cases = overrides["cases"]
        .as_array()
        .context("Missing script override images")?;
    ensure!(
        cases.len() == 12,
        "Expected twelve script override conditions"
    );
    for version in ["2.19", "2.21"] {
        for mode in [
            "inputs", "code", "metadata", "combined", "literal", "nullable",
        ] {
            let rows: Vec<_> = cases
                .iter()
                .filter(|row| row["document_version"] == version && row["mode"] == mode)
                .collect();
            ensure!(
                rows.len() == 1,
                "Missing or duplicate script override condition"
            );
            let row = rows[0];
            let (width, height) = dimensions(row)?;
            ensure!(
                (width, height) == (36, 12),
                "Wrong script override image geometry"
            );
            let prefix = format!("codec-script-overrides-{version}-{mode}");
            let before = format!("{prefix}-before.png");
            ensure!(row["before"] == before, "Wrong script override image name");
            let original = png(&before, &row["png_hash"], width, height)?;
            for stage in ["copied", "recopied"] {
                let name = format!("{prefix}-{stage}.png");
                ensure!(row[stage] == name, "Wrong script override image name");
                ensure!(
                    original == png(&name, &row["png_hash"], width, height)?,
                    "Script override pixels differ after {stage}"
                );
            }
            for stage in ["explicit", "reset", "inherited_input", "inherited_code"] {
                let name = format!("{prefix}-{stage}.png");
                ensure!(row[stage] == name, "Wrong script input image name");
                png(&name, &row[format!("{stage}_png_hash")], width, height)?;
            }
        }
    }
    let local = report("codec-browser-local-files.json")?;
    ensure!(
        local["source_removed"] == true
            && local["managed_bundle_reloaded_after_source_removal"] == true
            && local["project_resource_revision"]
                .as_str()
                .is_some_and(|revision| revision.len() == 64
                    && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
            && local["location_preserved"] == true
            && local["unsafe_host_file"] == false
            && local["sandbox"] == "enabled"
            && local["stats"]["fulfilled"] == 8
            && local["stats"]["denied"] == 1
            && local["stats"]["attached_targets"]
                .as_u64()
                .is_some_and(|count| count >= 2),
        "Local browser resource isolation proof missing"
    );
    png("codec-browser-local-files.png", &local["png_hash"], 32, 32)?;
    let extensionless = &local["extensionless"];
    ensure!(
        extensionless["content_type"] == "text/plain"
            && extensionless["executed"] == false
            && extensionless["equivalent"] == true,
        "Missing native extensionless-file comparison"
    );
    let native = png(
        "codec-browser-local-files-native-plain.png",
        &extensionless["png_hash"],
        64,
        64,
    )?;
    ensure!(
        native
            == png(
                "codec-browser-local-files-captured-plain.png",
                &extensionless["png_hash"],
                64,
                64
            )?,
        "Extensionless file pixels differ from native loading"
    );
    let copied = report("codec-browser-copy.json")?;
    ensure!(
        copied["sandbox"] == "enabled",
        "Browser copy sandbox proof missing"
    );
    let rows = copied["cases"]
        .as_array()
        .context("Missing browser copy cases")?;
    ensure!(
        rows.len() == 32,
        "Expected both schemas, eight browser contexts and both themes"
    );
    for version in ["2.19", "2.21"] {
        for mode in [
            "inherited",
            "literal",
            "query",
            "metadata",
            "combined",
            "descendant-url",
            "descendant-metadata",
            "replacement",
        ] {
            for theme in ["light", "dark"] {
                let matching = rows
                    .iter()
                    .filter(|row| {
                        row["document_version"] == version
                            && row["mode"] == mode
                            && row["theme"] == theme
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    matching.len() == 1,
                    "Missing or duplicate browser copy condition"
                );
                let row = matching[0];
                let width = match (
                    matches!(
                        mode,
                        "query" | "combined" | "descendant-url" | "replacement"
                    ),
                    theme == "dark",
                ) {
                    (false, false) => 32,
                    (false, true) => 40,
                    (true, false) => 48,
                    (true, true) => 56,
                };
                ensure!(
                    row["sources_removed"] == true && dimensions(row)? == (width, 16),
                    "Incomplete browser copy geometry/removal proof"
                );
                let stages = row["stages"]
                    .as_array()
                    .context("Missing browser copy stages")?;
                ensure!(
                    stages.len() == 4,
                    "Expected before, copied, recopied and detached browser images"
                );
                let mut original = None;
                let mut resource_revision = None;
                for stage in ["before", "copied", "recopied", "detached"] {
                    let matching = stages
                        .iter()
                        .filter(|row| row["stage"] == stage)
                        .collect::<Vec<_>>();
                    ensure!(
                        matching.len() == 1,
                        "Missing or duplicate browser copy stage"
                    );
                    let entry = matching[0];
                    let name = format!("codec-browser-copy-{version}-{mode}-{theme}-{stage}.png");
                    ensure!(entry["name"] == name, "Wrong browser copy image name");
                    let bytes = png(&name, &entry["png_hash"], width, 16)?;
                    let revision = entry["resource_revision"]
                        .as_str()
                        .context("Missing browser namespace revision")?;
                    ensure!(
                        revision.len() == 64
                            && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                        "Invalid browser namespace revision"
                    );
                    if let Some(original) = &original {
                        ensure!(*original == bytes, "Browser pixels changed after {stage}");
                    } else {
                        original = Some(bytes);
                    }
                    if let Some(original) = resource_revision {
                        ensure!(
                            original == revision,
                            "Browser namespace changed after {stage}"
                        );
                    } else {
                        resource_revision = Some(revision);
                    }
                }
            }
        }
    }
    let generated = report("codec-generated-browser.json")?;
    ensure!(
        generated["sandbox"] == "enabled",
        "Generated browser sandbox proof missing"
    );
    let rows = generated["cases"]
        .as_array()
        .context("Missing generated browser cases")?;
    ensure!(
        rows.len() == 14,
        "Expected both schemas and seven generated browser contexts"
    );
    for version in ["2.19", "2.21"] {
        for style in [
            "direct",
            "reference",
            "descendant",
            "replacement",
            "children",
            "inherited",
            "nested",
        ] {
            let matching = rows
                .iter()
                .filter(|row| row["document_version"] == version && row["style"] == style)
                .collect::<Vec<_>>();
            ensure!(
                matching.len() == 1,
                "Missing or duplicated generated browser case"
            );
            let row = matching[0];
            let expected = if style == "inherited" {
                json!([255, 0, 0, 255])
            } else {
                json!([0, 255, 0, 255])
            };
            ensure!(
                row["rgba"] == expected && row["sources_removed"] == true,
                "Generated browser source isolation proof missing"
            );
            let stages = row["stages"]
                .as_array()
                .context("Missing generated browser stages")?;
            ensure!(
                stages.len() == 3,
                "Expected original, copy and recopy generated pages"
            );
            let mut original = None;
            let mut revision = None;
            for stage in ["before", "copied", "recopied"] {
                let matching = stages
                    .iter()
                    .filter(|row| row["stage"] == stage)
                    .collect::<Vec<_>>();
                ensure!(
                    matching.len() == 1,
                    "Missing or duplicated generated browser stage"
                );
                let row = matching[0];
                let name = format!("codec-generated-browser-{version}-{style}-{stage}.png");
                ensure!(
                    row["name"] == name,
                    "Wrong generated browser evidence filename"
                );
                let bytes = png(&name, &row["png_hash"], 32, 16)?;
                let current = row["resource_revision"]
                    .as_str()
                    .context("Missing generated namespace revision")?;
                ensure!(current.len() == 64, "Invalid generated namespace revision");
                if let Some(original) = &original {
                    ensure!(
                        *original == bytes,
                        "Generated browser pixels changed after {stage}"
                    );
                } else {
                    original = Some(bytes);
                }
                if let Some(revision) = revision {
                    ensure!(
                        revision == current,
                        "Generated browser resource ownership changed after {stage}"
                    );
                } else {
                    revision = Some(current);
                }
            }
        }
    }
    let local = report("codec-local-pages.json")?;
    ensure!(
        local["sandbox"] == "enabled",
        "Local page sandbox proof missing"
    );
    let cases = local["cases"]
        .as_array()
        .context("Missing local page cases")?;
    let images = local["images"]
        .as_array()
        .context("Missing local page images")?;
    ensure!(
        cases.len() == 2 && images.len() == 14,
        "Expected both schemas and seven local page stages"
    );
    for version in ["2.19", "2.21"] {
        let cases = cases
            .iter()
            .filter(|row| row["document_version"] == version)
            .collect::<Vec<_>>();
        ensure!(cases.len() == 1, "Missing or duplicate local page schema");
        for proof in [
            "raster_cache_reused",
            "offscreen_resource_change_retired",
            "offscreen_denied_retired",
            "cancelled_process_reaped",
            "same_browser_after_offscreen_retirement",
        ] {
            ensure!(cases[0][proof] == true, "Missing local page proof: {proof}");
        }
        let mut states = std::collections::BTreeMap::new();
        let mut revisions = std::collections::BTreeMap::new();
        for (stage, width, rgba) in [
            ("first", 32, json!([0, 255, 0, 255])),
            ("changed", 32, json!([0, 0, 255, 255])),
            ("retained", 32, json!([0, 0, 255, 255])),
            ("resized", 40, json!([255, 255, 0, 255])),
            ("returned", 40, json!([255, 0, 0, 255])),
            ("restored", 40, json!([255, 0, 0, 255])),
            ("recovered", 40, json!([255, 0, 0, 255])),
        ] {
            let matching = images
                .iter()
                .filter(|row| row["document_version"] == version && row["stage"] == stage)
                .collect::<Vec<_>>();
            ensure!(matching.len() == 1, "Missing or duplicate local page stage");
            let row = matching[0];
            let name = format!("codec-local-page-{version}-{stage}.png");
            ensure!(
                row["name"] == name && dimensions(row)? == (width, 32) && row["rgba"] == rgba,
                "Wrong local page geometry/color"
            );
            png(&name, &row["png_hash"], width, 32)?;
            let acquisition = &row["acquisition"];
            let local_files = &acquisition["local_files"];
            ensure!(
                local_files["fulfilled"] == 5
                    && local_files["denied"] == 1
                    && local_files["attached_targets"]
                        .as_u64()
                        .is_some_and(|n| n >= 2),
                "Missing local page worker/denial proof"
            );
            let revision = acquisition["resources"]["revision"]
                .as_str()
                .context("Missing captured resource revision")?;
            ensure!(
                revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid captured revision"
            );
            revisions.insert(stage, revision);
            states.insert(stage, &acquisition["page_state"]);
        }
        ensure!(
            revisions["first"] != revisions["changed"]
                && revisions["changed"] == revisions["retained"]
                && revisions["retained"] == revisions["resized"]
                && revisions["returned"] != revisions["resized"]
                && revisions["returned"] == revisions["restored"]
                && revisions["restored"] == revisions["recovered"],
            "Stale local page resource revision"
        );
        for stage in ["retained", "resized", "returned", "restored", "recovered"] {
            ensure!(
                states[stage]["retained"] == true && states[stage]["id"].is_string(),
                "Missing retained page identity"
            );
        }
        ensure!(
            states["resized"]["reused"] == true
                && states["resized"]["id"] == states["retained"]["id"]
                && states["returned"]["id"] != states["retained"]["id"]
                && states["restored"]["id"] != states["returned"]["id"]
                && states["recovered"]["id"] != states["restored"]["id"],
            "Stale local page context"
        );
    }
    println!("Verified source identities, hashes and dimensions for 399 required PNGs");
    Ok(())
}

pub fn local_page_mcp_images(root: &Path, commit: &str) -> Result<()> {
    let report: Value = util::json(&root.join("codec-local-pages-mcp.json"))?;
    ensure!(
        report["source_commit"] == commit,
        "Wrong source for local page MCP images"
    );
    let images = report["images"]
        .as_array()
        .context("Missing local page MCP images")?;
    ensure!(
        images.len() == 8,
        "Expected both schemas and four MCP/CLI stages"
    );
    for version in ["2.19", "2.21"] {
        let mut changed = None;
        let mut revisions = std::collections::BTreeMap::new();
        for stage in ["first", "changed", "packed", "cli"] {
            let matching = images
                .iter()
                .filter(|row| row["document_version"] == version && row["stage"] == stage)
                .collect::<Vec<_>>();
            ensure!(
                matching.len() == 1,
                "Missing or duplicate local page MCP stage"
            );
            let row = matching[0];
            let name = format!("codec-local-page-{version}-mcp-{stage}.png");
            ensure!(
                row["name"] == name
                    && row["width"] == 32
                    && row["height"] == 32
                    && row["source_removed"] == matches!(stage, "packed" | "cli"),
                "Wrong local page MCP geometry/removal proof"
            );
            let bytes = performance::png(&root.join(&name), row, 32, 32)?;
            let artifacts = row["dynamic_artifacts"]
                .as_array()
                .context("Missing local page MCP acquisition")?;
            ensure!(
                artifacts.len() == 1
                    && artifacts[0]["acquisition"]["local_files"]["fulfilled"] == 2
                    && artifacts[0]["acquisition"]["local_files"]["denied"] == 0,
                "Incomplete local page MCP response proof"
            );
            let revision = artifacts[0]["acquisition"]["resources"]["revision"]
                .as_str()
                .context("Missing local page MCP revision")?;
            ensure!(
                revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid local page MCP revision"
            );
            revisions.insert(stage, revision);
            if stage == "changed" {
                changed = Some(bytes);
            } else if matches!(stage, "packed" | "cli") {
                ensure!(
                    changed.as_ref() == Some(&bytes),
                    "Packed MCP/CLI local page pixels changed"
                );
            }
        }
        ensure!(
            revisions["first"] != revisions["changed"]
                && revisions["changed"] == revisions["packed"]
                && revisions["packed"] == revisions["cli"],
            "Stale local page MCP namespace"
        );
    }
    println!("Verified source identities, hashes and dimensions for eight local page MCP/CLI PNGs");
    Ok(())
}
pub fn panel(s: &Session) -> Result<()> {
    let chromium = std::env::var_os("LAZYPENCIL_TEST_CHROMIUM").context("Chromium unavailable")?;
    let output = s.temporary.join("chromium-panel.png");
    let raw = s.capture(
        Command::new(s.executable("debug"))
            .env("LAZYPENCIL_CHROMIUM", chromium)
            .arg("--json")
            .arg("--data-dir")
            .arg(s.temporary.join("chromium-test-data"))
            .args([
                "render",
                "examples/dynamic-panel.pen",
                "--time",
                "0.5",
                "--output",
            ])
            .arg(&output),
        300,
        "chromium-panel.json",
    )?;
    let report: Value = serde_json::from_slice(&raw)?;
    ensure!(
        report["width"] == 1000
            && report["height"] == 640
            && report["diagnostics"] == json!([])
            && report["dynamic_artifacts"]
                .as_array()
                .is_some_and(|a| a.len() == 2),
        "Unexpected combined panel"
    );
    performance::png(&output, &report, 1000, 640)?;
    Ok(())
}
