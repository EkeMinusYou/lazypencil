//! Native CI stage ordering and composition. Process and catalog primitives
//! stay on Session so focused and complete workflows share the same checks.
use super::*;

impl Session {
    pub(super) fn run_ordered(&self, args: Arguments, concurrent_compilation: bool) -> Result<()> {
        let targets: Vec<String> = serde_json::from_str(&args.targets)?;
        if args.suite == "normal-ci" {
            ensure!(
                targets.is_empty(),
                "Normal CI scheduler checks require the complete default catalog"
            );
        }
        if args.suite == "browser-ci" {
            ensure!(
                targets
                    .iter()
                    .map(String::as_str)
                    .eq(focus::BROWSER_CI_TARGETS.iter().copied()),
                "Browser CI reproduction requires its complete test target set"
            );
        }
        let has = |s: &str| targets.iter().any(|t| t == s);
        let core = ["core", "conversation"].contains(&args.suite.as_str());
        match args.stage {
            Stage::BinariesStage => {
                let elapsed = normal::cargo_checks(self)?;
                binaries::stage(self, elapsed)
            }
            Stage::NormalConsumer => {
                let elapsed = binaries::restore(self, binaries::Subset::All)?;
                normal::consume(self, elapsed)?;
                binaries::completed(self, "normal")
            }
            Stage::PixelsConsumer => {
                let elapsed = binaries::restore(self, binaries::Subset::All)?;
                normal::consume_pixels(self, elapsed)?;
                binaries::completed(self, "pixels")
            }
            Stage::TerminalConsumer => {
                let elapsed = binaries::restore(self, binaries::Subset::Terminal)?;
                normal::consume_terminal(self, elapsed)?;
                binaries::completed(self, "terminal")
            }
            Stage::MaterialConsumer => {
                let _ = binaries::restore(self, binaries::Subset::Material)?;
                self.material_tests()?;
                binaries::completed(self, "material")
            }
            Stage::ShortConsumer => binaries::consume_short(self),
            Stage::RuntimePrepare => {
                let _ = binaries::restore(self, binaries::Subset::Runtime)?;
                self.reproduce_inputs(false)?;
                // Browser preparation publishes GITHUB_ENV values. Consumers
                // run in the next Actions step so both Chromium paths exist.
                browser::setup(self)
            }
            Stage::RuntimeConsumer => {
                ensure!(
                    self.temporary.join("binary-receipt.json").is_file(),
                    "Prepare the runtime catalog before running its tests"
                );
                clipboard::run(self)?;
                let runtime = self.runtime_lanes(false);
                let ownership = browser::ownership_only(self);
                let panel = browser::panel(self);
                let failures = [
                    ("runtime", runtime),
                    ("ownership", ownership),
                    ("panel", panel),
                ]
                .into_iter()
                .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error:#}")))
                .collect::<Vec<_>>();
                ensure!(failures.is_empty(), "{}", failures.join("\n"));
                binaries::completed(self, "runtime")
            }
            Stage::EvidenceGate => binaries::gate(self),
            Stage::Clipboard => clipboard::run(self),
            Stage::EvidenceBuild => release::observe(self),
            Stage::Candidate => candidate::run(self),
            Stage::CandidatePrepare => candidate::prepare(self),
            Stage::Source => {
                let archive = self.temporary.join("native-source.tar.gz");
                dist::source(&util::root(), &archive, 0, false)?;
                dist::verify(&archive, Some(&self.root))?;
                super::super::normalize_source(&self.root)?;
                self.cargo(
                    &["fmt", "--all", "--", "--check"],
                    180,
                    "source-format.log",
                    false,
                )
            }
            Stage::Notices => {
                notices::rust::verify(
                    &self.root,
                    &self.root.join("resources/licenses/rust"),
                    None,
                )?;
                notices::skia::verify_upstream(
                    &self.root.join("resources/licenses/native/upstream.json"),
                )?;
                notices::toolchain::upstream_policy(Some(
                    &self.root.join("resources/licenses/toolchain/upstream.json"),
                ))?;
                Ok(())
            }
            Stage::FocusCore => {
                self.core_reproductions(&args.suite, &targets)?;
                if ["graphics", "dynamic", "copy", "browser"].contains(&args.suite.as_str())
                    && has("project")
                {
                    self.cargo(
                        &[
                            "test",
                            "--locked",
                            "--no-default-features",
                            "--lib",
                            "project::browser_",
                            "--",
                            "--test-threads=2",
                        ],
                        900,
                        "focus-browser-core.log",
                        false,
                    )?;
                }
                if ["graphics", "dynamic", "copy", "browser"].contains(&args.suite.as_str())
                    && (has("scripts") || has("sampler_sources"))
                {
                    let mut command = vec!["test", "--locked", "--no-default-features"];
                    for name in ["project", "scripts", "sampler_sources"] {
                        if has(name) || name == "project" && has("scripts") {
                            command.extend(["--test", name]);
                        }
                    }
                    command.extend(["--", "--test-threads=2"]);
                    self.cargo(&command, 900, "focus-script-core.log", false)?;
                }
                Ok(())
            }
            Stage::FocusPrepare => {
                let cached = std::env::var("NATIVE_CACHE_MATCH").unwrap_or_default();
                let stages = if super::focus::build_before_lint(&args.suite, &cached) {
                    [Stage::FocusBuild, Stage::FocusLint]
                } else {
                    [Stage::FocusLint, Stage::FocusBuild]
                };
                for stage in stages {
                    self.run(Arguments {
                        stage,
                        suite: args.suite.clone(),
                        targets: args.targets.clone(),
                    })?;
                }
                Ok(())
            }
            Stage::FocusBuild => {
                if args.suite == "normal-ci" {
                    return self.build_catalog(&[]);
                }
                if args.suite == "browser-ci" {
                    return self.build_catalog(&targets);
                }
                let separate_core = self.separate_core()?;
                let plan =
                    commands::plan_with_core(&self.root, &args.suite, &targets, separate_core)?;
                for command in plan.build {
                    self.execute(
                        &command,
                        if core { 720 } else { 2400 },
                        "focus-build.log",
                        true,
                    )?;
                }
                Ok(())
            }
            Stage::FocusLint => {
                let mut command = vec!["cargo".into(), "clippy".into(), "--locked".into()];
                if core {
                    command.push("--no-default-features".into());
                }
                if args.suite != "shader" {
                    command.push("--lib".into());
                }
                for target in &targets {
                    command.extend(["--test".into(), target.clone()]);
                }
                command.extend(["--".into(), "-D".into(), "warnings".into()]);
                self.execute(&command, 300, "focus-clippy.log", false)
            }
            Stage::FocusReproduce => {
                let separate_core = self.separate_core()?;
                self.image_reproductions(!core, &targets)?;
                self.asset_maintenance_reproductions(!core, &targets)?;
                if has("sampler_sources") {
                    self.test(
                        !core,
                        "sampler_sources",
                        "file_url_",
                        &["--test-threads=2"],
                        "focus-file-url-samplers.log",
                    )?;
                }
                if has("repair") {
                    self.test(
                        !core,
                        "repair",
                        "",
                        &["--test-threads=2"],
                        "focus-asset-repair.log",
                    )?;
                }
                if args.suite == "dynamic" && has("mcp") {
                    self.test(
                        true,
                        "lib",
                        "runtime::profiles::pages::wire::tests::",
                        &["--test-threads=2"],
                        "focus-profile-channel.log",
                    )?;
                }
                if !core && has("font_sources") {
                    self.test(
                        true,
                        "font_sources",
                        commands::DEFERRED_FONT_REPRODUCTION,
                        &["--exact", "--test-threads=1"],
                        "focus-deferred-fonts.log",
                    )?;
                }
                if has("user_agent_config") {
                    self.test(
                        !core,
                        "user_agent_config",
                        "bound_mcp_memory_policy_is_platform_scoped_preserves_overrides_and_never_changes_the_adapter",
                        &["--exact", "--test-threads=1"],
                        "focus-agent-environment.log",
                    )?;
                }
                if std::env::var("RUNTIME_PARALLEL").as_deref() == Ok("true")
                    || !core && has("conversation_history")
                {
                    management::focus(self)?;
                }
                if !concurrent_compilation {
                    self.focus_terminal_reproduction(&args.suite, &targets)?;
                }
                if args.suite == "candidate" && has("tui") {
                    self.test(
                        true,
                        "lib",
                        "tui::preview::tests::fractional_fit_pan_round_trips_preserve_the_exact_render_region",
                        &["--exact", "--test-threads=1"],
                        "focus-camera-pan.log",
                    )?;
                }
                // The full pixel consumer already runs these owned targets in
                // parallel. Keep the same independent files, logs and limits
                // here, and join every started reproduction before continuing.
                let mut pixels: Vec<parallel::Task<'_>> = Vec::new();
                if !core && has("graphics") {
                    pixels.push((
                        "text-layer reproduction",
                        Box::new(|| {
                            self.test(
                                true,
                                "graphics",
                                focus::TEXT_LAYER_REPRODUCTION,
                                &["--exact", "--test-threads=1"],
                                "focus-text-layers.log",
                            )
                        }),
                    ));
                }
                if commands::color_font_reproduction(&args.suite, &targets).is_some() {
                    pixels.push((
                        "color-font reproduction",
                        Box::new(|| {
                            self.test(
                                true,
                                "font_sources",
                                commands::COLOR_FONT_REPRODUCTION,
                                &["--test-threads=1"],
                                "focus-color-fonts.log",
                            )
                        }),
                    ));
                    pixels.push((
                        "upstream emoji reproduction",
                        Box::new(|| self.real_emoji("focus-real-emoji.log")),
                    ));
                }
                if !pixels.is_empty() {
                    parallel::run(pixels)?;
                }
                if args.suite == "dynamic" && has("native_layout") {
                    self.test(
                        true,
                        "lib",
                        "native_cancellation",
                        &["--test-threads=2"],
                        "focus-native-layout.log",
                    )?;
                    self.test(
                        true,
                        "native_layout",
                        "",
                        &["--test-threads=2"],
                        "focus-native-layout.log",
                    )?;
                }
                if !core && has("conversation_history") {
                    self.test(
                        true,
                        "lib",
                        "conversation::history::",
                        &["--test-threads=2"],
                        "focus-history.log",
                    )?;
                    self.test(
                        true,
                        "conversation_history",
                        "",
                        &["--test-threads=1"],
                        "focus-history.log",
                    )?;
                    if has("agent") {
                        self.test(
                            true,
                            "agent",
                            focus::RECLAIM,
                            &["--exact", "--test-threads=1"],
                            "focus-history.log",
                        )?;
                    }
                    if has("mcp") {
                        self.test(
                            true,
                            "mcp",
                            focus::OWNED_MCP,
                            &["--exact", "--test-threads=1"],
                            "focus-history.log",
                        )?;
                    }
                    if has("tui") {
                        self.test(
                            true,
                            "lib",
                            "tui::tests::saved_log_",
                            &["--test-threads=2"],
                            "focus-history.log",
                        )?;
                        for filter in focus::HISTORY_PTY {
                            self.test(true, "tui", filter, &["--exact"], "focus-history.log")?;
                        }
                    }
                }
                if has("tui") && has("user_config") {
                    self.test(
                        true,
                        "lib",
                        "config::keys::",
                        &["--test-threads=2"],
                        "focus-input.log",
                    )?;
                    self.test(
                        true,
                        "user_config",
                        focus::INPUT_CONFIG,
                        &["--exact"],
                        "focus-input.log",
                    )?;
                    self.test(
                        true,
                        "tui",
                        focus::INPUT_PTY,
                        &["--exact"],
                        "focus-input.log",
                    )?;
                }
                if ["graphics", "dynamic", "copy", "browser"].contains(&args.suite.as_str())
                    && has("script_scope")
                {
                    // These pure namespace checks use the same core compiler
                    // conditions and two-thread harness as the complete core
                    // job. Standalone focus still owns and runs them here.
                    if !separate_core {
                        for filter in [
                            "project::bindings::",
                            "project::import::scripts::composition::tests::",
                        ] {
                            self.test(
                                false,
                                "lib",
                                filter,
                                &["--test-threads=2"],
                                "focus-bindings.log",
                            )?;
                        }
                    }
                    self.test(
                        true,
                        "lib",
                        "project::browser_context::",
                        &["--test-threads=2"],
                        "focus-bindings.log",
                    )?;
                    self.test(
                        true,
                        "lib",
                        "project::bindings::asset_tests::",
                        &["--test-threads=2"],
                        "focus-bindings.log",
                    )?;
                    // A standalone focus owns all affected groups. In the
                    // full pipeline, reproduce the exact failure first and
                    // leave the long groups to the mandatory normal consumer.
                    browser::focus_bindings(self, !concurrent_compilation)?;
                }
                if !concurrent_compilation {
                    self.focus_launch_reproduction(&args.suite)?;
                }
                if ["dynamic", "shader"].contains(&args.suite.as_str()) {
                    self.test(
                        true,
                        "shader_uniforms",
                        "",
                        &["--test-threads=2"],
                        "focus-dynamic-inputs.log",
                    )?;
                }
                Ok(())
            }
            Stage::FocusTest => {
                if args.suite == "candidate" && has("dynamic_render") {
                    self.test(
                        true,
                        "dynamic_render",
                        focus::RETAINED_PAGE,
                        &["--exact", "--ignored", "--test-threads=1"],
                        "focus-page-playback.log",
                    )?;
                }
                if args.suite == "normal-ci" {
                    // This reproduces the normal scheduler. The mandatory
                    // release lane owns release compilation independently.
                    let prepared = normal::prepare(self)?;
                    return normal::remaining(self, &prepared, false);
                }
                if args.suite == "browser-ci" {
                    if std::env::var("RUNTIME_PARALLEL").as_deref() == Ok("true") {
                        // The following focused runtime stage runs these exact
                        // lanes together with MCP, lifecycle and Material.
                        return Ok(());
                    }
                    return browser::tests(self);
                }
                for command in commands::plan(&self.root, &args.suite, &targets)?.checks {
                    self.execute(&command, 300, "focus-tests.log", true)?;
                }
                if args.suite == "copy" {
                    self.test(
                        true,
                        "script_scope",
                        "script_copy::",
                        &["--ignored", "--test-threads=1"],
                        "focus-copy-shader.log",
                    )?;
                }
                Ok(())
            }
            Stage::FocusRelease => {
                release::compile(self, "focus-release.log")?;
                process::checked(
                    Command::new(self.executable("release")).arg("--version"),
                    Duration::from_secs(30),
                )?;
                let record = crate::evidence::fingerprint(
                    &crate::evidence::link_map::path(&self.target),
                    "release/lazypencil.map",
                    64 * 1024 * 1024,
                )?;
                ensure!(
                    record["bytes"].as_u64().unwrap_or(0) > 0,
                    "Missing focused release map"
                );
                Ok(())
            }
            Stage::CoreBuild => {
                fs::write(
                    self.temporary.join("lazypencil-tests.log"),
                    process::text(Command::new("rustc").arg("-vV"))?,
                )?;
                self.cargo(
                    &["test", "--locked", "--no-default-features", "--no-run"],
                    2400,
                    "core-build.log",
                    false,
                )
            }
            Stage::CoreReproduce => self.core_reproductions(&args.suite, &targets),
            Stage::CoreTest => management::core(self),
            Stage::CorePlatform => {
                self.execute(
                    &commands::core_platform_build(),
                    2400,
                    "core-platform-build.log",
                    false,
                )?;
                normal::run_checks(
                    &commands::core_platform_tests(),
                    Instant::now() + Duration::from_secs(900),
                    |args, seconds| self.execute(args, seconds, "core-platform.log", true),
                )
            }
            Stage::CoreLint => self.cargo(
                &[
                    "clippy",
                    "--locked",
                    "--no-default-features",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
                900,
                "lazypencil-tests.log",
                true,
            ),
            Stage::DefaultBuild => {
                let log = self.temporary.join("lazypencil-tests.log");
                if !log.exists() {
                    fs::write(log, process::text(Command::new("rustc").arg("-vV"))?)?;
                }
                self.build_catalog(&[])
            }
            Stage::DefaultDevelopmentBuild => self.execute(
                &commands::catalog_builds(&[])[0],
                2700,
                "default-development-build.log",
                false,
            ),
            Stage::BuildAndReproduce => {
                if ["ci", "normal-ci", "browser-ci"].contains(&args.suite.as_str()) {
                    return self.build_catalog(&[]);
                }
                let narrow = self.separate_core()?
                    && std::env::var("RUNTIME_PARALLEL").as_deref() != Ok("true")
                    && !targets
                        .iter()
                        .any(|target| target == "conversation_history")
                    && !(args.suite == "dynamic" && has("agent"));
                let early = if narrow {
                    commands::reproduction_targets(&args.suite, &targets)
                } else {
                    targets.clone()
                };
                if narrow && early.is_empty() {
                    return self.build_catalog(&[]);
                }
                if narrow {
                    for command in
                        commands::plan_with_core(&self.root, &args.suite, &targets, true)?.build
                    {
                        self.execute(
                            &commands::reproduction_build(&command, &early),
                            2400,
                            "focus-build.log",
                            true,
                        )?;
                    }
                } else {
                    self.run(Arguments {
                        stage: Stage::FocusBuild,
                        suite: args.suite.clone(),
                        targets: args.targets.clone(),
                    })?;
                }
                let catalog =
                    self.compile_catalog(&early, "early-build", &std::env::var("GITHUB_SHA")?)?;
                util::publish(
                    &self.temporary.join("early-test-executables.json"),
                    &util::encode(&catalog)?,
                    false,
                )?;
                // The selected units are already fresh under the same Cargo
                // conditions. Their immutable executables can run while Cargo
                // compiles the remaining targets. Join both lanes on failure.
                parallel::run(vec![
                    (
                        "affected reproductions",
                        Box::new(|| {
                            self.run_ordered(
                                Arguments {
                                    stage: Stage::FocusReproduce,
                                    suite: args.suite.clone(),
                                    targets: args.targets.clone(),
                                },
                                true,
                            )
                        }),
                    ),
                    (
                        "remaining test compilation",
                        Box::new(|| {
                            // The normal build was checked before the early
                            // catalog. Keep the test graph active while its
                            // executables run; do not replace their CLI binary.
                            let catalog = self.compile_test_catalog(
                                &[],
                                "default-build",
                                &std::env::var("GITHUB_SHA")?,
                            )?;
                            self.publish_catalog(&catalog)
                        }),
                    ),
                ])?;
                // The PTY tests a two-second JavaScript budget, and dynamic
                // tests 300ms launch/operation/render budgets. Run both after
                // compilation joins, as on the quiet full terminal consumer.
                self.focus_terminal_reproduction(&args.suite, &targets)?;
                self.focus_launch_reproduction(&args.suite)
            }
            Stage::Reproduce => self.reproduce_inputs(true),
            Stage::DefaultTest => management::default(self),
            Stage::DefaultRelease => normal::run(self),
            Stage::CompleteTests => {
                // The release profile builds on another runner. These lanes
                // consume immutable debug executables and distinct retained
                // filenames. Native-layout and script comparison inputs are
                // complete before the final browser image/ownership gate.
                // Preserve the terminal script's existing two-second deadline
                // in an otherwise quiet host before starting browser workers.
                let prepared = normal::prepare(self)?;
                let result = parallel::run(vec![
                    (
                        "default tests",
                        Box::new(|| normal::remaining(self, &prepared, false)),
                    ),
                    ("runtime suites", Box::new(|| self.runtime_lanes(true))),
                ]);
                let ownership = browser::ownership(self);
                let failures = [("test lanes", result), ("browser evidence", ownership)]
                    .into_iter()
                    .filter_map(|(name, result)| {
                        result.err().map(|error| format!("{name}: {error:#}"))
                    })
                    .collect::<Vec<_>>();
                ensure!(failures.is_empty(), "{}", failures.join("\n"));
                Ok(())
            }
            Stage::BrowserSetup => {
                if args.suite == "release" {
                    browser::setup_binary(self, &self.executable("release"))
                } else if args.suite == "candidate" {
                    browser::setup_binary(
                        self,
                        &self.temporary.join("candidate-install/bin/lazypencil"),
                    )
                } else {
                    browser::setup(self)
                }
            }
            Stage::BrowserTest => browser::tests(self),
            Stage::RuntimeTests | Stage::FocusRuntimeRelease => {
                let compile_release = matches!(args.stage, Stage::FocusRuntimeRelease);
                if args.suite == "focus" || compile_release {
                    let targets = focus::BROWSER_CI_TARGETS
                        .iter()
                        .copied()
                        .chain(["mcp", "native_layout", "optional_icons", "tui"])
                        .collect::<std::collections::BTreeSet<_>>();
                    self.runtime_catalog(
                        &targets.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                        &std::env::var("GITHUB_SHA")?,
                    )?;
                    browser::focus_inputs(self)?;
                }
                // Prepare every debug executable before release Cargo takes its
                // build lock. Runtime tests execute immutable binaries directly;
                // release writes a separate profile and retained log. Join both
                // groups before ownership checks or resource measurements.
                let result = if compile_release {
                    parallel::run(vec![
                        (
                            "release compilation",
                            Box::new(|| release::compile(self, "focus-release-parallel.log")),
                        ),
                        ("runtime suites", Box::new(|| self.runtime_lanes(true))),
                    ])
                } else {
                    self.runtime_lanes(true)
                };
                let ownership = browser::ownership(self);
                let failures = [("runtime lanes", result), ("browser ownership", ownership)]
                    .into_iter()
                    .filter_map(|(name, result)| {
                        result.err().map(|error| format!("{name}: {error:#}"))
                    })
                    .collect::<Vec<_>>();
                ensure!(failures.is_empty(), "{}", failures.join("\n"));
                Ok(())
            }
            Stage::Mcp => self.mcp_tests(),
            Stage::LongTests => self.long_tests(true),
            Stage::Panel => browser::panel(self),
            Stage::DefaultLint => self.cargo(
                &[
                    "clippy",
                    "--locked",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
                900,
                "lazypencil-tests.log",
                true,
            ),
            Stage::Release => release::smoke(self),
            Stage::Process => {
                self.cargo(
                    &[
                        "test",
                        "--locked",
                        "--no-default-features",
                        "--test",
                        "agent",
                        "--no-run",
                    ],
                    900,
                    "process-build.log",
                    false,
                )?;
                self.test(false,"agent","embedded_conversation_uses_the_real_helper_for_supervision_and_bound_mcp_edits",&["--exact","--nocapture"],"process-reproduction.log")?;
                self.cargo(
                    &[
                        "test",
                        "--locked",
                        "--no-default-features",
                        "--no-fail-fast",
                        "--",
                        "--test-threads=2",
                    ],
                    600,
                    "process-tests.log",
                    false,
                )?;
                self.cargo(
                    &[
                        "clippy",
                        "--locked",
                        "--no-default-features",
                        "--all-targets",
                        "--",
                        "-D",
                        "warnings",
                    ],
                    600,
                    "process-clippy.log",
                    false,
                )?;
                process_report(self)
            }
        }
    }
}
