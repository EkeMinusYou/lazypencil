use super::{message, options, prompt, python, ready, runtime};
use lazypencil::agent::AgentHost;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn alive(pid: u32) -> bool {
    // Liveness queries only. Tests never signal a stale, fixture-reported PID.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn tree(directory: &Path) -> Vec<u32> {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(bytes) = fs::read(directory.join("tree-ready.json")) {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            let pids = ["adapter", "branch", "leaf"]
                .map(|key| u32::try_from(value[key].as_u64().unwrap()).unwrap())
                .to_vec();
            assert!(pids.iter().copied().all(alive));
            return pids;
        }
        assert!(Instant::now() < end, "Adapter tree did not become ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn gone(pids: &[u32]) {
    let end = Instant::now() + Duration::from_secs(3);
    while pids.iter().copied().any(alive) {
        assert!(
            Instant::now() < end,
            "Adapter processes survived: {:?}",
            pids.iter()
                .copied()
                .filter(|pid| alive(*pid))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn close_reaps_the_owned_tree_without_stopping_another_session() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut a = AgentHost::spawn(options("process-tree", first.path())).unwrap();
        let mut b = AgentHost::spawn(options("process-tree", second.path())).unwrap();
        ready(&mut a).await;
        ready(&mut b).await;
        let a_pids = tree(first.path());
        let b_pids = tree(second.path());
        a.close().await.unwrap();
        gone(&a_pids);
        assert!(b_pids.iter().copied().all(alive));
        b.close().await.unwrap();
        gone(&b_pids);
    });
}

#[test]
fn dropping_host_and_runtime_reaps_the_adapter_tree() {
    let directory = TempDir::new().unwrap();
    let rt = runtime();
    let (host, pids) = rt.block_on(async {
        let mut host = AgentHost::spawn(options("process-tree", directory.path())).unwrap();
        ready(&mut host).await;
        let pids = tree(directory.path());
        (host, pids)
    });
    drop(host);
    drop(rt);
    gone(&pids);
}

#[test]
fn uncooperative_cancellation_reaps_the_adapter_tree() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host = AgentHost::spawn(options("process-tree-hang", directory.path())).unwrap();
        ready(&mut host).await;
        let pids = tree(directory.path());
        let cancel = CancellationToken::new();
        host.prompt(prompt(), cancel.clone()).await.unwrap();
        assert_eq!(message(&mut host).await["type"], "update");
        cancel.cancel();
        let event = message(&mut host).await;
        assert_eq!(event["type"], "connection_error");
        assert_eq!(event["error"]["code"], "CANCELLED");
        assert_eq!(host.close().await.unwrap_err().code, "CANCELLED");
        gone(&pids);
    });
}

struct Parent(Child);

impl Drop for Parent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn cli_owner(directory: &Path, pressure: bool) -> Parent {
    Parent(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .current_dir(directory)
            .arg("--data-dir")
            .arg(directory.join("data"))
            .args(["--json", "prompt"])
            .arg(directory.join("design.pen"))
            .args([
                "--create",
                "--non-interactive",
                "--timeout",
                "60",
                "--adapter",
            ])
            .arg(python())
            .arg(format!("--adapter-arg={}", super::fixture().display()))
            .arg(if pressure {
                "--adapter-arg=process-tree-pressure"
            } else {
                "--adapter-arg=process-tree-hang"
            })
            .arg("Wait until this host is terminated")
            .stdin(Stdio::null())
            .stdout(if pressure {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn killed_cli_owners_reap_their_tree_and_preserve_another_owner() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let mut a = cli_owner(first.path(), false);
    let mut b = cli_owner(second.path(), false);
    let a_pids = tree(first.path());
    let b_pids = tree(second.path());
    a.0.kill().unwrap();
    a.0.wait().unwrap();
    gone(&a_pids);
    assert!(b_pids.iter().copied().all(alive));
    b.0.kill().unwrap();
    b.0.wait().unwrap();
    gone(&b_pids);
    for directory in [first.path(), second.path()] {
        let document: Value =
            serde_json::from_slice(&fs::read(directory.join("design.pen")).unwrap()).unwrap();
        assert!(document["children"].as_array().unwrap().is_empty());
    }
}

#[test]
fn killed_cli_reaps_the_adapter_tree_with_an_undrained_event_pipe() {
    use std::os::fd::AsRawFd;
    let directory = TempDir::new().unwrap();
    let mut owner = cli_owner(directory.path(), true);
    let pids = tree(directory.path());
    let fd = owner.0.stdout.as_ref().unwrap().as_raw_fd();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let mut available: libc::c_int = 0;
        // SAFETY: FIONREAD fills one integer for this live, borrowed pipe FD.
        assert_eq!(
            unsafe { libc::ioctl(fd, libc::FIONREAD, &mut available) },
            0
        );
        if available >= 4096 {
            break;
        }
        assert!(
            Instant::now() < end,
            "The event pipe never received the flood"
        );
        assert!(owner.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    gone(&pids);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_subreaper_cleans_detached_adapter_children_and_grandchildren() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut host =
            AgentHost::spawn(options("process-tree-detached", directory.path())).unwrap();
        ready(&mut host).await;
        let pids = tree(directory.path());
        host.close().await.unwrap();
        gone(&pids);
    });
}

#[test]
fn worker_without_a_live_owner_never_starts_the_adapter() {
    let directory = TempDir::new().unwrap();
    let mut worker = Parent(
        Command::new(env!("CARGO_BIN_EXE_lazypencil"))
            .current_dir(directory.path())
            .args(["__agent_process_worker", "2147483647", "--"])
            .arg(python())
            .args([
                "-c",
                "from pathlib import Path; Path('must-not-exist').touch()",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(
            Instant::now() < end,
            "Worker without an owner failed to exit"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!directory.path().join("must-not-exist").exists());
}

#[test]
fn missing_adapter_preserves_the_public_unavailable_error() {
    let directory = TempDir::new().unwrap();
    runtime().block_on(async {
        let mut opts = options("unused", directory.path());
        opts.command = directory.path().join("missing-adapter");
        let mut host = AgentHost::spawn(opts).unwrap();
        let event = message(&mut host).await;
        assert_eq!(event["type"], "connection_error");
        assert_eq!(event["error"]["code"], "AGENT_UNAVAILABLE");
        assert_eq!(
            event["error"]["details"]["supervisor_phase"],
            "adapter_spawn"
        );
        assert_eq!(host.close().await.unwrap_err().code, "AGENT_UNAVAILABLE");
    });
}
