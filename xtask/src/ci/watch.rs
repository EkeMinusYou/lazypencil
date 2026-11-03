//! Watch a single source-bound run without model-driven polling or redispatch.
use crate::process;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{path::Path, process::Command};

pub fn run(root: &Path, run: u64, head: &str, interval: u64) -> Result<()> {
    ensure!(
        run > 0 && (5..=300).contains(&interval),
        "Expected a run ID and a 5..300 second interval"
    );
    ensure!(
        head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()),
        "Expected a full commit SHA"
    );
    let identity: Value = serde_json::from_str(&process::text(
        Command::new("gh").current_dir(root).args([
            "run",
            "view",
            &run.to_string(),
            "--json",
            "databaseId,headSha",
        ]),
    )?)?;
    ensure!(
        identity["databaseId"].as_u64() == Some(run) && identity["headSha"].as_str() == Some(head),
        "CI run does not match the requested source commit"
    );
    let status = Command::new("gh")
        .current_dir(root)
        .args([
            "run",
            "watch",
            &run.to_string(),
            "--exit-status",
            "--interval",
            &interval.to_string(),
            "--compact",
        ])
        .status()?;
    ensure!(
        status.success(),
        "CI run failed, was cancelled, or could not be observed: {status}"
    );
    Ok(())
}
