//! Launch and argument failures must not start document or agent work.
use std::process::{Command, Stdio};

fn command(temp: &tempfile::TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lazypencil"));
    command
        .current_dir(temp.path())
        .env_remove("LAZYPENCIL_COMPLETE")
        .env("LAZYPENCIL_CONFIG_DIR", temp.path().join("configuration"))
        .env("LAZYPENCIL_DATA_DIR", temp.path().join("data"))
        .env("LAZYPENCIL_LOCK_DIR", temp.path().join("locks"))
        .stdin(Stdio::null());
    command
}
#[test]
fn help_and_unknown_commands_do_not_enter_a_picker_or_create_user_data() {
    let temp = tempfile::tempdir().unwrap();
    for flag in ["--help", "--version"] {
        assert!(command(&temp).arg(flag).output().unwrap().status.success());
    }
    for args in [
        vec!["reneder"],
        vec!["design.pen"],
        vec!["mcp", "--file", "design.pen"],
        vec!["tui", "--unknown-option"],
    ] {
        let output = command(&temp).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    assert!(!temp.path().join("data").exists());
    assert!(!temp.path().join("locks").exists());
}
#[cfg(feature = "tui")]
#[test]
fn no_argument_and_explicit_picker_require_a_terminal_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec!["tui"],
        vec!["tui", "--create"],
        vec!["tui", "--session", "some-id"],
        vec!["--json", "tui"],
    ] {
        let output = command(&temp).args(args).output().unwrap();
        assert!(!output.status.success());
        if output.stdout.is_empty() {
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("terminal"), "{error}");
        } else {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
        }
    }
    assert!(!temp.path().join("data").exists());
    assert!(!temp.path().join("locks").exists());
}

#[cfg(feature = "tui")]
#[test]
fn optional_tui_file_keeps_its_position_before_the_optional_prompt() {
    use clap::{CommandFactory, FromArgMatches};
    for (words, expected_file, expected_prompt) in [
        (vec!["tui"], ".", ""),
        (vec!["tui", "design.pen"], "design.pen", ""),
        (
            vec!["tui", "design.pen", "Make it blue"],
            "design.pen",
            "Make it blue",
        ),
        (vec!["tui", "design.pen", "--create"], "design.pen", ""),
    ] {
        let matches = lazypencil::cli::Arguments::command()
            .try_get_matches_from(std::iter::once("lazypencil").chain(words))
            .unwrap();
        let parsed = lazypencil::cli::Arguments::from_arg_matches(&matches).unwrap();
        let lazypencil::cli::Command::Tui(args) = parsed.command else {
            panic!("Expected TUI")
        };
        assert_eq!(args.prompt.file, std::path::PathBuf::from(expected_file));
        assert_eq!(args.prompt.prompt, expected_prompt);
    }
}
