//! Sample an existing verified executable; never invoke an application build.
use crate::{ci::setup, dist, process, util};
use anyhow::{Context, Result, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    pub run: u64,
    #[arg(long)]
    pub revision: String,
    #[arg(long)]
    pub archive_sha256: String,
    #[arg(long)]
    pub binary_sha256: String,
    #[arg(long)]
    pub output: std::path::PathBuf,
}

fn sha(value: &str, length: usize) -> Result<()> {
    ensure!(
        value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid checksum/revision"
    );
    Ok(())
}

fn completed_source(run: &Value, repository: &str, revision: &str) -> Result<()> {
    ensure!(
        run["conclusion"] == "success"
            && run["status"] == "completed"
            && run["head_sha"] == revision
            && run["repository"]["full_name"] == repository
            && run["path"] == ".github/workflows/ci.yml",
        "Expected a successful source-bound CI run"
    );
    Ok(())
}

fn sample_count(text: &str) -> Result<usize> {
    let mut count = 0;
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        ensure!(
            line.len() <= 16 && line.bytes().all(|b| b.is_ascii_hexdigit()),
            "Expected one user instruction pointer per CPU sample"
        );
        count += 1;
    }
    ensure!(count > 0, "Profiler collected no CPU samples");
    Ok(count)
}

fn perf_binary(root: &Path, kernel: &str) -> Result<std::path::PathBuf> {
    let matching = root.join("linux-tools").join(kernel).join("perf");
    if matching.is_file() {
        return Ok(matching.canonicalize()?);
    }
    // Ubuntu's wrapper requires an exact kernel package; the generic userspace
    // tool also understands software cpu-clock events on hosted Azure kernels.
    let mut candidates = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("linux-tools-")
        {
            let binary = entry.path().join("perf");
            if binary.is_file() {
                candidates.push(binary.canonicalize()?);
            }
        }
    }
    candidates.sort();
    candidates
        .pop()
        .context("Missing installed perf executable")
}

fn capture(command: &mut Command, output: &Path, seconds: u64) -> Result<Vec<u8>> {
    let result = process::output_limited(command, Duration::from_secs(seconds), 4 * 1024 * 1024)?;
    fs::write(output, &result.stdout)?;
    fs::write(output.with_extension("stderr.log"), &result.stderr)?;
    ensure!(
        !result.timed_out && result.status.success(),
        "Profiling command failed; see {}",
        output.display()
    );
    Ok(result.stdout)
}

impl Arguments {
    pub fn run(self) -> Result<()> {
        ensure!(
            cfg!(all(target_os = "linux", target_arch = "x86_64"))
                && std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
            "CPU sampling requires an owned Linux x64 GHA runner"
        );
        sha(&self.revision, 40)?;
        sha(&self.archive_sha256, 64)?;
        sha(&self.binary_sha256, 64)?;
        let repository = std::env::var("GITHUB_REPOSITORY")?;
        ensure!(
            repository.split('/').count() == 2
                && repository
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b)),
            "Invalid repository"
        );
        fs::create_dir(&self.output)?;
        let output = self.output.canonicalize()?;
        let temporary = tempfile::tempdir()?;
        let metadata: Value = serde_json::from_slice(&process::checked(
            Command::new("gh").args([
                "api",
                &format!("repos/{repository}/actions/runs/{}", self.run),
            ]),
            Duration::from_secs(60),
        )?)?;
        completed_source(&metadata, &repository, &self.revision)?;
        let name = format!("focus-candidate-ubuntu-24.04-{}-attempt-1", self.revision);
        let artifacts: Value = serde_json::from_slice(&process::checked(
            Command::new("gh").args([
                "api",
                &format!(
                    "repos/{repository}/actions/runs/{}/artifacts?per_page=100",
                    self.run
                ),
            ]),
            Duration::from_secs(60),
        )?)?;
        let matches = artifacts["artifacts"]
            .as_array()
            .context("Missing artifacts")?
            .iter()
            .filter(|a| a["name"] == name)
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1
                && matches[0]["expired"] == false
                && matches[0]["size_in_bytes"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= 64 * 1024 * 1024),
            "Missing, expired or oversized candidate"
        );
        let download = temporary.path().join("download");
        capture(
            Command::new("gh")
                .args([
                    "run",
                    "download",
                    &self.run.to_string(),
                    "--repo",
                    &repository,
                    "--name",
                    &name,
                    "--dir",
                ])
                .arg(&download),
            &output.join("download.log"),
            120,
        )?;
        let archive = download.join("lazypencil-candidate.tar.gz");
        ensure!(
            util::file_digest(&archive)? == self.archive_sha256,
            "Candidate archive changed"
        );
        let package = temporary.path().join("package");
        let identity = dist::binary::verify(&archive, Some(&package))?;
        ensure!(
            identity["source_revision"] == self.revision
                && identity["binary_sha256"] == self.binary_sha256
                && identity["target"] == "x86_64-unknown-linux-gnu",
            "Candidate identity differs"
        );
        let source = temporary.path().join("source");
        let source_identity = dist::verify(
            &package.join("source/lazypencil-source.tar.gz"),
            Some(&source),
        )?;
        let binary = package.join("bin/lazypencil");
        setup::linux_profile_dependencies()?;
        let kernel = process::text(Command::new("uname").arg("-r"))?;
        let perf = perf_binary(Path::new("/usr/lib"), kernel.trim())?;
        let version = process::text(Command::new(&perf).arg("--version"))?;
        let paranoid: i32 = fs::read_to_string("/proc/sys/kernel/perf_event_paranoid")?
            .trim()
            .parse()?;
        if paranoid > 2 {
            process::checked(
                Command::new("sudo").args(["sysctl", "-w", "kernel.perf_event_paranoid=2"]),
                Duration::from_secs(30),
            )?;
        }
        let mut scenes = Vec::new();
        for name in ["dashboard", "shadow"] {
            for scale in [0.5_f32, 1.0] {
                let label = format!("{name}-{}", (scale * 100.0) as u32);
                let scene = temporary.path().join(&label);
                fs::create_dir(&scene)?;
                fs::write(scene.join("lazypencil.toml"), b"version=1\n")?;
                let fixtures = source.join("tests/fixtures/performance");
                let document = scene.join("design.pen");
                fs::copy(fixtures.join(format!("{name}.pen")), &document)?;
                if name == "dashboard" {
                    fs::copy(fixtures.join("avatar.png"), scene.join("avatar.png"))?;
                }
                let png = scene.join("render.png");
                let render = |program: &Path| {
                    let mut command = Command::new(program);
                    command
                        .current_dir(&scene)
                        .env_remove("GH_TOKEN")
                        .env_remove("GITHUB_TOKEN")
                        .env("LC_ALL", "C")
                        .env("DEBUGINFOD_URLS", "")
                        .env("LAZYPENCIL_CONFIG_DIR", scene.join("user"));
                    command
                };
                let arguments = vec![
                    "--data-dir".into(),
                    scene.join("data").to_string_lossy().into_owned(),
                    "--json".into(),
                    "render".into(),
                    document.to_string_lossy().into_owned(),
                    "--scale".into(),
                    scale.to_string(),
                    "--output".into(),
                    png.to_string_lossy().into_owned(),
                ];
                let raw = capture(
                    render(&binary).args(&arguments),
                    &output.join(format!("{label}-control.json")),
                    30,
                )?;
                let validate = |raw: &[u8]| {
                    super::validate_render_dimensions(
                        raw,
                        &png,
                        usize::from(name == "dashboard"),
                        (1920.0 * scale) as u32,
                        (1080.0 * scale) as u32,
                    )
                };
                let expected = validate(&raw)?;
                let mut samples = Vec::new();
                for index in 0..3 {
                    fs::remove_file(&png)?;
                    let sample = format!("{label}-sample-{index}");
                    let experiment = output.join(format!("{sample}.data"));
                    let raw = capture(
                        render(&perf)
                            .args([
                                "record",
                                "-e",
                                "cpu-clock:u",
                                "-F",
                                "997",
                                "--call-graph",
                                "dwarf,8192",
                                "--no-buildid-cache",
                                "-m",
                                "128",
                                "-o",
                            ])
                            .arg(&experiment)
                            .arg("--")
                            .arg(&binary)
                            .args(&arguments),
                        &output.join(format!("{sample}.json")),
                        30,
                    )?;
                    ensure!(
                        validate(&raw)? == expected,
                        "Profiling changed rendered PNG bytes"
                    );
                    let ips = capture(
                        Command::new(&perf)
                            .env("LC_ALL", "C")
                            .env("DEBUGINFOD_URLS", "")
                            // Lost-event records deliberately fail the IP-only
                            // validator instead of silently biasing the report.
                            .args([
                                "script",
                                "--hide-call-graph",
                                "--show-lost-events",
                                "-F",
                                "ip",
                                "-i",
                            ])
                            .arg(&experiment),
                        &output.join(format!("{sample}-ips.txt")),
                        60,
                    )?;
                    let count = sample_count(std::str::from_utf8(&ips)?);
                    for (kind, options) in [
                        (
                            "functions",
                            [
                                "--no-children",
                                "--call-graph",
                                "none",
                                "--percent-limit",
                                "0.1",
                            ],
                        ),
                        (
                            "calltree",
                            [
                                "--children",
                                "--call-graph",
                                "graph,1,caller",
                                "--percent-limit",
                                "1",
                            ],
                        ),
                    ] {
                        capture(
                            Command::new(&perf)
                                .env("LC_ALL", "C")
                                .env("DEBUGINFOD_URLS", "")
                                .env("PERF_PAGER", "cat")
                                .args(["report", "--stdio", "-n", "--sort", "dso,symbol"])
                                .args(options)
                                .arg("-i")
                                .arg(&experiment),
                            &output.join(format!("{sample}-{kind}.txt")),
                            60,
                        )?;
                    }
                    samples.push(json!({"data":format!("{sample}.data"),"cpu_samples":count.as_ref().ok().copied(),"sampling_error":count.err().map(|e|e.to_string()),"functions":format!("{sample}-functions.txt"),"calltree":format!("{sample}-calltree.txt")}));
                }
                fs::copy(&png, output.join(format!("{label}.png")))?;
                let bytes = util::walk(&output)?
                    .iter()
                    .try_fold(0_u64, |n, p| -> Result<u64> {
                        Ok(n + fs::metadata(p)?.len())
                    })?;
                ensure!(bytes <= 128 * 1024 * 1024, "Oversized profiler experiments");
                scenes.push(json!({"name":name,"scale":scale,"document_sha256":util::file_digest(&document)?,"png_sha256":expected,"profiled_exports":3,"samples":samples}));
            }
        }
        ensure!(
            util::file_digest(&binary)? == self.binary_sha256,
            "Profiled executable changed"
        );
        let valid_samples = scenes.iter().all(|s| {
            s["samples"].as_array().is_some_and(|samples| {
                samples.len() == 3
                    && samples
                        .iter()
                        .all(|p| p["cpu_samples"].as_u64().is_some_and(|n| n > 0))
            })
        });
        let report = json!({"format_version":2,"source_run":self.run,"source_revision":self.revision,"collector_revision":std::env::var("GITHUB_SHA").ok(),"candidate":identity,"source":source_identity,"profiler":version.trim(),"profiler_path":perf,"kernel":kernel.trim(),"initial_perf_event_paranoid":paranoid,"requested_sampling_frequency_hz":997,"valid_cpu_samples":valid_samples,"binary_archiving":false,"scenes":scenes,"scope":"Native CLI user CPU samples covering load/layout/render/PNG at two scales; not TUI latency, controlled cold starts or complete performance acceptance. Optimized symbols and call stacks may be incomplete. Sample counts are not elapsed CPU seconds."});
        util::publish(&output.join("report.json"), &util::encode(&report)?, false)?;
        ensure!(
            valid_samples,
            "CPU samples missing; inspect retained reports and raw data"
        );
        util::print(&json!({"cpu_profile":"ok","scenes":4,"profiled_exports":12}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_samples_require_instruction_pointers() {
        assert_eq!(
            sample_count("  7f001234abcd\n\n  abcdef0123456789 \n").unwrap(),
            2
        );
        for text in [
            "",
            "\n",
            "sample lost\n",
            "fffffffffffffffff\n",
            "7f001234abcd extra\n",
        ] {
            assert!(sample_count(text).is_err());
        }
    }
    #[test]
    fn perf_selection_prefers_kernel_match_then_installed_generic_tool() {
        let root = tempfile::tempdir().unwrap();
        assert!(perf_binary(root.path(), "azure").is_err());
        let generic = root.path().join("linux-tools-6.8-generic/perf");
        fs::create_dir_all(generic.parent().unwrap()).unwrap();
        fs::write(&generic, b"tool").unwrap();
        assert_eq!(
            perf_binary(root.path(), "azure").unwrap(),
            generic.canonicalize().unwrap()
        );
        let exact = root.path().join("linux-tools/azure/perf");
        fs::create_dir_all(exact.parent().unwrap()).unwrap();
        fs::write(&exact, b"matching tool").unwrap();
        assert_eq!(
            perf_binary(root.path(), "azure").unwrap(),
            exact.canonicalize().unwrap()
        );
    }
    #[test]
    fn downloaded_candidate_requires_successful_matching_source_run() {
        let revision = "a".repeat(40);
        let good = json!({"status":"completed","conclusion":"success","head_sha":revision,"repository":{"full_name":"owner/repo"},"path":".github/workflows/ci.yml"});
        completed_source(&good, "owner/repo", &revision).unwrap();
        for (key, value) in [
            ("status", json!("in_progress")),
            ("conclusion", json!("failure")),
            ("head_sha", json!("b".repeat(40))),
            ("path", json!("other.yml")),
        ] {
            let mut changed = good.clone();
            changed[key] = value;
            assert!(completed_source(&changed, "owner/repo", &revision).is_err());
        }
        assert!(completed_source(&good, "another/repo", &revision).is_err());
        assert!(sha("../invalid", 40).is_err());
    }
}
