//! Exercise only fixture-owned clipboards on disposable hosted runners.
use super::*;

pub fn run(session: &Session) -> Result<()> {
    ensure!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && matches!(std::env::consts::OS, "linux" | "macos"),
        "Native clipboard verification requires an isolated initial-platform GHA runner"
    );
    if cfg!(target_os = "linux")
        && std::env::var("LAZYPENCIL_TEST_CLIPBOARD_DISPLAY").as_deref() != Ok("isolated-xvfb")
    {
        let executable = std::env::current_exe()?;
        session.capture(
            Command::new("xvfb-run")
                .args(["-a", "-s", "-screen 0 1280x800x24 -nolisten tcp"])
                .arg(executable)
                .args(["ci", "native", "clipboard"])
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("WAYLAND_SOCKET")
                .env("XDG_SESSION_TYPE", "x11")
                .env("LAZYPENCIL_TEST_CLIPBOARD_DISPLAY", "isolated-xvfb"),
            300,
            "native-clipboard-display.log",
        )?;
        return Ok(());
    }
    if cfg!(target_os = "linux") {
        ensure!(
            std::env::var_os("DISPLAY").is_some(),
            "Missing isolated clipboard display"
        );
    }
    let output = session.temporary.join("clipboard-evidence");
    let mut command = session.command(
        &[
            "cargo",
            "test",
            "--locked",
            "--test",
            "tui",
            commands::NATIVE_CLIPBOARD_TEST,
            "--",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]
        .map(str::to_owned),
    );
    command
        .env("LAZYPENCIL_TEST_NATIVE_CLIPBOARD", "owned-runner-clipboard")
        .env("LAZYPENCIL_TEST_TUI_ARTIFACTS", &output);
    session.capture(&mut command, 240, "native-clipboard.log")?;
    let report: Value = util::json(&output.join("native-clipboard/report.json"))?;
    ensure!(
        report["source_commit"] == std::env::var("GITHUB_SHA")?
            && report["images"] == 2
            && report["pixels_compared"] == 3072
            && report["draft_preserved_until_send"] == true
            && report["document_unchanged"] == true
            && report["normal_exit_and_terminal_modes_restored"] == true,
        "Missing or mismatched native clipboard evidence"
    );
    util::print(&json!({"native_clipboard":"passed","report":report}))
}
