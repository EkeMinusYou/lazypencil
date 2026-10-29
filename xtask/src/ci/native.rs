use super::{commands, focus};
use crate::{dist, notices, process, util};
use anyhow::{Context, Result, ensure};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};

mod binaries;
pub mod browser;
mod candidate;
mod clipboard;
mod management;
mod normal;
pub mod parallel;
pub mod release;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Stage {
    EvidenceBuild,
    Candidate,
    CandidatePrepare,
    Source,
    Notices,
    FocusCore,
    FocusPrepare,
    FocusBuild,
    FocusLint,
    FocusReproduce,
    FocusTest,
    FocusRelease,
    FocusRuntimeRelease,
    CoreBuild,
    CoreTest,
    CoreLint,
    DefaultBuild,
    BuildAndReproduce,
    Reproduce,
    DefaultTest,
    DefaultRelease,
    CompleteTests,
    BinariesStage,
    NormalConsumer,
    PixelsConsumer,
    RuntimePrepare,
    RuntimeConsumer,
    EvidenceGate,
    Clipboard,
    BrowserSetup,
    BrowserTest,
    RuntimeTests,
    Mcp,
    LongTests,
    Panel,
    DefaultLint,
    Release,
    Process,
}
#[derive(Args)]
pub struct Arguments {
    #[arg(value_enum)]
    pub stage: Stage,
    #[arg(long, default_value = "core")]
    pub suite: String,
    #[arg(long, default_value = "[]")]
    pub targets: String,
}
impl Arguments {
    pub fn run(self) -> Result<()> {
        Session::hosted()?.run(self)
    }
}

pub struct Session {
    pub root: PathBuf,
    pub temporary: PathBuf,
    pub target: PathBuf,
    timing_lock: Mutex<()>,
}
impl Session {
    pub fn hosted() -> Result<Self> {
        ensure!(
            std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
            "Native builds belong on GitHub Actions"
        );
        commands::optimized_tests()?;
        let temporary =
            PathBuf::from(std::env::var_os("RUNNER_TEMP").context("RUNNER_TEMP required")?);
        let target = PathBuf::from(
            std::env::var_os("CARGO_TARGET_DIR").context("CARGO_TARGET_DIR required")?,
        );
        ensure!(
            temporary.is_absolute() && target.is_absolute() && target.starts_with(&temporary),
            "Hosted build paths must be absolute and isolated"
        );
        Ok(Self {
            root: temporary.join("lazypencil-source"),
            temporary,
            target,
            timing_lock: Mutex::new(()),
        })
    }
    pub fn executable(&self, profile: &str) -> PathBuf {
        self.target.join(profile).join(if cfg!(windows) {
            "lazypencil.exe"
        } else {
            "lazypencil"
        })
    }
    pub fn command(&self, invocation: &[String]) -> Command {
        let invocation = commands::test_profile(
            invocation,
            std::env::var(commands::TEST_PROFILE_ENV).as_deref() == Ok("optimized-debug-v1"),
        );
        let jobs = if cfg!(target_os = "macos") {
            3
        } else if cfg!(target_os = "linux") {
            2
        } else {
            1
        };
        let invocation = commands::build_jobs_for(&invocation, jobs);
        let mut command = Command::new(&invocation[0]);
        command
            .args(&invocation[1..])
            .current_dir(&self.root)
            .env("CARGO_TARGET_DIR", &self.target);
        command
    }
    pub fn execute(
        &self,
        invocation: &[String],
        seconds: u64,
        log: &str,
        append: bool,
    ) -> Result<()> {
        self.execute_command(&mut self.command(invocation), seconds, log, append)
    }
    fn execute_command(
        &self,
        command: &mut Command,
        seconds: u64,
        log: &str,
        append: bool,
    ) -> Result<()> {
        self.require_consumer_command(command)?;
        let invocation = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let started = Instant::now();
        let result = process::logged(
            command,
            Duration::from_secs(seconds),
            &self.temporary.join(log),
            append,
        );
        self.record_timing(&invocation, started.elapsed(), result.is_ok(), log)?;
        result
    }
    fn real_emoji(&self, log: &str) -> Result<()> {
        let output = self.temporary.join("emoji-fonts");
        let started = Instant::now();
        let result = crate::fixtures::emoji::prepare(
            &self.root.join("tests/fixtures/fonts/emoji-upstream.json"),
            &output,
        );
        self.record_timing(
            &["prepare-pinned-emoji-fonts".into()],
            started.elapsed(),
            result.is_ok(),
            log,
        )?;
        result?;
        let invocation = self.test_invocation(
            true,
            "font_sources",
            "real_emoji::",
            &["--ignored", "--test-threads=1"],
        )?;
        let mut command = self.command(&invocation);
        command
            .env("LAZYPENCIL_TEST_EMOJI_DIR", output)
            .env("CARGO_MANIFEST_DIR", &self.root)
            .env("CARGO_MANIFEST_PATH", self.root.join("Cargo.toml"));
        self.execute_command(&mut command, 300, log, false)
    }
    fn reproduce_inputs(&self, emoji: bool) -> Result<()> {
        if emoji {
            self.real_emoji("real-emoji.log")?;
        }
        self.test(
            true,
            "lib",
            "tui::tests::",
            &["--test-threads=2"],
            "tui-reproduction.log",
        )?;
        self.test(
            true,
            "mcp",
            "cancelling_a_running_script_unblocks_the_mcp_document_worker",
            &["--exact", "--test-threads=1"],
            "chromium-script-cancel.log",
        )?;
        for log in [
            "real-emoji.log",
            "tui-reproduction.log",
            "chromium-script-cancel.log",
        ] {
            if !emoji && log == "real-emoji.log" {
                continue;
            }
            fs::OpenOptions::new()
                .append(true)
                .open(self.temporary.join("lazypencil-tests.log"))?
                .write_all(&fs::read(self.temporary.join(log))?)?;
        }
        Ok(())
    }

    fn record_timing(
        &self,
        invocation: &[String],
        elapsed: Duration,
        success: bool,
        log: &str,
    ) -> Result<()> {
        let line = format!(
            "{}\n",
            json!({"source_commit":std::env::var("GITHUB_SHA").ok(),"command":invocation,"elapsed_seconds":elapsed.as_secs_f64(),"success":success,"log":log})
        );
        let _guard = self
            .timing_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Timing writer panicked"))?;
        let mut timings = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.temporary.join("ci-timings.jsonl"))?;
        timings.write_all(line.as_bytes())?;
        Ok(())
    }
    pub fn cargo(&self, args: &[&str], seconds: u64, log: &str, append: bool) -> Result<()> {
        self.execute(
            &std::iter::once("cargo")
                .chain(args.iter().copied())
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            seconds,
            log,
            append,
        )
    }
    pub fn test(
        &self,
        features: bool,
        target: &str,
        filter: &str,
        arguments: &[&str],
        log: &str,
    ) -> Result<()> {
        let invocation = self.test_invocation(features, target, filter, arguments)?;
        let mut command = self.command(&invocation);
        command
            .env("CARGO_MANIFEST_DIR", &self.root)
            .env("CARGO_MANIFEST_PATH", self.root.join("Cargo.toml"));
        self.execute_command(&mut command, 300, log, true)
    }
    fn test_invocation(
        &self,
        features: bool,
        target: &str,
        filter: &str,
        arguments: &[&str],
    ) -> Result<Vec<String>> {
        if features
            && ["test-executables.json", "early-test-executables.json"]
                .iter()
                .any(|name| self.temporary.join(name).exists())
        {
            let catalog = self.verified_catalog(&[])?;
            if let Some(binary) = catalog.binaries.get(target) {
                let mut args = vec![binary.path.to_string_lossy().into_owned()];
                if !filter.is_empty() {
                    args.push(filter.into());
                }
                args.extend(arguments.iter().map(|arg| (*arg).to_owned()));
                return Ok(args);
            }
        }
        let mut args = vec!["cargo", "test", "--locked"];
        if !features {
            args.push("--no-default-features");
        }
        if target == "lib" {
            args.push("--lib");
        } else {
            args.extend(["--test", target]);
        }
        if !filter.is_empty() {
            args.push(filter);
        }
        args.push("--");
        args.extend(arguments);
        Ok(args.into_iter().map(str::to_owned).collect())
    }
    fn require_consumer_command(&self, command: &Command) -> Result<()> {
        let cargo = matches!(
            Path::new(command.get_program())
                .file_name()
                .and_then(|name| name.to_str()),
            Some("cargo" | "cargo.exe")
        );
        ensure!(
            !cargo || !self.temporary.join("binary-receipt.json").exists(),
            "A compiled-catalog consumer cannot invoke Cargo"
        );
        Ok(())
    }

    pub fn capture(&self, command: &mut Command, seconds: u64, path: &str) -> Result<Vec<u8>> {
        self.require_consumer_command(command)?;
        let invocation = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let started = Instant::now();
        let result = process::output(
            command.current_dir(&self.root),
            Duration::from_secs(seconds),
        );
        self.record_timing(
            &invocation,
            started.elapsed(),
            result
                .as_ref()
                .is_ok_and(|result| !result.timed_out && result.status.success()),
            path,
        )?;
        let result = result?;
        fs::write(self.temporary.join(path), &result.stdout)?;
        fs::write(
            self.temporary.join(format!("{path}.stderr.log")),
            &result.stderr,
        )?;
        ensure!(
            !result.timed_out && result.status.success(),
            "Command failed: {command:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(result.stdout)
    }
    pub fn run(&self, args: Arguments) -> Result<()> {
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
                let elapsed = binaries::restore(self, false)?;
                normal::consume(self, elapsed)?;
                binaries::completed(self, "normal")
            }
            Stage::PixelsConsumer => {
                let elapsed = binaries::restore(self, false)?;
                normal::consume_pixels(self, elapsed)?;
                binaries::completed(self, "pixels")
            }
            Stage::RuntimePrepare => {
                let _ = binaries::restore(self, true)?;
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
                let runtime = self.runtime_lanes();
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
                super::normalize_source(&self.root)?;
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
                if args.suite == "dynamic" && has("agent") {
                    self.test(false,"agent","browser_ownership::killed_profile_owners_reap_children_preserve_persistent_data_and_release_only_their_lease",
                        &["--exact","--test-threads=1"],"focus-profile-ownership.log")?;
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
                if std::env::var("RUNTIME_PARALLEL").as_deref() == Ok("true") {
                    management::focus(self)?;
                }
                if let Some(command) = commands::terminal_reproduction(&args.suite, &targets) {
                    self.execute(&command, 300, "focus-terminal-modes.log", false)?;
                }
                if !core && has("graphics") {
                    self.test(
                        true,
                        "graphics",
                        focus::TEXT_LAYER_REPRODUCTION,
                        &["--exact", "--test-threads=1"],
                        "focus-text-layers.log",
                    )?;
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
                if let Some(command) = commands::color_font_reproduction(&args.suite, &targets) {
                    self.execute(&command, 300, "focus-color-fonts.log", false)?;
                    self.real_emoji("focus-real-emoji.log")?;
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
                        false,
                        "lib",
                        "conversation::management::",
                        &["--test-threads=2"],
                        "focus-history.log",
                    )?;
                    for filter in ["conversation::management::", "conversation::history::"] {
                        self.test(
                            true,
                            "lib",
                            filter,
                            &["--test-threads=2"],
                            "focus-history.log",
                        )?;
                    }
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
                    browser::focus_bindings(self)?;
                }
                if args.suite == "dynamic" {
                    self.test(
                        true,
                        "dynamic_render",
                        "",
                        &["--test-threads=2"],
                        "focus-dynamic-inputs.log",
                    )?;
                    self.test(
                        true,
                        "lib",
                        "dynamic::worker::",
                        &["--test-threads=2"],
                        "focus-dynamic-inputs.log",
                    )?;
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
                    return normal::run(self);
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
            Stage::CoreTest => management::core(self),
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
            Stage::BuildAndReproduce => {
                if ["ci", "normal-ci", "browser-ci"].contains(&args.suite.as_str()) {
                    return self.build_catalog(&[]);
                }
                self.run(Arguments {
                    stage: Stage::FocusBuild,
                    suite: args.suite.clone(),
                    targets: args.targets.clone(),
                })?;
                let catalog =
                    self.compile_catalog(&targets, "early-build", &std::env::var("GITHUB_SHA")?)?;
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
                            self.run(Arguments {
                                stage: Stage::FocusReproduce,
                                suite: args.suite.clone(),
                                targets: args.targets.clone(),
                            })
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
                ])
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
                    ("runtime suites", Box::new(|| self.runtime_lanes())),
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
                    self.build_catalog(
                        &targets.into_iter().map(str::to_owned).collect::<Vec<_>>(),
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
                        ("runtime suites", Box::new(|| self.runtime_lanes())),
                    ])
                } else {
                    self.runtime_lanes()
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
            Stage::LongTests => self.long_tests(),
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
    fn separate_core(&self) -> Result<bool> {
        std::env::var("LAZYPENCIL_CI_SEPARATE_CORE")
            .unwrap_or_else(|_| "false".into())
            .parse::<bool>()
            .context("LAZYPENCIL_CI_SEPARATE_CORE must be true or false")
    }

    fn build_catalog(&self, targets: &[String]) -> Result<()> {
        let catalog =
            self.compile_catalog(targets, "default-build", &std::env::var("GITHUB_SHA")?)?;
        self.publish_catalog(&catalog)
    }

    fn publish_catalog(&self, catalog: &Catalog) -> Result<()> {
        util::publish(
            &self.temporary.join("test-executables.json"),
            &util::encode(catalog)?,
            false,
        )
    }

    // Early reproductions need compiler-reported identities before the runtime
    // stage publishes its complete catalog. Keep their records independent.
    fn compile_catalog(&self, targets: &[String], log: &str, commit: &str) -> Result<Catalog> {
        let [normal, _] = commands::catalog_builds(targets);
        self.execute(&normal, 2700, &format!("{log}.log"), false)?;
        self.compile_test_catalog(targets, log, commit)
    }

    fn compile_test_catalog(&self, targets: &[String], log: &str, commit: &str) -> Result<Catalog> {
        let [_, mut tests] = commands::catalog_builds(targets);
        if !targets.is_empty() && self.root.join("src/lib.rs").is_file() {
            tests.push("--lib".into());
        }
        let mut command = self.command(&tests);
        let raw = self.capture(&mut command, 2700, &format!("{log}.jsonl"))?;
        let mut binaries = BTreeMap::new();
        for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let event: Value = serde_json::from_slice(line)?;
            if event["reason"] == "compiler-artifact"
                && event["profile"]["test"] == true
                && event["package_id"]
                    .as_str()
                    .is_some_and(|id| id.contains("#lazypencil@"))
                && let Some(path) = event["executable"].as_str()
            {
                let path = PathBuf::from(path);
                ensure!(
                    path.starts_with(self.target.join("debug/deps")),
                    "Unexpected test executable path"
                );
                let name = event["target"]["name"]
                    .as_str()
                    .context("Test target name required")?;
                let key = if event["target"]["kind"]
                    .as_array()
                    .is_some_and(|k| k.contains(&json!("lib")))
                {
                    "lib"
                } else {
                    name
                };
                ensure!(
                    binaries
                        .insert(
                            key.to_owned(),
                            Binary {
                                sha256: util::file_digest(&path)?,
                                path
                            }
                        )
                        .is_none(),
                    "Duplicate test executable"
                );
            }
        }
        let required: Vec<_> = if targets.is_empty() {
            vec!["mcp", "tui", "optional_icons"]
        } else {
            targets.iter().map(String::as_str).collect()
        };
        ensure!(
            required.iter().all(|s| binaries.contains_key(*s)),
            "Missing parallel test binaries"
        );
        Ok(Catalog {
            source_commit: commit.into(),
            binaries,
        })
    }
    fn runtime_lanes(&self) -> Result<()> {
        let prepared_browser = browser::prepare(self)?;
        // MCP and idle lifecycle clients own separate documents, process trees
        // and automatic CDP ports. Material has an independent settings path.
        parallel::run(vec![
            (
                "browser pixels",
                Box::new(|| browser::run_prepared(self, prepared_browser)),
            ),
            ("MCP dynamic images", Box::new(|| self.mcp_tests())),
            ("lifecycle and Material", Box::new(|| self.long_tests())),
        ])
    }

    fn mcp_tests(&self) -> Result<()> {
        let catalog = self.verified_catalog(&["mcp", "agent"])?;
        let mut failures = Vec::new();
        for test in [
            "real_mcp_shader_assets_uniform_edits_and_browser_refresh_return_image_content",
            "local_pages::real_mcp_and_cli_local_pages_declare_edit_pack_and_render_after_source_removal",
        ] {
            if let Err(error) = self.mcp_check(&catalog, "mcp", test, true, "chromium-mcp.log") {
                failures.push(format!("{test}: {error:#}"));
            }
        }
        if let Err(error) = browser::local_page_mcp_images(
            &self.temporary.join("codec-reference"),
            &std::env::var("GITHUB_SHA")?,
        ) {
            failures.push(format!("MCP image evidence: {error:#}"));
        }
        if let Err(error) = self.mcp_check(
            &catalog,
            "agent",
            "supervised_agent_mcp_renders_script_browser_and_shader_pixels",
            false,
            "chromium-supervision.log",
        ) {
            failures.push(format!("Supervised MCP images: {error:#}"));
        }
        ensure!(failures.is_empty(), "{}", failures.join("\n"));
        Ok(())
    }

    fn mcp_check(
        &self,
        catalog: &Catalog,
        target: &str,
        filter: &str,
        exact: bool,
        log: &str,
    ) -> Result<()> {
        let mut args = vec![
            catalog.binaries[target].path.to_string_lossy().into_owned(),
            filter.into(),
        ];
        if exact {
            args.push("--exact".into());
        }
        args.extend(["--ignored", "--test-threads=1"].map(str::to_owned));
        let mut command = self.command(&args);
        command
            .env("CARGO_MANIFEST_DIR", &self.root)
            .env("CARGO_MANIFEST_PATH", self.root.join("Cargo.toml"))
            .env(
                "LAZYPENCIL_CONFIG_DIR",
                self.temporary.join("runtime-mcp-settings"),
            );
        self.execute_command(&mut command, 300, log, true)
    }

    fn lifecycle_tests(&self, catalog: &Catalog, target: &str, log: &str) -> Result<()> {
        let args = commands::lifecycle(&catalog.binaries[target].path, target)?;
        let mut command = self.command(&args);
        command.env(
            "LAZYPENCIL_CONFIG_DIR",
            self.temporary
                .join(format!("runtime-lifecycle-{target}-settings")),
        );
        self.execute_command(&mut command, 300, log, false)
    }

    fn long_tests(&self) -> Result<()> {
        super::setup::material_fonts(
            &self.root.join("resources/icons/catalogue.json"),
            &self.temporary.join("lazypencil-material-fonts"),
        )?;
        let catalog = self.verified_catalog(&["mcp", "tui", "optional_icons"])?;
        // These independent lanes use immutable debug binaries and all join
        // before subsequent smoke tests and resource measurements.
        parallel::run(vec![
            (
                "MCP lifecycle",
                Box::new(|| self.lifecycle_tests(&catalog, "mcp", "chromium-mcp-idle.log")),
            ),
            (
                "TUI lifecycle",
                Box::new(|| self.lifecycle_tests(&catalog, "tui", "chromium-tui.log")),
            ),
            (
                "Material verification",
                Box::new(|| -> Result<()> {
                    let mut failures = Vec::new();
                    for (target, filter, log) in [
                        (
                            "optional_icons",
                            "official_material_fonts",
                            "lazypencil-material-fonts.log",
                        ),
                        ("mcp", "real_mcp_material", "lazypencil-material-mcp.log"),
                    ] {
                        let args = vec![
                            catalog.binaries[target].path.to_string_lossy().into_owned(),
                            filter.into(),
                            "--ignored".into(),
                            "--nocapture".into(),
                            "--test-threads=1".into(),
                        ];
                        let mut command = self.command(&args);
                        command
                            .env(
                                "LAZYPENCIL_TEST_MATERIAL_DIR",
                                self.temporary.join("lazypencil-material-fonts"),
                            )
                            .env(
                                "LAZYPENCIL_CONFIG_DIR",
                                self.temporary.join("lazypencil-material-settings"),
                            );
                        if let Err(error) = self.execute_command(&mut command, 3600, log, false) {
                            failures.push(format!("{target}: {error:#}"));
                        }
                    }
                    ensure!(failures.is_empty(), "{}", failures.join("\n"));
                    Ok(())
                }),
            ),
        ])
    }

    fn verified_catalog(&self, required: &[&str]) -> Result<Catalog> {
        let complete = self.temporary.join("test-executables.json");
        let catalog: Catalog = util::json(&if complete.exists() {
            complete
        } else {
            self.temporary.join("early-test-executables.json")
        })?;
        catalog.validate(&self.target, &std::env::var("GITHUB_SHA")?, required)?;
        Ok(catalog)
    }
}
#[derive(Serialize, Deserialize)]
struct Binary {
    path: PathBuf,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Catalog {
    source_commit: String,
    binaries: BTreeMap<String, Binary>,
}
impl Catalog {
    fn validate(&self, target: &Path, commit: &str, required: &[&str]) -> Result<()> {
        ensure!(
            self.source_commit == commit,
            "Test executables belong to another commit"
        );
        ensure!(
            required
                .iter()
                .all(|name| self.binaries.contains_key(*name)),
            "Required test executable missing"
        );
        for binary in self.binaries.values() {
            ensure!(
                binary.path.parent() == Some(target.join("debug/deps").as_path())
                    && util::file_digest(&binary.path)? == binary.sha256,
                "Test executable changed after compilation"
            );
        }
        Ok(())
    }
}
fn process_report(s: &Session) -> Result<()> {
    let log = fs::read_to_string(s.temporary.join("process-tests.log"))?;
    let mut cases = vec!["closing_the_host_reaps_its_adapter_process_group".to_owned()];
    cases.extend(
        [
            "close_reaps_the_owned_tree_without_stopping_another_session",
            "dropping_host_and_runtime_reaps_the_adapter_tree",
            "uncooperative_cancellation_reaps_the_adapter_tree",
            "killed_cli_owners_reap_their_tree_and_preserve_another_owner",
            "killed_cli_reaps_the_adapter_tree_with_an_undrained_event_pipe",
            "worker_without_a_live_owner_never_starts_the_adapter",
            "missing_adapter_preserves_the_public_unavailable_error",
        ]
        .map(|n| format!("agent_ownership::{n}")),
    );
    if cfg!(target_os = "linux") {
        cases.push(
            "agent_ownership::linux_subreaper_cleans_detached_adapter_children_and_grandchildren"
                .into(),
        );
    }
    for case in &cases {
        ensure!(
            log.contains(&format!("test {case} ... ok")),
            "Required lifecycle case missing: {case}"
        );
    }
    let pattern =
        regex::Regex::new(r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored")?;
    let results: Vec<_> = pattern.captures_iter(&log).collect();
    ensure!(
        !results.is_empty() && results.iter().all(|c| &c[1] == "ok" && &c[3] == "0"),
        "Process tests failed or missing"
    );
    util::publish(
        &s.temporary.join("process-report.json"),
        &util::encode(
            &json!({"source_commit":std::env::var("GITHUB_SHA")?,"platform":std::env::var("RUNNER_OS")?,"rustc":process::text(Command::new("rustc").arg("-vV"))?,"configuration":"no-default-features","passed":results.iter().map(|c|c[2].parse::<usize>().unwrap()).sum::<usize>(),"required_cases":cases,"scope":"Actual deterministic CLI/MCP/agent processes; no Skia, model requests or subscription credentials."}),
        )?,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn early_catalog_uses_cargo_outputs_without_reading_or_replacing_the_later_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let session = Session {
            root: temp.path().join("source"),
            target: temp.path().join("target"),
            temporary: temp.path().to_owned(),
            timing_lock: Mutex::new(()),
        };
        fs::create_dir_all(session.root.join("src")).unwrap();
        fs::create_dir_all(session.root.join("tests")).unwrap();
        fs::write(
            session.root.join("Cargo.toml"),
            "[workspace]\n[package]\nname='lazypencil'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        fs::write(
            session.root.join("Cargo.lock"),
            "version = 4\n[[package]]\nname = 'lazypencil'\nversion = '0.1.0'\n",
        )
        .unwrap();
        fs::write(session.root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(
            session.root.join("src/lib.rs"),
            "#[test] fn library_probe() { assert_eq!(10, 10); }\n",
        )
        .unwrap();
        let test = session.root.join("tests/script_scope.rs");
        fs::write(
            &test,
            "#[test] fn generated_browser_probe() { assert_eq!(10, 10); }\n",
        )
        .unwrap();
        let later = session.temporary.join("test-executables.json");
        assert!(!later.exists());
        let first = session
            .compile_catalog(&["script_scope".into()], "early-first", "fixture-source")
            .unwrap();
        first
            .validate(&session.target, "fixture-source", &["script_scope"])
            .unwrap();
        assert!(first.binaries.contains_key("lib"));
        fs::write(
            session.root.join("tests/remaining.rs"),
            "#[test] fn remaining_probe() { assert_eq!(20, 20); }\n",
        )
        .unwrap();
        let complete = session
            .compile_test_catalog(
                &["script_scope".into(), "remaining".into()],
                "remaining-build",
                "fixture-source",
            )
            .unwrap();
        for name in ["lib", "script_scope"] {
            assert_eq!(first.binaries[name].sha256, complete.binaries[name].sha256);
        }
        assert!(complete.binaries.contains_key("remaining"));
        let invocation = |catalog: &Catalog| {
            vec![
                catalog.binaries["script_scope"]
                    .path
                    .to_string_lossy()
                    .into_owned(),
                "generated_browser_probe".into(),
                "--exact".into(),
            ]
        };
        let result = process::output(
            &mut session.command(&invocation(&first)),
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(!result.timed_out && result.status.success());
        assert!(!later.exists());
        fs::write(&later, b"a later stage owns this catalog").unwrap();
        fs::write(
            &test,
            "#[test] fn generated_browser_probe() { assert_eq!(10, 11); }\n",
        )
        .unwrap();
        let edited = session
            .compile_catalog(&["script_scope".into()], "early-edited", "fixture-source")
            .unwrap();
        edited
            .validate(&session.target, "fixture-source", &["script_scope"])
            .unwrap();
        assert_ne!(
            first.binaries["script_scope"].sha256,
            edited.binaries["script_scope"].sha256
        );
        assert!(
            first
                .validate(&session.target, "fixture-source", &["script_scope"])
                .is_err()
        );
        let result = process::output(
            &mut session.command(&invocation(&edited)),
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(!result.timed_out && !result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("generated_browser_probe ... FAILED")
        );
        assert_eq!(fs::read(later).unwrap(), b"a later stage owns this catalog");
    }

    #[test]
    fn reused_test_catalog_rejects_missing_mutated_foreign_and_outside_binaries() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let deps = target.join("debug/deps");
        fs::create_dir_all(&deps).unwrap();
        let path = deps.join("browser-fixture");
        fs::write(&path, b"owned compiled fixture").unwrap();
        let mut catalog = Catalog {
            source_commit: "fixture-source".into(),
            binaries: BTreeMap::from([(
                "browser_readiness".into(),
                Binary {
                    sha256: util::file_digest(&path).unwrap(),
                    path: path.clone(),
                },
            )]),
        };
        let required = ["browser_readiness"];
        catalog
            .validate(&target, "fixture-source", &required)
            .unwrap();
        assert!(
            catalog
                .validate(&target, "other-source", &required)
                .is_err()
        );
        assert!(
            catalog
                .validate(&target, "fixture-source", &["missing"])
                .is_err()
        );
        fs::write(&path, b"modified fixture").unwrap();
        assert!(
            catalog
                .validate(&target, "fixture-source", &required)
                .is_err()
        );
        let outside = temp.path().join("outside");
        fs::write(&outside, b"owned compiled fixture").unwrap();
        catalog.binaries.get_mut("browser_readiness").unwrap().path = outside;
        assert!(
            catalog
                .validate(&target, "fixture-source", &required)
                .is_err()
        );
    }

    #[test]
    fn parallel_timings_preserve_complete_records_and_their_source_identity() {
        let temp = tempfile::tempdir().unwrap();
        let session = Session {
            root: temp.path().join("source"),
            target: temp.path().join("target"),
            temporary: temp.path().to_owned(),
            timing_lock: Mutex::new(()),
        };
        let payload = "日本語🔍".repeat(512);
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let session = &session;
                let payload = &payload;
                scope.spawn(move || {
                    for sample in 0..16 {
                        session
                            .record_timing(
                                &[format!("{worker}:{sample}"), payload.clone()],
                                Duration::from_millis(10),
                                true,
                                "parallel.log",
                            )
                            .unwrap();
                    }
                });
            }
        });
        let lines = fs::read_to_string(temp.path().join("ci-timings.jsonl")).unwrap();
        let mut identities = BTreeSet::new();
        for line in lines.lines() {
            let record: Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["command"][1], payload);
            assert_eq!(
                record["source_commit"],
                json!(std::env::var("GITHUB_SHA").ok())
            );
            assert_eq!(record["elapsed_seconds"], 0.01);
            assert_eq!(record["success"], true);
            assert!(identities.insert(record["command"][0].as_str().unwrap().to_owned()));
        }
        assert_eq!(identities.len(), 128);
    }
}
