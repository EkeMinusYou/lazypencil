//! Run the complete default suite without holding Cargo's release build lock.
use super::*;

const TEST_SECONDS: u64 = 900;
const INDEPENDENT: [&str; 3] = ["script_scope", "graphics", "mesh"];

pub fn run(s: &Session) -> Result<()> {
    let prepared = prepare(s)?;
    remaining(s, &prepared, true)
}

pub(super) struct Prepared {
    checks: Vec<commands::Invocation>,
    scripts: Vec<commands::Invocation>,
    deadline: Instant,
}

pub(super) fn prepare(s: &Session) -> Result<Prepared> {
    let catalog = s.verified_catalog(&[])?;
    let binaries = catalog
        .binaries
        .iter()
        .map(|(name, binary)| (name.clone(), binary.path.clone()))
        .collect();
    let mut checks = commands::default_tests(&s.root, &binaries)?;
    let tui = catalog
        .binaries
        .get("tui")
        .context("Missing TUI test executable")?;
    let terminal = checks
        .iter()
        .position(|args| args.first().is_some_and(|path| Path::new(path) == tui.path))
        .context("Missing complete TUI test invocation")?;
    let terminal_reproduction = checks.remove(terminal);
    let independent = INDEPENDENT
        .map(|name| {
            catalog
                .binaries
                .get(name)
                .map(|binary| binary.path.as_path())
                .with_context(|| format!("Missing {name} test executable"))
        })
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let (scripts, checks) = split_checks(checks, &independent)?;
    let deadline = Instant::now() + Duration::from_secs(TEST_SECONDS);
    let management = s.execute(
        &commands::management_tests(true),
        TEST_SECONDS,
        "lazypencil-tests.log",
        true,
    );
    // Rustdoc needs Cargo too. Run it before starting the independent release
    // lane, and retain the original shared 15-minute test budget.
    let doctests = s.execute(
        &commands::default_doctests(),
        TEST_SECONDS,
        "lazypencil-tests.log",
        true,
    );
    // The TUI target deliberately changes a two-second script deadline.
    // Run its unchanged complete invocation before CPU-heavy release
    // compilation, then remove it from the later parallel lane.
    // Its elapsed time still consumes the original shared test budget.
    let terminal = run_lane(
        s,
        &[terminal_reproduction],
        deadline,
        "lazypencil-tests.log",
    );
    let failures = [
        ("shared cleanup worker tests", management),
        ("default doctests", doctests),
        ("terminal script policy reproduction", terminal),
    ]
    .into_iter()
    .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error:#}")))
    .collect::<Vec<_>>();
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(Prepared {
        checks,
        scripts,
        deadline,
    })
}

pub(super) fn remaining(s: &Session, prepared: &Prepared, compile_release: bool) -> Result<()> {
    let Prepared {
        checks,
        scripts,
        deadline,
    } = prepared;
    let mut tasks: Vec<parallel::Task<'_>> = vec![
        (
            "default tests",
            Box::new(|| run_lane(s, checks, *deadline, "lazypencil-tests.log")),
        ),
        (
            "script-scope and native pixel tests",
            Box::new(|| run_lane(s, scripts, *deadline, "lazypencil-script-scope.log")),
        ),
    ];
    if compile_release {
        tasks.push((
            "release compilation",
            Box::new(|| release::compile(s, "lazypencil-release.log")),
        ));
    }
    let remaining = parallel::run(tasks);
    remaining.context("Remaining default tests and release compilation failed")
}

fn split_checks(
    checks: Vec<commands::Invocation>,
    independent: &[&Path],
) -> Result<(Vec<commands::Invocation>, Vec<commands::Invocation>)> {
    // These targets own temporary documents and distinct retained image names.
    // Fill the existing script lane after its long first target; retain exactly
    // two test processes and their original per-process thread limits.
    let mut checks = checks;
    let mut scripts = Vec::new();
    for path in independent {
        let positions = checks
            .iter()
            .enumerate()
            .filter_map(|(index, args)| {
                args.first()
                    .is_some_and(|name| Path::new(name) == *path)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        ensure!(
            positions.len() == 1,
            "Expected exactly one check for {}",
            path.display()
        );
        scripts.push(checks.remove(positions[0]));
    }
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
        s.execute_command(&mut command, seconds, log, true)
    })
}

pub(super) fn run_checks(
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
    fn balanced_lanes_preserve_commands_and_reject_missing_or_repeated_targets() {
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
        let graphics = root.join("verified-graphics-test");
        let mesh = root.join("verified-mesh-test");
        let native = [named("verified-graphics-test"), named("verified-mesh-test")];
        all.extend(native.clone());
        let paths = [script.as_path(), graphics.as_path(), mesh.as_path()];
        let (scripts, rest) = split_checks(all, &paths).unwrap();
        assert_eq!(scripts, [vec![scope.clone()], native.to_vec()].concat());
        assert_eq!(rest, others);
        for index in 0..scripts.len() {
            let mut missing = scripts.clone();
            missing.remove(index);
            missing.extend(others.clone());
            assert!(split_checks(missing, &paths).is_err());
            let mut repeated = scripts.clone();
            repeated.push(scripts[index].clone());
            repeated.extend(others.clone());
            assert!(split_checks(repeated, &paths).is_err());
        }
        assert!(split_checks(rest, &paths).is_err());
        assert!(split_checks(vec![scope.clone(), scope], &paths).is_err());
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
