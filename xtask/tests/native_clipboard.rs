use lazypencil_xtask::{ci::focus, util};
use std::{fs, process::Command};

#[test]
fn native_clipboard_checks_require_owned_hosted_execution_and_run_before_expansion() {
    for path in [
        "src/clipboard.rs",
        "tests/tui.rs",
        "tests/support/terminal/mod.rs",
        "xtask/src/ci/native/clipboard.rs",
    ] {
        assert!(focus::native_clipboard_changed(&[path.into()]));
    }
    assert!(!focus::native_clipboard_changed(&[
        "docs/tui.md".into(),
        "src/layout.rs".into()
    ]));
    let plan = focus::select(
        &util::root(),
        &["xtask/src/ci/native/clipboard.rs".into()],
        "auto",
    )
    .unwrap();
    assert_eq!(plan.suite, "candidate");
    let result = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .env_remove("GITHUB_ACTIONS")
        .args(["ci", "native", "clipboard"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let workflow = fs::read_to_string(util::root().join(".github/workflows/native.yml")).unwrap();
    assert!(
        workflow.find("id: native-clipboard-focus").unwrap()
            < workflow.find("native focus-test").unwrap()
    );
    assert!(workflow.contains("if: inputs.phase == 'focus' && inputs.native_clipboard"));
    assert!(
        workflow
            .contains("Verify fixture-owned native clipboard images and actual Ctrl+V delivery")
    );
}
