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
pub fn tests(s: &Session) -> Result<()> {
    s.cargo(
        &[
            "test",
            "--locked",
            "--no-fail-fast",
            "--test",
            "dynamic_render",
            "--test",
            "browser_readiness",
            "--test",
            "shader_uniforms",
            "--test",
            "sampler_sources",
            "--test",
            "imports",
            "--test",
            "windows_agent",
            "--test",
            "script_scope",
            "--",
            "--ignored",
            "--test-threads=1",
        ],
        1200,
        "chromium-render.log",
        false,
    )?;
    s.test(true,"agent","browser_ownership::real_browser_drop_and_owner_death_reap_the_tree_and_preserve_other_owners",&["--exact","--ignored","--test-threads=1"],"chromium-render.log")?;
    required_images(
        &s.temporary.join("codec-reference"),
        &std::env::var("GITHUB_SHA")?,
    )
}
pub fn required_images(root: &Path, commit: &str) -> Result<()> {
    let report = |name: &str| -> Result<Value> {
        let v: Value = util::json(&root.join(name))?;
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
            "variable",
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
                ensure!(
                    row["sources_removed"] == true && dimensions(row)? == (32, 16),
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
                    let bytes = png(&name, &entry["png_hash"], 32, 16)?;
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
    println!("Verified source identities, hashes and dimensions for 343 required PNGs");
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
