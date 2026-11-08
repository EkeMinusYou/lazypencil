//! Select the completion contract independently of early reproduction ordering.
use crate::{process, util};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{path::Path, process::Command, time::Duration};

mod outcome;
pub use outcome::{gate, write_gate_report};

pub fn documentation(path: &str) -> bool {
    (path.ends_with(".md") && (!path.contains('/') || path.starts_with("docs/")))
        || (path.starts_with("tests/fixtures/") && path.ends_with("/README.md"))
}
fn presentation(path: &str) -> bool {
    [
        "src/tui/mod.rs",
        "src/tui/composer.rs",
        "src/tui/nodes.rs",
        "src/tui/footer.rs",
        "src/tui/footer_tests.rs",
        "src/tui/theme.rs",
        "src/tui/agent_status.rs",
        "src/tui/markdown.rs",
        "src/tui/controls.rs",
        "src/tui/input.rs",
        "src/tui/events.rs",
        "src/tui/outbox.rs",
        "src/tui/references.rs",
        "src/tui/documents.rs",
        "src/tui/documents/view.rs",
        "src/tui/saved.rs",
        "src/tui/checkpoints.rs",
        "src/tui/tests.rs",
        "tests/tui.rs",
        "tests/user_config.rs",
        "tests/support/terminal/cases.rs",
        "tests/support/terminal/mod.rs",
    ]
    .contains(&path)
        || path.starts_with("src/config/keys/")
        || path.starts_with("src/tui/tests/")
}

pub fn select(event: &str, paths: &[String]) -> (&'static str, &'static str) {
    if event != "pull_request" {
        return (
            "full",
            "Integration, manual and reusable runs retain complete verification",
        );
    }
    if paths.iter().all(|p| documentation(p)) {
        return ("docs", "Repository prose has no native compiler consumers");
    }
    let inputs: Vec<_> = paths
        .iter()
        .filter(|p| !documentation(p))
        .map(String::as_str)
        .collect();
    if inputs.iter().all(|p| {
        [
            "xtask/src/dev.rs",
            "xtask/src/ci/watch.rs",
            "xtask/tests/development.rs",
            "xtask/tests/architecture.rs",
        ]
        .contains(p)
            || p.starts_with("xtask/src/dev/")
    }) {
        return (
            "tools",
            "Independent developer tools are checked without native application builds",
        );
    }
    if inputs.iter().all(|p| presentation(p)) {
        return (
            "focused",
            "Presentation and key changes use library, TUI, agent and configuration regressions",
        );
    }
    if super::focus::agent_only(paths) {
        return (
            "focused",
            "ACP and adapter settings use core agent, history, MCP and configuration regressions",
        );
    }
    if super::focus::runtime_only(paths) {
        return (
            "focused",
            "Runtime installation uses its existing core consumer tests",
        );
    }
    if inputs.iter().all(|p| {
        [
            "src/guidance.rs",
            "src/service/guidance.rs",
            "tests/guidance.rs",
        ]
        .contains(p)
            || p.starts_with("resources/guidance/")
    }) {
        return (
            "focused",
            "Embedded guidance uses its existing core consumer tests",
        );
    }
    (
        "full",
        "Shared, native, build, dependency or unclassified changes require the complete graph",
    )
}

pub fn presentation_only(paths: &[String]) -> bool {
    paths
        .iter()
        .filter(|p| !documentation(p))
        .all(|p| presentation(p))
}

fn pr_base(root: &Path, event: &Value) -> Result<String> {
    let valid_hash =
        |value: &str| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    let base = event["pull_request"]["base"]["sha"]
        .as_str()
        .unwrap_or_default();
    ensure!(valid_hash(base), "Missing PR base identity");
    if let Some(head) = event["pull_request"]["head"]["sha"].as_str() {
        ensure!(valid_hash(head), "Invalid PR head identity");
        let commit = process::text(Command::new("git").current_dir(root).args([
            "rev-list",
            "--parents",
            "-n",
            "1",
            "HEAD",
        ]))?;
        let parents: Vec<_> = commit.split_whitespace().collect();
        if parents
            .first()
            .is_none_or(|commit| !commit.eq_ignore_ascii_case(head))
        {
            ensure!(
                parents.len() == 3 && parents[2].eq_ignore_ascii_case(head),
                "Checkout does not match the advertised PR head or its tested merge"
            );
            // GitHub can refresh the merge ref after capturing the event's
            // base SHA. Its first parent is the base actually tested here.
            process::checked(
                Command::new("git").current_dir(root).args([
                    "merge-base",
                    "--is-ancestor",
                    base,
                    parents[1],
                ]),
                Duration::from_secs(30),
            )?;
            return Ok(parents[1].to_owned());
        }
    }
    Ok(process::text(
        Command::new("git")
            .current_dir(root)
            .args(["merge-base", base, "HEAD"]),
    )?
    .trim()
    .to_owned())
}

/// PRs verify against the tested merge's base; main retains the verified baseline.
pub fn changed_paths(root: &Path, event_name: &str) -> Result<Vec<String>> {
    if event_name != "pull_request" {
        return super::focus::changed_paths(root);
    }
    let changes = (|| -> Result<String> {
        let event: Value = util::json(Path::new(&std::env::var("GITHUB_EVENT_PATH")?))?;
        let base = pr_base(root, &event)?;
        process::text(Command::new("git").current_dir(root).args([
            "diff",
            "--name-only",
            "-z",
            &base,
            "HEAD",
        ]))
    })();
    let paths = match changes {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!(
                "Unable to establish the PR diff; requiring complete verification: {error:#}"
            );
            return Ok(vec!["Cargo.toml".into()]);
        }
    };
    Ok(paths
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect())
}

pub fn check_documentation(root: &Path) -> Result<()> {
    let event: Value = util::json(Path::new(&std::env::var("GITHUB_EVENT_PATH")?))?;
    let base = pr_base(root, &event)?;
    process::checked(
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--check", &base, "HEAD"]),
        Duration::from_secs(30),
    )?;
    Ok(())
}
