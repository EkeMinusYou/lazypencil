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

fn python() -> PathBuf {
    std::env::var_os("LAZYPENCIL_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| "python".into())
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/acp_fixture.py")
}

fn options(directory: &Path, mode: &str) -> AgentOptions {
    let mut options = AgentOptions::new(python(), directory);
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
            "Owned adapter descendant {} survived",
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
            .arg(python())
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
        .arg(python())
        .args([
            "-c",
            "from pathlib import Path; Path('must-not-exist').write_text('unexpected')",
        ])
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
