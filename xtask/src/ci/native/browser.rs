use super::*;
use crate::performance;

mod evidence;
pub use evidence::{local_page_mcp_images, required_images, required_images_for};

pub fn setup(s: &Session) -> Result<()> {
    setup_binary(s, &s.executable("debug"))?;
    if std::env::var("HEADED_BROWSER_TESTS").as_deref() != Ok("true") {
        return Ok(());
    }
    let raw = s.capture(
        Command::new(std::env::current_exe()?)
            .args(["fixtures", "browser-ui", "--output"])
            .arg(s.temporary.join("chromium-ui")),
        600,
        "chromium-ui-setup.json",
    )?;
    let report: Value = serde_json::from_slice(&raw)?;
    let executable = PathBuf::from(
        report["executable"]
            .as_str()
            .context("Full browser executable missing")?,
    )
    .canonicalize()?;
    ensure!(
        executable.starts_with(s.temporary.join("chromium-ui").canonicalize()?),
        "Full browser outside owned fixture directory"
    );
    if cfg!(target_os = "linux") {
        sandbox_named(s, &executable, "chromium-ui")?;
    }
    Ok(())
}

pub fn setup_binary(s: &Session, binary: &Path) -> Result<()> {
    let data = s.temporary.join("chromium-test-data");
    s.capture(
        Command::new(binary)
            .arg("--json")
            .arg("--data-dir")
            .arg(&data)
            .args(["runtime", "setup", "chromium"]),
        600,
        "chromium-setup.json",
    )?;
    let raw = s.capture(
        Command::new(binary)
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
    sandbox_named(s, executable, "chromium")
}

fn sandbox_named(s: &Session, executable: &Path, name: &str) -> Result<()> {
    ensure!(
        ["chromium", "chromium-ui"].contains(&name),
        "Unknown owned browser sandbox role"
    );
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
        "abi <abi/4.0>,\ninclude <tunables/global>\nprofile lazypencil_ci_{} \"{text}\" flags=(unconfined) {{\n  userns,\n}}\n",
        name.replace('-', "_")
    );
    let path = s.temporary.join(format!("{name}-userns.apparmor"));
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
        &s.temporary.join(format!("{name}-userns-policy.json")),
        &util::encode(
            &json!({"executable":text,"profile_sha256":util::digest(policy),"kernel_restriction_before":1,"kernel_restriction_after":1,"loaded":true}),
        )?,
        false,
    )
}
// Each target owns its temporary documents, browser profiles and process groups.
// Their retained filenames occupy separate namespaces. Readiness cases also
// have independent profiles and output names; other targets remain serial.
// Keep ownership/destruction after every producer has joined.
pub const TEST_LANES: [&[&str]; 3] = [
    &["browser_readiness"],
    &["dynamic_render", "script_scope"],
    &[
        "shader_uniforms",
        "sampler_sources",
        "imports",
        "windows_agent",
    ],
];

pub fn test_threads(target: &str) -> usize {
    if target == "browser_readiness" { 2 } else { 1 }
}

pub const BINDING_REPRODUCTION: &str = "script_bindings::generated_sources::generated_replacements_keep_their_code_owner_under_later_overrides";

pub fn binding_commands(binary: &Path, complete: bool) -> Vec<Vec<String>> {
    [
        BINDING_REPRODUCTION,
        "generated_browser_",
        "script_bindings::",
        "script_copy::",
    ]
    .into_iter()
    .take(if complete { 4 } else { 1 })
    .map(|filter| {
        // Complete default verification already runs these owned cases with
        // two harness threads. Retain separate logs and the original filters.
        let threads = if filter == "script_bindings::" { 2 } else { 1 };
        let mut command = vec![
            binary.to_string_lossy().into_owned(),
            filter.into(),
            format!("--test-threads={threads}"),
        ];
        if filter == BINDING_REPRODUCTION {
            command.push("--exact".into());
        }
        if filter == "script_bindings::" {
            command.extend(["--skip".into(), BINDING_REPRODUCTION.into()]);
        }
        command
    })
    .collect()
}

/// These CPU/layout checks own distinct temporary projects and script workers.
/// Only the script-bindings group writes codec-script-overrides artifacts.
pub fn focus_bindings(s: &Session, complete: bool) -> Result<()> {
    let commit = std::env::var("GITHUB_SHA")?;
    let catalog = if s.temporary.join("early-test-executables.json").exists() {
        s.verified_catalog(&["script_scope"])?
    } else {
        s.compile_catalog(&["script_scope".into()], "focus-binding-build", &commit)?
    };
    catalog.validate(&s.target, &commit, &["script_scope"])?;
    let checks = binding_commands(&catalog.binaries["script_scope"].path, complete);
    let execute = |index: usize, log: &str| {
        let mut command = s.command(&checks[index]);
        command
            .env("CARGO_MANIFEST_DIR", &s.root)
            .env("CARGO_MANIFEST_PATH", s.root.join("Cargo.toml"));
        s.execute_command(&mut command, 300, log, false)
    };
    // Reproduce the changed input boundary before unrelated long layouts. The
    // later binding group excludes this exact same-condition invocation.
    execute(0, "focus-generated-code-owner.log")?;
    if !complete {
        // The mandatory normal consumer runs this same immutable catalog's
        // complete default target. Do not hold every consumer behind a second
        // copy of its long binding, generated-browser and legacy-copy groups.
        return Ok(());
    }
    let groups = parallel::run(vec![
        (
            "generated browser layouts",
            Box::new(|| execute(1, "focus-browser-bindings.log")),
        ),
        (
            "script bindings and overrides",
            Box::new(|| execute(2, "focus-script-bindings.log")),
        ),
    ]);
    // Preserve the final legacy-copy check even if either joined group failed.
    let copies = execute(3, "focus-copy-bindings.log");
    let failures = [("binding groups", groups), ("legacy script copies", copies)]
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error:#}")))
        .collect::<Vec<_>>();
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

pub fn tests(s: &Session) -> Result<()> {
    lanes(s)?;
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
        if let Err(error) = s.execute_command(&mut command, 300, "focus-runtime-inputs.log", true) {
            failures.push(format!("{target}: {error:#}"));
        }
    }
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

const HEADED_LOGIN: &str = "browser_profiles::headed_login_closes_normally_and_preserves_storage_for_the_same_full_browser";
const HEADED_LOG: &str = "chromium-headed-login.log";
const PROFILE_STORAGE: &str = "browser_profiles::dedicated_profiles_persist_full_storage_isolate_acquisitions_and_keep_secrets_out_of_metadata";
const PROFILE_LOG: &str = "chromium-profile-storage.log";

pub(super) struct Prepared {
    catalog: Catalog,
    deadline: Instant,
}

fn required_targets() -> Vec<&'static str> {
    let mut required: Vec<_> = TEST_LANES.into_iter().flatten().copied().collect();
    required.push("agent");
    required
}

pub(super) fn prepare(s: &Session) -> Result<Prepared> {
    let required = required_targets();
    let catalog = s.verified_catalog(&required)?;
    let deadline = Instant::now() + Duration::from_secs(1200);
    // Full Chrome login and persistent-profile restoration include bounded
    // startup/first-acquisition deadlines. Keep them away from competing
    // software WebGL/browser producers without changing their own deadlines.
    for (case, log) in [(HEADED_LOGIN, HEADED_LOG), (PROFILE_STORAGE, PROFILE_LOG)] {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "Browser suite deadline exceeded");
        s.execute(
            &[
                catalog.binaries["browser_readiness"]
                    .path
                    .to_string_lossy()
                    .into_owned(),
                case.into(),
                "--exact".into(),
                "--ignored".into(),
                "--test-threads=1".into(),
            ],
            remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0),
            log,
            false,
        )?;
    }
    Ok(Prepared { catalog, deadline })
}

pub fn lanes(s: &Session) -> Result<()> {
    run_prepared(s, prepare(s)?)
}

pub(super) fn run_prepared(s: &Session, prepared: Prepared) -> Result<()> {
    let Prepared { catalog, deadline } = prepared;
    let result = parallel::run(
        TEST_LANES
            .into_iter()
            .enumerate()
            .map(|(index, targets)| -> parallel::Task<'_> {
                let catalog = &catalog;
                (
                    [
                        "browser readiness",
                        "dynamic rendering and scripts",
                        "shaders and samplers",
                    ][index],
                    Box::new(move || {
                        let mut failures = Vec::new();
                        for target in targets {
                            let remaining = deadline.saturating_duration_since(Instant::now());
                            let outcome = if remaining.is_zero() {
                                Err(anyhow::anyhow!("Browser suite deadline exceeded"))
                            } else {
                                let mut args = vec![
                                    catalog.binaries[*target]
                                        .path
                                        .to_string_lossy()
                                        .into_owned(),
                                    "--ignored".into(),
                                    format!("--test-threads={}", test_threads(target)),
                                ];
                                if *target == "browser_readiness" {
                                    args.extend(["--skip".into(), HEADED_LOGIN.into()]);
                                    args.extend(["--skip".into(), PROFILE_STORAGE.into()]);
                                }
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
    combined.write_all(&fs::read(s.temporary.join(HEADED_LOG))?)?;
    combined.write_all(&fs::read(s.temporary.join(PROFILE_LOG))?)?;
    for target in required_targets() {
        let path = s.temporary.join(format!("chromium-suite-{target}.log"));
        if path.exists() {
            writeln!(combined, "\nBrowser test target: {target}")?;
            combined.write_all(&fs::read(path)?)?;
        }
    }
    result
}

pub fn ownership(s: &Session) -> Result<()> {
    ownership_only(s)?;
    required_images(
        &s.temporary.join("codec-reference"),
        &std::env::var("GITHUB_SHA")?,
    )
}

pub fn ownership_only(s: &Session) -> Result<()> {
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
    )
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
