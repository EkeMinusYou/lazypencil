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

pub mod browser;
pub mod parallel;
pub mod release;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Stage {
    EvidenceBuild,
    Source,
    Notices,
    FocusCore,
    FocusPrepare,
    FocusBuild,
    FocusLint,
    FocusReproduce,
    FocusTest,
    CoreBuild,
    CoreTest,
    CoreLint,
    DefaultBuild,
    Reproduce,
    DefaultTest,
    BrowserSetup,
    BrowserTest,
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
        let mut command = Command::new(&invocation[0]);
        command.args(&invocation[1..]).current_dir(&self.root);
        command
    }
    pub fn execute(
        &self,
        invocation: &[String],
        seconds: u64,
        log: &str,
        append: bool,
    ) -> Result<()> {
        self.execute_command(
            invocation,
            &mut self.command(invocation),
            seconds,
            log,
            append,
        )
    }
    fn execute_command(
        &self,
        invocation: &[String],
        command: &mut Command,
        seconds: u64,
        log: &str,
        append: bool,
    ) -> Result<()> {
        let started = Instant::now();
        let result = process::logged(
            command,
            Duration::from_secs(seconds),
            &self.temporary.join(log),
            append,
        );
        self.record_timing(invocation, started.elapsed(), result.is_ok(), log)?;
        result
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
        let mut args = vec!["test", "--locked"];
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
        self.cargo(&args, 300, log, true)
    }
    pub fn capture(&self, command: &mut Command, seconds: u64, path: &str) -> Result<Vec<u8>> {
        let result = process::output(
            command.current_dir(&self.root),
            Duration::from_secs(seconds),
        )?;
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
        let has = |s: &str| targets.iter().any(|t| t == s);
        let core = ["core", "conversation"].contains(&args.suite.as_str());
        match args.stage {
            Stage::EvidenceBuild => {
                super::cache::prune(&self.root, &self.target)?;
                let compiler = process::text(Command::new("rustc").arg("-vV"))?;
                let host = compiler
                    .lines()
                    .find_map(|line| line.strip_prefix("host: "))
                    .context("Compiler does not report its host")?;
                let map = self.temporary.join("release-link.map");
                let extra = crate::evidence::link_map::rustc_arguments(host, &map)?;
                let mut command = Command::new("cargo");
                command.args([
                    if extra.is_some() { "rustc" } else { "build" },
                    "--locked",
                    "--release",
                    "--bin",
                    "lazypencil",
                    "--message-format=json",
                ]);
                if let Some(extra) = extra {
                    ensure!(!map.exists() && !map.is_symlink(), "Link map must be fresh");
                    // Root-only rustc arguments keep dependency/Skia fingerprints
                    // unchanged. Do not introduce workspace-wide RUSTFLAGS.
                    command.arg("--").args(extra);
                }
                self.capture(&mut command, 3600, "release-cargo.jsonl")?;
                process::checked(
                    Command::new(self.executable("release")).arg("--version"),
                    Duration::from_secs(30),
                )?;
                Ok(())
            }
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
                if ["graphics", "dynamic", "copy"].contains(&args.suite.as_str()) && has("project")
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
                if ["graphics", "dynamic", "copy"].contains(&args.suite.as_str())
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
                let plan = commands::plan(&self.root, &args.suite, &targets)?;
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
                if ["graphics", "dynamic", "copy"].contains(&args.suite.as_str())
                    && has("script_scope")
                {
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
                    for filter in ["script_bindings::", "script_copy::"] {
                        self.test(
                            true,
                            "script_scope",
                            filter,
                            &["--test-threads=1"],
                            "focus-bindings.log",
                        )?;
                    }
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
            Stage::CoreTest => self.cargo(
                &[
                    "test",
                    "--locked",
                    "--no-default-features",
                    "--no-fail-fast",
                    "--",
                    "--test-threads=2",
                ],
                900,
                "lazypencil-tests.log",
                true,
            ),
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
            Stage::DefaultBuild => self.build_catalog(),
            Stage::Reproduce => {
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
                for log in ["tui-reproduction.log", "chromium-script-cancel.log"] {
                    fs::OpenOptions::new()
                        .append(true)
                        .open(self.temporary.join("lazypencil-tests.log"))?
                        .write_all(&fs::read(self.temporary.join(log))?)?;
                }
                Ok(())
            }
            Stage::DefaultTest => self.cargo(
                &[
                    "test",
                    "--locked",
                    "--no-fail-fast",
                    "--",
                    "--test-threads=2",
                    "--skip",
                    "tui::tests::",
                    "--skip",
                    "cancelling_a_running_script_unblocks_the_mcp_document_worker",
                ],
                900,
                "lazypencil-tests.log",
                true,
            ),
            Stage::BrowserSetup => browser::setup(self),
            Stage::BrowserTest => browser::tests(self),
            Stage::Mcp => {
                for test in [
                    "real_mcp_shader_assets_uniform_edits_and_browser_refresh_return_image_content",
                    "local_pages::real_mcp_and_cli_local_pages_declare_edit_pack_and_render_after_source_removal",
                ] {
                    self.test(
                        true,
                        "mcp",
                        test,
                        &["--exact", "--ignored", "--test-threads=1"],
                        "chromium-mcp.log",
                    )?;
                }
                browser::local_page_mcp_images(
                    &self.temporary.join("codec-reference"),
                    &std::env::var("GITHUB_SHA")?,
                )?;
                self.test(
                    true,
                    "agent",
                    "supervised_agent_mcp_renders_script_browser_and_shader_pixels",
                    &["--ignored", "--test-threads=1"],
                    "chromium-supervision.log",
                )
            }
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
    fn build_catalog(&self) -> Result<()> {
        let raw = self.capture(
            Command::new("cargo").args([
                "test",
                "--locked",
                "--no-run",
                "--message-format=json-render-diagnostics",
            ]),
            2700,
            "default-build.jsonl",
        )?;
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
        ensure!(
            ["mcp", "tui", "optional_icons"]
                .iter()
                .all(|s| binaries.contains_key(*s)),
            "Missing parallel test binaries"
        );
        util::publish(
            &self.temporary.join("test-executables.json"),
            &util::encode(&Catalog {
                source_commit: std::env::var("GITHUB_SHA")?,
                binaries,
            })?,
            false,
        )?;
        self.cargo(&["build", "--locked"], 2700, "default-build.log", false)
    }
    fn long_tests(&self) -> Result<()> {
        super::setup::material_fonts(
            &self.root.join("resources/icons/catalogue.json"),
            &self.temporary.join("lazypencil-material-fonts"),
        )?;
        let catalog: Catalog = util::json(&self.temporary.join("test-executables.json"))?;
        ensure!(
            catalog.source_commit == std::env::var("GITHUB_SHA")?,
            "Test executables belong to another commit"
        );
        for binary in catalog.binaries.values() {
            ensure!(
                binary.path.starts_with(self.target.join("debug/deps"))
                    && util::file_digest(&binary.path)? == binary.sha256,
                "Test executable changed after compilation"
            );
        }
        // Only compilation overlaps the two test lanes. They execute immutable
        // debug binaries, so no test waits for Cargo's release build lock.
        // Joining every lane keeps subsequent smoke tests and measurements isolated.
        parallel::run(vec![
            (
                "release compilation",
                Box::new(|| release::compile(self, "lazypencil-release.log")),
            ),
            (
                "process lifecycle",
                Box::new(|| -> Result<()> {
                    for (target, filter, log) in [
                        (
                            "mcp",
                            "idle_mcp_reaps_script_page_and_shader_processes_without_another_request",
                            "chromium-mcp-idle.log",
                        ),
                        ("tui", "", "chromium-tui.log"),
                    ] {
                        let mut args =
                            vec![catalog.binaries[target].path.to_string_lossy().into_owned()];
                        if !filter.is_empty() {
                            args.push(filter.into());
                        }
                        args.extend(["--ignored".into(), "--test-threads=1".into()]);
                        self.execute(&args, 300, log, false)?;
                    }
                    Ok(())
                }),
            ),
            (
                "Material verification",
                Box::new(|| -> Result<()> {
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
                        self.execute_command(&args, &mut command, 3600, log, false)?;
                    }
                    Ok(())
                }),
            ),
        ])
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
