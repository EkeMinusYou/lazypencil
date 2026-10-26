//! Run the complete default suite without holding Cargo's release build lock.
use super::*;

const TEST_SECONDS: u64 = 900;

pub fn run(s: &Session) -> Result<()> {
    let catalog = s.verified_catalog(&[])?;
    let binaries = catalog
        .binaries
        .iter()
        .map(|(name, binary)| (name.clone(), binary.path.clone()))
        .collect();
    let checks = commands::default_tests(&s.root, &binaries)?;
    let (scripts, checks) = split_checks(
        checks,
        &catalog
            .binaries
            .get("script_scope")
            .context("Missing script-scope test executable")?
            .path,
    )?;
    let deadline = Instant::now() + Duration::from_secs(TEST_SECONDS);
    // Rustdoc needs Cargo too. Run it before starting the independent release
    // lane, and retain the original shared 15-minute test budget.
    s.execute(
        &commands::default_doctests(),
        TEST_SECONDS,
        "lazypencil-tests.log",
        true,
    )?;
    parallel::run(vec![
        (
            "release compilation",
            Box::new(|| release::compile(s, "lazypencil-release.log")),
        ),
        (
            "default tests",
            Box::new(|| run_lane(s, &checks, deadline, "lazypencil-tests.log")),
        ),
        (
            "script-scope tests",
            Box::new(|| run_lane(s, &scripts, deadline, "lazypencil-script-scope.log")),
        ),
    ])
}

fn split_checks(
    checks: Vec<commands::Invocation>,
    script: &Path,
) -> Result<(Vec<commands::Invocation>, Vec<commands::Invocation>)> {
    // This target owns temporary documents/data and exclusive artifact paths.
    // Select its verified catalog path, never an executable-name prefix.
    let (scripts, checks): (Vec<_>, Vec<_>) = checks
        .into_iter()
        .partition(|args| args.first().is_some_and(|path| Path::new(path) == script));
    ensure!(
        scripts.len() == 1,
        "Expected exactly one script-scope check"
    );
    Ok((scripts, checks))
}

fn run_lane(
    s: &Session,
    checks: &[commands::Invocation],
    deadline: Instant,
    log: &str,
) -> Result<()> {
    run_checks(checks, deadline, |args, seconds| {
        let mut command = s.command(args);
        command
            .env("CARGO_MANIFEST_DIR", &s.root)
            .env("CARGO_MANIFEST_PATH", s.root.join("Cargo.toml"));
        s.execute_command(args, &mut command, seconds, log, true)
    })
}

fn run_checks(
    commands: &[commands::Invocation],
    deadline: Instant,
    mut execute: impl FnMut(&[String], u64) -> Result<()>,
) -> Result<()> {
    let mut failures = Vec::new();
    for command in commands {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let result = if remaining.is_zero() {
            Err(anyhow::anyhow!("Default test deadline exceeded"))
        } else {
            execute(
                command,
                remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0),
            )
        };
        if let Err(error) = result {
            failures.push(format!("{}: {error:#}", command[0]));
        }
    }
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_preserves_each_command_and_rejects_missing_or_repeated_script_checks() {
        let root = std::env::temp_dir();
        let script = root.join("verified-script-test");
        let named = |name: &str| vec![root.join(name).to_string_lossy().into_owned()];
        let scope = vec![
            script.to_string_lossy().into_owned(),
            "--test-threads=2".into(),
            "--skip".into(),
            "existing-reproduction".into(),
        ];
        let others = vec![
            named("lib"),
            named("verified-script-test-lookalike"),
            named("tui"),
        ];
        let mut all = others.clone();
        all.insert(1, scope.clone());
        let (scripts, rest) = split_checks(all, &script).unwrap();
        assert_eq!(scripts.as_slice(), std::slice::from_ref(&scope));
        assert_eq!(rest, others);
        assert!(split_checks(rest, &script).is_err());
        assert!(split_checks(vec![scope.clone(), scope], &script).is_err());
    }

    #[test]
    fn default_lane_keeps_no_fail_fast_and_rejects_expired_checks() {
        let commands = vec![vec!["first".into()], vec!["second".into()]];
        let mut seen = Vec::new();
        let result = run_checks(
            &commands,
            Instant::now() + Duration::from_secs(TEST_SECONDS),
            |args, seconds| {
                assert!((1..=TEST_SECONDS).contains(&seconds));
                seen.push(args[0].clone());
                ensure!(args[0] != "first", "Expected first target failure");
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(seen, ["first", "second"]);
        let result = run_checks(
            &commands,
            Instant::now() - Duration::from_secs(1),
            |_, _| panic!("Expired checks must not start another process"),
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("deadline exceeded")
        );
    }
}
