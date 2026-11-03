use super::*;

#[test]
fn optimized_test_commands_keep_runtime_filters_checks_and_other_profiles() {
    let words = |s: &[&str]| s.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let original = words(&[
        "cargo",
        "test",
        "--locked",
        "--no-run",
        "--",
        "--ignored",
        "--test-threads=2",
    ]);
    let configured = ci::commands::test_profile(&original, true);
    assert_eq!(&configured[..2], &original[..2]);
    assert_eq!(&configured[8..], &original[2..]);
    assert!(
        configured
            .iter()
            .any(|s| s == "profile.test.package.lazypencil.debug-assertions=true")
    );
    assert!(
        configured
            .iter()
            .any(|s| s == "profile.test.package.lazypencil.overflow-checks=true")
    );
    assert_eq!(ci::commands::test_profile(&original, false), original);
    for args in [
        &["cargo", "build"][..],
        &["cargo", "rustc", "--release"],
        &["cargo", "test", "--release"],
        &["cargo", "test", "--profile=release"],
        &["cargo", "test", "--profile", "custom"],
        &["cargo"],
        &[],
    ] {
        let args = words(args);
        assert_eq!(ci::commands::test_profile(&args, true), args);
    }
}

#[test]
fn native_build_scheduling_preserves_compiler_arguments_and_test_execution() {
    let words = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    for args in [
        words(&["cargo", "build", "--locked"]),
        words(&["cargo", "test", "--locked", "--no-run"]),
        words(&["cargo", "clippy", "--all-targets", "--", "-D", "warnings"]),
        words(&[
            "cargo",
            "rustc",
            "--release",
            "--",
            "-C",
            "link-arg=-Wl,-Map=/tmp/output.map",
        ]),
    ] {
        let scheduled = ci::commands::build_jobs_for(&args, 2);
        assert_eq!(&scheduled[..2], &args[..2]);
        assert_eq!(&scheduled[2..4], &["--jobs", "2"]);
        assert_eq!(&scheduled[4..], &args[2..]);
        assert_eq!(ci::commands::build_jobs_for(&args, 1), args);
        assert_eq!(ci::commands::build_jobs_for(&scheduled, 2), scheduled);
        let mac = ci::commands::build_jobs_for(&args, 3);
        assert_eq!(&mac[..2], &args[..2]);
        assert_eq!(&mac[2..4], &["--jobs", "3"]);
        assert_eq!(&mac[4..], &args[2..]);
        assert_eq!(ci::commands::build_jobs_for(&mac, 3), mac);
    }
    for args in [
        &["cargo", "test", "--locked", "--", "--no-run"][..],
        &["cargo", "test", "--locked", "--", "--test-threads=2"],
        &["cargo", "build", "--jobs=1"],
        &["cargo", "build", "-j1"],
        &["cargo", "build", "-j", "1"],
        &["cargo", "build", "--jobs", "1"],
        &["cargo"],
        &[],
        &["/tmp/cargo", "build"],
    ] {
        let args = words(args);
        assert_eq!(ci::commands::build_jobs_for(&args, 2), args);
        assert_eq!(ci::commands::build_jobs_for(&args, 3), args);
    }
}

#[test]
fn warm_dependencies_put_lints_before_codegen_while_cold_rendering_keeps_its_build_budget() {
    for suite in [
        "core",
        "conversation",
        "graphics",
        "copy",
        "dynamic",
        "shader",
    ] {
        assert!(focus::build_before_lint(suite, ""));
        assert!(!focus::build_before_lint(suite, "native-v2-hosted"));
        assert!(!focus::build_before_lint(
            suite,
            "checkpoint-native-v2-hosted"
        ));
        assert!(!focus::build_before_lint(
            suite,
            "checkpoint-native-v2-hosted-full-402-1"
        ));
        assert_eq!(
            focus::build_before_lint(suite, "core-native-v2-hosted"),
            !["core", "conversation"].contains(&suite)
        );
    }
}
