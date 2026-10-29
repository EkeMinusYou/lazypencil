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
    normal::run_checks(
        &[commands::management_tests(true)],
        Instant::now() + Duration::from_secs(900),
        |args, seconds| s.execute(args, seconds, "focus-management.log", true),
    )
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
