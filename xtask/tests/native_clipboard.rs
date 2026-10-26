use lazypencil_xtask::{
    ci::{commands, focus},
    util,
};
use std::{fs, path::Path, process::Command};

#[test]
fn isolated_clipboard_is_excluded_from_both_shared_ignored_tui_schedules() {
    let clipboard = commands::NATIVE_CLIPBOARD_TEST;
    let plan = commands::plan(&util::root(), "dynamic", &["tui".into()]).unwrap();
    let tui = plan
        .checks
        .iter()
        .find(|command| command.contains(&"--include-ignored".into()))
        .unwrap();
    let lifecycle = commands::lifecycle(Path::new("tui-test"), "tui").unwrap();
    for command in [tui, &lifecycle] {
        assert!(command.windows(2).any(|args| args == ["--skip", clipboard]));
        assert!(!command.iter().any(|arg| arg
            == "real_pty_dynamic_previews_survive_idle_cleanup_and_refresh_without_losing_drafts"));
    }
    let mcp = commands::lifecycle(Path::new("mcp-test"), "mcp").unwrap();
    assert!(mcp.contains(
        &"idle_mcp_reaps_script_page_and_shader_processes_without_another_request".into()
    ));
    assert!(!mcp.contains(&"--skip".into()));
    assert!(commands::lifecycle(Path::new("unexpected"), "agent").is_err());
}

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
    let changed = [
        "tests/tui.rs".into(),
        "xtask/src/performance/tui.rs".into(),
        "xtask/src/ci/native.rs".into(),
    ];
    let plan = focus::select(&util::root(), &changed, "auto").unwrap();
    assert_eq!(plan.suite, "candidate");
    assert_eq!(plan.targets, ["render", "tui"]);
    let mut mixed = changed.to_vec();
    mixed.push("src/runtime.rs".into());
    let broader = focus::select(&util::root(), &mixed, "auto").unwrap();
    assert_eq!(broader.suite, "dynamic");
    assert!(
        broader
            .targets
            .iter()
            .any(|target| target == "dynamic_render")
    );
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
