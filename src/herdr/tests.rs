use super::*;
use crate::fixture_support;
use std::sync::mpsc;
use tempfile::TempDir;

#[test]
fn herdr_requires_explicit_environment_and_complete_absolute_target() {
    let binary = std::env::current_exe().unwrap().into_os_string();
    for environment in [None, Some(""), Some("0"), Some("true"), Some("1 ")] {
        assert!(
            Target::from_environment(|key| {
                assert_eq!(key, "HERDR_ENV", "Do not inspect Herdr paths outside Herdr");
                environment.map(OsString::from)
            })
            .is_none()
        );
    }
    for (pane, path) in [
        (None, Some(binary.clone())),
        (Some(""), Some(binary.clone())),
        (Some("w1:p1"), None),
        (Some("w1:p1"), Some(OsString::new())),
        (Some("w1:p1"), Some("herdr".into())),
    ] {
        assert!(
            Target::from_environment(|key| match key {
                "HERDR_ENV" => Some("1".into()),
                "HERDR_PANE_ID" => pane.map(OsString::from),
                "HERDR_BIN_PATH" => path.clone(),
                _ => unreachable!(),
            })
            .is_none()
        );
    }
    let target = Target::from_environment(|key| match key {
        "HERDR_ENV" => Some("1".into()),
        "HERDR_PANE_ID" => Some("w1:p1".into()),
        "HERDR_BIN_PATH" => Some(binary.clone()),
        _ => unreachable!(),
    })
    .unwrap();
    assert_eq!(target.binary.as_os_str(), binary);
    assert_eq!(target.pane, "w1:p1");

    let mut reporter = Reporter::start(State::Idle, None);
    reporter.update(State::Working);
    reporter.update(State::Blocked);
    assert!(reporter.worker.is_none());
    assert!(reporter.shared.0.lock().unwrap().pending.is_none());
}

#[test]
fn herdr_reports_only_semantic_state_with_its_own_identity_and_release() {
    let target = Target {
        binary: "unused".into(),
        pane: "w1:p1".into(),
    };
    let prefix = [
        "pane",
        "report-agent",
        "w1:p1",
        "--source",
        "lazypencil",
        "--agent",
        "lazypencil",
        "--state",
    ];
    for (state, label) in [
        (State::Idle, "idle"),
        (State::Working, "working"),
        (State::Blocked, "blocked"),
    ] {
        let mut expected = prefix.map(OsString::from).to_vec();
        expected.push(label.into());
        if state == State::Blocked {
            expected.extend(["--message", "Waiting for permission"].map(OsString::from));
        }
        expected.extend(["--seq", "42"].map(OsString::from));
        assert_eq!(target.arguments(Some(state), 42), expected);
    }
    assert_eq!(
        target.arguments(None, 43),
        [
            "pane",
            "release-agent",
            "w1:p1",
            "--source",
            "lazypencil",
            "--agent",
            "lazypencil",
            "--seq",
            "43",
        ]
        .map(OsString::from)
    );
}

#[test]
fn herdr_slow_reports_coalesce_without_blocking_and_release_after_the_last_call() {
    let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
    let (sent, received) = mpsc::channel();
    let (resume, stalled) = mpsc::channel();
    let worker_shared = shared.clone();
    let worker = thread::spawn(move || {
        let mut first = true;
        run(worker_shared, |state, seq| {
            sent.send((state, seq)).unwrap();
            if first {
                first = false;
                stalled.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });
    });
    let mut reporter = Reporter {
        shared,
        worker: Some(worker),
        last: State::Idle,
    };
    reporter.update(State::Working);
    let first = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(first.0, Some(State::Working));
    for state in [State::Blocked, State::Idle, State::Working, State::Blocked] {
        reporter.update(state);
    }
    assert_eq!(
        reporter.shared.0.lock().unwrap().pending,
        Some(State::Blocked),
        "Only the latest state is queued while the sender is stalled"
    );
    resume.send(()).unwrap();
    let blocked = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(blocked.0, Some(State::Blocked));
    reporter.update(State::Blocked);
    assert!(reporter.shared.0.lock().unwrap().pending.is_none());
    reporter.update(State::Idle);
    let idle = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(idle.0, Some(State::Idle));
    drop(reporter);
    let release = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(release.0, None);
    assert!(first.1 < blocked.1 && blocked.1 < idle.1 && idle.1 < release.1);
    assert!(received.recv().is_err());
}

#[test]
fn herdr_missing_failed_and_hung_commands_are_bounded_and_reaped() {
    let temp = TempDir::new().unwrap();
    let mut target = Target {
        binary: temp.path().join("missing"),
        pane: "w1:p1".into(),
    };
    target.report(Some(State::Working), 1);
    for (name, record) in [
        (
            "failed",
            serde_json::json!({"status":1,"stdout":"noise","stderr":"noise"}),
        ),
        (
            "hung",
            serde_json::json!({"sleep_ms":60_000,"pid_file":temp.path().join("pid")}),
        ),
    ] {
        target.binary = temp
            .path()
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        fixture_support::process(&target.binary, record);
        let start = Instant::now();
        target.report(Some(State::Working), 2);
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    #[cfg(unix)]
    {
        let pid: i32 = std::fs::read_to_string(temp.path().join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "Timed-out child was reaped"
        );
    }
}
