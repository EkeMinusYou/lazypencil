use crate::{process, util};
use anyhow::{Context, Result, ensure};
use clap::Args;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
pub mod allocations;
pub mod profile;
#[cfg(unix)]
pub mod resources;
pub mod tui;

#[derive(Args)]
pub struct Arguments {
    #[arg(long)]
    pub binary: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long, default_value_t = 20)]
    pub samples: usize,
    #[arg(long, default_value_t = 30.0)]
    pub timeout: f64,
}

impl Arguments {
    pub fn run(self) -> Result<()> {
        let report = benchmark(&self.binary, self.samples, self.timeout)?;
        util::publish(&self.output, &util::encode(&report)?, false)?;
        util::print(
            &json!({"native_benchmark":"ok","scenes":2,"repeated_samples_per_scene":self.samples}),
        )
    }
}

pub fn png(path: &Path, report: &Value, width: u32, height: u32) -> Result<Vec<u8>> {
    let bytes = util::read(path, 16 * 1024 * 1024)?;
    ensure!(
        bytes.len() >= 24
            && &bytes[..8] == b"\x89PNG\r\n\x1a\n"
            && u32::from_be_bytes(bytes[16..20].try_into()?) == width
            && u32::from_be_bytes(bytes[20..24].try_into()?) == height,
        "Unexpected PNG geometry"
    );
    ensure!(
        report["png_hash"] == util::digest(&bytes),
        "PNG differs from its render report"
    );
    Ok(bytes)
}

pub fn validate_render(raw: &[u8], output: &Path, asset_count: usize) -> Result<String> {
    validate_render_dimensions(raw, output, asset_count, 1920, 1080)
}

pub fn validate_render_dimensions(
    raw: &[u8],
    output: &Path,
    asset_count: usize,
    width: u32,
    height: u32,
) -> Result<String> {
    let report: Value = serde_json::from_slice(raw)?;
    ensure!(
        report["width"] == width && report["height"] == height && report["node_count"] == 1000,
        "Benchmark node count or dimensions differ"
    );
    ensure!(
        report["diagnostics"] == json!([])
            && report["assets"]
                .as_array()
                .is_some_and(|a| a.len() == asset_count)
            && report["dynamic_artifacts"] == json!([])
            && report["script_executions"] == json!([]),
        "Unexpected benchmark diagnostics, assets or dynamic work"
    );
    Ok(util::digest(png(output, &report, width, height)?))
}

pub fn measure(command: &mut Command, timeout: f64) -> Result<(Value, Vec<u8>)> {
    ensure!(
        timeout.is_finite() && timeout > 0.0,
        "Expected finite positive measurement deadline"
    );
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?));
    let start = Instant::now();
    let mut child = command.spawn()?;
    let pid = child.id();
    let mut child_finished = false;
    let result = (|| -> Result<Value> {
        loop {
            #[cfg(unix)]
            {
                let mut status = 0;
                let mut resources = std::mem::MaybeUninit::<libc::rusage>::zeroed();
                let reaped = unsafe {
                    libc::wait4(
                        pid as i32,
                        &mut status,
                        libc::WNOHANG,
                        resources.as_mut_ptr(),
                    )
                };
                ensure!(reaped >= 0, "Cannot observe child resource usage");
                if reaped != 0 {
                    child_finished = true;
                    let resources = unsafe { resources.assume_init() };
                    ensure!(
                        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                        "Native measurement child failed: {status}"
                    );
                    let rss = resources.ru_maxrss as u64
                        * if cfg!(target_os = "macos") { 1 } else { 1024 };
                    ensure!(rss > 0, "OS did not supply positive peak RSS");
                    let millis =
                        |t: libc::timeval| t.tv_sec as f64 * 1000.0 + t.tv_usec as f64 / 1000.0;
                    return Ok(
                        json!({"peak_rss_bytes":rss,"user_cpu_ms":millis(resources.ru_utime),"system_cpu_ms":millis(resources.ru_stime),"resource_method":"wait4/ru_maxrss;ru_utime;ru_stime","elapsed_ms":start.elapsed().as_secs_f64()*1000.0}),
                    );
                }
            }
            #[cfg(windows)]
            {
                if let Some(status) = child.try_wait()? {
                    child_finished = true;
                    ensure!(
                        status.success(),
                        "Native measurement child failed: {status}"
                    );
                    return windows_usage(&child, start.elapsed());
                }
            }
            ensure!(
                start.elapsed().as_secs_f64() < timeout,
                "Native measurement child exceeded its deadline"
            );
            ensure!(
                stdout.metadata()?.len() <= 4 * 1024 * 1024
                    && stderr.metadata()?.len() <= 4 * 1024 * 1024,
                "Measurement child exceeded its log limit"
            );
            thread::sleep(Duration::from_millis(1));
        }
    })();
    if result.is_err() && !child_finished {
        let _ = child.kill();
        let _ = child.wait();
    }
    let usage = result?;
    ensure!(
        start.elapsed().as_secs_f64() <= timeout,
        "Native measurement child exceeded its deadline"
    );
    stdout.seek(SeekFrom::Start(0))?;
    stderr.seek(SeekFrom::Start(0))?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.take(4 * 1024 * 1024 + 1).read_to_end(&mut out)?;
    stderr.take(4 * 1024 * 1024 + 1).read_to_end(&mut err)?;
    ensure!(
        out.len() <= 4 * 1024 * 1024 && err.len() <= 4 * 1024 * 1024,
        "Measurement child exceeded its log limit"
    );
    Ok((usage, out))
}

#[cfg(windows)]
fn windows_usage(child: &std::process::Child, elapsed: Duration) -> Result<Value> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::GetProcessTimes,
        },
    };
    let handle = child.as_raw_handle();
    let mut memory = unsafe { std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>() };
    memory.cb = std::mem::size_of_val(&memory) as u32;
    let (mut created, mut exited, mut kernel, mut user) = unsafe {
        (
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
        )
    };
    ensure!(
        unsafe { GetProcessMemoryInfo(handle, &mut memory, memory.cb) } != 0
            && unsafe {
                GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)
            } != 0,
        "Cannot observe Windows resource usage"
    );
    let millis =
        |t: FILETIME| (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) as f64 / 10000.0;
    ensure!(
        memory.PeakWorkingSetSize > 0,
        "OS did not supply positive RSS"
    );
    Ok(
        json!({"peak_rss_bytes":memory.PeakWorkingSetSize,"user_cpu_ms":millis(user),"system_cpu_ms":millis(kernel),"resource_method":"GetProcessMemoryInfo/PeakWorkingSetSize;GetProcessTimes","elapsed_ms":elapsed.as_secs_f64()*1000.0}),
    )
}

pub fn sample_statistics(values: &[f64]) -> Result<Value> {
    ensure!(!values.is_empty(), "Expected repeated samples");
    ensure!(
        values.iter().all(|v| v.is_finite() && *v >= 0.0),
        "Non-finite or negative measurement"
    );
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let median = if n.is_multiple_of(2) {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
    };
    Ok(
        json!({"min":values[0],"median":median,"p95_nearest_rank":values[(n*95).div_ceil(100)-1],"max":values[n-1]}),
    )
}

pub fn summarize(samples: &[Value]) -> Result<Value> {
    let mut result = json!({});
    for field in [
        "elapsed_ms",
        "peak_rss_bytes",
        "user_cpu_ms",
        "system_cpu_ms",
    ] {
        let values = samples
            .iter()
            .map(|s| s[field].as_f64().context("Invalid measurement"))
            .collect::<Result<Vec<_>>>()?;
        result[field] = sample_statistics(&values)?;
    }
    Ok(result)
}

pub fn benchmark(binary: &Path, samples: usize, timeout: f64) -> Result<Value> {
    ensure!(
        (2..=200).contains(&samples),
        "Expected 2..200 repeated samples"
    );
    let binary = binary.canonicalize()?;
    let original = util::file_digest(&binary)?;
    #[cfg(windows)]
    let (system, release) = (
        "Windows".to_owned(),
        process::text(Command::new("cmd").args(["/C", "ver"]))?,
    );
    #[cfg(not(windows))]
    let (system, release) = (
        process::text(Command::new("uname").arg("-s"))?,
        process::text(Command::new("uname").arg("-r"))?,
    );
    let mut report = json!({"format_version":1,"binary_sha256":original,"source_commit":std::env::var("GITHUB_SHA").ok(),
        "environment":{"os":system.trim(),"release":release.trim(),"machine":std::env::consts::ARCH,"cpu_count":thread::available_parallelism().map(|n|n.get()).ok(),"runner_os":std::env::var("RUNNER_OS").ok(),"runner_arch":std::env::var("RUNNER_ARCH").ok(),"runner_image":std::env::var("ImageVersion").ok()},
        "scope":"fresh native CLI child; load/layout/render/encode/write; no process-group or interaction claim","filesystem_cache":"uncontrolled; first invocation is not a cold-cache measurement","completion_poll_seconds":0.001,"warm_samples_per_scene":samples,"scenes":[]});
    let temp = tempfile::tempdir()?;
    let fixtures = util::root().join("tests/fixtures/performance");
    for name in ["shadow", "dashboard"] {
        let scene = temp.path().join(name);
        fs::create_dir(&scene)?;
        fs::write(scene.join("lazypencil.toml"), b"version=1\n")?;
        let document = scene.join("design.pen");
        fs::copy(fixtures.join(format!("{name}.pen")), &document)?;
        let mut assets = json!({});
        if name == "dashboard" {
            let avatar = scene.join("avatar.png");
            fs::copy(fixtures.join("avatar.png"), &avatar)?;
            assets["avatar.png"] = json!(util::file_digest(&avatar)?);
        }
        let output = scene.join("render.png");
        let mut results = Vec::new();
        let mut png_hash = None;
        for _ in 0..=samples {
            if output.exists() {
                fs::remove_file(&output)?;
            }
            let (sample, raw) = measure(
                Command::new(&binary)
                    .current_dir(&scene)
                    .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("user"))
                    .arg("--data-dir")
                    .arg(temp.path().join("data"))
                    .args(["--json", "render"])
                    .arg(&document)
                    .arg("--output")
                    .arg(&output),
                timeout,
            )?;
            let actual = validate_render(&raw, &output, usize::from(name == "dashboard"))?;
            ensure!(
                png_hash.as_ref().is_none_or(|hash| hash == &actual),
                "Repeated exports produced different pixels"
            );
            png_hash = Some(actual);
            results.push(sample);
        }
        let summary = summarize(&results[1..])?;
        report["scenes"].as_array_mut().unwrap().push(json!({"name":name,"document_sha256":util::file_digest(&document)?,"width":1920,"height":1080,"expanded_nodes":1000,"diagnostics":0,"dynamic_artifacts":0,"script_executions":0,"assets":assets,"png_sha256":png_hash,"first_invocation":results[0],"repeated":results[1..],"summary":summary}));
    }
    ensure!(
        util::file_digest(&binary)? == original,
        "Measured executable changed"
    );
    Ok(report)
}
