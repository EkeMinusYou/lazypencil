//! Optional owned-process summaries after the complete idle measurement.
use super::resources::{BoundRoles, Snapshot};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;

pub fn targets(roles: &BoundRoles, snapshot: &Snapshot) -> Result<Vec<(&'static str, u32)>> {
    let mut result: Vec<(&'static str, u32)> = Vec::new();
    for name in ["tui", "mcp"] {
        let expected = roles
            .processes
            .get(name)
            .context("Missing allocation role")?;
        let actual = snapshot
            .processes
            .get(&expected.pid)
            .context("Allocation process is no longer owned")?;
        ensure!(
            expected.pid > 1
                && actual.pid == expected.pid
                && actual.parent == expected.parent
                && actual.group == expected.group
                && actual.start_ticks == expected.start_ticks,
            "Allocation process changed identity"
        );
        ensure!(
            !result.iter().any(|(_, pid)| *pid == expected.pid),
            "Allocation roles alias a process"
        );
        result.push((name, expected.pid));
    }
    Ok(result)
}

pub fn capture(directory: &Path, roles: &BoundRoles) -> Result<Value> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
        "Allocation summaries require an owned GHA runner"
    );
    #[cfg(target_os = "macos")]
    {
        use crate::{process, util};
        use std::{process::Command, time::Duration};
        let root = roles.processes.get("tui").context("Missing owned TUI")?.pid;
        let current = super::resources::snapshot(root)?;
        let targets = targets(roles, &current)?;
        let mut observations = Vec::new();
        for (role, pid) in targets {
            // This is a new bounded diagnostic, not a shorter application wait.
            // The normal 30-second idle counters have already been finalized.
            let output = process::output_limited(
                Command::new("/usr/bin/vmmap").args(["-summary", &pid.to_string()]),
                Duration::from_secs(4),
                4 * 1024 * 1024,
            );
            let observation = match output {
                Ok(output) => {
                    let stdout = format!("allocation-{role}-vmmap.txt");
                    let stderr = format!("allocation-{role}-vmmap.stderr.txt");
                    util::publish(&directory.join(&stdout), &output.stdout, false)?;
                    util::publish(&directory.join(&stderr), &output.stderr, false)?;
                    json!({"role":role,"pid":pid,"status":if output.timed_out {"timed-out"} else if output.status.success() {"ok"} else {"command-failed"},
                        "exit_code":output.status.code(),"stdout":stdout,"stdout_sha256":util::digest(&output.stdout),
                        "stderr":stderr,"stderr_sha256":util::digest(&output.stderr)})
                }
                Err(error) => {
                    json!({"role":role,"pid":pid,"status":"unavailable","error":error.to_string()})
                }
            };
            observations.push(observation);
        }
        Ok(
            json!({"method":"macOS vmmap -summary","after_completed_idle":true,"included_in_idle_cpu_measurement":false,
            "per_command_timeout_seconds":4,"observations":observations,
            "scope":"Read-only allocation diagnostics for the same owned TUI and bound MCP, after original idle statistics. Raw allocator categories are not substituted for RSS accounting or acceptance."}),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (directory, roles);
        Ok(
            json!({"status":"unsupported","scope":"Allocation summaries are currently available only on macOS; original measurements are unchanged."}),
        )
    }
}
