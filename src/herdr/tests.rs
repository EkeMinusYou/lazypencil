use super::*;

#[test]
fn herdr_core_requires_its_terminal_environment() {
    for (value, expected) in [(None, false), (Some(""), false), (Some("t7"), true)] {
        assert_eq!(
            core_environment(|key| {
                assert_eq!(key, "HC_TERMINAL_ID");
                value.map(OsString::from)
            }),
            expected
        );
    }
}

#[test]
fn herdr_core_reports_start_activity_permission_completion_and_exit() {
    let terminal = tempfile::tempfile().unwrap();
    let mut reporter = Reporter::start(State::Idle, Some(terminal.try_clone().unwrap()));
    for state in [
        State::Idle,
        State::Working,
        State::Blocked,
        State::Blocked,
        State::Working,
        State::Idle,
        State::Idle,
    ] {
        reporter.update(state);
    }
    drop(reporter);
    use std::io::{Read, Seek};
    let mut terminal = terminal;
    terminal.rewind().unwrap();
    let mut output = String::new();
    terminal.read_to_string(&mut output).unwrap();
    let expected = [
        "idle:app=lazypencil",
        "working:app=lazypencil",
        "blocked:app=lazypencil:kind=permission",
        "working:app=lazypencil",
        "done:app=lazypencil",
        "clear:app=lazypencil",
    ]
    .map(|fields| format!("\x1b]7501;state={fields}\x1b\\"))
    .concat();
    assert_eq!(output, expected);
    assert!(report_terminal(&mut [][..], Some(State::Working), false).is_err());
}

#[test]
fn herdr_core_ignores_missing_terminals_and_failed_writes() {
    let mut disabled = Reporter::start(State::Idle, None);
    disabled.update(State::Working);
    disabled.update(State::Blocked);
    disabled.update(State::Idle);
    drop(disabled);

    let file = tempfile::NamedTempFile::new().unwrap();
    let read_only = File::open(file.path()).unwrap();
    let mut failed = Reporter::start(State::Idle, Some(read_only));
    failed.update(State::Working);
    failed.update(State::Blocked);
    failed.update(State::Idle);
    drop(failed);
    assert!(std::fs::read(file.path()).unwrap().is_empty());
}
