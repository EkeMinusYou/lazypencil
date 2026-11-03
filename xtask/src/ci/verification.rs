//! Select the completion contract independently of early reproduction ordering.
use crate::{process, util};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{path::Path, process::Command, time::Duration};

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
        "src/tui/tests.rs",
        "tests/tui.rs",
        "tests/user_config.rs",
        "tests/support/terminal/cases.rs",
        "tests/support/terminal/mod.rs",
    ]
    .contains(&path)
        || path.starts_with("src/config/keys/")
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

/// PRs verify their merge result against the PR base; main retains the verified baseline.
pub fn changed_paths(root: &Path, event_name: &str) -> Result<Vec<String>> {
    if event_name != "pull_request" {
        return super::focus::changed_paths(root);
    }
    let changes = (|| -> Result<String> {
        let event: Value = util::json(Path::new(&std::env::var("GITHUB_EVENT_PATH")?))?;
        let base = event["pull_request"]["base"]["sha"]
            .as_str()
            .unwrap_or_default();
        ensure!(
            base.len() == 40 && base.bytes().all(|b| b.is_ascii_hexdigit()),
            "Missing PR base identity"
        );
        let base = process::text(Command::new("git").current_dir(root).args([
            "merge-base",
            base,
            "HEAD",
        ]))?;
        process::text(Command::new("git").current_dir(root).args([
            "diff",
            "--name-only",
            "-z",
            base.trim(),
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
    let base = event["pull_request"]["base"]["sha"]
        .as_str()
        .unwrap_or_default();
    ensure!(
        base.len() == 40 && base.bytes().all(|b| b.is_ascii_hexdigit()),
        "Missing PR base identity"
    );
    process::checked(
        Command::new("git")
            .current_dir(root)
            .args(["diff", "--check", base, "HEAD"]),
        Duration::from_secs(30),
    )?;
    Ok(())
}

pub fn gate(mode: &str, results: &Value) -> Result<Value> {
    ensure!(
        ["docs", "tools", "focused", "full"].contains(&mode),
        "Unknown verification mode"
    );
    let jobs = [
        "plan",
        "audit",
        "shader-preprocessor",
        "native",
        "targeted",
        "source",
    ];
    let map = results
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Expected job results"))?;
    ensure!(
        map.len() == jobs.len() && jobs.iter().all(|k| map.contains_key(*k)),
        "Incomplete verification job results"
    );
    for job in jobs {
        let expected = job == "plan"
            || mode == "full" && ["audit", "native", "source"].contains(&job)
            || mode == "tools" && job == "audit"
            || mode == "focused" && job == "targeted";
        let status = results[job]["result"].as_str().unwrap_or_default();
        ensure!(
            if mode == "full" && job == "shader-preprocessor" {
                ["success", "skipped"].contains(&status)
            } else if expected {
                status == "success"
            } else {
                status == "skipped"
            },
            "Unexpected {job} result for {mode}: {status}"
        );
    }
    Ok(json!({"verification":mode,"result":"success","complete_source":mode=="full"}))
}
