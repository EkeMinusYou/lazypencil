//! Sample an owned process tree, including reparented members of its session group.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// A separate activity observation, never enabled in the latency trials.
/// A sampling interval is not a guarantee of observing transient kernel peaks.
pub struct ActiveTrace {
    root: u32,
    started: Instant,
    next: Instant,
    points: Vec<ActivePoint>,
}

#[derive(Serialize)]
struct ActivePoint {
    elapsed_ms: f64,
    observation_ms: f64,
    snapshot: Snapshot,
}

impl ActiveTrace {
    const INTERVAL: Duration = Duration::from_millis(50);

    pub fn new(root: u32) -> Result<Self> {
        ensure!(root > 0, "Invalid activity root");
        let now = Instant::now();
        let mut trace = Self {
            root,
            started: now,
            next: now,
            points: Vec::new(),
        };
        trace.observe()?;
        Ok(trace)
    }

    fn observe(&mut self) -> Result<()> {
        ensure!(
            self.points.len() < 16384,
            "Activity observation exceeded its sample bound"
        );
        let before = Instant::now();
        let snapshot = snapshot(self.root)?;
        self.points.push(ActivePoint {
            elapsed_ms: before.duration_since(self.started).as_secs_f64() * 1000.0,
            observation_ms: before.elapsed().as_secs_f64() * 1000.0,
            snapshot,
        });
        self.next = Instant::now() + Self::INTERVAL;
        Ok(())
    }

    pub fn observe_if_due(&mut self) -> Result<()> {
        if Instant::now() >= self.next {
            self.observe()?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<serde_json::Value> {
        self.observe()?;
        let maximum_rss = self
            .points
            .iter()
            .map(|point| point.snapshot.rss_bytes())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .context("No activity observations")?;
        let maximum_gap = self
            .points
            .windows(2)
            .map(|points| points[1].elapsed_ms - points[0].elapsed_ms)
            .fold(0.0, f64::max);
        let maximum_processes = self
            .points
            .iter()
            .map(|point| point.snapshot.processes.len())
            .max()
            .unwrap();
        Ok(serde_json::json!({
            "scope":"Separate native TUI startup and four complete pan/zoom operations; no agent request",
            "root_pid":self.root, "periodic_sampling_interval_ms":Self::INTERVAL.as_millis(),
            "actual_maximum_sampling_gap_ms":maximum_gap,
            "maximum_observed_process_tree_rss_bytes":maximum_rss,
            "maximum_observed_processes":maximum_processes,
            "transient_kernel_peak_established":false,
            "included_in_latency_distributions":false,
            "points":self.points
        }))
    }
}
#[cfg(target_os = "linux")]
use std::{fs, io::Read};

#[derive(Clone, Debug, Serialize)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub group: u32,
    pub start_ticks: Option<u64>,
    pub rss_bytes: u64,
    pub cpu_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub processes: BTreeMap<u32, Process>,
    /// Per-process delta uncertainty; Linux accounts for both utime and stime.
    pub cpu_quantum_ms: f64,
    pub method: &'static str,
}

pub fn linux_stat(text: &str, page_bytes: u64, ticks_per_second: u64) -> Result<Process> {
    ensure!(
        page_bytes > 0 && ticks_per_second > 0,
        "Invalid kernel units"
    );
    let (pid, rest) = text.split_once(" (").context("Missing process name")?;
    // The command name can itself contain spaces, parentheses and newlines.
    let (_, fields) = rest.rsplit_once(") ").context("Incomplete process name")?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    ensure!(fields.len() >= 22, "Incomplete process counters");
    let number = |index: usize| -> Result<u64> { Ok(fields[index].parse()?) };
    let cpu = number(11)?
        .checked_add(number(12)?)
        .context("CPU counter overflow")?;
    Ok(Process {
        pid: pid.parse()?,
        parent: fields[1].parse()?,
        group: fields[2].parse()?,
        start_ticks: Some(number(19)?),
        rss_bytes: number(21)?
            .checked_mul(page_bytes)
            .context("RSS overflow")?,
        cpu_ms: cpu as f64 * 1000.0 / ticks_per_second as f64,
    })
}

pub fn cpu_time_ms(text: &str) -> Result<f64> {
    let (days, time) = match text.split_once('-') {
        Some((days, time)) => (days.parse::<u64>()?, time),
        None => (0, text),
    };
    let parts = time.split(':').collect::<Vec<_>>();
    ensure!((2..=3).contains(&parts.len()), "Invalid ps CPU clock");
    let seconds = parts[parts.len() - 1].parse::<f64>()?;
    let minutes = parts[parts.len() - 2].parse::<u64>()?;
    let hours = if parts.len() == 3 {
        parts[0].parse::<u64>()?
    } else {
        0
    };
    ensure!(
        seconds.is_finite() && (0.0..60.0).contains(&seconds),
        "Invalid CPU seconds"
    );
    ensure!(parts.len() == 2 || minutes < 60, "Invalid CPU minutes");
    let millis =
        ((days as f64 * 24.0 + hours as f64) * 3600.0 + minutes as f64 * 60.0 + seconds) * 1000.0;
    ensure!(millis.is_finite(), "CPU clock overflow");
    Ok(millis)
}

pub fn ps_table(text: &str) -> Result<Vec<Process>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            ensure!(fields.len() == 5, "Unexpected ps resource columns");
            Ok(Process {
                pid: fields[0].parse()?,
                parent: fields[1].parse()?,
                group: fields[2].parse()?,
                start_ticks: None,
                rss_bytes: fields[3]
                    .parse::<u64>()?
                    .checked_mul(1024)
                    .context("RSS overflow")?,
                cpu_ms: cpu_time_ms(fields[4])?,
            })
        })
        .collect()
}

pub fn owned(
    root: u32,
    processes: Vec<Process>,
    quantum: f64,
    method: &'static str,
) -> Result<Snapshot> {
    ensure!(
        root > 0 && quantum.is_finite() && quantum > 0.0,
        "Invalid resource identity or resolution"
    );
    let mut all = BTreeMap::new();
    for process in processes {
        if process.pid == 0 {
            continue;
        }
        ensure!(
            process.cpu_ms.is_finite() && process.cpu_ms >= 0.0,
            "Invalid process counter"
        );
        ensure!(
            all.insert(process.pid, process).is_none(),
            "Duplicate process identity"
        );
    }
    ensure!(
        all.contains_key(&root),
        "Owned process is no longer observable"
    );
    let mut selected = BTreeMap::new();
    loop {
        let previous = selected.len();
        for (&pid, process) in &all {
            if pid == root || process.group == root || selected.contains_key(&process.parent) {
                selected.insert(pid, process.clone());
            }
        }
        if selected.len() == previous {
            break;
        }
    }
    Ok(Snapshot {
        processes: selected,
        cpu_quantum_ms: quantum,
        method,
    })
}

impl Snapshot {
    pub fn rss_bytes(&self) -> Result<u64> {
        self.processes.values().try_fold(0u64, |sum, process| {
            sum.checked_add(process.rss_bytes)
                .context("Process-tree RSS overflow")
        })
    }

    /// Bound clock quantization rather than rounding an unobservable delta down to zero.
    pub fn idle_cpu_upper_percent(&self, after: &Self, elapsed: Duration) -> Result<f64> {
        ensure!(
            !elapsed.is_zero()
                && self.method == after.method
                && self.cpu_quantum_ms == after.cpu_quantum_ms,
            "Changed resource clock or empty interval"
        );
        ensure!(
            self.processes.keys().eq(after.processes.keys()),
            "Process membership changed during idle measurement"
        );
        let mut cpu_ms = 0.0;
        for (&pid, before) in &self.processes {
            let after = &after.processes[&pid];
            ensure!(
                before.start_ticks == after.start_ticks
                    && before.parent == after.parent
                    && before.group == after.group,
                "Process identity changed during idle measurement"
            );
            ensure!(after.cpu_ms >= before.cpu_ms, "CPU counter went backwards");
            cpu_ms += after.cpu_ms - before.cpu_ms + self.cpu_quantum_ms;
        }
        Ok(cpu_ms / elapsed.as_secs_f64() / 10.0)
    }
}

pub fn snapshot(root: u32) -> Result<Snapshot> {
    #[cfg(target_os = "linux")]
    {
        let pages = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        ensure!(pages > 0 && ticks > 0, "Kernel resource units unavailable");
        let mut processes = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            // Other processes can disappear while /proc is enumerated. The root
            // remains mandatory, and idle comparisons require stable membership.
            if let Ok(file) = fs::File::open(entry.path().join("stat")) {
                let mut bytes = Vec::new();
                if file.take(65537).read_to_end(&mut bytes).is_err() {
                    continue;
                }
                ensure!(bytes.len() <= 65536, "Oversized kernel process record");
                processes.push(linux_stat(
                    std::str::from_utf8(&bytes)?,
                    pages as u64,
                    ticks as u64,
                )?);
            }
        }
        owned(
            root,
            processes,
            2000.0 / ticks as f64,
            "Linux /proc stat; process-tree resident pages and CPU ticks",
        )
    }
    #[cfg(target_os = "macos")]
    {
        let output = crate::process::checked(
            Command::new("ps")
                .args(["-A", "-o", "pid=,ppid=,pgid=,rss=,time="])
                .env("LC_ALL", "C"),
            Duration::from_secs(10),
        )?;
        owned(
            root,
            ps_table(std::str::from_utf8(&output)?)?,
            10.0,
            "macOS ps; process-tree RSS KiB and CPU centiseconds",
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    anyhow::bail!("Resource sampling is unavailable for this target")
}
