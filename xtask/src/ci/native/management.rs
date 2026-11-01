//! Isolate test cases sharing the process-global bounded cleanup queue.
use super::*;

pub fn core(s: &Session) -> Result<()> {
    // Preserve the original combined 15-minute core-test budget and continue
    // the remaining checks even if the isolated namespace reports a failure.
    normal::run_checks(
        &[commands::management_tests(false), commands::core_tests()],
        Instant::now() + Duration::from_secs(900),
        |args, seconds| s.execute(args, seconds, "lazypencil-tests.log", true),
    )
}

pub fn focus(s: &Session) -> Result<()> {
    // FocusCore runs the core feature check before default compilation. Keep
    // this resource group in its own default-feature harness without switching
    // the published CLI or recompiling the core graph on the default builder.
    // Use the verified library executable while unrelated test targets compile;
    // invoking Cargo here would wait for their shared build lock.
    let invocation = s.test_invocation(
        true,
        "lib",
        commands::MANAGEMENT_REPRODUCTION,
        &["--test-threads=1"],
    )?;
    let mut command = s.command(&invocation);
    command
        .env("CARGO_MANIFEST_DIR", &s.root)
        .env("CARGO_MANIFEST_PATH", s.root.join("Cargo.toml"));
    s.execute_command(&mut command, 900, "focus-management.log", true)
}

pub fn default(s: &Session) -> Result<()> {
    normal::run_checks(
        &[
            commands::management_tests(true),
            commands::default_cargo_tests(),
        ],
        Instant::now() + Duration::from_secs(900),
        |args, seconds| s.execute(args, seconds, "lazypencil-tests.log", true),
    )
}
