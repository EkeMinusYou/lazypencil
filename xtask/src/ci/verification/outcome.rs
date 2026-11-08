//! One completion policy supplies both the gate and its human-readable evidence.
use crate::util;
use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{fs::OpenOptions, io::Write, path::Path};

const JOBS: [&str; 6] = [
    "plan",
    "audit",
    "shader-preprocessor",
    "native",
    "targeted",
    "source",
];

#[derive(Clone, Copy)]
enum Requirement {
    Success,
    Skipped,
    Component,
}
impl Requirement {
    fn for_job(mode: &str, job: &str) -> Self {
        if mode == "full" && job == "shader-preprocessor" {
            return Self::Component;
        }
        if job == "plan"
            || mode == "full" && ["audit", "native", "source"].contains(&job)
            || mode == "tools" && job == "audit"
            || mode == "focused" && job == "targeted"
        {
            Self::Success
        } else {
            Self::Skipped
        }
    }
    fn accepts(self, status: &str) -> bool {
        match self {
            Self::Success => status == "success",
            Self::Skipped => status == "skipped",
            Self::Component => matches!(status, "success" | "skipped"),
        }
    }
    fn expected(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Skipped => "skipped",
            Self::Component => "success or skipped",
        }
    }
    fn reason(self) -> &'static str {
        match self {
            Self::Success => "Required by the selected verification contract",
            Self::Skipped => {
                "Not selected by this contract; no verification claim is made for this job"
            }
            Self::Component => {
                "Rebuilt when its inputs change; otherwise the plan verifies the checked-in component"
            }
        }
    }
}

#[derive(Serialize)]
struct Job<'a> {
    name: &'static str,
    actual: &'a str,
    expected: &'static str,
    accepted: bool,
    reason: &'static str,
}
#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    verification: &'a str,
    selection_reason: &'a str,
    result: &'static str,
    complete_source: bool,
    jobs: Vec<Job<'a>>,
    failures: Vec<String>,
}
impl<'a> Report<'a> {
    fn assess(mode: &'a str, reason: &'a str, results: &'a Value) -> Self {
        let mut failures = Vec::new();
        let known = ["docs", "tools", "focused", "full"].contains(&mode);
        if !known {
            failures.push("Unknown or missing verification contract; inspect the plan job".into());
        }
        match results.as_object() {
            None => failures.push("Expected a complete object of verification job results".into()),
            Some(map) => {
                for key in map.keys().filter(|key| !JOBS.contains(&key.as_str())) {
                    failures.push(format!("Unexpected verification job: {key}"));
                }
            }
        }
        let jobs = JOBS
            .into_iter()
            .map(|name| {
                let actual = results
                    .get(name)
                    .and_then(|job| job.get("result"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("missing");
                let requirement = Requirement::for_job(mode, name);
                let accepted = known && requirement.accepts(actual);
                if known && !accepted {
                    failures.push(format!(
                        "{name}: expected {}, received {actual}",
                        requirement.expected()
                    ));
                }
                Job {
                    name,
                    actual,
                    expected: if known {
                        requirement.expected()
                    } else {
                        "unknown contract"
                    },
                    accepted,
                    reason: requirement.reason(),
                }
            })
            .collect();
        let success = failures.is_empty();
        Self {
            schema_version: 1,
            verification: mode,
            selection_reason: reason,
            result: if success { "success" } else { "failure" },
            complete_source: success && mode == "full",
            jobs,
            failures,
        }
    }
    fn require_success(&self) -> Result<Value> {
        ensure!(
            self.failures.is_empty(),
            "Verification failed:\n{}\nInspect the listed jobs in this existing run; a skipped required job is not a passing check.",
            self.failures.join("\n")
        );
        Ok(serde_json::to_value(self)?)
    }
    fn markdown(&self) -> String {
        let cell = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('|', "&#124;")
                .replace('`', "&#96;")
                .replace(['\r', '\n'], " ")
        };
        let mut text = format!(
            "## Verification: {}\n\nContract: **{}**\n\nSelection: {}\n\nComplete-source verification: **{}**\n\n| Job | Actual | Required | Assessment | Reason |\n| --- | --- | --- | --- | --- |\n",
            self.result,
            cell(self.verification),
            cell(self.selection_reason),
            self.complete_source
        );
        for job in &self.jobs {
            text.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                job.name,
                cell(job.actual),
                job.expected,
                if job.accepted {
                    "expected"
                } else {
                    "needs attention"
                },
                job.reason
            ));
        }
        if !self.failures.is_empty() {
            text.push_str("\nFailures requiring investigation:\n\n");
            for failure in &self.failures {
                text.push_str(&format!("- {}\n", cell(failure)));
            }
            text.push_str("\nOpen the listed jobs in this existing run to inspect their first failed step. Required failure, cancellation, skip, or missing evidence fails this gate.\n");
        }
        text
    }
}

pub fn gate(mode: &str, results: &Value) -> Result<Value> {
    Report::assess(mode, "Selection reason was not supplied", results).require_success()
}

/// Publish diagnostic evidence before enforcing the unchanged completion policy.
pub fn write_gate_report(
    mode: &str,
    reason: &str,
    results: &Value,
    output: Option<&Path>,
    summary: Option<&Path>,
) -> Result<Value> {
    let report = Report::assess(mode, reason, results);
    if let Some(output) = output {
        util::publish(output, &util::encode(&report)?, true)?;
    }
    if let Some(summary) = summary {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(summary)?
            .write_all(report.markdown().as_bytes())?;
    }
    report.require_success()
}
