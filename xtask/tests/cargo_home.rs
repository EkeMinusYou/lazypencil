use lazypencil_xtask::{notices, process};
use std::{path::PathBuf, process::Command, time::Duration};

#[test]
fn cargo_home_precedence_does_not_require_unused_environment_variables() {
    const CASE: &str = "LAZYPENCIL_TEST_CARGO_HOME_CASE";
    if let Ok(case) = std::env::var(CASE) {
        let explicit = case
            .starts_with("explicit")
            .then(|| PathBuf::from("argument-cargo"));
        let actual = notices::resolve_cargo_home(explicit);
        match case.as_str() {
            "explicit" | "explicit-over-env" => {
                assert_eq!(actual.unwrap(), PathBuf::from("argument-cargo"))
            }
            "cargo-env" => assert_eq!(actual.unwrap(), PathBuf::from("environment-cargo")),
            "home" => assert_eq!(actual.unwrap(), PathBuf::from("user-home/.cargo")),
            "missing" => assert!(
                actual
                    .unwrap_err()
                    .to_string()
                    .contains("CARGO_HOME or HOME is required")
            ),
            _ => panic!("Unknown case"),
        }
        return;
    }
    for case in [
        "explicit",
        "explicit-over-env",
        "cargo-env",
        "home",
        "missing",
    ] {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cargo_home_precedence_does_not_require_unused_environment_variables",
                "--nocapture",
            ])
            .env_remove("HOME")
            .env_remove("CARGO_HOME")
            .env(CASE, case);
        if matches!(case, "explicit-over-env" | "cargo-env") {
            command.env("CARGO_HOME", "environment-cargo");
        }
        if case == "home" {
            command.env("HOME", "user-home");
        }
        process::checked(&mut command, Duration::from_secs(10)).unwrap();
    }
}
