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
    let deadline = Instant::now() + Duration::from_secs(TEST_SECONDS);
    // Rustdoc needs Cargo too. Run it before starting the independent release
    // lane, and retain the original shared 15-minute test budget.
    s.execute(
        &commands::default_doctests(),
        TEST_SECONDS,
        "lazypencil-tests.log",
        true,
    )?;
    let tests = || {
        run_checks(&checks, deadline, |args, seconds| {
            let mut command = s.command(args);
            command
                .env("CARGO_MANIFEST_DIR", &s.root)
                .env("CARGO_MANIFEST_PATH", s.root.join("Cargo.toml"));
            s.execute_command(args, &mut command, seconds, "lazypencil-tests.log", true)
        })
    };
    parallel::run(vec![
        (
            "release compilation",
            Box::new(|| release::compile(s, "lazypencil-release.log")),
        ),
        ("default tests", Box::new(tests)),
    ])
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
