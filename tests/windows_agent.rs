#![cfg(windows)]

use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
use lazypencil::agent::{AgentEvent, AgentHost, AgentOptions};
use serde_json::Value;
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        TerminateProcess, WaitForSingleObject,
    },
};

#[path = "support/fixture.rs"]
mod fixture_support;

fn helper() -> PathBuf {
    fixture_support::helper()
}

fn fixture() -> PathBuf {
    "acp".into()
}

fn options(directory: &Path, mode: &str) -> AgentOptions {
    let mut options = AgentOptions::new(helper(), directory);
    options.supervisor_executable = Some(env!("CARGO_BIN_EXE_lazypencil").into());
    options.args = vec![fixture().to_str().unwrap().into(), mode.into()];
    options.setup_timeout = Duration::from_secs(15);
    options.turn_timeout = Duration::from_secs(15);
    options.cancel_grace = Duration::from_millis(300);
    options
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn message(host: &mut AgentHost) -> Value {
    match tokio::time::timeout(Duration::from_secs(20), host.next_event())
        .await
        .unwrap()
        .unwrap()
    {
        AgentEvent::Message(value) => value,
        event => panic!("Unexpected agent event: {event:?}"),
    }
}

async fn ready(host: &mut AgentHost) {
    assert_eq!(message(host).await["type"], "initialized");
    assert_eq!(message(host).await["type"], "ready");
}

struct Process {
    handle: OwnedHandle,
    pid: u32,
}

impl Process {
    fn open(pid: u32) -> Self {
        // SAFETY: The fixture reported this live, owned process. Retaining the
        // kernel handle avoids PID reuse while checking exit and test cleanup.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                0,
                pid,
            )
        };
        assert!(
            !raw.is_null(),
            "Cannot open fixture process {pid}: {}",
            std::io::Error::last_os_error()
        );
        Self {
            handle: unsafe { OwnedHandle::from_raw_handle(raw) },
            pid,
        }
    }

    fn alive(&self) -> bool {
        let status = unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) };
        assert!(matches!(status, WAIT_TIMEOUT | WAIT_OBJECT_0));
        status == WAIT_TIMEOUT
    }

    fn assert_exited(&self) {
        assert_eq!(
            unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 3000) },
            WAIT_OBJECT_0,
            "Owned descendant {} survived",
            self.pid
        );
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // A failed assertion must not leave the intentionally long-lived fixture.
        if unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } == WAIT_TIMEOUT {
            unsafe {
                TerminateProcess(self.handle.as_raw_handle(), 1);
                WaitForSingleObject(self.handle.as_raw_handle(), 1000);
            }
        }
    }
}

async fn tree(directory: &Path) -> Vec<Process> {
    let path = directory.join("tree-ready.json");
    let value: Value = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(bytes) = std::fs::read(&path) {
                break serde_json::from_slice(&bytes).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Owned process tree did not become ready");
    let processes = ["adapter", "branch", "leaf"]
        .map(|name| Process::open(u32::try_from(value[name].as_u64().unwrap()).unwrap()));
    assert!(processes.iter().all(Process::alive));
    processes.into()
}

#[test]
fn windows_host_close_reaps_its_whole_tree_and_preserves_another_session() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut first_host = AgentHost::spawn(options(first.path(), "process-tree")).unwrap();
        let mut second_host = AgentHost::spawn(options(second.path(), "process-tree")).unwrap();
        ready(&mut first_host).await;
        ready(&mut second_host).await;
        let first_tree = tree(first.path()).await;
        let second_tree = tree(second.path()).await;
        first_host.close().await.unwrap();
        first_tree.iter().for_each(Process::assert_exited);
        assert!(
            second_tree.iter().all(Process::alive),
            "Closing a session terminated another session's process"
        );
        second_host.close().await.unwrap();
        second_tree.iter().for_each(Process::assert_exited);
    });
}

#[test]
fn windows_uncooperative_cancellation_reaps_adapter_and_detached_descendants() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options(directory.path(), "process-tree-hang")).unwrap();
        ready(&mut host).await;
        let processes = tree(directory.path()).await;
        let cancel = CancellationToken::new();
        host.prompt(
            vec![ContentBlock::Text(TextContent::new(
                "Wait for cancellation",
            ))],
            cancel.clone(),
        )
        .await
        .unwrap();
        assert_eq!(message(&mut host).await["type"], "update");
        cancel.cancel();
        let event = message(&mut host).await;
        assert_eq!(event["type"], "connection_error");
        assert_eq!(event["error"]["code"], "CANCELLED");
        assert_eq!(host.close().await.unwrap_err().code, "CANCELLED");
        processes.iter().for_each(Process::assert_exited);
    });
}

#[test]
fn windows_dropping_host_and_runtime_reaps_the_owned_process_tree() {
    let directory = TempDir::new().unwrap();
    let runtime = runtime();
    let (host, processes) = runtime.block_on(async {
        let mut host = AgentHost::spawn(options(directory.path(), "process-tree")).unwrap();
        ready(&mut host).await;
        let processes = tree(directory.path()).await;
        (host, processes)
    });
    drop(host);
    drop(runtime);
    processes.iter().for_each(Process::assert_exited);
}

struct Parent(Child);
impl Drop for Parent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn windows_force_killing_the_cli_reaps_the_live_adapter_tree() {
    let directory = TempDir::new().unwrap();
    let mut parent = Parent(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .current_dir(directory.path())
            .arg("--data-dir")
            .arg(directory.path().join("data"))
            .args(["--json", "prompt"])
            .arg(directory.path().join("design.pen"))
            .args([
                "--create",
                "--non-interactive",
                "--timeout",
                "60",
                "--adapter",
            ])
            .arg(helper())
            .arg(format!("--adapter-arg={}", fixture().display()))
            .arg("--adapter-arg=process-tree-hang")
            .arg("Wait until the test terminates this host")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let processes = runtime().block_on(tree(directory.path()));
    parent.0.kill().unwrap();
    parent.0.wait().unwrap();
    processes.iter().for_each(Process::assert_exited);
}

#[test]
fn windows_worker_without_a_live_owner_never_launches_the_adapter() {
    let directory = TempDir::new().unwrap();
    let marker = directory.path().join("must-not-exist");
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(directory.path())
        .arg("__agent_process_worker")
        .arg(format!("Local\\lazypencil-agent-{}", uuid::Uuid::new_v4()))
        .arg("--")
        .arg(helper())
        .args(["mark", "must-not-exist"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!marker.exists());
}

#[test]
fn windows_missing_adapter_keeps_the_public_unavailable_error() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut options = options(directory.path(), "process-tree");
        options.command = directory.path().join("missing-adapter.exe");
        let mut host = AgentHost::spawn(options).unwrap();
        let event = message(&mut host).await;
        assert_eq!(event["type"], "connection_error");
        assert_eq!(event["error"]["code"], "AGENT_UNAVAILABLE");
        assert_eq!(
            event["error"]["details"]["supervisor_phase"],
            "adapter_spawn"
        );
        assert!(event["error"]["details"]["os_error"].is_number());
        assert_eq!(host.close().await.unwrap_err().code, "AGENT_UNAVAILABLE");
        assert!(!directory.path().join("tree-ready.json").exists());
    });
}

fn browser_fixture(directory: &Path) -> lazypencil::runtime::InstalledBrowser {
    let executable = directory.join("browser with spaces 日本語.exe");
    fixture_support::install(
        &executable,
        &["browser-tree".into(), directory.to_str().unwrap().into()],
    );
    lazypencil::runtime::InstalledBrowser {
        executable,
        version: "fixture".into(),
        platform: "fixture".into(),
    }
}

fn launch_browser_fixture(
    directory: &Path,
    budget: &lazypencil::cdp::Budget<'_>,
) -> lazypencil::Result<lazypencil::cdp::Browser> {
    lazypencil::cdp::Browser::launch_with_supervisor(
        &browser_fixture(directory),
        lazypencil::cdp::Purpose::Page,
        budget,
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
}

#[test]
fn windows_browser_startup_cancellation_and_deadline_reap_detached_descendants() {
    for cancel in [true, false] {
        let directory = TempDir::new().unwrap();
        let token = CancellationToken::new();
        std::thread::scope(|scope| {
            let startup = scope.spawn(|| {
                let budget = lazypencil::cdp::Budget::new(Duration::from_secs(15), Some(&token));
                launch_browser_fixture(directory.path(), &budget)
                    .err()
                    .expect("The fixture never opens CDP")
            });
            let processes = runtime().block_on(tree(directory.path()));
            if cancel {
                token.cancel();
            }
            let error = startup.join().unwrap();
            assert_eq!(
                error.code,
                if cancel {
                    "CANCELLED"
                } else {
                    "RENDER_TIMEOUT"
                }
            );
            processes.iter().for_each(Process::assert_exited);
        });
    }
}

#[test]
fn windows_browser_fixture_host() {
    // A fresh test process is the real owner in the forced-parent-death case.
    // The child CLI used for supervision is always the regular binary.
    let Some(directory) = std::env::var_os("LAZYPENCIL_TEST_BROWSER_TREE_HOST") else {
        return;
    };
    if std::env::var_os("LAZYPENCIL_TEST_BROWSER_REAL").is_some() {
        let mut browser = real_browser();
        let pids = browser_pids(&mut browser);
        let temporary = Path::new(&directory).join("real-pids.json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(&pids).unwrap()).unwrap();
        std::fs::rename(temporary, Path::new(&directory).join("real-pids.json")).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        panic!("The parent test must terminate this real-browser owner");
    }
    let budget = lazypencil::cdp::Budget::new(Duration::from_secs(60), None);
    let result = launch_browser_fixture(Path::new(&directory), &budget);
    panic!(
        "The parent test must terminate this host during startup: {:?}",
        result.err()
    );
}

#[test]
fn windows_killing_a_browser_owner_reaps_its_tree_and_preserves_another_owner() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let spawn = |directory: &Path| {
        Parent(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "windows_browser_fixture_host", "--nocapture"])
                .env("LAZYPENCIL_TEST_BROWSER_TREE_HOST", directory)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut first_host = spawn(first.path());
    let mut second_host = spawn(second.path());
    let first_tree = runtime().block_on(tree(first.path()));
    let second_tree = runtime().block_on(tree(second.path()));
    first_host.0.kill().unwrap();
    first_host.0.wait().unwrap();
    first_tree.iter().for_each(Process::assert_exited);
    assert!(second_tree.iter().all(Process::alive));
    second_host.0.kill().unwrap();
    second_host.0.wait().unwrap();
    second_tree.iter().for_each(Process::assert_exited);
}

#[test]
fn windows_browser_worker_without_an_owner_never_launches_the_browser() {
    let directory = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazypencil"))
        .current_dir(directory.path())
        .arg("__browser_process_worker")
        .arg(format!(
            "Local\\lazypencil-browser-{}",
            uuid::Uuid::new_v4()
        ))
        .arg(directory.path().join("status.json"))
        .arg("--")
        .arg(helper())
        .args(["mark", "must-not-exist"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!directory.path().join("must-not-exist").exists());
    let status: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("status.json")).unwrap())
            .unwrap();
    assert_eq!(status["phase"], "job_join");
}

#[test]
fn windows_missing_browser_preserves_a_bounded_structured_startup_error() {
    let directory = TempDir::new().unwrap();
    let installed = lazypencil::runtime::InstalledBrowser {
        executable: directory.path().join("missing-browser.exe"),
        version: "fixture".into(),
        platform: "fixture".into(),
    };
    let error = lazypencil::cdp::Browser::launch_with_supervisor(
        &installed,
        lazypencil::cdp::Purpose::Page,
        &lazypencil::cdp::Budget::new(Duration::from_secs(5), None),
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .err()
    .expect("Missing executable must fail before the deadline");
    assert_eq!(error.code, "RUNTIME_UNAVAILABLE");
    let details = error.details.unwrap();
    assert_eq!(details["supervisor_phase"], "browser_spawn");
    assert!(details["os_error"].is_number());
}

fn real_browser() -> lazypencil::cdp::Browser {
    let installed = lazypencil::runtime::InstalledBrowser {
        executable: std::env::var_os("LAZYPENCIL_TEST_CHROMIUM")
            .expect("Set LAZYPENCIL_TEST_CHROMIUM")
            .into(),
        version: "external-unverified".into(),
        platform: "win64".into(),
    };
    lazypencil::cdp::Browser::launch_with_supervisor(
        &installed,
        lazypencil::cdp::Purpose::Page,
        &lazypencil::cdp::Budget::new(Duration::from_secs(30), None),
        Path::new(env!("CARGO_BIN_EXE_lazypencil")),
    )
    .unwrap()
}

fn browser_pids(browser: &mut lazypencil::cdp::Browser) -> Vec<u32> {
    let budget = lazypencil::cdp::Budget::new(Duration::from_secs(15), None);
    browser
        .command(
            None,
            "Target.createTarget",
            serde_json::json!({"url":"data:text/html,<p>owned browser renderer</p>"}),
            &budget,
        )
        .unwrap();
    let info = browser
        .command(
            None,
            "SystemInfo.getProcessInfo",
            serde_json::json!({}),
            &budget,
        )
        .unwrap();
    let pids: Vec<_> = info["processInfo"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| u32::try_from(item["id"].as_u64().unwrap()).unwrap())
        .collect();
    assert!(
        pids.contains(&browser.process_id()),
        "Diagnostics must name Chromium rather than its worker"
    );
    assert!(
        pids.len() >= 2,
        "The fixture must observe real browser descendants"
    );
    pids
}

#[test]
#[ignore = "Requires verified sandboxed Chromium on Windows"]
fn windows_real_browser_drop_and_owner_death_reap_descendants_and_isolate_owners() {
    let mut browser = real_browser();
    let processes: Vec<_> = browser_pids(&mut browser)
        .into_iter()
        .map(Process::open)
        .collect();
    drop(browser);
    processes.iter().for_each(Process::assert_exited);

    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let spawn = |directory: &Path| {
        let log = std::fs::File::create(directory.join("browser-owner.log")).unwrap();
        Parent(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "windows_browser_fixture_host", "--nocapture"])
                .env("LAZYPENCIL_TEST_BROWSER_TREE_HOST", directory)
                .env("LAZYPENCIL_TEST_BROWSER_REAL", "1")
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        )
    };
    let mut first_host = spawn(first.path());
    let mut second_host = spawn(second.path());
    let ready = |directory: &Path, owner: &mut Parent| -> Vec<Process> {
        let deadline = std::time::Instant::now() + Duration::from_secs(45);
        loop {
            if let Ok(bytes) = std::fs::read(directory.join("real-pids.json")) {
                let pids: Vec<u32> = serde_json::from_slice(&bytes).unwrap();
                return pids.into_iter().map(Process::open).collect();
            }
            if let Some(status) = owner.0.try_wait().unwrap() {
                panic!(
                    "Real browser owner exited before readiness ({status}): {}",
                    std::fs::read_to_string(directory.join("browser-owner.log")).unwrap()
                );
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Real browser owner did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let first_tree = ready(first.path(), &mut first_host);
    let second_tree = ready(second.path(), &mut second_host);
    first_host.0.kill().unwrap();
    first_host.0.wait().unwrap();
    first_tree.iter().for_each(Process::assert_exited);
    assert!(second_tree.iter().all(Process::alive));
    second_host.0.kill().unwrap();
    second_host.0.wait().unwrap();
    second_tree.iter().for_each(Process::assert_exited);
}
