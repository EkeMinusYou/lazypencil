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

fn sampled_cpu(text: &str) -> Result<f64> {
    let total = regex::Regex::new(r"(?m)^\s*([0-9]+\.[0-9]+)\s+([0-9]+\.[0-9]+)\s+<Total>\s*$")?;
    let totals = total.captures_iter(text).collect::<Vec<_>>();
    ensure!(totals.len() == 1, "Missing or repeated sampled CPU total");
    let seconds: f64 = totals[0][1].parse()?;
    ensure!(
        seconds.is_finite() && seconds > 0.0,
        "Profiler collected no CPU samples"
    );
    Ok(seconds)
}

fn render_payload<'a>(bytes: &'a [u8], experiment: &Path) -> Result<&'a [u8]> {
    let text = std::str::from_utf8(bytes)?;
    let payload = if text.starts_with("Creating experiment directory ") {
        let (notice, payload) = text
            .split_once('\n')
            .context("Incomplete collector notice")?;
        let prefix = format!(
            "Creating experiment directory {} (Process ID: ",
            experiment.display()
        );
        let pid = notice
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(") ..."))
            .context("Unexpected collector notice")?;
        ensure!(
            pid.parse::<u32>().is_ok_and(|pid| pid > 0),
            "Invalid collector PID"
        );
        payload
    } else {
        text
    };
    ensure!(
        payload.trim_start().starts_with('{'),
        "Missing native render JSON"
    );
    Ok(payload.as_bytes())
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
        let version = process::text(Command::new("gprofng").arg("--version"))?;
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
                let mut experiments = Vec::new();
                for index in 0..3 {
                    fs::remove_file(&png)?;
                    let experiment = output.join(format!("{label}-sample-{index}.er"));
                    let raw = capture(
                        render(Path::new("gprofng"))
                            // Keep default resource sampling: gprofng 2.42 uses
                            // that initialization path to prime its shared timer.
                            .args(["collect", "app", "-p", "hi", "-a", "off", "-o"])
                            .arg(&experiment)
                            .arg(&binary)
                            .args(&arguments),
                        &output.join(format!("{label}-sample-{index}.json")),
                        30,
                    )?;
                    ensure!(
                        validate(render_payload(&raw, &experiment)?)? == expected,
                        "Profiling changed rendered PNG bytes"
                    );
                    experiments.push(experiment);
                }
                capture(
                    Command::new("gprofng")
                        .env("LC_ALL", "C")
                        .args(["display", "text", "-header"])
                        .args(&experiments),
                    &output.join(format!("{label}-header.txt")),
                    60,
                )?;
                let functions = capture(
                    Command::new("gprofng")
                        .env("LC_ALL", "C")
                        .args([
                            "display",
                            "text",
                            "-metrics",
                            "e.totalcpu:i.totalcpu:name",
                            "-sort",
                            "e.totalcpu",
                            "-limit",
                            "80",
                            "-functions",
                        ])
                        .args(&experiments),
                    &output.join(format!("{label}-functions.txt")),
                    60,
                )?;
                let cpu = sampled_cpu(std::str::from_utf8(&functions)?);
                capture(
                    Command::new("gprofng")
                        .env("LC_ALL", "C")
                        .args(["display", "text", "-limit", "300", "-calltree"])
                        .args(&experiments),
                    &output.join(format!("{label}-calltree.txt")),
                    60,
                )?;
                fs::copy(&png, output.join(format!("{label}.png")))?;
                let bytes = util::walk(&output)?
                    .iter()
                    .try_fold(0_u64, |n, p| -> Result<u64> {
                        Ok(n + fs::metadata(p)?.len())
                    })?;
                ensure!(bytes <= 128 * 1024 * 1024, "Oversized profiler experiments");
                scenes.push(json!({"name":name,"scale":scale,"document_sha256":util::file_digest(&document)?,"png_sha256":expected,"profiled_exports":3,"sampled_cpu_seconds":cpu.as_ref().ok().copied(),"sampling_error":cpu.err().map(|e|e.to_string()),"header":format!("{label}-header.txt"),"functions":format!("{label}-functions.txt"),"calltree":format!("{label}-calltree.txt")}));
            }
        }
        ensure!(
            util::file_digest(&binary)? == self.binary_sha256,
            "Profiled executable changed"
        );
        let valid_samples = scenes.iter().all(|s| {
            s["sampled_cpu_seconds"]
                .as_f64()
                .is_some_and(|cpu| cpu > 0.0)
        });
        let report = json!({"format_version":1,"source_run":self.run,"source_revision":self.revision,"collector_revision":std::env::var("GITHUB_SHA").ok(),"candidate":identity,"source":source_identity,"profiler":version.trim(),"requested_sampling_interval_ms":1,"valid_cpu_samples":valid_samples,"binary_archiving":false,"scenes":scenes,"scope":"Native CLI load/layout/render/PNG CPU samples at two scales; not TUI latency, controlled cold starts or complete performance acceptance. Optimized symbols and call stacks may be incomplete."});
        util::publish(&output.join("report.json"), &util::encode(&report)?, false)?;
        ensure!(
            valid_samples,
            "CPU samples missing; inspect retained experiment headers and raw data"
        );
        util::print(&json!({"cpu_profile":"ok","scenes":4,"profiled_exports":12}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collector_notice_is_separated_without_accepting_arbitrary_stdout() {
        let path = Path::new("/owned/sample.er");
        let raw = b"Creating experiment directory /owned/sample.er (Process ID: 2837) ...\n{\"width\":960}\n";
        assert_eq!(render_payload(raw, path).unwrap(), b"{\"width\":960}\n");
        assert_eq!(render_payload(b"{}\n", path).unwrap(), b"{}\n");
        for bad in [
            "Creating experiment directory /another/sample.er (Process ID: 2837) ...\n{}",
            "Creating experiment directory /owned/sample.er (Process ID: invalid) ...\n{}",
            "Creating experiment directory ",
            "unexpected output\n{}",
        ] {
            assert!(render_payload(bad.as_bytes(), path).is_err());
        }
    }
    #[test]
    fn cpu_samples_require_one_positive_total() {
        assert_eq!(
            sampled_cpu("Excl. Incl. Name\n 0.325 0.325 <Total>\n 0.2 0.3 SkCanvas::draw\n")
                .unwrap(),
            0.325
        );
        for text in [
            "",
            "0.000 0.000 <Total>\n",
            "0.1 0.1 <Total>\n0.2 0.2 <Total>\n",
            "NaN NaN <Total>\n",
        ] {
            assert!(sampled_cpu(text).is_err());
        }
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
